use std::{collections::BTreeSet, sync::Mutex};

use crate::policy::{PathPolicy, ProjectPath, Refusal};
use crate::workspace::{MAX_FILE_BYTES, ToolError, Workspace, WorkspaceError};

const SEPARATOR: &str = "/";

pub(crate) struct Writing {
    policy: PathPolicy,
    written: Mutex<BTreeSet<ProjectPath>>,
}

impl Workspace {
    pub fn writing(mut self, writable: &[String]) -> Result<Self, WorkspaceError> {
        if !writable.is_empty() {
            self.writing = Some(Writing {
                policy: PathPolicy::reading(writable)?,
                written: Mutex::default(),
            });
        }
        Ok(self)
    }

    pub fn can_write(&self) -> bool {
        self.writing.is_some()
    }

    pub fn written(&self) -> Vec<String> {
        let Some(writing) = &self.writing else {
            return Vec::new();
        };
        let written = writing
            .written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        written
            .iter()
            .map(|path| path.as_str().to_owned())
            .collect()
    }

    pub fn write_file(&self, raw_path: &str, content: &str) -> Result<String, ToolError> {
        let target = self.target(raw_path)?;
        if content.len() as u64 > MAX_FILE_BYTES {
            return Err(ToolError::TooLarge {
                path: target.as_str().to_owned(),
                limit: MAX_FILE_BYTES,
            });
        }
        self.store(&target, content)?;
        Ok(format!(
            "wrote {}: {} lines",
            target.as_str(),
            content.lines().count()
        ))
    }

    pub fn edit_file(&self, raw_path: &str, old: &str, new: &str) -> Result<String, ToolError> {
        let target = self.target(raw_path)?;
        let shown = || target.as_str().to_owned();
        if old.is_empty() {
            return Err(ToolError::EmptyEdit(shown()));
        }
        let text = self.text_of(&target)?;
        match text.matches(old).count() {
            0 => Err(ToolError::EditNotFound(shown())),
            1 => {
                self.store(&target, &text.replacen(old, new, 1))?;
                Ok(format!("edited {}", target.as_str()))
            }
            count => Err(ToolError::EditAmbiguous {
                path: shown(),
                count,
            }),
        }
    }

    fn target(&self, raw_path: &str) -> Result<ProjectPath, ToolError> {
        let Some(writing) = &self.writing else {
            return Err(Refusal::NotWritable(raw_path.to_owned()).into());
        };
        let path = ProjectPath::parse(raw_path)?;
        writable(&writing.policy, &path)?;
        let resolved = self.resolved_target(&path)?;
        writable(&writing.policy, &resolved)?;
        Ok(resolved)
    }

    fn resolved_target(&self, path: &ProjectPath) -> Result<ProjectPath, ToolError> {
        let parts: Vec<&str> = path.as_str().split(SEPARATOR).collect();
        let existing = (1..=parts.len()).rev().find_map(|count| {
            let prefix = parts[..count].join(SEPARATOR);
            let canonical = self.dir.canonicalize(&prefix).ok()?;
            Some((count, canonical.to_string_lossy().into_owned()))
        });
        let (count, base) = existing.unwrap_or((0, String::new()));
        let rest = parts[count..].join(SEPARATOR);
        let joined: Vec<&str> = [base.as_str(), rest.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect();
        Ok(ProjectPath::parse(&joined.join(SEPARATOR))?)
    }

    fn store(&self, target: &ProjectPath, content: &str) -> Result<(), ToolError> {
        let failed = |source: std::io::Error| ToolError::NotWritten {
            path: target.as_str().to_owned(),
            reason: source.to_string(),
        };
        if let Some((folder, _)) = target.as_str().rsplit_once(SEPARATOR) {
            self.dir.create_dir_all(folder).map_err(failed)?;
        }
        self.dir.write(target.as_str(), content).map_err(failed)?;
        if let Some(writing) = &self.writing {
            let mut written = writing
                .written
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            written.insert(target.clone());
        }
        Ok(())
    }
}

fn writable(policy: &PathPolicy, path: &ProjectPath) -> Result<(), Refusal> {
    match policy.permits(path) {
        Ok(()) => Ok(()),
        Err(Refusal::OutOfScope(_) | Refusal::Excluded(_)) => {
            Err(Refusal::NotWritable(path.as_str().to_owned()))
        }
        Err(
            refusal @ (Refusal::Protected(_)
            | Refusal::NotInsideProject(_)
            | Refusal::NotGiven(_)
            | Refusal::NotWritable(_)),
        ) => Err(refusal),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use tempfile::TempDir;

    use super::*;
    use crate::workspace::test_support::{reading, sample};

    fn writer(dir: &TempDir, writable: &[&str]) -> Workspace {
        let writable: Vec<String> = writable.iter().map(|raw| (*raw).to_owned()).collect();
        reading(dir, &["**"])
            .writing(&writable)
            .expect("valid write rights")
    }

    fn on_disk(dir: &TempDir, path: &str) -> String {
        fs::read_to_string(dir.path().join(path)).expect("file on disk")
    }

    #[test]
    fn new_file_is_written_with_its_folders_and_remembered() {
        let dir = sample();
        let workspace = writer(&dir, &["src/**"]);
        let answer = workspace
            .write_file("src/clip/board.rs", "fn copy() {}\n")
            .expect("written");
        assert_eq!(answer, "wrote src/clip/board.rs: 1 lines");
        assert_eq!(on_disk(&dir, "src/clip/board.rs"), "fn copy() {}\n");
        assert_eq!(workspace.written(), ["src/clip/board.rs"]);
    }

    #[test]
    fn existing_file_is_replaced_whole() {
        let dir = sample();
        let workspace = writer(&dir, &["src/**"]);
        workspace
            .write_file("./src//main.rs", "fn main() {}\n")
            .expect("written");
        assert_eq!(on_disk(&dir, "src/main.rs"), "fn main() {}\n");
        assert_eq!(workspace.written(), ["src/main.rs"]);
    }

    #[test]
    fn paths_outside_the_write_rights_are_refused_and_left_alone() {
        let dir = sample();
        let workspace = writer(&dir, &["src/**", "!src/tui/**"]);
        for path in ["docs/DESIGN.md", "src/tui/app.rs", "src/TUI/app.rs"] {
            let refused = workspace.write_file(path, "changed");
            assert!(
                matches!(refused, Err(ToolError::Refused(Refusal::NotWritable(_)))),
                "{path}: {refused:?}"
            );
        }
        assert_eq!(on_disk(&dir, "docs/DESIGN.md"), "# design\n");
        assert_eq!(on_disk(&dir, "src/tui/app.rs"), "pub struct App;\n");
        assert!(workspace.written().is_empty());
    }

    #[test]
    fn protected_and_outside_paths_are_refused_even_with_full_rights() {
        let dir = sample();
        let workspace = writer(&dir, &["**"]);
        for path in [
            ".env",
            ".git/config",
            ".blind/agents/coder/contract.json",
            "certs/server.pem",
        ] {
            let refused = workspace.write_file(path, "changed");
            assert!(
                matches!(refused, Err(ToolError::Refused(Refusal::Protected(_)))),
                "{path}: {refused:?}"
            );
        }
        for path in ["../outside.rs", "/etc/hosts"] {
            let refused = workspace.write_file(path, "changed");
            assert!(
                matches!(
                    refused,
                    Err(ToolError::Refused(Refusal::NotInsideProject(_)))
                ),
                "{path}: {refused:?}"
            );
        }
    }

    #[test]
    fn link_into_a_protected_folder_cannot_be_written_through() {
        let dir = sample();
        symlink("../.git", dir.path().join("src/git")).expect("folder link");
        symlink("../.env", dir.path().join("src/settings")).expect("file link");
        let workspace = writer(&dir, &["src/**"]);
        for path in ["src/git/config", "src/git/hooks/pre-commit", "src/settings"] {
            let refused = workspace.write_file(path, "changed");
            assert!(refused.is_err(), "{path}");
        }
        assert_ne!(on_disk(&dir, ".env"), "changed");
        assert_ne!(on_disk(&dir, ".git/config"), "changed");
        assert!(!dir.path().join(".git/hooks/pre-commit").exists());
    }

    #[test]
    fn link_that_leaves_the_project_cannot_be_written_through() {
        let dir = sample();
        let outside = tempfile::tempdir().expect("outside");
        symlink(outside.path(), dir.path().join("src/away")).expect("link");
        let workspace = writer(&dir, &["src/**"]);
        assert!(
            workspace
                .write_file("src/away/escape.rs", "changed")
                .is_err()
        );
        assert!(!outside.path().join("escape.rs").exists());
    }

    #[test]
    fn one_exact_piece_of_text_is_replaced() {
        let dir = sample();
        let workspace = writer(&dir, &["src/**"]);
        let answer = workspace
            .edit_file("src/main.rs", "run();", "start();")
            .expect("edited");
        assert_eq!(answer, "edited src/main.rs");
        assert_eq!(
            on_disk(&dir, "src/main.rs"),
            "fn main() {\n    start();\n}\n"
        );
        assert_eq!(workspace.written(), ["src/main.rs"]);
    }

    #[test]
    fn edit_needs_text_that_appears_exactly_once() {
        let dir = sample();
        let workspace = writer(&dir, &["src/**"]);
        assert!(matches!(
            workspace.edit_file("src/main.rs", "missing", "x"),
            Err(ToolError::EditNotFound(_))
        ));
        assert!(matches!(
            workspace.edit_file("src/main.rs", "n", "x"),
            Err(ToolError::EditAmbiguous { count: 3, .. })
        ));
        assert!(matches!(
            workspace.edit_file("src/main.rs", "", "x"),
            Err(ToolError::EmptyEdit(_))
        ));
        assert_eq!(on_disk(&dir, "src/main.rs"), "fn main() {\n    run();\n}\n");
        assert!(workspace.written().is_empty());
    }

    #[test]
    fn file_larger_than_the_limit_is_not_written() {
        let dir = sample();
        let workspace = writer(&dir, &["src/**"]);
        let huge = "x".repeat(usize::try_from(MAX_FILE_BYTES).expect("small") + 1);
        assert!(matches!(
            workspace.write_file("src/huge.rs", &huge),
            Err(ToolError::TooLarge { .. })
        ));
        assert!(!dir.path().join("src/huge.rs").exists());
    }

    #[test]
    fn workspace_without_write_rights_writes_nothing() {
        let dir = sample();
        let workspace = reading(&dir, &["**"]);
        assert!(!workspace.can_write());
        assert!(workspace.write_file("src/main.rs", "changed").is_err());
        assert_eq!(on_disk(&dir, "src/main.rs"), "fn main() {\n    run();\n}\n");
    }
}
