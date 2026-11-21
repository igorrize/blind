use blind_core::Usage;
use reqwest::{RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::ApiKey;
use super::sse::SseLines;

const END_OF_STREAM: &str = "[DONE]";
const ERROR_BODY_LIMIT: usize = 500;
const FUNCTION: &str = "function";

pub struct OpenAiCompatible {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<ApiKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    role: Role,
    content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("provider answered {status}: {body}")]
    Status { status: u16, body: String },
    #[error("provider sent a chunk that is not valid JSON: {0}")]
    BadChunk(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: &'a ToolSpec,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    stream: bool,
    stream_options: StreamOptions,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
}

#[derive(Default, Deserialize)]
struct Delta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    #[serde(default)]
    function: FunctionDelta,
}

#[derive(Default, Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<Value>,
}

#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Default)]
struct TurnBuilder {
    turn: Turn,
}

impl Message {
    pub fn system(content: &str) -> Self {
        Self::text(Role::System, content)
    }

    pub fn user(content: &str) -> Self {
        Self::text(Role::User, content)
    }

    pub fn assistant(content: &str) -> Self {
        Self::text(Role::Assistant, content)
    }

    pub fn assistant_turn(turn: &Turn) -> Self {
        Self {
            tool_calls: turn.tool_calls.iter().map(WireToolCall::from).collect(),
            ..Self::text(Role::Assistant, &turn.text)
        }
    }

    pub fn tool_result(call: &ToolCall, content: &str) -> Self {
        Self {
            tool_call_id: Some(call.id.clone()),
            ..Self::text(Role::Tool, content)
        }
    }

    fn text(role: Role, content: &str) -> Self {
        Self {
            role,
            content: content.to_owned(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

impl From<&ToolCall> for WireToolCall {
    fn from(call: &ToolCall) -> Self {
        Self {
            id: call.id.clone(),
            kind: FUNCTION,
            function: WireFunction {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            },
        }
    }
}

impl<'a> From<&'a ToolSpec> for WireTool<'a> {
    fn from(function: &'a ToolSpec) -> Self {
        Self {
            kind: FUNCTION,
            function,
        }
    }
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, api_key: Option<ApiKey>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
        }
    }

    pub async fn model_ids(&self) -> Result<Vec<String>, ProviderError> {
        let request = self.authorized(self.http.get(self.url("models")));
        let response = successful(request.send().await?).await?;
        let list: ModelList = response.json().await?;
        let mut ids: Vec<String> = list.data.into_iter().map(|entry| entry.id).collect();
        ids.sort();
        Ok(ids)
    }

    pub async fn stream_answer(
        &self,
        model: &str,
        messages: &[Message],
        on_text: impl FnMut(&str),
    ) -> Result<Usage, ProviderError> {
        let turn = self.turn(model, messages, &[], on_text).await?;
        Ok(turn.usage)
    }

    pub async fn turn(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        mut on_text: impl FnMut(&str),
    ) -> Result<Turn, ProviderError> {
        let body = ChatRequest {
            model,
            messages,
            tools: tools.iter().map(WireTool::from).collect(),
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
        };
        let request = self.authorized(self.http.post(self.url("chat/completions")));
        let mut response = successful(request.json(&body).send().await?).await?;
        let mut lines = SseLines::default();
        let mut builder = TurnBuilder::default();
        while let Some(bytes) = response.chunk().await? {
            for payload in lines.push(&bytes) {
                builder.absorb(&payload, &mut on_text)?;
            }
        }
        Ok(builder.finish())
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url)
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.api_key {
            Some(key) => request.bearer_auth(key.expose()),
            None => request,
        }
    }
}

impl TurnBuilder {
    fn absorb(
        &mut self,
        payload: &str,
        on_text: &mut impl FnMut(&str),
    ) -> Result<(), ProviderError> {
        if payload == END_OF_STREAM {
            return Ok(());
        }
        let chunk: ChatChunk = serde_json::from_str(payload)?;
        if let Some(reported) = chunk.usage {
            self.turn.usage = Usage {
                input_tokens: reported.prompt_tokens,
                output_tokens: reported.completion_tokens,
            };
        }
        for choice in chunk.choices {
            self.absorb_delta(choice.delta, on_text);
        }
        Ok(())
    }

    fn absorb_delta(&mut self, delta: Delta, on_text: &mut impl FnMut(&str)) {
        if let Some(text) = delta.content.filter(|text| !text.is_empty()) {
            on_text(&text);
            self.turn.text.push_str(&text);
        }
        for call in delta.tool_calls {
            self.absorb_call(call);
        }
    }

    fn absorb_call(&mut self, delta: ToolCallDelta) {
        let position = self.position_of(&delta);
        while self.turn.tool_calls.len() <= position {
            self.turn.tool_calls.push(ToolCall::default());
        }
        let call = &mut self.turn.tool_calls[position];
        if let Some(id) = delta.id.filter(|id| !id.is_empty()) {
            call.id = id;
        }
        call.name.push_str(&delta.function.name.unwrap_or_default());
        call.arguments
            .push_str(&arguments_text(delta.function.arguments));
    }

    fn position_of(&self, delta: &ToolCallDelta) -> usize {
        let started = self.turn.tool_calls.len();
        match delta.index {
            Some(index) => index,
            None if delta.id.is_some() => started,
            None => started.saturating_sub(1),
        }
    }

    fn finish(mut self) -> Turn {
        for (position, call) in self.turn.tool_calls.iter_mut().enumerate() {
            if call.id.is_empty() {
                call.id = format!("call_{position}");
            }
        }
        self.turn
    }
}

fn arguments_text(arguments: Option<Value>) -> String {
    match arguments {
        Some(Value::String(text)) => text,
        Some(structured) => structured.to_string(),
        None => String::new(),
    }
}

async fn successful(response: Response) -> Result<Response, ProviderError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(ProviderError::Status {
        status: status.as_u16(),
        body: body.chars().take(ERROR_BODY_LIMIT).collect(),
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use serde_json::json;
    use wiremock::ResponseTemplate;

    pub fn sse(chunks: &[serde_json::Value]) -> ResponseTemplate {
        let mut body: String = chunks
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .collect();
        body.push_str("data: [DONE]\n\n");
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
    }

    pub fn text_chunk(text: &str) -> serde_json::Value {
        json!({"choices": [{"delta": {"content": text}}]})
    }

    pub fn usage_chunk(input: u64, output: u64) -> serde_json::Value {
        json!({"choices": [], "usage": {"prompt_tokens": input, "completion_tokens": output}})
    }

    pub fn tool_call_chunk(id: &str, name: &str, arguments: &str) -> serde_json::Value {
        json!({"choices": [{"delta": {"tool_calls": [{
            "index": 0, "id": id, "type": "function",
            "function": {"name": name, "arguments": arguments}
        }]}}]})
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path},
    };

    use super::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};
    use super::*;
    use crate::providers::config::test_support::api_key;

    async fn serving(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    async fn answer(client: &OpenAiCompatible) -> Result<(String, Usage), ProviderError> {
        let mut text = String::new();
        let usage = client
            .stream_answer("glm-5.3", &[Message::user("hi")], |part| {
                text.push_str(part);
            })
            .await?;
        Ok((text, usage))
    }

    async fn turn_with_tools(server: &MockServer) -> Turn {
        let tool = ToolSpec {
            name: "complete",
            description: "Finish the job.",
            parameters: json!({"type": "object", "properties": {}}),
        };
        OpenAiCompatible::new(&server.uri(), None)
            .turn("glm-5.3", &[Message::user("go")], &[tool], |_| {})
            .await
            .expect("turn")
    }

    #[tokio::test]
    async fn streams_text_and_reports_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(json!({
                "model": "glm-5.3",
                "stream": true,
                "messages": [{"role": "user", "content": "hi"}]
            })))
            .respond_with(sse(&[
                text_chunk(""),
                text_chunk("Hello"),
                text_chunk(", world"),
                usage_chunk(7, 3),
            ]))
            .mount(&server)
            .await;
        let client =
            OpenAiCompatible::new(&format!("{}/v1/", server.uri()), Some(api_key("test-key")));
        let (text, usage) = answer(&client).await.expect("answer");
        assert_eq!(text, "Hello, world");
        assert_eq!(
            usage,
            Usage {
                input_tokens: 7,
                output_tokens: 3
            }
        );
    }

    #[tokio::test]
    async fn rejected_key_is_reported_with_status_and_body() {
        let server = serving(ResponseTemplate::new(401).set_body_string("unauthorized")).await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        let error = answer(&client).await.expect_err("must fail");
        assert_eq!(error.to_string(), "provider answered 401: unauthorized");
    }

    #[tokio::test]
    async fn broken_chunk_is_an_error() {
        let broken =
            ResponseTemplate::new(200).set_body_raw("data: {not json}\n\n", "text/event-stream");
        let server = serving(broken).await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        let error = answer(&client).await.expect_err("must fail");
        assert!(matches!(error, ProviderError::BadChunk(_)), "{error}");
    }

    #[tokio::test]
    async fn lists_model_ids_sorted() {
        let server = MockServer::start().await;
        let models = json!({"data": [{"id": "kimi"}, {"id": "glm"}]});
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(models))
            .mount(&server)
            .await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        assert_eq!(client.model_ids().await.expect("models"), ["glm", "kimi"]);
    }

    #[tokio::test]
    async fn tools_are_offered_as_functions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"tools": [{
                "type": "function",
                "function": {"name": "complete", "description": "Finish the job."}
            }]})))
            .respond_with(sse(&[text_chunk("ok")]))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(turn_with_tools(&server).await.text, "ok");
    }

    #[tokio::test]
    async fn tool_call_split_across_chunks_is_joined() {
        let server = serving(sse(&[
            tool_call_chunk("call_1", "complete", "{\"steps\":"),
            tool_call_chunk("", "", "[\"a\"]}"),
        ]))
        .await;
        let turn = turn_with_tools(&server).await;
        assert_eq!(
            turn.tool_calls,
            [ToolCall {
                id: "call_1".to_owned(),
                name: "complete".to_owned(),
                arguments: "{\"steps\":[\"a\"]}".to_owned()
            }]
        );
    }

    #[tokio::test]
    async fn structured_arguments_and_missing_ids_are_tolerated() {
        let chunk = json!({"choices": [{"delta": {"tool_calls": [{
            "function": {"name": "complete", "arguments": {"done": true}}
        }]}}]});
        let server = serving(sse(&[chunk])).await;
        let turn = turn_with_tools(&server).await;
        assert_eq!(turn.tool_calls[0].id, "call_0");
        assert_eq!(turn.tool_calls[0].arguments, "{\"done\":true}");
    }

    #[test]
    fn tool_messages_carry_the_call_and_its_result() {
        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "complete".to_owned(),
            arguments: "{}".to_owned(),
        };
        let turn = Turn {
            tool_calls: vec![call.clone()],
            ..Turn::default()
        };
        assert_eq!(
            serde_json::to_value(Message::assistant_turn(&turn)).expect("json"),
            json!({"role": "assistant", "content": "", "tool_calls": [{
                "id": "call_1", "type": "function",
                "function": {"name": "complete", "arguments": "{}"}
            }]})
        );
        assert_eq!(
            serde_json::to_value(Message::tool_result(&call, "rejected")).expect("json"),
            json!({"role": "tool", "content": "rejected", "tool_call_id": "call_1"})
        );
    }
}
