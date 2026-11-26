use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail, ensure};

const BRANCH_PREFIX: &str = "blind/";
const COPIES_FOLDER: &str = "blind/worktrees";
const BASE_MESSAGE: &str = "blind: base, the project as it was";
const AS_THE_HARNESS: [&str; 6] = [
    "-c",
    "user.name=blind",
    "-c",
    "user.email=blind@localhost",
    "-c",
    "commit.gpgsign=false",
];

pub struct Worktree {
    project: PathBuf,
    path: PathBuf,
    branch: String,
    base: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
    pub result: Option<String>,
    pub files: Vec<String>,
}

impl Worktree {
    pub fn create(project: &Path) -> anyhow::Result<Self> {
        Self::create_under(project, &copies_folder()?)
    }

    pub fn create_under(project: &Path, copies: &Path) -> anyhow::Result<Self> {
        let project = project
            .canonicalize()
            .with_context(|| format!("cannot open {}", project.display()))?;
        ensure_repository_root(&project)?;
        let (path, branch) = free_place(&project, copies);
        let place = path.to_string_lossy().into_owned();
        git(
            &project,
            &["worktree", "add", "-q", "-b", &branch, &place, "HEAD"],
        )?;
        let mut copy = Self {
            project,
            path,
            branch,
            base: String::new(),
        };
        copy.bring_uncommitted_work()?;
        copy.base = match copy.snapshot(BASE_MESSAGE)? {
            Some(base) => base,
            None => head(&copy.path)?,
        };
        Ok(copy)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn finish(&self, message: &str) -> anyhow::Result<Changes> {
        let result = self.snapshot(message)?;
        let files = match &result {
            Some(result) => lines(&git(
                &self.path,
                &["diff", "--name-only", &self.base, result],
            )?),
            None => Vec::new(),
        };
        if result.is_none() {
            self.remove()?;
        }
        Ok(Changes {
            path: self.path.clone(),
            branch: self.branch.clone(),
            base: self.base.clone(),
            result,
            files,
        })
    }

    fn bring_uncommitted_work(&self) -> anyhow::Result<()> {
        let patch = run(&self.project, &["diff", "--binary", "HEAD"], None)?.stdout;
        if !patch.is_empty() {
            let apply = ["apply", "--binary", "--whitespace=nowarn"];
            checked(&apply, run(&self.path, &apply, Some(&patch))?)?;
        }
        let untracked = git(
            &self.project,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?;
        for file in untracked.split('\0').filter(|file| !file.is_empty()) {
            self.bring(file)?;
        }
        Ok(())
    }

    fn bring(&self, file: &str) -> anyhow::Result<()> {
        let source = self.project.join(file);
        let is_plain_file = fs::symlink_metadata(&source).is_ok_and(|kind| kind.is_file());
        if !is_plain_file {
            return Ok(());
        }
        let target = self.path.join(file);
        if let Some(folder) = target.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::copy(&source, &target).with_context(|| format!("cannot copy {file} into the copy"))?;
        Ok(())
    }

    fn snapshot(&self, message: &str) -> anyhow::Result<Option<String>> {
        git(&self.path, &["add", "-A"])?;
        if git(&self.path, &["status", "--porcelain"])?.is_empty() {
            return Ok(None);
        }
        let commit = ["commit", "-q", "--no-verify", "-m", message];
        let as_the_harness: Vec<&str> = AS_THE_HARNESS.iter().chain(&commit).copied().collect();
        git(&self.path, &as_the_harness)?;
        head(&self.path).map(Some)
    }

    fn remove(&self) -> anyhow::Result<()> {
        let place = self.path.to_string_lossy().into_owned();
        git(&self.project, &["worktree", "remove", "--force", &place])?;
        git(&self.project, &["branch", "-q", "-D", &self.branch])?;
        Ok(())
    }
}

impl Changes {
    pub fn described(&self) -> String {
        let Some(result) = &self.result else {
            return "no files were changed; the copy is removed".to_owned();
        };
        let (base, result) = (short(&self.base), short(result));
        [
            "your project is untouched: the changes are in a copy".to_owned(),
            format!("changed: {}", self.files.join(", ")),
            format!("copy:    {}", self.path.display()),
            format!("branch:  {} (base {base}, result {result})", self.branch),
            format!("look:    git diff {base} {result}"),
            format!("take:    git diff {base} {result} | git apply"),
        ]
        .join("\n")
    }
}

fn short(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}

fn ensure_repository_root(project: &Path) -> anyhow::Result<()> {
    let top = git(project, &["rev-parse", "--show-toplevel"]).with_context(|| {
        format!(
            "agents that write need a git repository, and {} is not one",
            project.display()
        )
    })?;
    let top = PathBuf::from(top);
    let top = top.canonicalize().unwrap_or(top);
    ensure!(
        top == project,
        "agents that write need blind to run from the root of the repository: {}",
        top.display()
    );
    if !succeeds(project, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
        bail!("agents that write need at least one commit in the repository");
    }
    Ok(())
}

fn free_place(project: &Path, copies: &Path) -> (PathBuf, String) {
    let name = project
        .file_name()
        .map_or_else(|| "project".into(), |name| name.to_string_lossy());
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut run = started;
    loop {
        let path = copies.join(name.as_ref()).join(run.to_string());
        let branch = format!("{BRANCH_PREFIX}{run}");
        let reference = format!("refs/heads/{branch}");
        let taken =
            path.exists() || succeeds(project, &["show-ref", "--verify", "--quiet", &reference]);
        if !taken {
            return (path, branch);
        }
        run += 1;
    }
}

fn copies_folder() -> anyhow::Result<PathBuf> {
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
    let cache = cache.context("neither XDG_CACHE_HOME nor HOME is set, nowhere to put a copy")?;
    Ok(cache.join(COPIES_FOLDER))
}

fn head(dir: &Path) -> anyhow::Result<String> {
    git(dir, &["rev-parse", "HEAD"])
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

fn git(dir: &Path, arguments: &[&str]) -> anyhow::Result<String> {
    let output = checked(arguments, run(dir, arguments, None)?)?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned())
}

fn succeeds(dir: &Path, arguments: &[&str]) -> bool {
    run(dir, arguments, None).is_ok_and(|output| output.status.success())
}

fn checked(arguments: &[&str], output: Output) -> anyhow::Result<Output> {
    ensure!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

fn run(dir: &Path, arguments: &[&str], input: Option<&[u8]>) -> anyhow::Result<Output> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("git did not start; agents that write need git")?;
    let fed = match (child.stdin.take(), input) {
        (Some(mut stdin), Some(input)) => stdin.write_all(input),
        (Some(_), None) | (None, Some(_)) | (None, None) => Ok(()),
    };
    let output = child.wait_with_output().context("git did not finish")?;
    fed.context("git did not take its input")?;
    Ok(output)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{fs, path::Path};

    use tempfile::TempDir;

    use super::git;

    pub fn repository(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        git(dir.path(), &["init", "-q", "-b", "main"]).expect("git init");
        git(dir.path(), &["config", "user.name", "tester"]).expect("name");
        git(dir.path(), &["config", "user.email", "tester@localhost"]).expect("email");
        for (path, content) in files {
            write(dir.path(), path, content);
        }
        git(dir.path(), &["add", "-A"]).expect("add");
        git(dir.path(), &["commit", "-q", "-m", "first"]).expect("commit");
        dir
    }

    pub fn write(root: &Path, path: &str, content: &str) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().expect("parent")).expect("folders");
        fs::write(file, content).expect("write");
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::test_support::{repository, write};
    use super::*;

    const MAIN: &str = "fn main() {}\n";

    fn project() -> TempDir {
        repository(&[
            ("src/main.rs", MAIN),
            ("README.md", "# demo\n"),
            (".gitignore", "target/\n"),
        ])
    }

    fn copy_of(project: &TempDir, copies: &TempDir) -> Worktree {
        Worktree::create_under(project.path(), copies.path()).expect("a copy")
    }

    fn read(root: &Path, path: &str) -> String {
        fs::read_to_string(root.join(path)).expect("file")
    }

    fn refusal(project: &Path) -> String {
        let copies = tempfile::tempdir().expect("copies");
        let Err(error) = Worktree::create_under(project, copies.path()) else {
            panic!("a copy must be refused");
        };
        format!("{error:#}")
    }

    #[test]
    fn copy_holds_committed_uncommitted_and_new_files_but_not_ignored_ones() {
        let project = project();
        write(project.path(), "src/main.rs", "fn main() { run(); }\n");
        write(project.path(), "src/new.rs", "pub fn run() {}\n");
        write(project.path(), "target/debug/blind", "binary");
        let copies = tempfile::tempdir().expect("copies");
        let copy = copy_of(&project, &copies);
        assert_eq!(read(copy.path(), "src/main.rs"), "fn main() { run(); }\n");
        assert_eq!(read(copy.path(), "src/new.rs"), "pub fn run() {}\n");
        assert_eq!(read(copy.path(), "README.md"), "# demo\n");
        assert!(!copy.path().join("target").exists());
    }

    #[test]
    fn making_a_copy_leaves_the_project_as_it_was() {
        let project = project();
        write(project.path(), "src/main.rs", "fn main() { run(); }\n");
        let before = git(project.path(), &["status", "--porcelain"]).expect("status");
        let head_before = head(project.path()).expect("head");
        let copies = tempfile::tempdir().expect("copies");
        copy_of(&project, &copies);
        assert_eq!(
            git(project.path(), &["status", "--porcelain"]).expect("status"),
            before
        );
        assert_eq!(head(project.path()).expect("head"), head_before);
        assert_eq!(
            git(project.path(), &["branch", "--show-current"]).expect("branch"),
            "main"
        );
    }

    #[test]
    fn finished_copy_names_only_what_was_changed_after_the_base() {
        let project = project();
        write(project.path(), "src/main.rs", "fn main() { run(); }\n");
        let copies = tempfile::tempdir().expect("copies");
        let copy = copy_of(&project, &copies);
        write(copy.path(), "src/clipboard.rs", "pub fn copy() {}\n");
        write(copy.path(), "README.md", "# demo\n\nCopies text.\n");
        let changes = copy.finish("blind: add clipboard").expect("finished");
        assert_eq!(changes.files, ["README.md", "src/clipboard.rs"]);
        assert!(changes.result.is_some());
        assert_ne!(Some(&changes.base), changes.result.as_ref());
        assert_eq!(read(project.path(), "README.md"), "# demo\n");
        assert!(!project.path().join("src/clipboard.rs").exists());
    }

    #[test]
    fn take_command_brings_the_changes_into_the_project_on_top_of_local_work() {
        let project = project();
        write(project.path(), "src/main.rs", "fn main() { run(); }\n");
        let copies = tempfile::tempdir().expect("copies");
        let copy = copy_of(&project, &copies);
        write(copy.path(), "src/main.rs", "fn main() { run(); done(); }\n");
        write(copy.path(), "src/clipboard.rs", "pub fn copy() {}\n");
        let changes = copy.finish("blind: add clipboard").expect("finished");
        let result = changes.result.expect("a result");
        let patch = run(
            project.path(),
            &["diff", "--binary", &changes.base, &result],
            None,
        )
        .expect("diff")
        .stdout;
        let applied = run(project.path(), &["apply"], Some(&patch)).expect("apply");
        assert!(
            applied.status.success(),
            "{}",
            String::from_utf8_lossy(&applied.stderr)
        );
        assert_eq!(
            read(project.path(), "src/main.rs"),
            "fn main() { run(); done(); }\n"
        );
        assert_eq!(
            read(project.path(), "src/clipboard.rs"),
            "pub fn copy() {}\n"
        );
    }

    #[test]
    fn copy_without_changes_is_removed_with_its_branch() {
        let project = project();
        let copies = tempfile::tempdir().expect("copies");
        let copy = copy_of(&project, &copies);
        let changes = copy.finish("blind: nothing").expect("finished");
        assert_eq!(changes.result, None);
        assert!(changes.files.is_empty());
        assert!(!copy.path().exists());
        assert_eq!(
            git(project.path(), &["branch", "--list", "blind/*"]).expect("branches"),
            ""
        );
        assert_eq!(
            changes.described(),
            "no files were changed; the copy is removed"
        );
    }

    #[test]
    fn two_copies_of_one_project_get_their_own_places() {
        let project = project();
        let copies = tempfile::tempdir().expect("copies");
        let first = copy_of(&project, &copies);
        let second = copy_of(&project, &copies);
        assert_ne!(first.path(), second.path());
        assert_ne!(first.branch, second.branch);
    }

    #[test]
    fn harness_commits_under_its_own_name_and_skips_hooks() {
        let project = project();
        write(
            project.path(),
            ".git/hooks/pre-commit",
            "#!/bin/sh\nexit 1\n",
        );
        let hook = project.path().join(".git/hooks/pre-commit");
        let mut rights = fs::metadata(&hook).expect("hook").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut rights, 0o755);
        fs::set_permissions(&hook, rights).expect("executable hook");
        let copies = tempfile::tempdir().expect("copies");
        let copy = copy_of(&project, &copies);
        write(copy.path(), "src/new.rs", "pub fn run() {}\n");
        let changes = copy.finish("blind: add new").expect("finished");
        let result = changes.result.expect("a result");
        let author = git(copy.path(), &["log", "-1", "--format=%an <%ae>", &result]).expect("log");
        assert_eq!(author, "blind <blind@localhost>");
    }

    #[test]
    fn description_tells_where_the_changes_are_and_how_to_take_them() {
        let changes = Changes {
            path: PathBuf::from("/copies/demo/17"),
            branch: "blind/17".to_owned(),
            base: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            result: Some("bbbbbbbbbbbbbbbbbbbb".to_owned()),
            files: vec!["README.md".to_owned(), "src/new.rs".to_owned()],
        };
        assert_eq!(
            changes.described(),
            "your project is untouched: the changes are in a copy\n\
             changed: README.md, src/new.rs\n\
             copy:    /copies/demo/17\n\
             branch:  blind/17 (base aaaaaaaaaa, result bbbbbbbbbb)\n\
             look:    git diff aaaaaaaaaa bbbbbbbbbb\n\
             take:    git diff aaaaaaaaaa bbbbbbbbbb | git apply"
        );
    }

    #[test]
    fn folder_that_is_not_a_repository_cannot_be_copied() {
        let plain = tempfile::tempdir().expect("temp dir");
        let refused = refusal(plain.path());
        assert!(
            refused.starts_with("agents that write need a git repository"),
            "{refused}"
        );
    }

    #[test]
    fn repository_without_a_commit_cannot_be_copied() {
        let empty = tempfile::tempdir().expect("temp dir");
        git(empty.path(), &["init", "-q"]).expect("git init");
        assert_eq!(
            refusal(empty.path()),
            "agents that write need at least one commit in the repository"
        );
    }

    #[test]
    fn subfolder_of_a_repository_cannot_be_copied() {
        let project = project();
        let refused = refusal(&project.path().join("src"));
        assert!(
            refused
                .starts_with("agents that write need blind to run from the root of the repository"),
            "{refused}"
        );
    }
}
