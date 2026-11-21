use serde::Deserialize;

use crate::agent_name::AgentName;
use crate::schema::{ObjectSchema, Schema, SchemaProblem};

const TASK_FIELD: &str = "task";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Worker,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rights {
    #[serde(default)]
    read: Vec<String>,
    #[serde(default)]
    write: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    name: AgentName,
    kind: Kind,
    input: ObjectSchema,
    output: ObjectSchema,
    rights: Rights,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unchecked {
    name: AgentName,
    kind: Kind,
    input: Schema,
    output: Schema,
    #[serde(default)]
    rights: Rights,
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("not a valid contract: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("{part} must be an object")]
    NotAnObject { part: &'static str },
    #[error("{part}: {problem}")]
    BadSchema {
        part: &'static str,
        problem: SchemaProblem,
    },
    #[error("input must not declare `task`: the harness adds it to every step")]
    TaskIsReserved,
}

impl Contract {
    pub fn from_json(json: &str) -> Result<Self, ContractError> {
        let unchecked: Unchecked = serde_json::from_str(json)?;
        let input = checked_object("input", unchecked.input)?;
        let output = checked_object("output", unchecked.output)?;
        if input.declares(TASK_FIELD) {
            return Err(ContractError::TaskIsReserved);
        }
        Ok(Self {
            name: unchecked.name,
            kind: unchecked.kind,
            input,
            output,
            rights: unchecked.rights,
        })
    }

    pub fn name(&self) -> &AgentName {
        &self.name
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn input(&self) -> &ObjectSchema {
        &self.input
    }

    pub fn output(&self) -> &ObjectSchema {
        &self.output
    }

    pub fn rights(&self) -> &Rights {
        &self.rights
    }
}

impl Rights {
    pub fn read(&self) -> &[String] {
        &self.read
    }

    pub fn write(&self) -> &[String] {
        &self.write
    }
}

fn checked_object(part: &'static str, schema: Schema) -> Result<ObjectSchema, ContractError> {
    let Schema::Object(object) = schema else {
        return Err(ContractError::NotAnObject { part });
    };
    object
        .check()
        .map_err(|problem| ContractError::BadSchema { part, problem })?;
    Ok(object)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::Contract;

    pub const NO_FIELDS: &str = r#"{"type":"object","properties":{}}"#;

    pub fn worker(name: &str, input: &str, output: &str) -> Contract {
        let json =
            format!(r#"{{"name":"{name}","kind":"worker","input":{input},"output":{output}}}"#);
        Contract::from_json(&json).expect("valid contract")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::NO_FIELDS;
    use super::*;

    const REVIEWER: &str = r#"{
        "name": "reviewer",
        "kind": "worker",
        "input": {"type":"object","required":["files"],"properties":{
            "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}}}},
        "output": {"type":"object","required":["findings"],"properties":{
            "findings":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":300}}}},
        "rights": {"read": ["src/**"]}
    }"#;

    fn rejection(json: &str) -> String {
        Contract::from_json(json)
            .expect_err("contract must be rejected")
            .to_string()
    }

    fn with_parts(input: &str, output: &str) -> String {
        format!(r#"{{"name":"reviewer","kind":"worker","input":{input},"output":{output}}}"#)
    }

    #[test]
    fn parses_worker_contract() {
        let contract = Contract::from_json(REVIEWER).expect("valid contract");
        assert_eq!(contract.name().as_str(), "reviewer");
        assert_eq!(contract.kind(), Kind::Worker);
        assert!(contract.input().requires("files"));
        assert!(contract.output().requires("findings"));
        assert_eq!(contract.rights().read(), ["src/**"]);
        assert!(contract.rights().write().is_empty());
    }

    #[test]
    fn contract_without_rights_has_none() {
        let contract =
            Contract::from_json(&with_parts(NO_FIELDS, NO_FIELDS)).expect("valid contract");
        assert_eq!(contract.rights(), &Rights::default());
    }

    #[test]
    fn rejects_unknown_kind() {
        let error = rejection(&REVIEWER.replace("worker", "coordinator"));
        assert!(error.contains("unknown variant `coordinator`"), "{error}");
    }

    #[test]
    fn rejects_soft_fields_in_contract() {
        let error = rejection(&REVIEWER.replace(r#""kind""#, r#""prompt": "be nice", "kind""#));
        assert!(error.contains("unknown field `prompt`"), "{error}");
    }

    #[test]
    fn rejects_unknown_right() {
        let error = rejection(&REVIEWER.replace(r#""read""#, r#""exec""#));
        assert!(error.contains("unknown field `exec`"), "{error}");
    }

    #[test]
    fn rejects_invalid_agent_name() {
        let error = rejection(&REVIEWER.replace("reviewer", "../reviewer"));
        assert!(error.contains("agent name `../reviewer`"), "{error}");
    }

    #[test]
    fn rejects_input_that_is_not_an_object() {
        let error = rejection(&with_parts(r#"{"type":"string"}"#, NO_FIELDS));
        assert_eq!(error, "input must be an object");
    }

    #[test]
    fn rejects_output_with_undeclared_required_field() {
        let output = r#"{"type":"object","required":["findings"],"properties":{}}"#;
        let error = rejection(&with_parts(NO_FIELDS, output));
        assert_eq!(
            error,
            "output: `findings` is required but not declared in properties"
        );
    }

    #[test]
    fn rejects_input_that_declares_task() {
        let input = r#"{"type":"object","properties":{"task":{"type":"string"}}}"#;
        let error =
            Contract::from_json(&with_parts(input, NO_FIELDS)).expect_err("must be rejected");
        assert!(matches!(error, ContractError::TaskIsReserved));
    }
}
