pub mod config;
mod openai;
mod sse;

use config::{ProviderKind, ProvidersConfig};

pub use openai::{Message, OpenAiCompatible};

pub fn connect(config: &ProvidersConfig, provider: &str) -> anyhow::Result<OpenAiCompatible> {
    let provider = config.provider(provider)?;
    match provider.kind() {
        ProviderKind::OpenaiCompatible => Ok(OpenAiCompatible::new(
            provider.base_url(),
            provider.api_key()?,
        )),
    }
}
