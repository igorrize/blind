use std::fs;

use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use super::*;
use crate::providers::openai::test_support::{sse, tool_call_chunk, usage_chunk};

const FOUND: &str = r#"{"files":["src/main.rs"]}"#;
const LISTING: &str = r#"{"pattern":"**"}"#;
const EVERY_TOOL: [&str; 5] = ["complete", "find_files", "grep", "list_files", "read_file"];

fn explorer(limits: &str) -> Agent {
    let contract = r#"{"name":"explorer","kind":"worker",
        "input":{"type":"object","properties":{}},
        "output":{"type":"object","required":["files"],"properties":{
            "files":{"type":"array","items":{"type":"string"}}}},
        "rights":{"read":["**"]}}"#;
    let settings = format!(r#"{{"description":"Finds.","model":"test/any","limits":{limits}}}"#);
    Agent::from_parts(contract, &settings, "Find the files.".to_owned()).expect("valid agent")
}

fn project() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("main.rs");
    dir
}

fn listing(input_tokens: u64) -> ResponseTemplate {
    sse(&[
        tool_call_chunk("call_1", "list_files", LISTING),
        usage_chunk(input_tokens, 1),
    ])
}

fn listing_three_times() -> ResponseTemplate {
    let another = |index: u32| {
        json!({"choices": [{"delta": {"tool_calls": [{
            "index": index, "id": format!("call_{}", index + 1), "type": "function",
            "function": {"name": "list_files", "arguments": LISTING}
        }]}}]})
    };
    sse(&[another(0), another(1), another(2), usage_chunk(10, 1)])
}

fn completing() -> ResponseTemplate {
    sse(&[
        tool_call_chunk("call_1", COMPLETE, FOUND),
        usage_chunk(10, 1),
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

async fn serving_always(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn run(server: &MockServer, agent: &Agent) -> (StepOutcome, Vec<String>) {
    let project = project();
    let workspace = workspace_for(agent, Ground::Project(project.path())).expect("workspace");
    let client = OpenAiCompatible::new(&server.uri(), None);
    let mut told = Vec::new();
    let mut record = |progress: Progress<'_>| {
        if let Progress::ToldToFinish(reason) = progress {
            told.push(reason.to_owned());
        }
    };
    let input = json!({});
    let step = run_step(
        workspace.as_ref(),
        &client,
        agent,
        "ship it",
        &input,
        &mut record,
    );
    (step.await.expect("provider answers"), told)
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    let requests = server.received_requests().await.expect("recorded");
    requests
        .iter()
        .map(|request| serde_json::from_slice(&request.body).expect("json body"))
        .collect()
}

fn tool_names(request: &Value) -> Vec<&str> {
    let tools = request["tools"].as_array().expect("tools");
    tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect()
}

fn tool_results(request: &Value) -> Vec<&str> {
    let messages = request["messages"].as_array().expect("messages");
    messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str())
        .collect()
}

fn was_told_to_finish(request: &Value) -> bool {
    request
        .to_string()
        .contains("You may not look at the project any more")
}

#[tokio::test]
async fn agent_that_spent_its_tool_calls_is_left_with_complete_and_hands_in_the_result() {
    let server = serving_in_order(vec![listing(10), listing(10), completing()]).await;
    let (outcome, told) = run(&server, &explorer(r#"{"tool_calls":2}"#)).await;
    let StepOutcome::Completed { output, report } = outcome else {
        panic!("step must complete");
    };
    assert_eq!(output, json!({"files": ["src/main.rs"]}));
    assert!(report.forced_to_finish);
    assert_eq!(report.tool_calls, 2);
    assert_eq!(told, ["the tool call limit is reached"]);
    let bodies = request_bodies(&server).await;
    assert_eq!(tool_names(&bodies[0]), EVERY_TOOL);
    assert_eq!(tool_names(&bodies[1]), EVERY_TOOL);
    assert_eq!(tool_names(&bodies[2]), ["complete"]);
    assert!(!was_told_to_finish(&bodies[1]));
    assert!(was_told_to_finish(&bodies[2]));
}

#[tokio::test]
async fn agent_that_finishes_within_its_limits_is_not_forced() {
    let server = serving_in_order(vec![listing(10), completing()]).await;
    let (outcome, told) = run(&server, &explorer(r#"{"tool_calls":2}"#)).await;
    let StepOutcome::Completed { report, .. } = outcome else {
        panic!("step must complete");
    };
    assert!(!report.forced_to_finish);
    assert!(told.is_empty());
}

#[tokio::test]
async fn calls_beyond_the_limit_in_one_turn_are_not_run() {
    let server = serving_in_order(vec![listing_three_times(), completing()]).await;
    let (outcome, _) = run(&server, &explorer(r#"{"tool_calls":2}"#)).await;
    let StepOutcome::Completed { report, .. } = outcome else {
        panic!("step must complete");
    };
    assert_eq!(report.tool_calls, 2);
    let bodies = request_bodies(&server).await;
    assert_eq!(
        tool_results(&bodies[1]),
        ["src/main.rs", "src/main.rs", NO_MORE_CALLS]
    );
}

#[tokio::test]
async fn agent_that_keeps_looking_after_being_told_to_finish_gets_nothing_and_fails() {
    let server = serving_always(listing(10)).await;
    let (outcome, told) = run(&server, &explorer(r#"{"tool_calls":3}"#)).await;
    let StepOutcome::Failed { report, stop } = outcome else {
        panic!("step must fail");
    };
    assert_eq!(stop, Stop::NoValidResult);
    assert_eq!(report.tool_calls, 3);
    assert_eq!(report.turns, 3 + MAX_FAILED_TURNS);
    assert_eq!(told.len(), 1);
    let bodies = request_bodies(&server).await;
    for late in &bodies[3..] {
        assert_eq!(tool_names(late), ["complete"]);
    }
    let last = bodies.last().expect("requests");
    assert_eq!(tool_results(last).last(), Some(&NO_MORE_CALLS));
}

#[tokio::test]
async fn agent_that_spent_two_thirds_of_its_tokens_is_told_to_finish() {
    let server = serving_in_order(vec![listing(100), listing(100), completing()]).await;
    let (outcome, told) = run(&server, &explorer(r#"{"tokens":300}"#)).await;
    let StepOutcome::Completed { report, .. } = outcome else {
        panic!("step must complete");
    };
    assert!(report.forced_to_finish);
    assert_eq!(told, ["two thirds of the token budget are spent"]);
    let bodies = request_bodies(&server).await;
    assert_eq!(tool_names(&bodies[1]), EVERY_TOOL);
    assert_eq!(tool_names(&bodies[2]), ["complete"]);
}

#[tokio::test]
async fn agent_that_spends_every_token_is_stopped_even_while_finishing() {
    let server = serving_always(listing(200)).await;
    let (outcome, _) = run(&server, &explorer(r#"{"tokens":300}"#)).await;
    let StepOutcome::Failed { report, stop } = outcome else {
        panic!("step must fail");
    };
    assert_eq!(stop, Stop::OutOfTokens);
    assert_eq!(report.turns, 2);
}

#[tokio::test]
async fn last_turns_are_kept_for_finishing() {
    let server = serving_always(listing(10)).await;
    let (outcome, told) = run(&server, &explorer(r#"{"turns":5}"#)).await;
    let StepOutcome::Failed { report, stop } = outcome else {
        panic!("step must fail");
    };
    assert_eq!(stop, Stop::OutOfTurns);
    assert_eq!(report.turns, 5);
    assert_eq!(report.tool_calls, 3);
    assert_eq!(told, ["only the last turns are left"]);
    let bodies = request_bodies(&server).await;
    assert_eq!(bodies.len(), 5);
    assert_eq!(tool_names(&bodies[2]), EVERY_TOOL);
    assert_eq!(tool_names(&bodies[3]), ["complete"]);
    assert_eq!(tool_names(&bodies[4]), ["complete"]);
}

#[tokio::test]
async fn agent_without_files_is_never_told_to_finish() {
    let contract = r#"{"name":"planner","kind":"worker",
        "input":{"type":"object","properties":{}},
        "output":{"type":"object","required":["files"],"properties":{
            "files":{"type":"array","items":{"type":"string"}}}}}"#;
    let settings = r#"{"description":"Plans.","model":"test/any","limits":{"turns":2}}"#;
    let planner = Agent::from_parts(contract, settings, "Plan.".to_owned()).expect("valid agent");
    let server = serving_in_order(vec![completing()]).await;
    let (outcome, told) = run(&server, &planner).await;
    let StepOutcome::Completed { report, .. } = outcome else {
        panic!("step must complete");
    };
    assert!(!report.forced_to_finish);
    assert!(told.is_empty());
    assert!(!was_told_to_finish(&request_bodies(&server).await[0]));
}
