use std::time::Duration;

use blind_core::Usage;
use serde_json::Value;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use super::*;
use crate::providers::openai::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};

const PATIENCE: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(300);

fn next_event(backend: &Backend) -> BackendEvent {
    backend
        .events
        .recv_timeout(PATIENCE)
        .expect("an event in time")
}

fn long_conversation() -> Conversation {
    let mut conversation = Conversation::default();
    for turn in 0..5 {
        conversation.say(format!(
            "please run `cargo test` and edit src/lib.rs, try {turn}"
        ));
        conversation.receive(ReplyEvent::Text("I cannot do that myself.".to_owned()));
        conversation.receive(ReplyEvent::Done(Usage::default()));
    }
    conversation.say("then call a worker".to_owned());
    conversation
}

async fn backend_answering(response: ResponseTemplate) -> (MockServer, Backend) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    let client = OpenAiCompatible::new(&server.uri(), None);
    let backend = Backend::start(client, "glm-5.3".to_owned()).expect("backend starts");
    (server, backend)
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    let requests = server.received_requests().await.expect("recorded");
    requests
        .iter()
        .map(|request| serde_json::from_slice(&request.body).expect("json body"))
        .collect()
}

#[tokio::test]
async fn root_is_never_offered_a_tool() {
    let (server, backend) = backend_answering(sse(&[text_chunk("ok"), usage_chunk(1, 1)])).await;
    assert!(backend.ask(&long_conversation()));
    next_event(&backend);
    next_event(&backend);
    let bodies = request_bodies(&server).await;
    assert_eq!(bodies.len(), 1);
    let request = bodies[0].as_object().expect("object");
    assert!(!request.contains_key("tools"), "{request:?}");
    assert!(!request.contains_key("tool_choice"), "{request:?}");
}

#[tokio::test]
async fn root_history_has_no_tool_messages() {
    let (server, backend) = backend_answering(sse(&[text_chunk("ok")])).await;
    assert!(backend.ask(&long_conversation()));
    next_event(&backend);
    let bodies = request_bodies(&server).await;
    let messages = bodies[0]["messages"].as_array().expect("messages");
    for message in messages {
        let role = message["role"].as_str().expect("role");
        assert!(["system", "user", "assistant"].contains(&role), "{message}");
        assert!(message.get("tool_calls").is_none(), "{message}");
    }
}

#[tokio::test]
async fn tool_call_in_roots_answer_runs_nothing() {
    let reaching_for_shell = sse(&[
        tool_call_chunk("call_1", "bash", r#"{"command":"rm -rf ."}"#),
        usage_chunk(4, 2),
    ]);
    let (server, backend) = backend_answering(reaching_for_shell).await;
    assert!(backend.ask(&long_conversation()));
    let usage = Usage {
        input_tokens: 4,
        output_tokens: 2,
    };
    assert_eq!(
        next_event(&backend),
        BackendEvent::Reply(ReplyEvent::Done(usage))
    );
    assert!(backend.events.recv_timeout(QUIET).is_err());
    assert_eq!(request_bodies(&server).await.len(), 1);
}
