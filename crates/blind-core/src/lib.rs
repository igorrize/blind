mod agent;
mod agent_name;
mod chain;
mod contract;
mod limits;
mod model_ref;
mod reading_tool;
mod schema;
mod usage;

pub use agent::{Agent, AgentError, AgentSettings, CONTRACT_FILE, PROMPT_FILE, SETTINGS_FILE};
pub use agent_name::{AgentName, InvalidAgentName};
pub use chain::{ChainError, check_chain};
pub use contract::{Contract, ContractError, Kind, Rights};
pub use limits::{LimitOutOfRange, Limits};
pub use model_ref::{InvalidModelRef, ModelRef};
pub use reading_tool::ReadingTool;
pub use schema::{Mismatch, ObjectSchema, Schema, SchemaProblem, Violation};
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
