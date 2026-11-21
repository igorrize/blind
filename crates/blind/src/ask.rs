use std::{
    io::{self, Write},
    process::ExitCode,
};

use blind_core::{ModelRef, Usage};

use crate::providers::{self, Message, OpenAiCompatible, config};
use crate::runtime;

pub fn run(prompt: &str, model: Option<ModelRef>) -> anyhow::Result<ExitCode> {
    let config = config::load()?;
    let model = model.unwrap_or_else(|| config.default_model().clone());
    let client = providers::connect(&config, model.provider())?;
    let mut stdout = io::stdout();
    let answer = print_answer(&client, model.model(), prompt, &mut stdout);
    let usage = runtime::block_on(answer)??;
    eprintln!("{}", usage_line(&model, usage));
    Ok(ExitCode::SUCCESS)
}

async fn print_answer(
    client: &OpenAiCompatible,
    model: &str,
    prompt: &str,
    out: &mut impl Write,
) -> anyhow::Result<Usage> {
    let mut written = Ok(());
    let print = |text: &str| {
        if written.is_ok() {
            written = write!(out, "{text}").and_then(|()| out.flush());
        }
    };
    let usage = client
        .stream_answer(model, &[Message::user(prompt)], print)
        .await?;
    written?;
    writeln!(out)?;
    Ok(usage)
}

fn usage_line(model: &ModelRef, usage: Usage) -> String {
    format!(
        "{model} · tokens: {} in, {} out",
        usage.input_tokens, usage.output_tokens
    )
}

#[cfg(test)]
mod tests {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    use super::*;

    #[tokio::test]
    async fn prints_the_streamed_answer_and_ends_the_line() {
        let server = MockServer::start().await;
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"42\"}}]}\n\ndata: [DONE]\n\n";
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let client = OpenAiCompatible::new(&server.uri(), None);
        let mut out = Vec::new();
        print_answer(&client, "glm-5.3", "answer?", &mut out)
            .await
            .expect("answer");
        assert_eq!(String::from_utf8(out).expect("utf-8"), "42\n");
    }

    #[test]
    fn usage_line_names_the_model_and_token_counts() {
        let model: ModelRef = "ollama-cloud/glm-5.3".parse().expect("valid");
        let usage = Usage {
            input_tokens: 7,
            output_tokens: 3,
        };
        assert_eq!(
            usage_line(&model, usage),
            "ollama-cloud/glm-5.3 · tokens: 7 in, 3 out"
        );
    }
}
