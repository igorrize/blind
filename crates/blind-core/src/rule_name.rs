use std::fmt;

use serde::Deserialize;

use crate::agent_name::{MAX_LENGTH, is_valid};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct RuleName(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("rule name `{0}` must be 1-{max} characters of a-z, 0-9, `-` or `_`", max = MAX_LENGTH)]
pub struct InvalidRuleName(String);

impl RuleName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RuleName {
    type Error = InvalidRuleName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if is_valid(&name) {
            Ok(Self(name))
        } else {
            Err(InvalidRuleName(name))
        }
    }
}

impl fmt::Display for RuleName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_same_names_as_agents() {
        let name = RuleName::try_from("rust-2024".to_owned()).expect("valid");
        assert_eq!(name.as_str(), "rust-2024");
    }

    #[test]
    fn rejects_names_that_could_leave_the_rules_folder() {
        for name in ["../secrets", "a/b", "", "Rust", ".hidden"] {
            assert!(RuleName::try_from(name.to_owned()).is_err(), "{name:?}");
        }
    }
}
