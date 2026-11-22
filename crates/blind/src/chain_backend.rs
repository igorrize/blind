use std::{
    io,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use tokio::runtime::Runtime;

use crate::chain_command;
use crate::chain_log::RunEvent;
use crate::chain_run::{self, ChainEvent, ChainOutcome, Link};
use crate::runtime;
use crate::step::Progress;

const REMINDED: &str = "no `complete` call, reminding the agent";

pub type Linker = Box<dyn Fn(&[String]) -> anyhow::Result<Vec<Link>> + Send>;

pub struct ChainBackend {
    requests: Sender<Run>,
    events: Receiver<RunEvent>,
}

struct Run {
    agents: Vec<String>,
    task: String,
}

struct Worker {
    runtime: Runtime,
    linker: Linker,
    events: Sender<RunEvent>,
}

impl ChainBackend {
    pub fn start(linker: Linker) -> io::Result<Self> {
        let (requests, pending) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let worker = Worker {
            runtime: runtime::single_threaded()?,
            linker,
            events,
        };
        thread::spawn(move || worker.serve(&pending));
        Ok(Self {
            requests,
            events: received,
        })
    }

    pub fn run(&self, agents: &[String], task: &str) -> bool {
        let run = Run {
            agents: agents.to_vec(),
            task: task.to_owned(),
        };
        self.requests.send(run).is_ok()
    }

    pub fn poll(&self) -> Option<RunEvent> {
        self.events.try_recv().ok()
    }
}

impl Worker {
    fn serve(self, pending: &Receiver<Run>) {
        for run in pending {
            let ending = self.perform(&run);
            self.emit(ending);
        }
    }

    fn perform(&self, run: &Run) -> RunEvent {
        let links = match (self.linker)(&run.agents) {
            Ok(links) => links,
            Err(error) => return RunEvent::Failed(format!("{error:#}")),
        };
        let mut forward = |event: ChainEvent<'_>| self.emit(told(event));
        let chain = chain_run::run_chain(&links, &run.task, &mut forward);
        match self.runtime.block_on(chain) {
            Ok(outcome) => ending(outcome),
            Err(error) => RunEvent::Failed(error.to_string()),
        }
    }

    fn emit(&self, event: RunEvent) {
        self.events.send(event).ok();
    }
}

fn told(event: ChainEvent<'_>) -> RunEvent {
    match event {
        ChainEvent::StepStarted(agent) => RunEvent::StepStarted(agent.to_owned()),
        ChainEvent::Step(progress) => progress_told(progress),
        ChainEvent::StepResult(output) => RunEvent::StepResult(format!("{output:#}")),
        ChainEvent::StepFinished(record) => {
            RunEvent::StepFinished(chain_command::step_line(record))
        }
    }
}

fn progress_told(progress: Progress<'_>) -> RunEvent {
    match progress {
        Progress::Text(text) => RunEvent::Text(text.to_owned()),
        Progress::ToolCalled {
            name,
            arguments,
            refusal,
        } => RunEvent::ToolCalled {
            call: format!("{name} {arguments}"),
            refusal: refusal.map(str::to_owned),
        },
        Progress::Rejected(reason) => RunEvent::Note(reason.to_owned()),
        Progress::Reminded => RunEvent::Note(REMINDED.to_owned()),
        Progress::ToldToFinish(reason) => RunEvent::Note(format!(
            "{reason}: tools are taken away, the agent must finish now"
        )),
    }
}

fn ending(outcome: ChainOutcome) -> RunEvent {
    match outcome {
        ChainOutcome::Completed { output, steps } => RunEvent::Completed {
            output: format!("{output:#}"),
            total: chain_command::total_line(&steps),
        },
        ChainOutcome::Incompatible(error) => RunEvent::Stopped(format!(
            "chain is not compatible, no model was called: {error}"
        )),
        ChainOutcome::Stopped { at, reason, .. } => RunEvent::Stopped(format!(
            "stopped at `{at}`: {}",
            chain_command::explained(&reason)
        )),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use anyhow::anyhow;

    use super::ChainBackend;

    pub fn unable_to_run() -> ChainBackend {
        ChainBackend::start(Box::new(|_| Err(anyhow!("no agents here")))).expect("backend starts")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    use super::*;
    use crate::crew::test_support::{FILES, FINDINGS, NO_FIELDS, agent};
    use crate::providers::OpenAiCompatible;
    use crate::providers::openai::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};

    const PATIENCE: Duration = Duration::from_secs(5);
    const FOUND: &str = r#"{"files":["src/main.rs"]}"#;
    const REVIEWED: &str = r#"{"findings":["looks fine"]}"#;

    fn completing(arguments: &str) -> ResponseTemplate {
        sse(&[
            tool_call_chunk("call_1", "complete", arguments),
            usage_chunk(10, 5),
        ])
    }

    async fn serving_in_order(responses: Vec<ResponseTemplate>) -> MockServer {
        let server = MockServer::start().await;
        for response in responses {
            Mock::given(method("POST"))
                .respond_with(response)
                .up_to_n_times(1)
                .mount(&server)
                .await;
        }
        server
    }

    fn explorer_then_reviewer(server: &MockServer) -> ChainBackend {
        let uri = server.uri();
        let linker = move |names: &[String]| -> anyhow::Result<Vec<Link>> {
            let links = names.iter().map(|name| {
                let (input, output) = if name == "explorer" {
                    (NO_FIELDS, FILES)
                } else {
                    (FILES, FINDINGS)
                };
                Link {
                    agent: agent(name, input, output, "{}"),
                    client: OpenAiCompatible::new(&uri, None),
                    workspace: None,
                }
            });
            Ok(links.collect())
        };
        ChainBackend::start(Box::new(linker)).expect("backend starts")
    }

    fn events_until_the_end(backend: &ChainBackend) -> Vec<RunEvent> {
        let mut events = Vec::new();
        loop {
            let event = backend
                .events
                .recv_timeout(PATIENCE)
                .expect("an event in time");
            let ended = event.verdict().is_some();
            events.push(event);
            if ended {
                return events;
            }
        }
    }

    fn names(agents: &[&str]) -> Vec<String> {
        agents.iter().map(|name| (*name).to_owned()).collect()
    }

    #[tokio::test]
    async fn chain_runs_step_by_step_and_reports_every_stage() {
        let server = serving_in_order(vec![completing(FOUND), completing(REVIEWED)]).await;
        let backend = explorer_then_reviewer(&server);
        assert!(backend.run(&names(&["explorer", "reviewer"]), "check it"));
        let events = events_until_the_end(&backend);
        let stages: Vec<&str> = events
            .iter()
            .map(|event| match event {
                RunEvent::StepStarted(_) => "started",
                RunEvent::StepFinished(_) => "finished",
                RunEvent::StepResult(_) => "result",
                RunEvent::Completed { .. } => "completed",
                RunEvent::Text(_)
                | RunEvent::ToolCalled { .. }
                | RunEvent::Note(_)
                | RunEvent::Stopped(_)
                | RunEvent::Failed(_) => "other",
            })
            .collect();
        assert_eq!(
            stages,
            [
                "started",
                "finished",
                "result",
                "started",
                "finished",
                "result",
                "completed"
            ]
        );
        assert_eq!(events[0], RunEvent::StepStarted("explorer".to_owned()));
        let Some(RunEvent::Completed { output, total }) = events.last() else {
            panic!("chain must complete");
        };
        assert!(output.contains("looks fine"), "{output}");
        assert_eq!(total, "chain · steps: 2 · tokens: 20 in, 10 out");
    }

    #[tokio::test]
    async fn incompatible_chain_is_stopped_before_any_request() {
        let server = serving_in_order(vec![]).await;
        let backend = explorer_then_reviewer(&server);
        backend.run(&names(&["reviewer"]), "check it");
        let events = events_until_the_end(&backend);
        let [RunEvent::Stopped(why)] = events.as_slice() else {
            panic!("chain must be stopped at once: {events:?}");
        };
        assert!(
            why.starts_with("chain is not compatible, no model was called"),
            "{why}"
        );
        assert!(
            server
                .received_requests()
                .await
                .expect("recorded")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn step_that_never_finishes_stops_the_chain_with_the_reason() {
        let chatty = sse(&[text_chunk("I will just talk."), usage_chunk(10, 5)]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(chatty)
            .mount(&server)
            .await;
        let backend = explorer_then_reviewer(&server);
        backend.run(&names(&["explorer", "reviewer"]), "check it");
        let events = events_until_the_end(&backend);
        assert_eq!(
            events.last(),
            Some(&RunEvent::Stopped(
                "stopped at `explorer`: it gave no valid result".to_owned()
            ))
        );
        assert!(events.contains(&RunEvent::Text("I will just talk.".to_owned())));
        assert!(events.contains(&RunEvent::Note(REMINDED.to_owned())));
    }

    #[test]
    fn chain_that_cannot_be_prepared_fails_with_the_reason() {
        let backend = test_support::unable_to_run();
        backend.run(&names(&["explorer"]), "check it");
        assert_eq!(
            events_until_the_end(&backend),
            [RunEvent::Failed("no agents here".to_owned())]
        );
    }

    #[tokio::test]
    async fn backend_runs_one_chain_after_another() {
        let server = serving_in_order(vec![completing(FOUND), completing(FOUND)]).await;
        let backend = explorer_then_reviewer(&server);
        for _ in 0..2 {
            backend.run(&names(&["explorer"]), "check it");
            let events = events_until_the_end(&backend);
            assert!(matches!(events.last(), Some(RunEvent::Completed { .. })));
        }
    }
}
