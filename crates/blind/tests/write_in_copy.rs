mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use common::{
    blind_in, calling, completing, model_answering_in_order, request_texts, stderr, stdout,
    workspace,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const WRITER: &str = r#"{"name":"writer","kind":"worker",
    "input":{"type":"object","properties":{}},
    "output":{"type":"object","required":["changed","summary"],"properties":{
        "changed":{"type":"array","items":{"type":"string"}},
        "summary":{"type":"string"}}},
    "rights":{"read":["**"],"write":["src/**"],"written_as":"changed"}}"#;
const REVIEWER: &str = r#"{"name":"reviewer","kind":"worker",
    "input":{"type":"object","required":["changed"],"properties":{
        "changed":{"type":"array","items":{"type":"string"}}}},
    "output":{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","items":{"type":"string"}}}},
    "rights":{"read":["**"],"read_from":"changed"}}"#;
const WRITTEN: &str = r#"{"summary":"added the clipboard"}"#;
const REVIEWED: &str = r#"{"findings":["looks fine"]}"#;
const LOCAL_WORK: &str = "fn main() { run(); }\n";

struct Scene {
    workspace: TempDir,
    project: TempDir,
    copies: TempDir,
}

fn git(dir: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(arguments)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned()
}

fn repository_with_local_work() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "tester"]);
    git(dir.path(), &["config", "user.email", "tester@localhost"]);
    fs::create_dir(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("main.rs");
    fs::write(dir.path().join("README.md"), "# demo\n").expect("readme");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    fs::write(dir.path().join("src/main.rs"), LOCAL_WORK).expect("local work");
    dir
}

fn scene(provider_url: &str, project: TempDir) -> Scene {
    Scene {
        workspace: workspace(provider_url, &[("writer", WRITER), ("reviewer", REVIEWER)]),
        project,
        copies: tempfile::tempdir().expect("copies"),
    }
}

fn run_chain(scene: &Scene, agents: &[&str]) -> std::process::Output {
    let arguments = [&["chain", "run"], agents, &["--task", "add a clipboard"]].concat();
    blind_in(
        scene.workspace.path(),
        scene.project.path(),
        scene.copies.path(),
        &arguments,
    )
}

fn writing(path: &str, content: &str) -> wiremock::ResponseTemplate {
    calling("write_file", &json!({"path": path, "content": content}))
}

fn copy_named_in(report: &str) -> PathBuf {
    let line = report
        .lines()
        .find_map(|line| line.strip_prefix("copy:"))
        .unwrap_or_else(|| panic!("no copy in the report: {report}"));
    PathBuf::from(line.trim())
}

#[tokio::test]
async fn writer_works_in_a_copy_and_the_project_stays_as_it_was() {
    let server = model_answering_in_order(vec![
        writing("src/clipboard.rs", "pub fn copy() {}\n"),
        completing(WRITTEN),
        completing(REVIEWED),
    ])
    .await;
    let scene = scene(&server.uri(), repository_with_local_work());
    let status_before = git(scene.project.path(), &["status", "--porcelain"]);
    let output = run_chain(&scene, &["writer", "reviewer"]);
    let report = stderr(&output);
    assert!(output.status.success(), "{report}");
    let printed: Value = serde_json::from_str(&stdout(&output)).expect("stdout is JSON");
    assert_eq!(printed, json!({"findings": ["looks fine"]}));
    assert!(!scene.project.path().join("src/clipboard.rs").exists());
    assert_eq!(
        fs::read_to_string(scene.project.path().join("src/main.rs")).expect("main.rs"),
        LOCAL_WORK
    );
    assert_eq!(
        git(scene.project.path(), &["status", "--porcelain"]),
        status_before
    );
    for expected in [
        "your project is untouched",
        "changed: src/clipboard.rs",
        "look:    git diff",
        "take:    git diff",
    ] {
        assert!(
            report.contains(expected),
            "missing {expected:?} in {report}"
        );
    }
}

#[tokio::test]
async fn copy_holds_the_local_work_and_what_the_writer_added() {
    let server = model_answering_in_order(vec![
        writing("src/clipboard.rs", "pub fn copy() {}\n"),
        completing(WRITTEN),
        completing(REVIEWED),
    ])
    .await;
    let scene = scene(&server.uri(), repository_with_local_work());
    let output = run_chain(&scene, &["writer", "reviewer"]);
    let copy = copy_named_in(&stderr(&output));
    assert!(copy.starts_with(scene.copies.path()), "{}", copy.display());
    assert_eq!(
        fs::read_to_string(copy.join("src/clipboard.rs")).expect("clipboard.rs"),
        "pub fn copy() {}\n"
    );
    assert_eq!(
        fs::read_to_string(copy.join("src/main.rs")).expect("main.rs"),
        LOCAL_WORK
    );
    let branches = git(scene.project.path(), &["branch", "--list", "blind/*"]);
    assert!(branches.contains("blind/"), "{branches}");
}

#[tokio::test]
async fn next_step_is_told_by_the_harness_which_files_were_written() {
    let server = model_answering_in_order(vec![
        writing("src/clipboard.rs", "pub fn copy() {}\n"),
        completing(WRITTEN),
        completing(REVIEWED),
    ])
    .await;
    let scene = scene(&server.uri(), repository_with_local_work());
    run_chain(&scene, &["writer", "reviewer"]);
    let requests = request_texts(&server).await;
    assert_eq!(requests.len(), 3);
    let to_reviewer = &requests[2];
    assert!(to_reviewer.contains("Prompt of reviewer."), "{to_reviewer}");
    assert!(to_reviewer.contains("src/clipboard.rs"), "{to_reviewer}");
}

#[tokio::test]
async fn refused_writes_leave_no_copy_behind() {
    let server = model_answering_in_order(vec![
        writing("README.md", "rewritten"),
        writing(".git/config", "rewritten"),
        writing("../outside.rs", "rewritten"),
        completing(WRITTEN),
    ])
    .await;
    let scene = scene(&server.uri(), repository_with_local_work());
    let output = run_chain(&scene, &["writer"]);
    let report = stderr(&output);
    assert!(output.status.success(), "{report}");
    assert!(
        report.contains("no files were changed; the copy is removed"),
        "{report}"
    );
    assert_eq!(report.matches('✗').count(), 3, "{report}");
    assert_eq!(
        git(scene.project.path(), &["branch", "--list", "blind/*"]),
        ""
    );
    assert_eq!(
        fs::read_to_string(scene.project.path().join("README.md")).expect("readme"),
        "# demo\n"
    );
    let printed: Value = serde_json::from_str(&stdout(&output)).expect("stdout is JSON");
    assert_eq!(printed["changed"], json!([]));
}

#[tokio::test]
async fn writer_outside_a_git_repository_is_refused_before_any_model_call() {
    let server = model_answering_in_order(vec![completing(WRITTEN)]).await;
    let scene = scene(&server.uri(), tempfile::tempdir().expect("plain folder"));
    let output = run_chain(&scene, &["writer"]);
    assert!(!output.status.success());
    assert_eq!(stdout(&output), "");
    let report = stderr(&output);
    assert!(
        report.contains("agents that write need a git repository"),
        "{report}"
    );
    assert!(request_texts(&server).await.is_empty());
}
