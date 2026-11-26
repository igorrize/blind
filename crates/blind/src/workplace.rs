use std::path::{Path, PathBuf};

use blind_core::Agent;

use crate::step::Ground;
use crate::worktree::{Changes, Worktree};

const LONGEST_SUBJECT: usize = 72;

pub struct Workplace {
    project: PathBuf,
    copy: Option<Worktree>,
}

impl Workplace {
    pub fn for_agents<'a>(
        agents: impl IntoIterator<Item = &'a Agent>,
        project: &Path,
    ) -> anyhow::Result<Self> {
        let someone_writes = agents.into_iter().any(writes);
        let copy = if someone_writes {
            Some(Worktree::create(project)?)
        } else {
            None
        };
        Ok(Self {
            project: project.to_owned(),
            copy,
        })
    }

    pub fn ground(&self) -> Ground<'_> {
        match &self.copy {
            Some(copy) => Ground::Copy(copy.path()),
            None => Ground::Project(&self.project),
        }
    }

    pub fn changes(&self, agents: &str, task: &str) -> anyhow::Result<Option<Changes>> {
        let Some(copy) = &self.copy else {
            return Ok(None);
        };
        copy.finish(&subject(agents, task)).map(Some)
    }
}

fn writes(agent: &Agent) -> bool {
    !agent.contract().rights().write().is_empty()
}

fn subject(agents: &str, task: &str) -> String {
    let first_line = task.lines().next().unwrap_or_default();
    let whole = format!("blind: {agents}: {first_line}");
    whole.chars().take(LONGEST_SUBJECT).collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    use super::Workplace;

    pub fn in_place(project: &Path) -> Workplace {
        Workplace {
            project: project.to_owned(),
            copy: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::test_support::{NO_FIELDS, agent};
    use crate::worktree::test_support::repository;

    fn reader() -> Agent {
        agent("explorer", NO_FIELDS, NO_FIELDS, r#"{"read":["**"]}"#)
    }

    #[test]
    fn agents_that_only_read_work_in_the_project_itself() {
        let project = tempfile::tempdir().expect("temp dir");
        let workplace = Workplace::for_agents([&reader()], project.path()).expect("a workplace");
        assert_eq!(workplace.ground(), Ground::Project(project.path()));
        assert_eq!(workplace.changes("explorer", "look").expect("ok"), None);
    }

    #[test]
    fn writer_without_a_repository_gets_no_workplace() {
        let plain = tempfile::tempdir().expect("temp dir");
        let writer = agent(
            "writer",
            NO_FIELDS,
            NO_FIELDS,
            r#"{"read":["**"],"write":["src/**"]}"#,
        );
        let Err(error) = Workplace::for_agents([&reader(), &writer], plain.path()) else {
            panic!("must be refused");
        };
        assert!(
            format!("{error:#}").starts_with("agents that write need a git repository"),
            "{error:#}"
        );
    }

    #[test]
    fn repository_is_not_needed_while_nobody_writes() {
        let project = repository(&[("src/main.rs", "fn main() {}\n")]);
        let workplace = Workplace::for_agents([&reader()], project.path()).expect("a workplace");
        assert_eq!(workplace.ground(), Ground::Project(project.path()));
    }

    #[test]
    fn subject_names_the_agents_and_the_first_line_of_the_task_within_the_limit() {
        assert_eq!(
            subject("red → writer", "add copy\nwith details"),
            "blind: red → writer: add copy"
        );
        let long = subject("writer", &"x".repeat(200));
        assert_eq!(long.chars().count(), LONGEST_SUBJECT);
    }
}
