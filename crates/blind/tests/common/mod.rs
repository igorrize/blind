#![allow(dead_code)]

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

pub const TURN_LIMIT: usize = 4;

const KEY_VARIABLE: &str = "BLIND_TEST_KEY";
const SETTINGS: &str = r#"{"description":"Works.","model":"test/any-model"}"#;

pub fn sse(chunks: &[Value]) -> ResponseTemplate {
    let mut body: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

pub fn completing(arguments: &str) -> ResponseTemplate {
    sse(&[json!({"choices": [{"delta": {"tool_calls": [{
        "index": 0, "id": "call_1", "type": "function",
        "function": {"name": "complete", "arguments": arguments}
    }]}}]})])
}

pub fn talking(text: &str) -> ResponseTemplate {
    sse(&[json!({"choices": [{"delta": {"content": text}}]})])
}

pub async fn model_answering(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

pub async fn model_answering_in_order(responses: Vec<ResponseTemplate>) -> MockServer {
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

pub fn workspace(provider_url: &str, contracts: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let config = dir.path().join("config/blind");
    fs::create_dir_all(&config).expect("config dir");
    let providers = format!(
        "[defaults]\nmodel = \"test/any-model\"\n\n[providers.test]\nkind = \"openai-compatible\"\n\
         base_url = \"{provider_url}\"\napi_key = {{ env = \"{KEY_VARIABLE}\" }}\n"
    );
    fs::write(config.join("providers.toml"), providers).expect("providers.toml");
    for (name, contract) in contracts {
        write_agent(&dir.path().join("agents").join(name), name, contract);
    }
    dir
}

fn write_agent(folder: &Path, name: &str, contract: &str) {
    fs::create_dir_all(folder).expect("agent dir");
    fs::write(folder.join("contract.json"), contract).expect("contract.json");
    fs::write(folder.join("agent.json"), SETTINGS).expect("agent.json");
    fs::write(folder.join("prompt.md"), format!("Prompt of {name}.")).expect("prompt.md");
}

pub fn blind(workspace: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_blind"))
        .args(arguments)
        .arg("--dir")
        .arg(workspace.join("agents"))
        .env("XDG_CONFIG_HOME", workspace.join("config"))
        .env(KEY_VARIABLE, "test-key")
        .output()
        .expect("blind runs")
}

pub fn blind_in(workspace: &Path, project: &Path, copies: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_blind"))
        .args(arguments)
        .arg("--dir")
        .arg(workspace.join("agents"))
        .arg("--project")
        .arg(project)
        .env("XDG_CONFIG_HOME", workspace.join("config"))
        .env("XDG_CACHE_HOME", copies)
        .env(KEY_VARIABLE, "test-key")
        .output()
        .expect("blind runs")
}

pub fn calling(tool: &str, arguments: &Value) -> ResponseTemplate {
    sse(&[json!({"choices": [{"delta": {"tool_calls": [{
        "index": 0, "id": "call_1", "type": "function",
        "function": {"name": tool, "arguments": arguments.to_string()}
    }]}}]})])
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub async fn request_texts(server: &MockServer) -> Vec<String> {
    let requests = server.received_requests().await.expect("recorded");
    requests
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect()
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
