use serde::Deserialize;

use crate::agent::{AgentError, check_text, followed_by_rules};
use crate::contract::{Contract, Kind};
use crate::rule_name::RuleName;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootSettings {
    description: String,
    #[serde(default)]
    rules: Vec<RuleName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootAgent {
    contract: Contract,
    settings: RootSettings,
    prompt: String,
}

impl RootAgent {
    pub fn from_parts(
        contract_json: &str,
        settings_json: &str,
        prompt: String,
    ) -> Result<Self, AgentError> {
        let contract = Contract::from_json(contract_json)?;
        if contract.kind() != Kind::Root {
            return Err(AgentError::NotARoot(contract.name().clone()));
        }
        let settings: RootSettings = serde_json::from_str(settings_json)?;
        check_text(&settings.description, &prompt, &settings.rules)?;
        Ok(Self {
            contract,
            settings,
            prompt,
        })
    }

    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn rules(&self) -> &[RuleName] {
        &self.settings.rules
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn with_rules(mut self, texts: &[String]) -> Self {
        self.prompt = followed_by_rules(&self.prompt, texts);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT: &str = r#"{"name":"root","kind":"root","input":{"type":"object","properties":{
        "summary":{"type":"string"}}}}"#;
    const SETTINGS: &str = r#"{"description":"Talks with the human about this project."}"#;
    const PROMPT: &str = "Keep answers short.";

    fn rejection(contract: &str, settings: &str) -> String {
        RootAgent::from_parts(contract, settings, PROMPT.to_owned())
            .expect_err("root must be rejected")
            .to_string()
    }

    #[test]
    fn root_is_built_from_a_root_contract_settings_and_prompt() {
        let root =
            RootAgent::from_parts(CONTRACT, SETTINGS, PROMPT.to_owned()).expect("valid root");
        assert!(root.contract().input().declares("summary"));
        assert_eq!(root.prompt(), PROMPT);
        assert!(root.rules().is_empty());
    }

    #[test]
    fn worker_contract_cannot_make_root() {
        let worker = r#"{"name":"explorer","kind":"worker","input":{"type":"object","properties":{}},
            "output":{"type":"object","properties":{}}}"#;
        assert_eq!(
            rejection(worker, SETTINGS),
            "contract.json: `explorer` is a worker contract, not root"
        );
    }

    #[test]
    fn root_model_is_chosen_in_the_settings_window_not_in_its_files() {
        let with_model = r#"{"description":"Talks.","model":"ollama-cloud/glm-5.3"}"#;
        let error = rejection(CONTRACT, with_model);
        assert!(error.contains("unknown field `model`"), "{error}");
    }

    #[test]
    fn root_follows_its_rules_after_the_prompt() {
        let settings = r#"{"description":"Talks.","rules":["style"]}"#;
        let root = RootAgent::from_parts(CONTRACT, settings, PROMPT.to_owned())
            .expect("valid root")
            .with_rules(&["Answer in Russian.".to_owned()]);
        assert_eq!(root.prompt(), "Keep answers short.\n\nAnswer in Russian.\n");
    }
}
