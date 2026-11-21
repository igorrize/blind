use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingTool {
    FindFiles,
    Grep,
    ListFiles,
    ReadFile,
}

impl ReadingTool {
    pub const ALL: [ReadingTool; 4] = [
        ReadingTool::FindFiles,
        ReadingTool::Grep,
        ReadingTool::ListFiles,
        ReadingTool::ReadFile,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ReadingTool::FindFiles => "find_files",
            ReadingTool::Grep => "grep",
            ReadingTool::ListFiles => "list_files",
            ReadingTool::ReadFile => "read_file",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_parsed_from_its_own_name() {
        for tool in ReadingTool::ALL {
            let parsed: ReadingTool =
                serde_json::from_str(&format!("\"{}\"", tool.name())).expect("known tool");
            assert_eq!(parsed, tool);
        }
    }

    #[test]
    fn tools_outside_the_catalogue_are_rejected() {
        for name in ["bash", "write_file", "READ_FILE", ""] {
            let parsed = serde_json::from_str::<ReadingTool>(&format!("\"{name}\""));
            assert!(parsed.is_err(), "{name:?} must be rejected");
        }
    }
}
