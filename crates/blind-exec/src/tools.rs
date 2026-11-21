use blind_core::ReadingTool;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::workspace::{LineRange, ToolError, Workspace};

pub struct ToolDescription {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindFiles {
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grep {
    query: String,
    #[serde(default)]
    regex: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFiles {
    pattern: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFile {
    path: String,
    first_line: Option<usize>,
    last_line: Option<usize>,
}

impl Workspace {
    pub fn offered(&self) -> Vec<ToolDescription> {
        self.tools().iter().map(|tool| described(*tool)).collect()
    }

    pub fn call(
        &self,
        name: &str,
        arguments: &str,
        given: Option<&[String]>,
    ) -> Option<Result<String, ToolError>> {
        let allowed = self.tools().iter().copied();
        let tool = allowed.into_iter().find(|tool| tool.name() == name)?;
        Some(match tool {
            ReadingTool::FindFiles => self.call_find_files(arguments),
            ReadingTool::Grep => self.call_grep(arguments),
            ReadingTool::ListFiles => self.call_list_files(arguments),
            ReadingTool::ReadFile => self.call_read_file(arguments, given),
        })
    }

    fn call_find_files(&self, arguments: &str) -> Result<String, ToolError> {
        let wanted: FindFiles = parsed(arguments)?;
        self.find_files(&wanted.query)
    }

    fn call_grep(&self, arguments: &str) -> Result<String, ToolError> {
        let wanted: Grep = parsed(arguments)?;
        self.grep(&wanted.query, wanted.regex)
    }

    fn call_list_files(&self, arguments: &str) -> Result<String, ToolError> {
        let wanted: ListFiles = parsed(arguments)?;
        self.list_files(&wanted.pattern)
    }

    fn call_read_file(
        &self,
        arguments: &str,
        given: Option<&[String]>,
    ) -> Result<String, ToolError> {
        let wanted: ReadFile = parsed(arguments)?;
        let lines = LineRange {
            first: wanted.first_line,
            last: wanted.last_line,
        };
        self.read_file(&wanted.path, lines, given)
    }
}

fn described(tool: ReadingTool) -> ToolDescription {
    let (description, parameters) = match tool {
        ReadingTool::FindFiles => (
            "Find files by name. Fuzzy and typo-tolerant, best matches first. \
             Use one or two words, for example `contract` or `tui app`. \
             Use it when you look for a file, not for text inside files.",
            object(&["query"], json!({"query": {"type": "string"}})),
        ),
        ReadingTool::Grep => (
            "Find lines that contain a text. Search for one bare identifier, \
             for example `load_agent`. Narrow the files by starting the query with \
             a constraint: `*.rs load_agent`, `src/ load_agent`, `!tests/ load_agent`. \
             Set `regex` to true only when you need a regular expression.",
            object(
                &["query"],
                json!({"query": {"type": "string"}, "regex": {"type": "boolean"}}),
            ),
        ),
        ReadingTool::ListFiles => (
            "List every file whose path matches a glob pattern, \
             for example `src/**/*.rs`. Use it to see what a folder contains.",
            object(&["pattern"], json!({"pattern": {"type": "string"}})),
        ),
        ReadingTool::ReadFile => (
            "Read a text file with line numbers. Give `first_line` and `last_line` \
             to read a part of it. Read only the files you really need, \
             and ask for several files in one turn.",
            object(
                &["path"],
                json!({
                    "path": {"type": "string"},
                    "first_line": {"type": "integer", "minimum": 1},
                    "last_line": {"type": "integer", "minimum": 1}
                }),
            ),
        ),
    };
    ToolDescription {
        name: tool.name(),
        description,
        parameters,
    }
}

fn object(required: &[&str], properties: Value) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn parsed<T: DeserializeOwned>(arguments: &str) -> Result<T, ToolError> {
    serde_json::from_str(arguments).map_err(|error| ToolError::BadArguments(error.to_string()))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::workspace::test_support::{project, reading, reading_with};

    const FIND_FILES: &str = "find_files";
    const GREP: &str = "grep";
    const LIST_FILES: &str = "list_files";
    const READ_FILE: &str = "read_file";

    fn sources() -> TempDir {
        project(&[("src/main.rs", "fn main() {}\n"), (".env", "SECRET")])
    }

    fn called(tool: &str, arguments: &str) -> Option<Result<String, ToolError>> {
        reading(&sources(), &["**"]).call(tool, arguments, None)
    }

    fn answer(tool: &str, arguments: &str) -> String {
        match called(tool, arguments).expect("known tool") {
            Ok(text) => text,
            Err(error) => format!("error: {error}"),
        }
    }

    #[test]
    fn every_tool_is_described_with_a_closed_schema() {
        let tools = reading(&sources(), &["**"]).offered();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name).collect();
        assert_eq!(names, [FIND_FILES, GREP, LIST_FILES, READ_FILE]);
        for tool in tools {
            assert_eq!(tool.parameters["additionalProperties"], json!(false));
            assert!(!tool.description.is_empty());
        }
    }

    #[test]
    fn tools_are_called_by_name_with_json_arguments() {
        assert_eq!(answer(FIND_FILES, r#"{"query":"main"}"#), "src/main.rs");
        assert_eq!(
            answer(GREP, r#"{"query":"main"}"#),
            "src/main.rs:1: fn main() {}"
        );
        assert_eq!(answer(LIST_FILES, r#"{"pattern":"**"}"#), "src/main.rs");
        assert_eq!(
            answer(
                READ_FILE,
                r#"{"path":"src/main.rs","first_line":1,"last_line":1}"#
            ),
            "    1  fn main() {}\n"
        );
    }

    #[test]
    fn only_allowed_tools_are_offered_and_handled() {
        let dir = sources();
        let searching = [ReadingTool::FindFiles, ReadingTool::Grep];
        let workspace = reading_with(&dir, &["**"], &searching);
        let names: Vec<&str> = workspace.offered().iter().map(|tool| tool.name).collect();
        assert_eq!(names, [FIND_FILES, GREP]);
        assert!(workspace.call(GREP, r#"{"query":"main"}"#, None).is_some());
        assert!(
            workspace
                .call(READ_FILE, r#"{"path":"src/main.rs"}"#, None)
                .is_none()
        );
        assert!(
            workspace
                .call(LIST_FILES, r#"{"pattern":"**"}"#, None)
                .is_none()
        );
    }

    #[test]
    fn read_file_respects_the_list_of_given_files() {
        let workspace = reading(&sources(), &["**"]);
        let nothing_given: [String; 0] = [];
        let refused = workspace
            .call(READ_FILE, r#"{"path":"src/main.rs"}"#, Some(&nothing_given))
            .expect("known tool")
            .expect_err("must be refused");
        assert_eq!(
            refused.to_string(),
            "`src/main.rs` is not among the files given to this agent"
        );
    }

    #[test]
    fn refusals_come_back_as_errors_without_the_content() {
        let refused = answer(READ_FILE, r#"{"path":".env"}"#);
        assert_eq!(refused, "error: `.env` is protected and cannot be accessed");
    }

    #[test]
    fn arguments_outside_the_schema_are_rejected() {
        for arguments in [
            r#"{"path":"src/main.rs","follow_links":true}"#,
            r"{}",
            "not json",
        ] {
            let rejected = answer(READ_FILE, arguments);
            let expected = "error: arguments do not fit the tool";
            assert!(rejected.starts_with(expected), "{rejected}");
        }
    }

    #[test]
    fn unknown_tool_is_not_handled() {
        assert!(called("bash", r#"{"command":"ls"}"#).is_none());
        assert!(called("write_file", r#"{"path":"a","content":"b"}"#).is_none());
    }
}
