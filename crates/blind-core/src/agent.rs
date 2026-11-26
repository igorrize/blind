use serde::Deserialize;

use crate::agent_name::AgentName;
use crate::contract::{Contract, ContractError, Kind};
use crate::limits::{LimitOutOfRange, Limits};
use crate::model_ref::ModelRef;
use crate::rule_name::RuleName;

pub const CONTRACT_FILE: &str = "contract.json";
pub const SETTINGS_FILE: &str = "agent.json";
pub const PROMPT_FILE: &str = "prompt.md";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettings {
    description: String,
    model: ModelRef,
    #[serde(default)]
    limits: Limits,
    #[serde(default)]
    rules: Vec<RuleName>,
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
    #[error("{file}: {0}", file = SETTINGS_FILE)]
    Limits(#[from] LimitOutOfRange),
    #[error("{file} is empty", file = PROMPT_FILE)]
    EmptyPrompt,
    #[error("{file}: rule `{0}` is listed twice", file = SETTINGS_FILE)]
    RepeatedRule(RuleName),
    #[error("{file}: `{0}` is a root contract, not a worker", file = CONTRACT_FILE)]
    NotAWorker(AgentName),
    #[error("{file}: `{0}` is a worker contract, not root", file = CONTRACT_FILE)]
    NotARoot(AgentName),
}

impl Agent {
    pub fn from_parts(
        contract_json: &str,
        settings_json: &str,
        prompt: String,
    ) -> Result<Self, AgentError> {
        let contract = Contract::from_json(contract_json)?;
        if contract.kind() != Kind::Worker {
            return Err(AgentError::NotAWorker(contract.name().clone()));
        }
        let settings: AgentSettings = serde_json::from_str(settings_json)?;
        check_text(&settings.description, &prompt, &settings.rules)?;
        settings.limits.check()?;
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

    pub fn with_rules(mut self, texts: &[String]) -> Self {
        self.prompt = followed_by_rules(&self.prompt, texts);
        self
    }
}

impl AgentSettings {
    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn model(&self) -> &ModelRef {
        &self.model
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn rules(&self) -> &[RuleName] {
        &self.rules
    }
}

pub(crate) fn check_text(
    description: &str,
    prompt: &str,
    rules: &[RuleName],
) -> Result<(), AgentError> {
    if description.trim().is_empty() {
        return Err(AgentError::EmptyDescription);
    }
    if prompt.trim().is_empty() {
        return Err(AgentError::EmptyPrompt);
    }
    let repeated = rules
        .iter()
        .enumerate()
        .find(|(position, rule)| rules[..*position].contains(rule));
    match repeated {
        Some((_, rule)) => Err(AgentError::RepeatedRule(rule.clone())),
        None => Ok(()),
    }
}

pub(crate) fn followed_by_rules(prompt: &str, texts: &[String]) -> String {
    let parts = std::iter::once(prompt.trim_end()).chain(texts.iter().map(|text| text.trim()));
    let parts: Vec<&str> = parts.filter(|part| !part.is_empty()).collect();
    format!("{}\n", parts.join("\n\n"))
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
    fn agent_without_limits_gets_the_defaults() {
        let agent = Agent::from_parts(CONTRACT, SETTINGS, PROMPT.to_owned()).expect("valid agent");
        assert_eq!(agent.settings().limits(), Limits::default());
    }

    #[test]
    fn agent_can_tighten_its_limits() {
        let settings = SETTINGS.replace(r#""model""#, r#""limits":{"tool_calls":5},"model""#);
        let agent = Agent::from_parts(CONTRACT, &settings, PROMPT.to_owned()).expect("valid agent");
        assert_eq!(agent.settings().limits().tool_calls, 5);
    }

    #[test]
    fn limit_above_the_harness_ceiling_is_rejected() {
        let settings = SETTINGS.replace(r#""model""#, r#""limits":{"turns":61},"model""#);
        assert_eq!(
            rejection(CONTRACT, &settings, PROMPT),
            "agent.json: limits.turns must be between 1 and 60"
        );
    }

    fn with_rules(rules: &str) -> String {
        SETTINGS.replace(r#""model""#, &format!(r#""rules":{rules},"model""#))
    }

    #[test]
    fn agent_lists_the_rules_it_follows() {
        let agent = Agent::from_parts(
            CONTRACT,
            &with_rules(r#"["rust","tests"]"#),
            PROMPT.to_owned(),
        )
        .expect("valid agent");
        let names: Vec<&str> = agent
            .settings()
            .rules()
            .iter()
            .map(RuleName::as_str)
            .collect();
        assert_eq!(names, ["rust", "tests"]);
    }

    #[test]
    fn agent_without_rules_has_none() {
        let agent = Agent::from_parts(CONTRACT, SETTINGS, PROMPT.to_owned()).expect("valid agent");
        assert!(agent.settings().rules().is_empty());
    }

    #[test]
    fn rule_name_that_could_leave_the_rules_folder_is_rejected() {
        let error = rejection(CONTRACT, &with_rules(r#"["../secrets"]"#), PROMPT);
        assert!(error.contains("rule name `../secrets`"), "{error}");
    }

    #[test]
    fn rule_listed_twice_is_rejected() {
        assert_eq!(
            rejection(CONTRACT, &with_rules(r#"["rust","rust"]"#), PROMPT),
            "agent.json: rule `rust` is listed twice"
        );
    }

    #[test]
    fn rule_texts_follow_the_prompt_in_order() {
        let agent = Agent::from_parts(CONTRACT, SETTINGS, "Find the files.\n\n".to_owned())
            .expect("valid agent")
            .with_rules(&[
                "# Rust\n- no comments\n".to_owned(),
                "\n# Tests\n".to_owned(),
            ]);
        assert_eq!(
            agent.prompt(),
            "Find the files.\n\n# Rust\n- no comments\n\n# Tests\n"
        );
    }

    #[test]
    fn root_contract_cannot_make_a_worker() {
        let root = r#"{"name":"root","kind":"root","input":{"type":"object","properties":{}}}"#;
        assert_eq!(
            rejection(root, SETTINGS, PROMPT),
            "contract.json: `root` is a root contract, not a worker"
        );
    }

    #[test]
    fn rejects_blank_prompt() {
        assert_eq!(rejection(CONTRACT, SETTINGS, " \n"), "prompt.md is empty");
    }
}
