use serde::Deserialize;

use crate::contract::{Contract, ContractError};
use crate::model_ref::ModelRef;

pub const CONTRACT_FILE: &str = "contract.json";
pub const SETTINGS_FILE: &str = "agent.json";
pub const PROMPT_FILE: &str = "prompt.md";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettings {
    description: String,
    model: ModelRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    contract: Contract,
    settings: AgentSettings,
    prompt: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("{file}: {0}", file = CONTRACT_FILE)]
    Contract(#[from] ContractError),
    #[error("{file}: {0}", file = SETTINGS_FILE)]
    Settings(#[from] serde_json::Error),
    #[error("{file}: description is empty", file = SETTINGS_FILE)]
    EmptyDescription,
    #[error("{file} is empty", file = PROMPT_FILE)]
    EmptyPrompt,
}

impl Agent {
    pub fn from_parts(
        contract_json: &str,
        settings_json: &str,
        prompt: String,
    ) -> Result<Self, AgentError> {
        let contract = Contract::from_json(contract_json)?;
        let settings: AgentSettings = serde_json::from_str(settings_json)?;
        if settings.description.trim().is_empty() {
            return Err(AgentError::EmptyDescription);
        }
        if prompt.trim().is_empty() {
            return Err(AgentError::EmptyPrompt);
        }
        Ok(Self {
            contract,
            settings,
            prompt,
        })
    }

    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn settings(&self) -> &AgentSettings {
        &self.settings
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }
}

impl AgentSettings {
    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn model(&self) -> &ModelRef {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT: &str = r#"{"name":"explorer","kind":"worker",
        "input":{"type":"object","properties":{}},
        "output":{"type":"object","properties":{}}}"#;
    const SETTINGS: &str =
        r#"{"description":"Finds the files a task is about.","model":"ollama-cloud/gemma4:31b"}"#;
    const PROMPT: &str = "Find the files the task is about.";

    fn rejection(contract: &str, settings: &str, prompt: &str) -> String {
        Agent::from_parts(contract, settings, prompt.to_owned())
            .expect_err("agent must be rejected")
            .to_string()
    }

    #[test]
    fn builds_agent_from_contract_settings_and_prompt() {
        let agent = Agent::from_parts(CONTRACT, SETTINGS, PROMPT.to_owned()).expect("valid agent");
        assert_eq!(agent.contract().name().as_str(), "explorer");
        assert_eq!(
            agent.settings().description(),
            "Finds the files a task is about."
        );
        assert_eq!(
            agent.settings().model().to_string(),
            "ollama-cloud/gemma4:31b"
        );
        assert_eq!(agent.prompt(), PROMPT);
    }

    #[test]
    fn contract_problem_names_the_contract_file() {
        let error = rejection("{}", SETTINGS, PROMPT);
        assert!(error.starts_with("contract.json: "), "{error}");
    }

    #[test]
    fn rejects_hard_fields_in_settings() {
        let settings = SETTINGS.replace(r#""model""#, r#""rights":{},"model""#);
        let error = rejection(CONTRACT, &settings, PROMPT);
        assert!(
            error.starts_with("agent.json: unknown field `rights`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_model_without_provider() {
        let settings = SETTINGS.replace("ollama-cloud/", "");
        let error = rejection(CONTRACT, &settings, PROMPT);
        assert!(error.contains("must look like `provider/model`"), "{error}");
    }

    #[test]
    fn rejects_blank_description() {
        let settings = SETTINGS.replace("Finds the files a task is about.", "  ");
        assert_eq!(
            rejection(CONTRACT, &settings, PROMPT),
            "agent.json: description is empty"
        );
    }

    #[test]
    fn rejects_blank_prompt() {
        assert_eq!(rejection(CONTRACT, SETTINGS, " \n"), "prompt.md is empty");
    }
}
