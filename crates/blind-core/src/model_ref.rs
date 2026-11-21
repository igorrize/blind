use std::{fmt, str::FromStr};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct ModelRef {
    provider: String,
    model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("model `{0}` must look like `provider/model`")]
pub struct InvalidModelRef(String);

impl ModelRef {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

impl TryFrom<String> for ModelRef {
    type Error = InvalidModelRef;

    fn try_from(reference: String) -> Result<Self, Self::Error> {
        match reference.split_once('/') {
            Some((provider, model)) if !provider.is_empty() && !model.is_empty() => Ok(Self {
                provider: provider.to_owned(),
                model: model.to_owned(),
            }),
            Some(_) | None => Err(InvalidModelRef(reference)),
        }
    }
}

impl FromStr for ModelRef {
    type Err = InvalidModelRef;

    fn from_str(reference: &str) -> Result<Self, Self::Err> {
        Self::try_from(reference.to_owned())
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.provider, self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(reference: &str) -> Result<ModelRef, InvalidModelRef> {
        ModelRef::try_from(reference.to_owned())
    }

    #[test]
    fn splits_provider_from_model_at_the_first_slash() {
        let reference = parse("openrouter/vendor/model:tag").expect("valid");
        assert_eq!(reference.provider(), "openrouter");
        assert_eq!(reference.model(), "vendor/model:tag");
        assert_eq!(reference.to_string(), "openrouter/vendor/model:tag");
    }

    #[test]
    fn rejects_reference_without_provider_or_model() {
        for reference in ["", "gemma4", "/gemma4", "ollama-cloud/"] {
            assert!(parse(reference).is_err(), "{reference:?} must be rejected");
        }
    }
}
