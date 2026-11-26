use std::fs;

use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use super::*;
use crate::providers::openai::test_support::{sse, tool_call_chunk, usage_chunk};

const SETTINGS: &str = r#"{"description":"Writes.","model":"test/any"}"#;
const DONE: &str = r#"{"summary":"added the module"}"#;
const MAIN: &str = "fn main() {}\n";

fn writer(changed_limit: &str, rights: &str) -> Agent {
    let contract = format!(
        r#"{{"name":"writer","kind":"worker",
            "input":{{"type":"object","properties":{{}}}},
            "output":{{"type":"object","required":["changed","summary"],"properties":{{
                "changed":{{"type":"array"{changed_limit},"items":{{"type":"string"}}}},
                "summary":{{"type":"string"}}}}}},
            "rights":{rights}}}"#
    );
    Agent::from_parts(&contract, SETTINGS, "Write the code.".to_owned()).expect("valid agent")
}

fn reporting_writer() -> Agent {
    writer(
        "",
        r#"{"read":["**"],"write":["src/**"],"written_as":"changed"}"#,
    )
}

fn copy_of_a_project() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::create_dir_all(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("src/main.rs"), MAIN).expect("main.rs");
    fs::write(dir.path().join("docs/notes.md"), "notes\n").expect("notes.md");
    dir
}

fn calling(tool: &str, arguments: &Value) -> ResponseTemplate {
    sse(&[
        tool_call_chunk("call_1", tool, &arguments.to_string()),
        usage_chunk(10, 5),
    ])
}

fn completing(arguments: &str) -> ResponseTemplate {
    sse(&[
        tool_call_chunk("call_1", COMPLETE, arguments),
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

async fn run(server: &MockServer, agent: &Agent, copy: &TempDir) -> StepOutcome {
    let workspace = workspace_for(agent, Ground::Copy(copy.path())).expect("workspace");
    let client = OpenAiCompatible::new(&server.uri(), None);
    let input = json!({});
    let mut ignore = |_: Progress<'_>| {};
    let step = run_step(
        workspace.as_ref(),
        &client,
        agent,
        "ship it",
        &input,
        &mut ignore,
    );
    step.await.expect("provider answers")
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    let requests = server.received_requests().await.expect("recorded");
    requests
        .iter()
        .map(|request| serde_json::from_slice(&request.body).expect("json body"))
        .collect()
}

fn tool_named<'a>(request: &'a Value, name: &str) -> Option<&'a Value> {
    let tools = request["tools"].as_array().expect("tools");
    tools.iter().find(|tool| tool["function"]["name"] == name)
}

fn last_tool_result(request: &Value) -> &str {
    let messages = request["messages"].as_array().expect("messages");
    let result = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .expect("a tool result");
    result["content"].as_str().expect("text")
}

fn writing(path: &str, content: &str) -> ResponseTemplate {
    calling("write_file", &json!({"path": path, "content": content}))
}

#[tokio::test]
async fn writer_changes_files_and_the_harness_reports_which() {
    let copy = copy_of_a_project();
    let server = serving_in_order(vec![
        writing("src/clipboard.rs", "pub fn copy() {}\n"),
        calling(
            "edit_file",
            &json!({"path": "src/main.rs", "old": "{}", "new": "{ run(); }"}),
        ),
        completing(DONE),
    ])
    .await;
    let outcome = run(&server, &reporting_writer(), &copy).await;
    let StepOutcome::Completed { output, report } = outcome else {
        panic!("step must complete: {outcome:?}");
    };
    assert_eq!(
        output,
        json!({"summary": "added the module", "changed": ["src/clipboard.rs", "src/main.rs"]})
    );
    assert_eq!(report.tool_calls, 2);
    let written = fs::read_to_string(copy.path().join("src/main.rs")).expect("main.rs");
    assert_eq!(written, "fn main() { run(); }\n");
}

#[tokio::test]
async fn model_is_not_asked_for_the_field_the_harness_fills() {
    let copy = copy_of_a_project();
    let server = serving_in_order(vec![completing(DONE)]).await;
    run(&server, &reporting_writer(), &copy).await;
    let bodies = request_bodies(&server).await;
    let complete = tool_named(&bodies[0], COMPLETE).expect("complete is offered");
    let parameters = &complete["function"]["parameters"];
    assert_eq!(parameters["required"], json!(["summary"]));
    assert!(parameters["properties"].get("changed").is_none());
    let system = bodies[0]["messages"][0]["content"].as_str().expect("text");
    assert!(system.contains("do not list them"), "{system}");
    assert!(system.contains("You cannot run commands"), "{system}");
}

#[tokio::test]
async fn list_of_written_files_cannot_be_made_up_by_the_model() {
    let copy = copy_of_a_project();
    let boasting = r#"{"summary":"done","changed":["src/main.rs","src/other.rs"]}"#;
    let server = serving_in_order(vec![completing(boasting), completing(DONE)]).await;
    let outcome = run(&server, &reporting_writer(), &copy).await;
    let StepOutcome::Completed { output, report } = outcome else {
        panic!("step must complete: {outcome:?}");
    };
    assert_eq!(report.rejections, 1);
    assert_eq!(output["changed"], json!([]));
    let bodies = request_bodies(&server).await;
    let rejected = bodies[1].to_string();
    assert!(rejected.contains("`changed` is not declared"), "{rejected}");
}

#[tokio::test]
async fn write_outside_the_rights_is_refused_and_the_model_is_told() {
    let copy = copy_of_a_project();
    let server = serving_in_order(vec![
        writing("docs/notes.md", "rewritten"),
        completing(DONE),
    ])
    .await;
    let outcome = run(&server, &reporting_writer(), &copy).await;
    let StepOutcome::Completed { output, .. } = outcome else {
        panic!("step must complete: {outcome:?}");
    };
    assert_eq!(output["changed"], json!([]));
    let notes = fs::read_to_string(copy.path().join("docs/notes.md")).expect("notes.md");
    assert_eq!(notes, "notes\n");
    let bodies = request_bodies(&server).await;
    assert_eq!(
        last_tool_result(&bodies[1]),
        "error: `docs/notes.md` is outside the paths this agent may write"
    );
}

#[tokio::test]
async fn agent_that_only_reads_is_not_offered_the_writing_tools_even_in_a_copy() {
    let copy = copy_of_a_project();
    let reader = writer("", r#"{"read":["**"]}"#);
    let attempt = writing("src/main.rs", "changed");
    let both = r#"{"summary":"done","changed":[]}"#;
    let server = serving_in_order(vec![attempt, completing(both)]).await;
    run(&server, &reader, &copy).await;
    let bodies = request_bodies(&server).await;
    assert!(tool_named(&bodies[0], "write_file").is_none());
    assert!(tool_named(&bodies[0], "edit_file").is_none());
    assert_eq!(
        last_tool_result(&bodies[1]),
        "blocked: tool `write_file` does not exist"
    );
    let main = fs::read_to_string(copy.path().join("src/main.rs")).expect("main.rs");
    assert_eq!(main, MAIN);
}

#[tokio::test]
async fn writer_that_wrote_more_files_than_its_contract_holds_fails() {
    let copy = copy_of_a_project();
    let narrow = writer(
        r#","maxItems":1"#,
        r#"{"read":["**"],"write":["src/**"],"written_as":"changed"}"#,
    );
    let server = serving_in_order(vec![
        writing("src/one.rs", "pub fn one() {}\n"),
        writing("src/two.rs", "pub fn two() {}\n"),
        completing(DONE),
    ])
    .await;
    let outcome = run(&server, &narrow, &copy).await;
    let StepOutcome::Failed { stop, .. } = outcome else {
        panic!("step must fail: {outcome:?}");
    };
    assert_eq!(stop, Stop::WroteMoreThanItsContractHolds);
}

#[tokio::test]
async fn writer_without_written_as_names_its_files_itself() {
    let copy = copy_of_a_project();
    let trusting = writer("", r#"{"read":["**"],"write":["src/**"]}"#);
    let named = r#"{"summary":"done","changed":["src/whatever.rs"]}"#;
    let server = serving_in_order(vec![completing(named)]).await;
    let outcome = run(&server, &trusting, &copy).await;
    let StepOutcome::Completed { output, .. } = outcome else {
        panic!("step must complete: {outcome:?}");
    };
    assert_eq!(output["changed"], json!(["src/whatever.rs"]));
}

#[test]
fn writer_gets_a_workspace_only_in_a_copy() {
    let project = copy_of_a_project();
    let agent = reporting_writer();
    let refused = workspace_for(&agent, Ground::Project(project.path()))
        .err()
        .expect("must be refused");
    assert_eq!(
        refused.to_string(),
        "agent `writer` writes files, and agents may write only in a copy of the project"
    );
    let allowed = workspace_for(&agent, Ground::Copy(project.path())).expect("allowed");
    assert!(allowed.is_some_and(|workspace| workspace.can_write()));
}
