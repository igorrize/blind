use blind_core::{Agent, ChainError, Contract, Violation, check_chain};
use blind_exec::Workspace;
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
    StepStarted(&'a str),
    Step(Progress<'a>),
    StepResult(&'a Value),
    StepFinished(&'a StepRecord),
}

pub async fn run_chain(
    links: &[Link],
    task: &str,
    on_event: &mut impl FnMut(ChainEvent<'_>),
) -> Result<ChainOutcome, ProviderError> {
    let contracts: Vec<&Contract> = links.iter().map(|link| link.agent.contract()).collect();
    if let Err(error) = check_chain(&contracts) {
        return Ok(ChainOutcome::Incompatible(error));
    }
    let mut steps = Vec::new();
    let mut handed_over = json!({});
    for link in links {
        let name = link.agent.contract().name().as_str();
        let input = link.agent.contract().input().project(&handed_over);
        on_event(ChainEvent::StepStarted(name));
        let mut show = |progress: Progress<'_>| on_event(ChainEvent::Step(progress));
        let outcome = step::run_step(
            link.workspace.as_ref(),
            &link.client,
            &link.agent,
            task,
            &input,
            &mut show,
        )
        .await?;
        match outcome {
            StepOutcome::Completed { output, report } => {
                steps.push(finished(name, report, on_event));
                on_event(ChainEvent::StepResult(&output));
                handed_over = output;
            }
            StepOutcome::Failed { report, stop } => {
                steps.push(finished(name, report, on_event));
                return Ok(stopped(name, StopReason::StepFailed(stop), steps));
            }
            StepOutcome::InputRejected(violations) => {
                return Ok(stopped(name, StopReason::InputRejected(violations), steps));
            }
        }
    }
    Ok(ChainOutcome::Completed {
        output: handed_over,
        steps,
    })
}

fn finished(
    name: &str,
    report: StepReport,
    on_event: &mut impl FnMut(ChainEvent<'_>),
) -> StepRecord {
    let record = StepRecord {
        agent: name.to_owned(),
        report,
    };
    on_event(ChainEvent::StepFinished(&record));
    record
}

fn stopped(name: &str, reason: StopReason, steps: Vec<StepRecord>) -> ChainOutcome {
    ChainOutcome::Stopped {
        at: name.to_owned(),
        reason,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

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
    const FINDINGS: &str = r#"{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","items":{"type":"string"}}}}"#;
    const EXPLORED: &str = r#"{"files":["a.rs"],"notes":"PRIVATE-NOTE"}"#;
    const REVIEWED: &str = r#"{"findings":["unused import"]}"#;

    fn agent(name: &str, input: &str, output: &str) -> Agent {
        let contract =
            format!(r#"{{"name":"{name}","kind":"worker","input":{input},"output":{output}}}"#);
        Agent::from_parts(&contract, SETTINGS, format!("Prompt of {name}.")).expect("valid agent")
    }

    fn links(server: &MockServer, agents: Vec<Agent>) -> Vec<Link> {
        agents
            .into_iter()
            .map(|agent| Link {
                agent,
                client: OpenAiCompatible::new(&server.uri(), None),
                workspace: None,
            })
            .collect()
    }

    fn explorer_then_reviewer(server: &MockServer) -> Vec<Link> {
        let explorer = agent("explorer", NO_FIELDS, FILES_AND_NOTES);
        let reviewer = agent("reviewer", FILES, FINDINGS);
        links(server, vec![explorer, reviewer])
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

    async fn run(links: &[Link]) -> (ChainOutcome, Vec<String>) {
        let mut seen = Vec::new();
        let mut record = |event: ChainEvent<'_>| {
            if let ChainEvent::StepStarted(name) = event {
                seen.push(name.to_owned());
            }
        };
        let outcome = run_chain(links, "ship it", &mut record)
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
        let reviewer_first = links(&server, vec![agent("reviewer", FILES, FINDINGS)]);
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
        let (outcome, _) = run(&links(&server, vec![])).await;
        assert_eq!(outcome, ChainOutcome::Incompatible(ChainError::Empty));
    }
}
