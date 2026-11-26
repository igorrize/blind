use std::{path::Path, process::ExitCode};

use anyhow::Context;

use crate::agents::{self, Loaded};

const ROOT_FOLDER: &str = "root";

pub fn run(dir: &Path) -> anyhow::Result<ExitCode> {
    let loaded = agents::load_all(dir)
        .with_context(|| format!("cannot read agents from {}", dir.display()))?;
    print!("{}", report(&loaded));
    let root_fine = check_root(&root_folder(dir));
    Ok(exit_code(&loaded, root_fine))
}

fn root_folder(agents_dir: &Path) -> std::path::PathBuf {
    agents_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(ROOT_FOLDER)
}

fn check_root(dir: &Path) -> bool {
    match agents::load_root(dir) {
        Ok(Some(_)) => {
            println!("ok     {ROOT_FOLDER}  model chosen in settings");
            true
        }
        Ok(None) => true,
        Err(error) => {
            println!("error  {ROOT_FOLDER}  {error}");
            false
        }
    }
}

fn report(loaded: &[Loaded]) -> String {
    if loaded.is_empty() {
        return "no agents found\n".to_owned();
    }
    loaded.iter().map(line).collect()
}

fn line(loaded: &Loaded) -> String {
    match &loaded.outcome {
        Ok(agent) => format!("ok     {}  {}\n", loaded.folder, agent.settings().model()),
        Err(error) => format!("error  {}  {error}\n", loaded.folder),
    }
}

fn exit_code(loaded: &[Loaded], root_fine: bool) -> ExitCode {
    if root_fine && loaded.iter().all(|agent| agent.outcome.is_ok()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::test_support::write_agent;

    #[test]
    fn report_lists_valid_and_broken_agents() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_agent(dir.path(), "explorer", "explorer");
        write_agent(dir.path(), "fixer", "reviewer");
        let loaded = agents::load_all(dir.path()).expect("readable dir");
        assert_eq!(
            report(&loaded),
            "ok     explorer  local/tiny\n\
             error  fixer  folder is `fixer` but contract.json names the agent `reviewer`\n"
        );
        assert_eq!(exit_code(&loaded, true), ExitCode::FAILURE);
    }

    #[test]
    fn report_says_when_there_are_no_agents() {
        assert_eq!(report(&[]), "no agents found\n");
        assert_eq!(exit_code(&[], true), ExitCode::SUCCESS);
        assert_eq!(exit_code(&[], false), ExitCode::FAILURE);
    }
}
