use std::fs;

use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use super::*;
use crate::providers::openai::test_support::{sse, tool_call_chunk, usage_chunk};

const SETTINGS: &str = r#"{"description":"Works.","model":"test/any-model"}"#;
const SECRET: &str = "TOP-SECRET-VALUE";
const FOUND: &str = r#"{"files":["src/main.rs"]}"#;

fn explorer(rights: &str) -> Agent {
    let contract = format!(
        r#"{{"name":"explorer","kind":"worker",
            "input":{{"type":"object","properties":{{}}}},
            "output":{{"type":"object","required":["files"],"properties":{{
                "files":{{"type":"array","items":{{"type":"string"}}}}}}}},
            "rights":{rights}}}"#
    );
    Agent::from_parts(&contract, SETTINGS, "Find the files.".to_owned()).expect("valid agent")
}

fn project() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::create_dir_all(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("main.rs");
    fs::write(dir.path().join("docs/notes.md"), SECRET).expect("notes.md");
    fs::write(dir.path().join(".env"), SECRET).expect(".env");
    dir
}

fn calling(tool: &str, arguments: &str) -> ResponseTemplate {
    sse(&[
        tool_call_chunk("call_1", tool, arguments),
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

async fn run(server: &MockServer, agent: &Agent, project: &TempDir) -> StepOutcome {
    let workspace = workspace_for(agent, project.path()).expect("workspace");
    let client = OpenAiCompatible::new(&server.uri(), None);
    run_step(
        workspace.as_ref(),
        &client,
        agent,
        "ship it",
        &json!({}),
        &mut |_| {},
    )
    .await
    .expect("provider answers")
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

fn last_tool_result(request: &Value) -> &str {
    let messages = request["messages"].as_array().expect("messages");
    let result = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .expect("a tool result");
    result["content"].as_str().expect("text")
}

#[tokio::test]
async fn agent_with_read_rights_is_offered_the_reading_tools() {
    let server = serving_in_order(vec![calling(COMPLETE, FOUND)]).await;
    run(&server, &explorer(r#"{"read":["src/**"]}"#), &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(
        tool_names(&bodies[0]),
        ["complete", "find_files", "grep", "list_files", "read_file"]
    );
}

#[tokio::test]
async fn agent_without_rights_is_offered_only_complete() {
    let server = serving_in_order(vec![calling(COMPLETE, FOUND)]).await;
    run(&server, &explorer("{}"), &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(tool_names(&bodies[0]), ["complete"]);
}

#[tokio::test]
async fn agent_without_rights_cannot_read_even_if_it_calls_the_tool() {
    let reading = calling("read_file", r#"{"path":"src/main.rs"}"#);
    let server = serving_in_order(vec![reading, calling(COMPLETE, FOUND)]).await;
    run(&server, &explorer("{}"), &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(
        last_tool_result(&bodies[1]),
        "blocked: tool `read_file` does not exist"
    );
}

#[tokio::test]
async fn file_in_scope_is_read_and_counted_as_work() {
    let reading = calling("read_file", r#"{"path":"src/main.rs"}"#);
    let server = serving_in_order(vec![reading, calling(COMPLETE, FOUND)]).await;
    let outcome = run(&server, &explorer(r#"{"read":["src/**"]}"#), &project()).await;
    let StepOutcome::Completed { report, .. } = outcome else {
        panic!("step must complete");
    };
    assert_eq!(report.turns, 2);
    assert_eq!(report.tool_calls, 1);
    assert_eq!(report.rejections, 0);
    let bodies = request_bodies(&server).await;
    assert_eq!(last_tool_result(&bodies[1]), "    1  fn main() {}\n");
}

#[tokio::test]
async fn out_of_scope_and_protected_files_never_reach_the_model() {
    for path in ["docs/notes.md", ".env", "../outside", "/etc/passwd"] {
        let reading = calling("read_file", &json!({"path": path}).to_string());
        let server = serving_in_order(vec![reading, calling(COMPLETE, FOUND)]).await;
        run(&server, &explorer(r#"{"read":["src/**"]}"#), &project()).await;
        let bodies = request_bodies(&server).await;
        let result = last_tool_result(&bodies[1]);
        assert!(result.starts_with("error: "), "{path}: {result}");
        assert!(!bodies[1].to_string().contains(SECRET), "{path} leaked");
    }
}

#[tokio::test]
async fn search_does_not_leak_what_the_agent_may_not_read() {
    let searching = calling("grep", r#"{"query":"SECRET"}"#);
    let server = serving_in_order(vec![searching, calling(COMPLETE, FOUND)]).await;
    run(&server, &explorer(r#"{"read":["src/**"]}"#), &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(last_tool_result(&bodies[1]), "no matches");
    assert!(!bodies[1].to_string().contains(SECRET));
}

#[tokio::test]
async fn reading_tools_are_not_run_when_complete_is_called_in_the_same_turn() {
    let both = sse(&[
        tool_call_chunk("call_1", "read_file", r#"{"path":"src/main.rs"}"#),
        serde_json::json!({"choices": [{"delta": {"tool_calls": [{
            "index": 1, "id": "call_2", "type": "function",
            "function": {"name": "complete", "arguments": "{\"files\":7}"}
        }]}}]}),
    ]);
    let server = serving_in_order(vec![both, calling(COMPLETE, FOUND)]).await;
    let outcome = run(&server, &explorer(r#"{"read":["src/**"]}"#), &project()).await;
    let StepOutcome::Completed { report, .. } = outcome else {
        panic!("step must complete");
    };
    assert_eq!(report.tool_calls, 0);
    let second_request = request_bodies(&server).await[1].to_string();
    assert!(second_request.contains(NOT_RUN), "{second_request}");
    assert!(!second_request.contains("fn main"), "{second_request}");
}

fn reviewer() -> Agent {
    let contract = r#"{"name":"reviewer","kind":"worker",
        "input":{"type":"object","required":["files"],"properties":{
            "files":{"type":"array","items":{"type":"string"}}}},
        "output":{"type":"object","required":["files"],"properties":{
            "files":{"type":"array","items":{"type":"string"}}}},
        "rights":{"read":["**"],"read_from":"files"}}"#;
    Agent::from_parts(contract, SETTINGS, "Review the files.".to_owned()).expect("valid agent")
}

async fn review(server: &MockServer, project: &TempDir) -> StepOutcome {
    let agent = reviewer();
    let input = json!({"files": ["src/main.rs"]});
    let workspace = workspace_for(&agent, project.path()).expect("workspace");
    let client = OpenAiCompatible::new(&server.uri(), None);
    run_step(
        workspace.as_ref(),
        &client,
        &agent,
        "ship it",
        &input,
        &mut |_| {},
    )
    .await
    .expect("provider answers")
}

#[tokio::test]
async fn agent_with_a_tool_list_is_offered_only_those_tools() {
    let server = serving_in_order(vec![calling(COMPLETE, FOUND)]).await;
    let searching_only = explorer(r#"{"read":["**"],"tools":["find_files","grep"]}"#);
    run(&server, &searching_only, &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(tool_names(&bodies[0]), ["complete", "find_files", "grep"]);
}

#[tokio::test]
async fn tool_left_out_of_the_list_cannot_be_called() {
    let reading = calling("read_file", r#"{"path":"src/main.rs"}"#);
    let server = serving_in_order(vec![reading, calling(COMPLETE, FOUND)]).await;
    let searching_only = explorer(r#"{"read":["**"],"tools":["find_files","grep"]}"#);
    run(&server, &searching_only, &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(
        last_tool_result(&bodies[1]),
        "blocked: tool `read_file` does not exist"
    );
    assert!(!bodies[1].to_string().contains("fn main"));
}

#[tokio::test]
async fn agent_reads_a_file_it_was_given() {
    let reading = calling("read_file", r#"{"path":"src/main.rs"}"#);
    let server = serving_in_order(vec![reading, calling(COMPLETE, FOUND)]).await;
    review(&server, &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(last_tool_result(&bodies[1]), "    1  fn main() {}\n");
}

#[tokio::test]
async fn agent_cannot_read_a_file_it_was_not_given() {
    let wandering = calling("read_file", r#"{"path":"docs/notes.md"}"#);
    let server = serving_in_order(vec![wandering, calling(COMPLETE, FOUND)]).await;
    review(&server, &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(
        last_tool_result(&bodies[1]),
        "error: `docs/notes.md` is not among the files given to this agent"
    );
    assert!(!bodies[1].to_string().contains(SECRET));
}

#[tokio::test]
async fn refused_call_is_reported_as_refused_not_as_done() {
    let wandering = calling("read_file", r#"{"path":"docs/notes.md"}"#);
    let allowed = calling("read_file", r#"{"path":"src/main.rs"}"#);
    let server = serving_in_order(vec![wandering, allowed, calling(COMPLETE, FOUND)]).await;
    let agent = reviewer();
    let project = project();
    let workspace = workspace_for(&agent, project.path()).expect("workspace");
    let client = OpenAiCompatible::new(&server.uri(), None);
    let mut refusals = Vec::new();
    let mut record = |progress: Progress<'_>| {
        if let Progress::ToolCalled { refusal, .. } = progress {
            refusals.push(refusal.map(str::to_owned));
        }
    };
    let input = json!({"files": ["src/main.rs"]});
    run_step(
        workspace.as_ref(),
        &client,
        &agent,
        "ship it",
        &input,
        &mut record,
    )
    .await
    .expect("provider answers");
    let not_given = "`docs/notes.md` is not among the files given to this agent".to_owned();
    assert_eq!(refusals, [Some(not_given), None]);
}

#[tokio::test]
async fn agent_limited_to_given_files_can_still_search_wider() {
    let searching = calling("grep", r#"{"query":"SECRET"}"#);
    let server = serving_in_order(vec![searching, calling(COMPLETE, FOUND)]).await;
    review(&server, &project()).await;
    let bodies = request_bodies(&server).await;
    assert_eq!(
        last_tool_result(&bodies[1]),
        format!("docs/notes.md:1: {SECRET}")
    );
}

#[test]
fn agent_that_asks_for_write_rights_is_not_run() {
    let fixer = explorer(r#"{"read":["src/**"],"write":["src/**"]}"#);
    let refused = workspace_for(&fixer, project().path())
        .err()
        .expect("must be refused");
    assert_eq!(
        refused.to_string(),
        "agent `explorer` asks for write rights, which blind cannot grant yet"
    );
}

#[test]
fn agent_without_read_rights_gets_no_workspace() {
    assert!(
        workspace_for(&explorer("{}"), project().path())
            .expect("ok")
            .is_none()
    );
}
