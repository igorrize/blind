use std::{
    collections::BTreeMap,
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use blind_core::ModelRef;
use serde::Deserialize;

const CONFIG_FILE: &str = "blind/providers.toml";

pub const SAMPLE: &str = r#"[defaults]
model = "ollama-cloud/glm-5.3"

[providers.ollama-cloud]
kind = "openai-compatible"
base_url = "https://ollama.com/v1"
api_key = { env = "OLLAMA_API_KEY" }
"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvidersConfig {
    defaults: Defaults,
    providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Defaults {
    model: ModelRef,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    kind: ProviderKind,
    base_url: String,
    api_key: Option<KeySource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenaiCompatible,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeySource {
    env: String,
}

pub struct ApiKey(String);

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("not a valid providers config: {0}")]
    Malformed(#[from] toml::de::Error),
    #[error("provider `{name}` is not configured; configured: {known:?}")]
    UnknownProvider { name: String, known: Vec<String> },
    #[error("environment variable `{0}` with the API key is not set")]
    MissingKey(String),
}

impl ProvidersConfig {
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.provider(config.defaults.model.provider())?;
        Ok(config)
    }

    pub fn default_model(&self) -> &ModelRef {
        &self.defaults.model
    }

    pub fn provider(&self, name: &str) -> Result<&ProviderConfig, ConfigError> {
        self.providers
            .get(name)
            .ok_or_else(|| ConfigError::UnknownProvider {
                name: name.to_owned(),
                known: self.providers.keys().cloned().collect(),
            })
    }
}

impl ProviderConfig {
    pub fn kind(&self) -> ProviderKind {
        self.kind
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn api_key(&self) -> Result<Option<ApiKey>, ConfigError> {
        let Some(source) = &self.api_key else {
            return Ok(None);
        };
        match env::var(&source.env) {
            Ok(key) if !key.trim().is_empty() => Ok(Some(ApiKey(key))),
            Ok(_) | Err(_) => Err(ConfigError::MissingKey(source.env.clone())),
        }
    }
}

impl ApiKey {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(***)")
    }
}

pub fn load() -> anyhow::Result<ProvidersConfig> {
    let path = default_path()?;
    let text = read(&path)?;
    ProvidersConfig::from_toml(&text).with_context(|| format!("in {}", path.display()))
}

pub fn save_default_model(path: &Path, model: &ModelRef) -> anyhow::Result<()> {
    let updated = with_default_model(&read(path)?, model)?;
    fs::write(path, updated).with_context(|| format!("cannot write {}", path.display()))
}

fn with_default_model(text: &str, model: &ModelRef) -> anyhow::Result<String> {
    let mut document: toml_edit::DocumentMut = text.parse()?;
    document["defaults"]["model"] = toml_edit::value(model.to_string());
    Ok(document.to_string())
}

pub fn default_path() -> anyhow::Result<PathBuf> {
    let config_home = match env::var_os("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => home_dir()?.join(".config"),
    };
    Ok(config_home.join(CONFIG_FILE))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    let home = env::var_os("HOME").context("HOME is not set, cannot find the config")?;
    Ok(PathBuf::from(home))
}

fn read(path: &Path) -> anyhow::Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(missing_config(path)),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn missing_config(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "no providers config yet. Create {} with:\n\n{SAMPLE}",
        path.display()
    )
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::ApiKey;

    pub fn api_key(value: &str) -> ApiKey {
        ApiKey(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_the_default_model_keeps_the_rest_of_the_file() {
        let text = format!("# my providers\n{SAMPLE}");
        let model: ModelRef = "ollama-cloud/glm-5.3-flash".parse().expect("valid");
        let updated = with_default_model(&text, &model).expect("valid toml");
        assert_eq!(
            updated,
            text.replace("ollama-cloud/glm-5.3\"", "ollama-cloud/glm-5.3-flash\"")
        );
    }

    #[test]
    fn saved_default_model_is_read_back() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.toml");
        fs::write(&path, SAMPLE).expect("write config");
        let model: ModelRef = "ollama-cloud/kimi-k3.1-pro".parse().expect("valid");
        save_default_model(&path, &model).expect("saved");
        let saved = fs::read_to_string(&path).expect("read config");
        let config = ProvidersConfig::from_toml(&saved).expect("valid config");
        assert_eq!(config.default_model(), &model);
    }

    fn rejection(text: &str) -> String {
        ProvidersConfig::from_toml(text)
            .expect_err("config must be rejected")
            .to_string()
    }

    #[test]
    fn sample_config_is_valid() {
        let config = ProvidersConfig::from_toml(SAMPLE).expect("valid config");
        assert_eq!(config.default_model().to_string(), "ollama-cloud/glm-5.3");
        let provider = config.provider("ollama-cloud").expect("configured");
        assert_eq!(provider.kind(), ProviderKind::OpenaiCompatible);
        assert_eq!(provider.base_url(), "https://ollama.com/v1");
    }

    #[test]
    fn default_model_must_use_a_configured_provider() {
        let error = rejection(&SAMPLE.replace("ollama-cloud/glm-5.3", "openai/gpt"));
        assert_eq!(
            error,
            r#"provider `openai` is not configured; configured: ["ollama-cloud"]"#
        );
    }

    #[test]
    fn rejects_unknown_provider_kind() {
        let error = rejection(&SAMPLE.replace("openai-compatible", "anthropic"));
        assert!(error.contains("unknown variant `anthropic`"), "{error}");
    }

    #[test]
    fn rejects_a_key_written_into_the_file() {
        let error = rejection(&SAMPLE.replace(r#"{ env = "OLLAMA_API_KEY" }"#, r#""sk-secret""#));
        assert!(error.contains("api_key"), "{error}");
    }

    #[test]
    fn provider_without_key_source_needs_no_key() {
        let text = SAMPLE.replace("api_key = { env = \"OLLAMA_API_KEY\" }\n", "");
        let config = ProvidersConfig::from_toml(&text).expect("valid config");
        let key = config
            .provider("ollama-cloud")
            .expect("configured")
            .api_key();
        assert!(matches!(key, Ok(None)));
    }

    #[test]
    fn unset_key_variable_is_reported_by_name() {
        let text = SAMPLE.replace("OLLAMA_API_KEY", "BLIND_TEST_KEY_THAT_IS_NEVER_SET");
        let config = ProvidersConfig::from_toml(&text).expect("valid config");
        let error = config
            .provider("ollama-cloud")
            .expect("configured")
            .api_key()
            .expect_err("key is not set");
        assert_eq!(
            error.to_string(),
            "environment variable `BLIND_TEST_KEY_THAT_IS_NEVER_SET` with the API key is not set"
        );
    }

    #[test]
    fn api_key_is_hidden_in_debug_output() {
        assert_eq!(
            format!("{:?}", ApiKey("sk-secret".to_owned())),
            "ApiKey(***)"
        );
    }
}
