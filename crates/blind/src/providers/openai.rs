use blind_core::Usage;
use reqwest::{RequestBuilder, Response};
use serde::{Deserialize, Serialize};

use super::config::ApiKey;
use super::sse::SseLines;

const END_OF_STREAM: &str = "[DONE]";
const ERROR_BODY_LIMIT: usize = 500;

pub struct OpenAiCompatible {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<ApiKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    role: Role,
    content: String,
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

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
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

impl Message {
    pub fn system(content: &str) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: &str) -> Self {
        Self::new(Role::User, content)
    }

    pub fn assistant(content: &str) -> Self {
        Self::new(Role::Assistant, content)
    }

    fn new(role: Role, content: &str) -> Self {
        Self {
            role,
            content: content.to_owned(),
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
        mut on_text: impl FnMut(&str),
    ) -> Result<Usage, ProviderError> {
        let body = ChatRequest {
            model,
            messages,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
        };
        let request = self.authorized(self.http.post(self.url("chat/completions")));
        let mut response = successful(request.json(&body).send().await?).await?;
        let mut lines = SseLines::default();
        let mut usage = Usage::default();
        while let Some(bytes) = response.chunk().await? {
            for payload in lines.push(&bytes) {
                apply(&payload, &mut usage, &mut on_text)?;
            }
        }
        Ok(usage)
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

fn apply(
    payload: &str,
    usage: &mut Usage,
    on_text: &mut impl FnMut(&str),
) -> Result<(), ProviderError> {
    if payload == END_OF_STREAM {
        return Ok(());
    }
    let chunk: ChatChunk = serde_json::from_str(payload)?;
    if let Some(reported) = chunk.usage {
        *usage = Usage {
            input_tokens: reported.prompt_tokens,
            output_tokens: reported.completion_tokens,
        };
    }
    chunk
        .choices
        .iter()
        .filter_map(|choice| choice.delta.content.as_deref())
        .filter(|text| !text.is_empty())
        .for_each(on_text);
    Ok(())
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
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path},
    };

    use super::*;
    use crate::providers::config::test_support::api_key;

    const TWO_WORDS_THEN_USAGE: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\", world\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
        "data: [DONE]\n\n",
    );

    fn stream(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
    }

    async fn answer(client: &OpenAiCompatible) -> Result<(String, Usage), ProviderError> {
        let mut text = String::new();
        let usage = client
            .stream_answer("glm-5.3", &[Message::user("hi")], |part| {
                text.push_str(part)
            })
            .await?;
        Ok((text, usage))
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
            .respond_with(stream(TWO_WORDS_THEN_USAGE))
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
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        let error = answer(&client).await.expect_err("must fail");
        assert_eq!(error.to_string(), "provider answered 401: unauthorized");
    }

    #[tokio::test]
    async fn broken_chunk_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(stream("data: {not json}\n\n"))
            .mount(&server)
            .await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        let error = answer(&client).await.expect_err("must fail");
        assert!(matches!(error, ProviderError::BadChunk(_)), "{error}");
    }

    #[tokio::test]
    async fn lists_model_ids_sorted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"id": "kimi"}, {"id": "glm"}]})),
            )
            .mount(&server)
            .await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        assert_eq!(client.model_ids().await.expect("models"), ["glm", "kimi"]);
    }
}
