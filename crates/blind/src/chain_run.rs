use std::cell::RefCell;

use blind_core::{Agent, ChainError, Contract, Violation, check_chain, merged_outputs};
use blind_exec::Workspace;
use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::providers::{OpenAiCompatible, ProviderError};
use crate::step::{self, Progress, StepOutcome, StepReport, Stop};

pub struct Link {
    pub agent: Agent,
    pub client: OpenAiCompatible,
    pub workspace: Option<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRecord {
    pub agent: String,
    pub report: StepReport,
}

#[derive(Debug, PartialEq)]
pub enum ChainOutcome {
    Completed {
        output: Value,
        steps: Vec<StepRecord>,
    },
    Incompatible(ChainError),
    Stopped {
        at: String,
        reason: StopReason,
        steps: Vec<StepRecord>,
    },
}

#[derive(Debug, PartialEq)]
pub enum StopReason {
    InputRejected(Vec<Violation>),
    StepFailed(Stop),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChainEvent<'a> {
    StepStarted(&'a [&'a str]),
    Step {
        agent: &'a str,
        progress: Progress<'a>,
    },
    StepResult(&'a Value),
    StepFinished(&'a StepRecord),
}

type Listener<'a, F> = RefCell<&'a mut F>;

pub async fn run_chain<F: FnMut(ChainEvent<'_>)>(
    steps: &[Vec<Link>],
    task: &str,
    on_event: &mut F,
) -> Result<ChainOutcome, ProviderError> {
    if let Err(error) = check_chain(&contracts_of(steps)) {
        return Ok(ChainOutcome::Incompatible(error));
    }
    let listener = RefCell::new(on_event);
    let mut records = Vec::new();
    let mut handed_over = json!({});
    for step in steps {
        let agents: Vec<&str> = step.iter().map(name_of).collect();
        tell(&listener, ChainEvent::StepStarted(&agents));
        let runs = step
            .iter()
            .map(|link| run_link(link, task, &handed_over, &listener));
        let outcomes = join_all(runs).await;
        let mut outputs = Vec::new();
        let mut stop = None;
        for (link, outcome) in step.iter().zip(outcomes) {
            let agent = name_of(link);
            match outcome? {
                StepOutcome::Completed { output, report } => {
                    records.push(finished(agent, report, &listener));
                    tell(&listener, ChainEvent::StepResult(&output));
                    outputs.push(output);
                }
                StepOutcome::Failed { report, stop: why } => {
                    records.push(finished(agent, report, &listener));
                    stop.get_or_insert((agent, StopReason::StepFailed(why)));
                }
                StepOutcome::InputRejected(violations) => {
                    stop.get_or_insert((agent, StopReason::InputRejected(violations)));
                }
            }
        }
        if let Some((at, reason)) = stop {
            return Ok(stopped(at, reason, records));
        }
        handed_over = merged_outputs(&outputs);
    }
    Ok(ChainOutcome::Completed {
        output: handed_over,
        steps: records,
    })
}

fn contracts_of(steps: &[Vec<Link>]) -> Vec<Vec<&Contract>> {
    steps
        .iter()
        .map(|step| step.iter().map(|link| link.agent.contract()).collect())
        .collect()
}

fn name_of(link: &Link) -> &str {
    link.agent.contract().name().as_str()
}

async fn run_link<F: FnMut(ChainEvent<'_>)>(
    link: &Link,
    task: &str,
    handed_over: &Value,
    listener: &Listener<'_, F>,
) -> Result<StepOutcome, ProviderError> {
    let agent = name_of(link);
    let input = link.agent.contract().input().project(handed_over);
    let mut show = |progress: Progress<'_>| tell(listener, ChainEvent::Step { agent, progress });
    step::run_step(
        link.workspace.as_ref(),
        &link.client,
        &link.agent,
        task,
        &input,
        &mut show,
    )
    .await
}

fn tell<F: FnMut(ChainEvent<'_>)>(listener: &Listener<'_, F>, event: ChainEvent<'_>) {
    let mut on_event = listener.borrow_mut();
    (*on_event)(event);
}

fn finished<F: FnMut(ChainEvent<'_>)>(
    agent: &str,
    report: StepReport,
    listener: &Listener<'_, F>,
) -> StepRecord {
    let record = StepRecord {
        agent: agent.to_owned(),
        report,
    };
    tell(listener, ChainEvent::StepFinished(&record));
    record
}

fn stopped(agent: &str, reason: StopReason, steps: Vec<StepRecord>) -> ChainOutcome {
    ChainOutcome::Stopped {
        at: agent.to_owned(),
        reason,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method},
    };

    use super::*;
    use crate::providers::openai::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};
    use crate::step::MAX_FAILED_TURNS;

    const SETTINGS: &str = r#"{"description":"Works.","model":"test/any-model"}"#;
    const NO_FIELDS: &str = r#"{"type":"object","properties":{}}"#;
    const FILES_AND_NOTES: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":5,"items":{"type":"string","maxLength":50}},
        "notes":{"type":"string"}}}"#;
    const FILES: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":5,"items":{"type":"string","maxLength":50}}}}"#;
    const MANY_FILES: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":10,"items":{"type":"string","maxLength":50}}}}"#;
    const FINDINGS: &str = r#"{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","items":{"type":"string"}}}}"#;
    const EXPLORED: &str = r#"{"files":["a.rs"],"notes":"PRIVATE-NOTE"}"#;
    const REVIEWED: &str = r#"{"findings":["unused import"]}"#;
    const PAUSE: Duration = Duration::from_millis(500);

    fn agent(name: &str, input: &str, output: &str) -> Agent {
        let contract =
            format!(r#"{{"name":"{name}","kind":"worker","input":{input},"output":{output}}}"#);
        Agent::from_parts(&contract, SETTINGS, format!("Prompt of {name}.")).expect("valid agent")
    }

    fn link(server: &MockServer, agent: Agent) -> Link {
        Link {
            agent,
            client: OpenAiCompatible::new(&server.uri(), None),
            workspace: None,
        }
    }

    fn one_by_one(server: &MockServer, agents: Vec<Agent>) -> Vec<Vec<Link>> {
        agents
            .into_iter()
            .map(|agent| vec![link(server, agent)])
            .collect()
    }

    fn explorer_then_reviewer(server: &MockServer) -> Vec<Vec<Link>> {
        let explorer = agent("explorer", NO_FIELDS, FILES_AND_NOTES);
        let reviewer = agent("reviewer", FILES, FINDINGS);
        one_by_one(server, vec![explorer, reviewer])
    }

    fn two_explorers_then_reviewer(server: &MockServer) -> Vec<Vec<Link>> {
        vec![
            vec![
                link(server, agent("explorer", NO_FIELDS, FILES)),
                link(server, agent("scout", NO_FIELDS, FILES)),
            ],
            vec![link(server, agent("reviewer", MANY_FILES, FINDINGS))],
        ]
    }

    fn completing(arguments: &str) -> ResponseTemplate {
        sse(&[
            text_chunk("THINKING-ALOUD"),
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

    async fn answering_by_prompt(answers: Vec<(&str, ResponseTemplate)>) -> MockServer {
        let server = MockServer::start().await;
        for (agent, response) in answers {
            Mock::given(method("POST"))
                .and(body_string_contains(format!("Prompt of {agent}.")))
                .respond_with(response)
                .mount(&server)
                .await;
        }
        server
    }

    async fn run(steps: &[Vec<Link>]) -> (ChainOutcome, Vec<String>) {
        let mut seen = Vec::new();
        let mut record = |event: ChainEvent<'_>| {
            if let ChainEvent::StepStarted(agents) = event {
                seen.push(agents.join("+"));
            }
        };
        let outcome = run_chain(steps, "ship it", &mut record)
            .await
            .expect("provider answers");
        (outcome, seen)
    }

    async fn request_texts(server: &MockServer) -> Vec<String> {
        let requests = server.received_requests().await.expect("recorded");
        requests
            .iter()
            .map(|request| String::from_utf8_lossy(&request.body).into_owned())
            .collect()
    }

    fn agents_of(steps: &[StepRecord]) -> Vec<&str> {
        steps.iter().map(|step| step.agent.as_str()).collect()
    }

    #[tokio::test]
    async fn steps_run_in_order_and_the_last_result_is_the_chain_result() {
        let server = serving_in_order(vec![completing(EXPLORED), completing(REVIEWED)]).await;
        let (outcome, started) = run(&explorer_then_reviewer(&server)).await;
        let ChainOutcome::Completed { output, steps } = outcome else {
            panic!("chain must complete");
        };
        assert_eq!(output, json!({"findings": ["unused import"]}));
        assert_eq!(agents_of(&steps), ["explorer", "reviewer"]);
        assert_eq!(started, ["explorer", "reviewer"]);
    }

    #[tokio::test]
    async fn next_step_gets_the_task_and_only_the_fields_it_declares() {
        let server = serving_in_order(vec![completing(EXPLORED), completing(REVIEWED)]).await;
        run(&explorer_then_reviewer(&server)).await;
        let requests = request_texts(&server).await;
        let to_reviewer = &requests[1];
        assert!(to_reviewer.contains("ship it"), "{to_reviewer}");
        assert!(to_reviewer.contains("a.rs"), "{to_reviewer}");
        assert!(!to_reviewer.contains("PRIVATE-NOTE"), "{to_reviewer}");
    }

    #[tokio::test]
    async fn next_step_sees_neither_the_prompt_nor_the_words_of_the_previous_one() {
        let server = serving_in_order(vec![completing(EXPLORED), completing(REVIEWED)]).await;
        run(&explorer_then_reviewer(&server)).await;
        let requests = request_texts(&server).await;
        let to_reviewer = &requests[1];
        assert!(to_reviewer.contains("Prompt of reviewer."), "{to_reviewer}");
        assert!(
            !to_reviewer.contains("Prompt of explorer."),
            "{to_reviewer}"
        );
        assert!(!to_reviewer.contains("THINKING-ALOUD"), "{to_reviewer}");
    }

    #[tokio::test]
    async fn incompatible_chain_makes_no_request() {
        let server = serving_in_order(vec![completing(REVIEWED)]).await;
        let reviewer_first = one_by_one(&server, vec![agent("reviewer", FILES, FINDINGS)]);
        let (outcome, started) = run(&reviewer_first).await;
        assert!(
            matches!(outcome, ChainOutcome::Incompatible(_)),
            "{outcome:?}"
        );
        assert!(started.is_empty());
        assert!(request_texts(&server).await.is_empty());
    }

    #[tokio::test]
    async fn chain_stops_at_the_step_that_gives_no_valid_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(completing(r#"{"files":"not a list"}"#))
            .mount(&server)
            .await;
        let (outcome, started) = run(&explorer_then_reviewer(&server)).await;
        let ChainOutcome::Stopped { at, reason, steps } = outcome else {
            panic!("chain must stop");
        };
        assert_eq!(at, "explorer");
        assert_eq!(reason, StopReason::StepFailed(Stop::NoValidResult));
        assert_eq!(agents_of(&steps), ["explorer"]);
        assert_eq!(started, ["explorer"]);
        let requests = request_texts(&server).await;
        assert_eq!(
            requests.len(),
            usize::try_from(MAX_FAILED_TURNS).expect("small")
        );
        assert!(
            requests
                .iter()
                .all(|text| !text.contains("Prompt of reviewer."))
        );
    }

    #[tokio::test]
    async fn empty_chain_is_incompatible() {
        let server = serving_in_order(vec![]).await;
        let (outcome, _) = run(&one_by_one(&server, vec![])).await;
        assert_eq!(outcome, ChainOutcome::Incompatible(ChainError::Empty));
    }

    #[tokio::test]
    async fn agents_of_one_step_work_at_the_same_time() {
        let slow = |files: &str| completing(files).set_delay(PAUSE);
        let server = answering_by_prompt(vec![
            ("explorer", slow(r#"{"files":["a.rs"]}"#)),
            ("scout", slow(r#"{"files":["b.rs"]}"#)),
            ("reviewer", completing(REVIEWED)),
        ])
        .await;
        let started_at = Instant::now();
        let (outcome, started) = run(&two_explorers_then_reviewer(&server)).await;
        let elapsed = started_at.elapsed();
        assert!(elapsed < PAUSE * 2, "{elapsed:?}");
        assert!(
            matches!(outcome, ChainOutcome::Completed { .. }),
            "{outcome:?}"
        );
        assert_eq!(started, ["explorer+scout", "reviewer"]);
    }

    #[tokio::test]
    async fn next_step_gets_the_joined_results_in_the_order_of_the_step() {
        let server = answering_by_prompt(vec![
            (
                "explorer",
                completing(r#"{"files":["a.rs"]}"#).set_delay(PAUSE),
            ),
            ("scout", completing(r#"{"files":["b.rs"]}"#)),
            ("reviewer", completing(REVIEWED)),
        ])
        .await;
        let (outcome, _) = run(&two_explorers_then_reviewer(&server)).await;
        let ChainOutcome::Completed { steps, .. } = outcome else {
            panic!("chain must complete");
        };
        assert_eq!(agents_of(&steps), ["explorer", "scout", "reviewer"]);
        let requests = request_texts(&server).await;
        let to_reviewer = requests
            .iter()
            .find(|text| text.contains("Prompt of reviewer."))
            .expect("reviewer was asked");
        let first = to_reviewer.find("a.rs").expect("a.rs is handed over");
        let second = to_reviewer.find("b.rs").expect("b.rs is handed over");
        assert!(first < second, "{to_reviewer}");
    }

    #[tokio::test]
    async fn failure_of_one_agent_of_a_step_stops_the_chain_after_the_step() {
        let server = answering_by_prompt(vec![
            ("explorer", completing(r#"{"files":"not a list"}"#)),
            ("scout", completing(r#"{"files":["b.rs"]}"#)),
            ("reviewer", completing(REVIEWED)),
        ])
        .await;
        let (outcome, started) = run(&two_explorers_then_reviewer(&server)).await;
        let ChainOutcome::Stopped { at, steps, .. } = outcome else {
            panic!("chain must stop");
        };
        assert_eq!(at, "explorer");
        assert_eq!(agents_of(&steps), ["explorer", "scout"]);
        assert_eq!(started, ["explorer+scout"]);
        let requests = request_texts(&server).await;
        assert!(
            requests
                .iter()
                .all(|text| !text.contains("Prompt of reviewer."))
        );
    }

    #[tokio::test]
    async fn progress_names_the_agent_it_comes_from() {
        let server = answering_by_prompt(vec![
            ("explorer", completing(r#"{"files":["a.rs"]}"#)),
            ("scout", completing(r#"{"files":["b.rs"]}"#)),
            ("reviewer", completing(REVIEWED)),
        ])
        .await;
        let steps = two_explorers_then_reviewer(&server);
        let mut speakers = Vec::new();
        let mut record = |event: ChainEvent<'_>| {
            if let ChainEvent::Step { agent, .. } = event {
                speakers.push(agent.to_owned());
            }
        };
        run_chain(&steps, "ship it", &mut record)
            .await
            .expect("provider answers");
        speakers.sort();
        speakers.dedup();
        assert_eq!(speakers, ["explorer", "reviewer", "scout"]);
    }
}
