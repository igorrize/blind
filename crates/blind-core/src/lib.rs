mod agent;
mod agent_name;
mod chain;
mod contract;
mod limits;
mod model_ref;
mod reading_tool;
mod root_agent;
mod rule_name;
mod schema;
mod usage;

pub use agent::{Agent, AgentError, AgentSettings, CONTRACT_FILE, PROMPT_FILE, SETTINGS_FILE};
pub use agent_name::{AgentName, InvalidAgentName};
pub use chain::{ChainError, Group, check_chain, check_delivery};
pub use contract::{Contract, ContractError, Kind, Rights};
pub use limits::{LimitOutOfRange, Limits};
pub use model_ref::{InvalidModelRef, ModelRef};
pub use reading_tool::ReadingTool;
pub use root_agent::{RootAgent, RootSettings};
pub use rule_name::{InvalidRuleName, RuleName};
pub use schema::{Clash, Mismatch, ObjectSchema, Schema, SchemaProblem, Violation, merged_outputs};
pub use usage::Usage;

pub const NAME: &str = "blind";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comes_from_cargo() {
        assert!(!VERSION.is_empty());
        assert_eq!(NAME, "blind");
    }
}
