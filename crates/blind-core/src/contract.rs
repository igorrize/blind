use serde::Deserialize;

use crate::agent_name::AgentName;
use crate::reading_tool::ReadingTool;
use crate::schema::{ObjectSchema, Schema, SchemaProblem};

const TASK_FIELD: &str = "task";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Worker,
    Root,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rights {
    #[serde(default)]
    read: Vec<String>,
    #[serde(default)]
    write: Vec<String>,
    read_from: Option<String>,
    tools: Option<Vec<ReadingTool>>,
    written_as: Option<String>,
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
    output: Option<Schema>,
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
    #[error("rights: read_from names `{0}`, which is not an input field holding a list of strings")]
    ReadFromIsNotAListOfPaths(String),
    #[error("rights: read_from and tools make sense only together with read patterns")]
    FileRightsWithoutRead,
    #[error("rights: written_as makes sense only together with write patterns")]
    WrittenAsWithoutWrite,
    #[error(
        "rights: written_as names `{0}`, which is not an output field holding a list of strings"
    )]
    WrittenAsIsNotAListOfPaths(String),
    #[error("root cannot write: it only talks and reads")]
    RootCannotWrite,
    #[error("a worker needs an output: it is what it hands to the next step")]
    WorkerWithoutOutput,
    #[error("root has no output: it answers the human")]
    RootWithOutput,
    #[error("root input must not require {0:?}: every chain has to be able to reach root")]
    RootRequiresFields(Vec<String>),
}

impl Contract {
    pub fn from_json(json: &str) -> Result<Self, ContractError> {
        let unchecked: Unchecked = serde_json::from_str(json)?;
        let input = checked_object("input", unchecked.input)?;
        let output = checked_output(unchecked.kind, unchecked.output, &input)?;
        if input.declares(TASK_FIELD) {
            return Err(ContractError::TaskIsReserved);
        }
        unchecked.rights.check_against(&input)?;
        unchecked.rights.check_writing(unchecked.kind, &output)?;
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

    pub fn read_from(&self) -> Option<&str> {
        self.read_from.as_deref()
    }

    pub fn written_as(&self) -> Option<&str> {
        self.written_as.as_deref()
    }

    pub fn tools(&self) -> Vec<ReadingTool> {
        self.tools
            .clone()
            .unwrap_or_else(|| ReadingTool::ALL.to_vec())
    }

    fn check_against(&self, input: &ObjectSchema) -> Result<(), ContractError> {
        let narrows_reading = self.read_from.is_some() || self.tools.is_some();
        if narrows_reading && self.read.is_empty() {
            return Err(ContractError::FileRightsWithoutRead);
        }
        match &self.read_from {
            Some(field) if !input.holds_a_list_of_strings(field) => {
                Err(ContractError::ReadFromIsNotAListOfPaths(field.clone()))
            }
            Some(_) | None => Ok(()),
        }
    }

    fn check_writing(&self, kind: Kind, output: &ObjectSchema) -> Result<(), ContractError> {
        if kind == Kind::Root && !self.write.is_empty() {
            return Err(ContractError::RootCannotWrite);
        }
        match &self.written_as {
            Some(_) if self.write.is_empty() => Err(ContractError::WrittenAsWithoutWrite),
            Some(field) if !output.holds_a_list_of_strings(field) => {
                Err(ContractError::WrittenAsIsNotAListOfPaths(field.clone()))
            }
            Some(_) | None => Ok(()),
        }
    }
}

fn checked_output(
    kind: Kind,
    output: Option<Schema>,
    input: &ObjectSchema,
) -> Result<ObjectSchema, ContractError> {
    match (kind, output) {
        (Kind::Worker, Some(output)) => checked_object("output", output),
        (Kind::Worker, None) => Err(ContractError::WorkerWithoutOutput),
        (Kind::Root, Some(_)) => Err(ContractError::RootWithOutput),
        (Kind::Root, None) if !input.required().is_empty() => {
            Err(ContractError::RootRequiresFields(input.required().to_vec()))
        }
        (Kind::Root, None) => Ok(ObjectSchema::empty()),
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

    fn with_rights(rights: &str) -> String {
        let files = r#"{"type":"object","properties":{
            "files":{"type":"array","items":{"type":"string"}},
            "count":{"type":"integer"}}}"#;
        format!(
            r#"{{"name":"reviewer","kind":"worker","input":{files},"output":{NO_FIELDS},"rights":{rights}}}"#
        )
    }

    #[test]
    fn contract_without_tool_list_allows_every_reading_tool() {
        let contract = Contract::from_json(&with_rights(r#"{"read":["**"]}"#)).expect("valid");
        assert_eq!(contract.rights().tools(), ReadingTool::ALL);
        assert_eq!(contract.rights().read_from(), None);
    }

    #[test]
    fn contract_can_narrow_tools_and_reading() {
        let rights = r#"{"read":["**"],"read_from":"files","tools":["grep","read_file"]}"#;
        let contract = Contract::from_json(&with_rights(rights)).expect("valid");
        assert_eq!(
            contract.rights().tools(),
            [ReadingTool::Grep, ReadingTool::ReadFile]
        );
        assert_eq!(contract.rights().read_from(), Some("files"));
    }

    #[test]
    fn rejects_a_tool_outside_the_catalogue() {
        let error = rejection(&with_rights(r#"{"read":["**"],"tools":["bash"]}"#));
        assert!(error.contains("unknown variant `bash`"), "{error}");
    }

    #[test]
    fn read_from_must_name_an_input_list_of_strings() {
        for field in ["count", "absent"] {
            let rights = format!(r#"{{"read":["**"],"read_from":"{field}"}}"#);
            let error = rejection(&with_rights(&rights));
            assert!(
                error.contains("not an input field holding a list of strings"),
                "{error}"
            );
        }
    }

    #[test]
    fn narrowing_without_read_patterns_is_rejected() {
        for rights in [r#"{"read_from":"files"}"#, r#"{"tools":["grep"]}"#] {
            let error = rejection(&with_rights(rights));
            assert!(
                error.contains("only together with read patterns"),
                "{error}"
            );
        }
    }

    const ROOT_INPUT: &str = r#"{"type":"object","properties":{
        "summary":{"type":"string"},
        "findings":{"type":"array","items":{"type":"string"}}}}"#;

    fn root(input: &str, output: Option<&str>) -> String {
        let output = output
            .map(|output| format!(r#","output":{output}"#))
            .unwrap_or_default();
        format!(r#"{{"name":"root","kind":"root","input":{input}{output}}}"#)
    }

    #[test]
    fn root_takes_optional_fields_from_agents_and_has_no_output() {
        let contract = Contract::from_json(&root(ROOT_INPUT, None)).expect("valid root");
        assert_eq!(contract.kind(), Kind::Root);
        assert!(contract.input().declares("summary"));
        assert!(contract.input().required().is_empty());
        assert_eq!(contract.output(), &ObjectSchema::empty());
    }

    #[test]
    fn root_with_an_output_is_rejected() {
        assert_eq!(
            rejection(&root(ROOT_INPUT, Some(NO_FIELDS))),
            "root has no output: it answers the human"
        );
    }

    #[test]
    fn root_that_requires_a_field_is_rejected() {
        let strict = r#"{"type":"object","required":["summary"],"properties":{
            "summary":{"type":"string"}}}"#;
        assert_eq!(
            rejection(&root(strict, None)),
            r#"root input must not require ["summary"]: every chain has to be able to reach root"#
        );
    }

    #[test]
    fn worker_without_an_output_is_rejected() {
        let json =
            r#"{"name":"explorer","kind":"worker","input":{"type":"object","properties":{}}}"#;
        assert_eq!(
            rejection(json),
            "a worker needs an output: it is what it hands to the next step"
        );
    }

    const CHANGED: &str = r#"{"type":"object","required":["changed"],"properties":{
        "changed":{"type":"array","items":{"type":"string"}},
        "count":{"type":"integer"}}}"#;

    fn writer(rights: &str) -> String {
        format!(
            r#"{{"name":"writer","kind":"worker","input":{NO_FIELDS},"output":{CHANGED},"rights":{rights}}}"#
        )
    }

    #[test]
    fn writer_can_let_the_harness_report_what_it_wrote() {
        let rights = r#"{"read":["**"],"write":["crates/**"],"written_as":"changed"}"#;
        let contract = Contract::from_json(&writer(rights)).expect("valid");
        assert_eq!(contract.rights().written_as(), Some("changed"));
        assert_eq!(contract.rights().write(), ["crates/**"]);
    }

    #[test]
    fn written_as_needs_write_rights_and_an_output_list_of_strings() {
        assert_eq!(
            rejection(&writer(r#"{"read":["**"],"written_as":"changed"}"#)),
            "rights: written_as makes sense only together with write patterns"
        );
        for field in ["count", "absent"] {
            let rights = format!(r#"{{"write":["crates/**"],"written_as":"{field}"}}"#);
            let error = rejection(&writer(&rights));
            assert!(
                error.contains("not an output field holding a list of strings"),
                "{error}"
            );
        }
    }

    #[test]
    fn root_with_write_rights_is_rejected() {
        let json = r#"{"name":"root","kind":"root","input":{"type":"object","properties":{}},
            "rights":{"read":["**"],"write":["crates/**"]}}"#;
        assert_eq!(
            rejection(json),
            "root cannot write: it only talks and reads"
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
