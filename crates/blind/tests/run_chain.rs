mod common;

use std::process::Output;

use common::{
    TURN_LIMIT, blind, completing, model_answering, model_answering_in_order, request_texts,
    stdout, workspace,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const EXPLORER: &str = r#"{"name":"explorer","kind":"worker",
    "input":{"type":"object","properties":{}},
    "output":{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":5,"items":{"type":"string","maxLength":50}},
        "notes":{"type":"string"}}}}"#;
const REVIEWER: &str = r#"{"name":"reviewer","kind":"worker",
    "input":{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":5,"items":{"type":"string","maxLength":50}}}},
    "output":{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","items":{"type":"string"}}}}}"#;
const EXPLORED: &str = r#"{"files":["a.rs"],"notes":"PRIVATE-NOTE"}"#;
const REVIEWED: &str = r#"{"findings":["unused import"]}"#;

fn two_agents(provider_url: &str) -> TempDir {
    workspace(
        provider_url,
        &[("explorer", EXPLORER), ("reviewer", REVIEWER)],
    )
}

fn chain_run(workspace: &TempDir, agents: &[&str]) -> Output {
    let arguments = [&["chain", "run"], agents, &["--task", "ship it"]].concat();
    blind(workspace.path(), &arguments)
}

fn chain_check(workspace: &TempDir, agents: &[&str]) -> Output {
    blind(workspace.path(), &[&["chain", "check"], agents].concat())
}

#[tokio::test]
async fn chain_prints_only_the_checked_result_of_the_last_step() {
    let server = model_answering_in_order(vec![completing(EXPLORED), completing(REVIEWED)]).await;
    let output = chain_run(&two_agents(&server.uri()), &["explorer", "reviewer"]);
    assert!(output.status.success());
    let printed: Value = serde_json::from_str(&stdout(&output)).expect("stdout is JSON");
    assert_eq!(printed, json!({"findings": ["unused import"]}));
}

#[tokio::test]
async fn second_step_gets_the_task_and_only_its_declared_fields() {
    let server = model_answering_in_order(vec![completing(EXPLORED), completing(REVIEWED)]).await;
    chain_run(&two_agents(&server.uri()), &["explorer", "reviewer"]);
    let requests = request_texts(&server).await;
    assert_eq!(requests.len(), 2);
    let to_reviewer = &requests[1];
    for expected in ["ship it", "a.rs", "Prompt of reviewer."] {
        assert!(to_reviewer.contains(expected), "missing {expected:?}");
    }
    for hidden in ["PRIVATE-NOTE", "Prompt of explorer."] {
        assert!(!to_reviewer.contains(hidden), "leaked {hidden:?}");
    }
}

#[tokio::test]
async fn incompatible_order_is_refused_before_any_model_call() {
    let server = model_answering(completing(REVIEWED)).await;
    let workspace = two_agents(&server.uri());
    for refused in [
        chain_run(&workspace, &["reviewer", "explorer"]),
        chain_check(&workspace, &["reviewer", "explorer"]),
    ] {
        assert!(!refused.status.success());
        assert_eq!(stdout(&refused), "");
    }
    assert!(request_texts(&server).await.is_empty());
}

#[tokio::test]
async fn compatible_order_passes_the_check_without_a_model_call() {
    let server = model_answering(completing(REVIEWED)).await;
    let output = chain_check(&two_agents(&server.uri()), &["explorer", "reviewer"]);
    assert!(output.status.success());
    assert_eq!(stdout(&output), "ok: explorer → reviewer\n");
    assert!(request_texts(&server).await.is_empty());
}

#[tokio::test]
async fn failed_first_step_stops_the_chain_and_prints_nothing() {
    let server = model_answering(completing(r#"{"files":"not a list"}"#)).await;
    let output = chain_run(&two_agents(&server.uri()), &["explorer", "reviewer"]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    let requests = request_texts(&server).await;
    assert_eq!(requests.len(), TURN_LIMIT);
    assert!(
        requests
            .iter()
            .all(|text| !text.contains("Prompt of reviewer."))
    );
}

#[tokio::test]
async fn failed_last_step_prints_nothing_even_though_earlier_steps_passed() {
    let invalid_review = completing(r#"{"findings":"none"}"#);
    let server = model_answering_in_order(vec![completing(EXPLORED)]).await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(invalid_review)
        .mount(&server)
        .await;
    let output = chain_run(&two_agents(&server.uri()), &["explorer", "reviewer"]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    assert_eq!(request_texts(&server).await.len(), 1 + TURN_LIMIT);
}

#[tokio::test]
async fn plus_joins_agents_into_one_step_and_the_check_shows_them_side_by_side() {
    let server = model_answering(completing(REVIEWED)).await;
    let output = chain_check(
        &two_agents(&server.uri()),
        &["explorer", "reviewer+reviewer"],
    );
    assert!(output.status.success());
    assert_eq!(stdout(&output), "ok: explorer → reviewer + reviewer\n");
    assert!(request_texts(&server).await.is_empty());
}

#[tokio::test]
async fn agents_that_give_the_same_text_field_cannot_run_side_by_side() {
    let server = model_answering(completing(EXPLORED)).await;
    let workspace = two_agents(&server.uri());
    for refused in [
        chain_check(&workspace, &["explorer+explorer"]),
        chain_run(&workspace, &["explorer+explorer"]),
    ] {
        assert!(!refused.status.success());
        assert_eq!(stdout(&refused), "");
        let reason = String::from_utf8_lossy(&refused.stderr);
        assert!(reason.contains("cannot run beside `explorer`"), "{reason}");
    }
    assert!(request_texts(&server).await.is_empty());
}

#[tokio::test]
async fn results_of_agents_working_side_by_side_are_joined() {
    let server = model_answering_in_order(vec![completing(EXPLORED)]).await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(completing(REVIEWED))
        .mount(&server)
        .await;
    let output = chain_run(
        &two_agents(&server.uri()),
        &["explorer", "reviewer+reviewer"],
    );
    assert!(output.status.success());
    let printed: Value = serde_json::from_str(&stdout(&output)).expect("stdout is JSON");
    assert_eq!(
        printed,
        json!({"findings": ["unused import", "unused import"]})
    );
    assert_eq!(request_texts(&server).await.len(), 3);
}
