use std::{
    fs, io,
    path::{Path, PathBuf},
};

use blind_core::{
    Agent, AgentError, AgentName, CONTRACT_FILE, InvalidAgentName, PROMPT_FILE, RootAgent,
    RuleName, SETTINGS_FILE,
};

const RULES_FOLDER: &str = "rules";
const RULE_EXTENSION: &str = "md";

pub struct Loaded {
    pub folder: String,
    pub outcome: Result<Agent, LoadError>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read {file}: {source}")]
    Unreadable {
        file: &'static str,
        source: io::Error,
    },
    #[error(transparent)]
    Invalid(#[from] AgentError),
    #[error(transparent)]
    BadName(#[from] InvalidAgentName),
    #[error("there is no agent folder `{0}`")]
    NoSuchFolder(String),
    #[error("{file}: rights: {0}", file = CONTRACT_FILE)]
    BadRights(#[from] blind_exec::PolicyError),
    #[error("folder is `{folder}` but {file} names the agent `{name}`", file = CONTRACT_FILE)]
    NameMismatch { folder: String, name: String },
    #[error("rule `{name}`: cannot read {path}: {source}")]
    UnreadableRule {
        name: RuleName,
        path: String,
        source: io::Error,
    },
}

pub fn load_all(dir: &Path) -> io::Result<Vec<Loaded>> {
    let mut folders = agent_folders(dir)?;
    folders.sort();
    Ok(folders.iter().map(|folder| load(dir, folder)).collect())
}

pub fn load_one(dir: &Path, name: &str) -> Result<Agent, LoadError> {
    let name = AgentName::try_from(name.to_owned())?;
    let folder = dir.join(name.as_str());
    if !is_real_folder(&folder) {
        return Err(LoadError::NoSuchFolder(name.as_str().to_owned()));
    }
    load_agent(&folder, name.as_str(), &rules_folder(dir))
}

fn is_real_folder(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn agent_folders(dir: &Path) -> io::Result<Vec<String>> {
    let mut folders = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            folders.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(folders)
}

fn load(dir: &Path, folder: &str) -> Loaded {
    Loaded {
        folder: folder.to_owned(),
        outcome: load_agent(&dir.join(folder), folder, &rules_folder(dir)),
    }
}

pub fn load_root(dir: &Path) -> Result<Option<RootAgent>, LoadError> {
    if !is_real_folder(dir) {
        return Ok(None);
    }
    let contract = read(dir, CONTRACT_FILE)?;
    let settings = read(dir, SETTINGS_FILE)?;
    let prompt = read(dir, PROMPT_FILE)?;
    let root = RootAgent::from_parts(&contract, &settings, prompt)?;
    let texts = rule_texts(&rules_folder(dir), root.rules())?;
    Ok(Some(root.with_rules(&texts)))
}

fn rules_folder(agents_dir: &Path) -> PathBuf {
    agents_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(RULES_FOLDER)
}

fn load_agent(path: &Path, folder: &str, rules: &Path) -> Result<Agent, LoadError> {
    let contract = read(path, CONTRACT_FILE)?;
    let settings = read(path, SETTINGS_FILE)?;
    let prompt = read(path, PROMPT_FILE)?;
    let agent = Agent::from_parts(&contract, &settings, prompt)?;
    let texts = rule_texts(rules, agent.settings().rules())?;
    let agent = agent.with_rules(&texts);
    blind_exec::check_patterns(agent.contract().rights().read())?;
    blind_exec::check_patterns(agent.contract().rights().write())?;
    let name = agent.contract().name().as_str();
    if name == folder {
        Ok(agent)
    } else {
        Err(LoadError::NameMismatch {
            folder: folder.to_owned(),
            name: name.to_owned(),
        })
    }
}

fn rule_texts(rules: &Path, names: &[RuleName]) -> Result<Vec<String>, LoadError> {
    names.iter().map(|name| rule_text(rules, name)).collect()
}

fn rule_text(rules: &Path, name: &RuleName) -> Result<String, LoadError> {
    let path = rules.join(name.as_str()).with_extension(RULE_EXTENSION);
    fs::read_to_string(&path).map_err(|source| LoadError::UnreadableRule {
        name: name.clone(),
        path: path.display().to_string(),
        source,
    })
}

fn read(path: &Path, file: &'static str) -> Result<String, LoadError> {
    fs::read_to_string(path.join(file)).map_err(|source| LoadError::Unreadable { file, source })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{fs, path::Path};

    use blind_core::{CONTRACT_FILE, PROMPT_FILE, SETTINGS_FILE};

    pub fn write_agent(dir: &Path, folder: &str, name: &str) {
        let path = dir.join(folder);
        fs::create_dir(&path).expect("create agent folder");
        let contract = format!(
            r#"{{"name":"{name}","kind":"worker",
                "input":{{"type":"object","properties":{{}}}},
                "output":{{"type":"object","properties":{{}}}}}}"#
        );
        let settings = r#"{"description":"Does one job.","model":"local/tiny"}"#;
        fs::write(path.join(CONTRACT_FILE), contract).expect("write contract");
        fs::write(path.join(SETTINGS_FILE), settings).expect("write settings");
        fs::write(path.join(PROMPT_FILE), "Do the job.").expect("write prompt");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::write_agent;
    use super::*;

    fn outcome_text(loaded: &Loaded) -> String {
        match &loaded.outcome {
            Ok(agent) => format!("ok {}", agent.contract().name()),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn loads_agents_sorted_by_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "reviewer", "reviewer");
        write_agent(dir.path(), "explorer", "explorer");
        let loaded = load_all(dir.path()).expect("readable dir");
        let outcomes: Vec<String> = loaded.iter().map(outcome_text).collect();
        assert_eq!(outcomes, ["ok explorer", "ok reviewer"]);
    }

    #[test]
    fn plain_files_next_to_agent_folders_are_ignored() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("notes.txt"), "not an agent").expect("write file");
        assert!(load_all(dir.path()).expect("readable dir").is_empty());
    }

    #[test]
    fn missing_file_is_reported_by_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "explorer", "explorer");
        fs::remove_file(dir.path().join("explorer").join(PROMPT_FILE)).expect("remove prompt");
        let loaded = load_all(dir.path()).expect("readable dir");
        let error = outcome_text(&loaded[0]);
        assert!(error.starts_with("cannot read prompt.md: "), "{error}");
    }

    #[test]
    fn folder_must_match_the_contract_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "explorer", "reviewer");
        let loaded = load_all(dir.path()).expect("readable dir");
        assert_eq!(
            outcome_text(&loaded[0]),
            "folder is `explorer` but contract.json names the agent `reviewer`"
        );
    }

    fn refusal(dir: &Path, name: &str) -> String {
        load_one(dir, name)
            .expect_err("must be refused")
            .to_string()
    }

    #[test]
    fn one_agent_is_loaded_by_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "explorer", "explorer");
        let agent = load_one(dir.path(), "explorer").expect("valid agent");
        assert_eq!(agent.contract().name().as_str(), "explorer");
    }

    #[test]
    fn name_that_leaves_the_agents_folder_is_refused_before_any_file_is_read() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("agents")).expect("agents");
        write_agent(dir.path(), "outside", "outside");
        let agents = dir.path().join("agents");
        for name in ["../outside", "..", "a/b", "/etc", ""] {
            let refused = refusal(&agents, name);
            assert!(refused.starts_with("agent name `"), "{name:?}: {refused}");
        }
    }

    #[test]
    fn linked_agent_folder_is_refused_like_the_listing_skips_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("agents")).expect("agents");
        write_agent(dir.path(), "explorer", "explorer");
        let agents = dir.path().join("agents");
        std::os::unix::fs::symlink("../explorer", agents.join("explorer")).expect("link");
        assert!(load_all(&agents).expect("readable dir").is_empty());
        assert_eq!(
            refusal(&agents, "explorer"),
            "there is no agent folder `explorer`"
        );
    }

    #[test]
    fn unknown_agent_is_reported_by_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            refusal(dir.path(), "ghost"),
            "there is no agent folder `ghost`"
        );
    }

    fn agent_following(dir: &Path, rules: &str) -> PathBuf {
        let agents = dir.join("agents");
        fs::create_dir(&agents).expect("agents");
        write_agent(&agents, "explorer", "explorer");
        let settings =
            format!(r#"{{"description":"Does one job.","model":"local/tiny","rules":{rules}}}"#);
        fs::write(agents.join("explorer").join(SETTINGS_FILE), settings).expect("settings");
        agents
    }

    #[test]
    fn rules_are_read_from_the_folder_next_to_the_agents_and_follow_the_prompt() {
        let dir = tempfile::tempdir().expect("temp dir");
        let agents = agent_following(dir.path(), r#"["rust"]"#);
        fs::create_dir(dir.path().join("rules")).expect("rules");
        fs::write(dir.path().join("rules/rust.md"), "# Rust\nNo comments.\n").expect("rule");
        let agent = load_one(&agents, "explorer").expect("valid agent");
        assert_eq!(agent.prompt(), "Do the job.\n\n# Rust\nNo comments.\n");
    }

    #[test]
    fn missing_rule_file_is_reported_by_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let agents = agent_following(dir.path(), r#"["rust"]"#);
        let refused = refusal(&agents, "explorer");
        assert!(
            refused.starts_with("rule `rust`: cannot read "),
            "{refused}"
        );
        assert!(refused.contains("rules/rust.md"), "{refused}");
    }

    fn write_root(dir: &Path, contract: &str) {
        let root = dir.join("root");
        fs::create_dir(&root).expect("root");
        fs::write(root.join(CONTRACT_FILE), contract).expect("contract");
        fs::write(root.join(SETTINGS_FILE), r#"{"description":"Talks."}"#).expect("settings");
        fs::write(root.join(PROMPT_FILE), "Keep answers short.").expect("prompt");
    }

    #[test]
    fn root_is_loaded_from_its_own_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_root(
            dir.path(),
            r#"{"name":"root","kind":"root","input":{"type":"object","properties":{"summary":{"type":"string"}}}}"#,
        );
        let root = load_root(&dir.path().join("root"))
            .expect("valid root")
            .expect("root is there");
        assert!(root.contract().input().declares("summary"));
    }

    #[test]
    fn missing_root_folder_means_no_root_contract() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(
            load_root(&dir.path().join("root"))
                .expect("no error")
                .is_none()
        );
    }

    #[test]
    fn broken_root_is_an_error_and_not_silently_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_root(
            dir.path(),
            r#"{"name":"root","kind":"worker","input":{"type":"object","properties":{}},"output":{"type":"object","properties":{}}}"#,
        );
        let error = load_root(&dir.path().join("root")).expect_err("must be refused");
        assert_eq!(
            error.to_string(),
            "contract.json: `root` is a worker contract, not root"
        );
    }

    #[test]
    fn missing_agents_dir_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(load_all(&dir.path().join("absent")).is_err());
    }
}
