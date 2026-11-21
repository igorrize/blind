use std::fmt;

use serde::Deserialize;

const MAX_LENGTH: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct AgentName(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("agent name `{0}` must be 1-{max} characters of a-z, 0-9, `-` or `_`", max = MAX_LENGTH)]
pub struct InvalidAgentName(String);

impl AgentName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AgentName {
    type Error = InvalidAgentName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if is_valid(&name) {
            Ok(Self(name))
        } else {
            Err(InvalidAgentName(name))
        }
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_valid(name: &str) -> bool {
    (1..=MAX_LENGTH).contains(&name.len()) && name.chars().all(is_allowed)
}

fn is_allowed(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '-' | '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(name: &str) -> Result<AgentName, InvalidAgentName> {
        AgentName::try_from(name.to_owned())
    }

    #[test]
    fn accepts_lowercase_digits_dash_and_underscore() {
        assert_eq!(
            parse("test_writer-2").expect("valid").as_str(),
            "test_writer-2"
        );
    }

    #[test]
    fn rejects_empty_name() {
        assert!(parse("").is_err());
    }

    #[test]
    fn rejects_uppercase_spaces_and_path_separators() {
        for name in ["Reviewer", "code reviewer", "../reviewer", "a/b"] {
            assert!(parse(name).is_err(), "{name:?} must be rejected");
        }
    }

    #[test]
    fn rejects_name_longer_than_the_limit() {
        assert!(parse(&"a".repeat(MAX_LENGTH)).is_ok());
        assert!(parse(&"a".repeat(MAX_LENGTH + 1)).is_err());
    }
}
