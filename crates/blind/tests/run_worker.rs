mod common;

use std::process::Output;

use common::{
    TURN_LIMIT, blind, completing, model_answering, request_texts, stdout, talking, workspace,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const PLANNER: &str = r#"{"name":"planner","kind":"worker",
    "input":{"type":"object","properties":{"focus":{"type":"string"}}},
    "output":{"type":"object","required":["steps","risky"],"properties":{
        "steps":{"type":"array","maxItems":3,"items":{"type":"string","maxLength":40}},
        "risky":{"type":"boolean"}}}}"#;
const VALID_PLAN: &str = r#"{"steps":["read","write"],"risky":false}"#;

fn planner_workspace(provider_url: &str) -> TempDir {
    workspace(provider_url, &[("planner", PLANNER)])
}

fn run(workspace: &TempDir, agent: &str, extra: &[&str]) -> Output {
    let arguments = [&["run", agent, "--task", "ship it"], extra].concat();
    blind(workspace.path(), &arguments)
}

#[tokio::test]
async fn checked_result_is_the_only_thing_printed_to_stdout() {
    let server = model_answering(completing(VALID_PLAN)).await;
    let output = run(&planner_workspace(&server.uri()), "planner", &[]);
    assert!(output.status.success());
    let printed: Value = serde_json::from_str(&stdout(&output)).expect("stdout is JSON");
    assert_eq!(printed, json!({"steps": ["read", "write"], "risky": false}));
    assert_eq!(request_texts(&server).await.len(), 1);
}

#[tokio::test]
async fn result_that_breaks_the_contract_never_reaches_stdout() {
    let leaking = r#"{"steps":["read"],"risky":false,"secret":"contents of .env"}"#;
    let server = model_answering(completing(leaking)).await;
    let output = run(&planner_workspace(&server.uri()), "planner", &[]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    assert_eq!(request_texts(&server).await.len(), TURN_LIMIT);
}

#[tokio::test]
async fn model_that_answers_in_prose_produces_no_result() {
    let prose = r#"Done! {"steps":["I did it myself"],"risky":false}"#;
    let server = model_answering(talking(prose)).await;
    let output = run(&planner_workspace(&server.uri()), "planner", &[]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    assert_eq!(request_texts(&server).await.len(), TURN_LIMIT);
}

#[tokio::test]
async fn input_that_breaks_the_contract_never_reaches_the_model() {
    let server = model_answering(completing(VALID_PLAN)).await;
    let workspace = planner_workspace(&server.uri());
    let output = run(&workspace, "planner", &["--input", r#"{"focus":7}"#]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    assert!(request_texts(&server).await.is_empty());
}

#[tokio::test]
async fn unknown_agent_never_reaches_the_model() {
    let server = model_answering(completing(VALID_PLAN)).await;
    let output = run(&planner_workspace(&server.uri()), "ghost", &[]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    assert!(request_texts(&server).await.is_empty());
}
