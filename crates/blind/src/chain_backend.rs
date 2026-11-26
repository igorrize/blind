use std::{
    io,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use blind_core::{ObjectSchema, Violation};
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::chain_command;
use crate::chain_command::Prepared;
use crate::chain_log::{Kind, RunEvent};
use crate::chain_run::{self, ChainEvent, ChainOutcome};
use crate::lineup::Lineup;
use crate::runtime;
use crate::step::Progress;

pub type Linker = Box<dyn Fn(&Lineup) -> anyhow::Result<Prepared> + Send>;

pub struct ChainBackend {
    requests: Sender<Run>,
    events: Receiver<RunEvent>,
}

struct Run {
    lineup: Lineup,
    task: String,
}

struct Worker {
    runtime: Runtime,
    linker: Linker,
    root_input: ObjectSchema,
    events: Sender<RunEvent>,
}

impl ChainBackend {
    pub fn start(linker: Linker, root_input: ObjectSchema) -> io::Result<Self> {
        let (requests, pending) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let worker = Worker {
            runtime: runtime::single_threaded()?,
            linker,
            root_input,
            events,
        };
        thread::spawn(move || worker.serve(&pending));
        Ok(Self {
            requests,
            events: received,
        })
    }

    pub fn run(&self, lineup: &Lineup, task: &str) -> bool {
        let run = Run {
            lineup: lineup.clone(),
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
        let prepared = match (self.linker)(&run.lineup) {
            Ok(prepared) => prepared,
            Err(error) => return RunEvent::Failed(format!("{error:#}")),
        };
        let mut forward = |event: ChainEvent<'_>| self.emit(told(event));
        let chain = chain_run::run_chain(&prepared.steps, &run.task, &mut forward);
        let outcome = self.runtime.block_on(chain);
        if let Some(changes) = prepared.changes(&run.lineup, &run.task) {
            self.emit(RunEvent::Changes(changes));
        }
        match outcome {
            Ok(outcome) => ending(outcome, &self.root_input),
            Err(error) => RunEvent::Failed(error.to_string()),
        }
    }

    fn emit(&self, event: RunEvent) {
        self.events.send(event).ok();
    }
}

fn told(event: ChainEvent<'_>) -> RunEvent {
    match event {
        ChainEvent::StepStarted(agents) => {
            RunEvent::StepStarted(agents.iter().map(|agent| (*agent).to_owned()).collect())
        }
        ChainEvent::Step { agent, progress } => RunEvent::Step {
            agent: agent.to_owned(),
            kind: kind_of(&progress),
            text: progress.described(),
        },
        ChainEvent::StepResult(output) => RunEvent::StepResult(format!("{output:#}")),
        ChainEvent::StepFinished(record) => {
            RunEvent::StepFinished(chain_command::step_line(record))
        }
    }
}

fn kind_of(progress: &Progress<'_>) -> Kind {
    match progress {
        Progress::Text(_) => Kind::Said,
        Progress::ToolCalled { refusal: None, .. } => Kind::Called,
        Progress::ToolCalled {
            refusal: Some(_), ..
        } => Kind::Refused,
        Progress::Rejected(_) | Progress::Reminded | Progress::ToldToFinish(_) => Kind::Note,
    }
}

fn ending(outcome: ChainOutcome, root_input: &ObjectSchema) -> RunEvent {
    match outcome {
        ChainOutcome::Completed { output, steps } => {
            delivered(&output, root_input, chain_command::total_line(&steps))
        }
        ChainOutcome::Incompatible(error) => RunEvent::Stopped(format!(
            "chain is not compatible, no model was called: {error}"
        )),
        ChainOutcome::Stopped { at, reason, .. } => RunEvent::Stopped(format!(
            "stopped at `{at}`: {}",
            chain_command::explained(&reason)
        )),
    }
}

fn delivered(output: &Value, root_input: &ObjectSchema, total: String) -> RunEvent {
    let handed = root_input.project(output);
    let violations = root_input.violations(&handed);
    if violations.is_empty() {
        RunEvent::Completed {
            output: format!("{handed:#}"),
            total,
        }
    } else {
        let problems: Vec<String> = violations.iter().map(Violation::to_string).collect();
        RunEvent::Stopped(format!(
            "root's contract refused the result: {}",
            problems.join("; ")
        ))
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use anyhow::anyhow;

    use super::ChainBackend;

    pub fn unable_to_run() -> ChainBackend {
        let linker = Box::new(|_: &crate::lineup::Lineup| Err(anyhow!("no agents here")));
        ChainBackend::start(linker, blind_core::ObjectSchema::empty()).expect("backend starts")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    use super::*;
    use crate::chain_run::Link;
    use crate::crew::test_support::{FILES, FINDINGS, NO_FIELDS, agent, root};
    use crate::providers::OpenAiCompatible;
    use crate::providers::openai::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};
    use crate::step::REMINDED;

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
        let linker = move |lineup: &Lineup| -> anyhow::Result<Prepared> {
            let link = |name: &String| {
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
            };
            let steps = lineup.steps().iter();
            let steps = steps.map(|step| step.iter().map(link).collect()).collect();
            Ok(Prepared::in_place(steps))
        };
        ChainBackend::start(Box::new(linker), root_taking_files_and_findings())
            .expect("backend starts")
    }

    fn root_taking_files_and_findings() -> ObjectSchema {
        root(
            r#"{"type":"object","properties":{
            "files":{"type":"array","items":{"type":"string"}},
            "findings":{"type":"array","items":{"type":"string"}}}}"#,
        )
        .input()
        .clone()
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

    fn lineup(arguments: &[&str]) -> Lineup {
        let arguments: Vec<String> = arguments.iter().map(|text| (*text).to_owned()).collect();
        Lineup::parse(&arguments)
    }

    fn stage(event: &RunEvent) -> &'static str {
        match event {
            RunEvent::StepStarted(_) => "started",
            RunEvent::StepFinished(_) => "finished",
            RunEvent::StepResult(_) => "result",
            RunEvent::Completed { .. } => "completed",
            RunEvent::Step { .. }
            | RunEvent::Changes(_)
            | RunEvent::Stopped(_)
            | RunEvent::Failed(_) => "other",
        }
    }

    #[tokio::test]
    async fn chain_runs_step_by_step_and_reports_every_stage() {
        let server = serving_in_order(vec![completing(FOUND), completing(REVIEWED)]).await;
        let backend = explorer_then_reviewer(&server);
        assert!(backend.run(&lineup(&["explorer", "reviewer"]), "check it"));
        let events = events_until_the_end(&backend);
        let stages: Vec<&str> = events.iter().map(stage).collect();
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
        assert_eq!(
            events[0],
            RunEvent::StepStarted(vec!["explorer".to_owned()])
        );
        let Some(RunEvent::Completed { output, total }) = events.last() else {
            panic!("chain must complete");
        };
        assert!(output.contains("looks fine"), "{output}");
        assert_eq!(total, "chain · steps: 2 · tokens: 20 in, 10 out");
    }

    #[tokio::test]
    async fn step_of_two_agents_is_announced_with_both_names() {
        let server = serving_in_order(vec![
            completing(FOUND),
            completing(REVIEWED),
            completing(REVIEWED),
        ])
        .await;
        let backend = explorer_then_reviewer(&server);
        backend.run(&lineup(&["explorer", "reviewer+reviewer"]), "check it");
        let events = events_until_the_end(&backend);
        let both = RunEvent::StepStarted(vec!["reviewer".to_owned(), "reviewer".to_owned()]);
        assert!(events.contains(&both), "{events:?}");
        let Some(RunEvent::Completed { output, .. }) = events.last() else {
            panic!("chain must complete");
        };
        assert_eq!(output.matches("looks fine").count(), 2, "{output}");
    }

    #[tokio::test]
    async fn incompatible_chain_is_stopped_before_any_request() {
        let server = serving_in_order(vec![]).await;
        let backend = explorer_then_reviewer(&server);
        backend.run(&lineup(&["reviewer"]), "check it");
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
        backend.run(&lineup(&["explorer", "reviewer"]), "check it");
        let events = events_until_the_end(&backend);
        assert_eq!(
            events.last(),
            Some(&RunEvent::Stopped(
                "stopped at `explorer`: it gave no valid result".to_owned()
            ))
        );
        let by_explorer = |kind: Kind, text: &str| RunEvent::Step {
            agent: "explorer".to_owned(),
            kind,
            text: text.to_owned(),
        };
        assert!(events.contains(&by_explorer(Kind::Said, "I will just talk.")));
        assert!(events.contains(&by_explorer(Kind::Note, REMINDED)));
    }

    #[test]
    fn chain_that_cannot_be_prepared_fails_with_the_reason() {
        let backend = test_support::unable_to_run();
        backend.run(&lineup(&["explorer"]), "check it");
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
            backend.run(&lineup(&["explorer"]), "check it");
            let events = events_until_the_end(&backend);
            assert!(matches!(events.last(), Some(RunEvent::Completed { .. })));
        }
    }

    #[test]
    fn root_gets_only_the_fields_its_contract_declares() {
        let output = serde_json::json!({"findings": ["unused import"], "secret": "drop me"});
        let event = delivered(
            &output,
            &root_taking_files_and_findings(),
            "total".to_owned(),
        );
        let RunEvent::Completed { output, .. } = event else {
            panic!("result must be delivered");
        };
        assert!(output.contains("unused import"), "{output}");
        assert!(!output.contains("drop me"), "{output}");
    }

    #[test]
    fn result_of_the_wrong_type_is_refused_at_delivery() {
        let output = serde_json::json!({"findings": "one long text"});
        let event = delivered(
            &output,
            &root_taking_files_and_findings(),
            "total".to_owned(),
        );
        assert_eq!(
            event,
            RunEvent::Stopped(
                "root's contract refused the result: `findings` must be an array".to_owned()
            )
        );
    }

    #[test]
    fn root_without_a_contract_learns_only_that_the_chain_finished() {
        let output = serde_json::json!({"findings": ["unused import"]});
        let event = delivered(&output, &ObjectSchema::empty(), "total".to_owned());
        assert_eq!(
            event,
            RunEvent::Completed {
                output: "{}".to_owned(),
                total: "total".to_owned()
            }
        );
    }

    #[test]
    fn every_kind_of_progress_lands_in_its_own_kind_of_line() {
        let called = Progress::ToolCalled {
            name: "grep",
            arguments: "{}",
            refusal: None,
        };
        let refused = Progress::ToolCalled {
            name: "read_file",
            arguments: "{}",
            refusal: Some("not given"),
        };
        assert_eq!(kind_of(&Progress::Text("hi")), Kind::Said);
        assert_eq!(kind_of(&called), Kind::Called);
        assert_eq!(kind_of(&refused), Kind::Refused);
        assert_eq!(kind_of(&Progress::Reminded), Kind::Note);
    }
}
