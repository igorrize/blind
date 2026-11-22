use std::path::Path;

use blind_core::{Agent, CONTRACT_FILE, PROMPT_FILE, ReadingTool, Rights, SETTINGS_FILE};
use serde_json::Value;

use crate::agents::{self, Loaded};

const NOTHING: &str = "nothing";

pub fn of(dir: &Path) -> String {
    let place = dir.display().to_string();
    match agents::load_all(dir) {
        Ok(loaded) if loaded.is_empty() => no_agents(&place, "it has no agent folders"),
        Ok(loaded) => listed(&place, &loaded),
        Err(error) => no_agents(&place, &format!("it cannot be read: {error}")),
    }
}

fn no_agents(place: &str, why: &str) -> String {
    format!(
        "This project has no agents. The harness looks for them in `{place}`, but {why}. \
         An agent is a folder there with {CONTRACT_FILE}, {SETTINGS_FILE} and {PROMPT_FILE}."
    )
}

fn listed(place: &str, loaded: &[Loaded]) -> String {
    let agents: Vec<String> = loaded.iter().map(entry).collect();
    format!("{}\n\n{}", introduction(place), agents.join("\n\n"))
}

fn introduction(place: &str) -> String {
    format!(
        "The harness loaded the agents of this project from `{place}`: one folder per agent with \
         {CONTRACT_FILE} (what the harness enforces), {SETTINGS_FILE} (description, model, limits) \
         and {PROMPT_FILE}. The list below is everything you know about them. \
         You cannot call an agent: the user picks the agents and their order, and the harness runs them."
    )
}

fn entry(loaded: &Loaded) -> String {
    match &loaded.outcome {
        Ok(agent) => usable(agent),
        Err(error) => format!("## {}\ncannot be used: {error}", loaded.folder),
    }
}

fn usable(agent: &Agent) -> String {
    let contract = agent.contract();
    let settings = agent.settings();
    let limits = settings.limits();
    [
        format!("## {}", contract.name()),
        settings.description().to_owned(),
        format!("model: {}", settings.model()),
        format!("takes: {}", taken(&contract.input().to_json_schema())),
        format!(
            "returns: {}",
            or_nothing(fields(&contract.output().to_json_schema()))
        ),
        format!("reads: {}", reading(contract.rights())),
        format!(
            "writes: {}",
            or_nothing(contract.rights().write().join(", "))
        ),
        format!(
            "limits: {} turns, {} tokens, {} tool calls",
            limits.turns, limits.tokens, limits.tool_calls
        ),
    ]
    .join("\n")
}

fn taken(input: &Value) -> String {
    let declared = fields(input);
    if declared.is_empty() {
        "the task".to_owned()
    } else {
        format!("the task, {declared}")
    }
}

fn fields(schema: &Value) -> String {
    let required = schema["required"].as_array().into_iter().flatten();
    let required: Vec<&str> = required.filter_map(Value::as_str).collect();
    let properties = schema["properties"].as_object().into_iter().flatten();
    let described: Vec<String> = properties
        .map(|(name, field)| {
            let need = if required.contains(&name.as_str()) {
                "required"
            } else {
                "optional"
            };
            format!("{name} ({}, {need})", kind(field))
        })
        .collect();
    described.join(", ")
}

fn kind(field: &Value) -> String {
    match field["type"].as_str() {
        Some("array") => format!("list of {}", kind(&field["items"])),
        Some(simple) => simple.to_owned(),
        None => "value".to_owned(),
    }
}

fn reading(rights: &Rights) -> String {
    if rights.read().is_empty() {
        return NOTHING.to_owned();
    }
    let tools: Vec<&str> = rights.tools().into_iter().map(ReadingTool::name).collect();
    let scope = format!("{} with {}", rights.read().join(", "), tools.join(", "));
    match rights.read_from() {
        Some(field) => format!("{scope}; whole files only from the input field `{field}`"),
        None => scope,
    }
}

fn or_nothing(text: String) -> String {
    if text.is_empty() {
        NOTHING.to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::agents::test_support::write_agent;

    const REVIEWER_CONTRACT: &str = r#"{"name":"reviewer","kind":"worker",
        "input":{"type":"object","required":["files"],"properties":{
            "files":{"type":"array","items":{"type":"string"}},
            "strict":{"type":"boolean"}}},
        "output":{"type":"object","required":["findings"],"properties":{
            "findings":{"type":"array","items":{"type":"string"}}}},
        "rights":{"read":["src/**","docs/**"],"read_from":"files","tools":["grep","read_file"]}}"#;
    const REVIEWER_SETTINGS: &str = r#"{"description":"Reviews the given files.",
        "model":"local/tiny","limits":{"tool_calls":7}}"#;

    fn write_reviewer(dir: &Path) {
        let folder = dir.join("reviewer");
        fs::create_dir(&folder).expect("folder");
        fs::write(folder.join(CONTRACT_FILE), REVIEWER_CONTRACT).expect("contract");
        fs::write(folder.join(SETTINGS_FILE), REVIEWER_SETTINGS).expect("settings");
        fs::write(folder.join(PROMPT_FILE), "Review.").expect("prompt");
    }

    #[test]
    fn agent_is_described_by_what_its_contract_and_settings_say() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_reviewer(dir.path());
        let roster = of(dir.path());
        let expected = "## reviewer\n\
            Reviews the given files.\n\
            model: local/tiny\n\
            takes: the task, files (list of string, required), strict (boolean, optional)\n\
            returns: findings (list of string, required)\n\
            reads: src/**, docs/** with grep, read_file; whole files only from the input field `files`\n\
            writes: nothing\n\
            limits: 30 turns, 300000 tokens, 7 tool calls";
        assert!(roster.ends_with(expected), "{roster}");
    }

    #[test]
    fn agent_without_fields_or_rights_takes_the_task_and_touches_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "planner", "planner");
        let roster = of(dir.path());
        assert!(roster.contains("takes: the task\n"), "{roster}");
        assert!(roster.contains("returns: nothing\n"), "{roster}");
        assert!(roster.contains("reads: nothing\n"), "{roster}");
        assert!(roster.contains("writes: nothing\n"), "{roster}");
    }

    #[test]
    fn roster_names_the_folder_and_says_root_cannot_call_agents() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "planner", "planner");
        let roster = of(dir.path());
        assert!(
            roster.contains(&dir.path().display().to_string()),
            "{roster}"
        );
        assert!(roster.contains("You cannot call an agent"), "{roster}");
    }

    #[test]
    fn broken_agent_is_listed_as_unusable_with_the_reason() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "explorer", "explorer");
        write_agent(dir.path(), "renamed", "planner");
        let roster = of(dir.path());
        assert!(roster.contains("## explorer\nDoes one job."), "{roster}");
        assert!(
            roster.contains(
                "## renamed\ncannot be used: folder is `renamed` but contract.json names the agent `planner`"
            ),
            "{roster}"
        );
    }

    #[test]
    fn missing_folder_means_no_agents_and_says_where_they_belong() {
        let dir = tempfile::tempdir().expect("temp dir");
        let roster = of(&dir.path().join("absent"));
        assert!(
            roster.starts_with("This project has no agents."),
            "{roster}"
        );
        assert!(roster.contains("it cannot be read"), "{roster}");
        assert!(
            roster.contains("contract.json, agent.json and prompt.md"),
            "{roster}"
        );
    }

    #[test]
    fn empty_folder_means_no_agents() {
        let dir = tempfile::tempdir().expect("temp dir");
        let roster = of(dir.path());
        assert!(roster.contains("it has no agent folders"), "{roster}");
    }

    #[test]
    fn example_agents_are_all_listed() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/agents");
        let roster = of(&examples);
        for name in ["explorer", "fixer", "planner", "reviewer"] {
            assert!(roster.contains(&format!("## {name}\n")), "{roster}");
        }
        assert!(!roster.contains("cannot be used"), "{roster}");
    }
}
