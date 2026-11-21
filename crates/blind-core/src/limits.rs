use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    pub turns: u32,
    pub tokens: u64,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("limits.{name} must be between 1 and {ceiling}")]
pub struct LimitOutOfRange {
    name: &'static str,
    ceiling: u64,
}

impl Limits {
    pub const CEILING: Limits = Limits {
        turns: 60,
        tokens: 1_000_000,
        tool_calls: 100,
    };

    pub fn check(&self) -> Result<(), LimitOutOfRange> {
        let ceiling = Self::CEILING;
        in_range("turns", u64::from(self.turns), u64::from(ceiling.turns))?;
        in_range("tokens", self.tokens, ceiling.tokens)?;
        in_range(
            "tool_calls",
            u64::from(self.tool_calls),
            u64::from(ceiling.tool_calls),
        )
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            turns: 30,
            tokens: 300_000,
            tool_calls: 20,
        }
    }
}

fn in_range(name: &'static str, value: u64, ceiling: u64) -> Result<(), LimitOutOfRange> {
    if (1..=ceiling).contains(&value) {
        Ok(())
    } else {
        Err(LimitOutOfRange { name, ceiling })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(json: &str) -> Limits {
        serde_json::from_str(json).expect("valid limits")
    }

    #[test]
    fn missing_limits_take_the_defaults() {
        assert_eq!(parsed("{}"), Limits::default());
        let tighter = parsed(r#"{"tool_calls": 5}"#);
        assert_eq!(tighter.tool_calls, 5);
        assert_eq!(tighter.tokens, Limits::default().tokens);
    }

    #[test]
    fn defaults_and_ceiling_are_within_range() {
        assert_eq!(Limits::default().check(), Ok(()));
        assert_eq!(Limits::CEILING.check(), Ok(()));
    }

    #[test]
    fn limit_above_the_ceiling_is_rejected() {
        let greedy = parsed(r#"{"tokens": 1000001}"#);
        let problem = greedy.check().expect_err("above the ceiling");
        assert_eq!(
            problem.to_string(),
            "limits.tokens must be between 1 and 1000000"
        );
    }

    #[test]
    fn zero_limit_is_rejected() {
        assert!(parsed(r#"{"turns": 0}"#).check().is_err());
        assert!(parsed(r#"{"tool_calls": 0}"#).check().is_err());
    }

    #[test]
    fn unknown_limit_is_rejected() {
        assert!(serde_json::from_str::<Limits>(r#"{"dollars": 5}"#).is_err());
    }
}
