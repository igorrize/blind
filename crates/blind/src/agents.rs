use std::{fs, io, path::Path};

use blind_core::{
    Agent, AgentError, AgentName, CONTRACT_FILE, InvalidAgentName, PROMPT_FILE, SETTINGS_FILE,
};

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
    load_agent(&folder, name.as_str())
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
        outcome: load_agent(&dir.join(folder), folder),
    }
}

fn load_agent(path: &Path, folder: &str) -> Result<Agent, LoadError> {
    let contract = read(path, CONTRACT_FILE)?;
    let settings = read(path, SETTINGS_FILE)?;
    let prompt = read(path, PROMPT_FILE)?;
    let agent = Agent::from_parts(&contract, &settings, prompt)?;
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

    #[test]
    fn missing_agents_dir_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(load_all(&dir.path().join("absent")).is_err());
    }
}
