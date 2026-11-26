use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use blind_core::ReadingTool;
use cap_std::{ambient_authority, fs::Dir};
use globset::GlobMatcher;
use ignore::WalkBuilder;

use crate::policy::{PathPolicy, PolicyError, ProjectPath, Refusal, pattern};
use crate::search::Index;
use crate::writing::Writing;

pub const MAX_FILE_BYTES: u64 = 256 * 1024;
pub const MAX_LISTED: usize = 500;

const MAX_LINE_CHARS: usize = 300;
const GIT_FOLDER: &str = ".git";

pub struct Workspace {
    root: PathBuf,
    pub(crate) dir: Dir,
    policy: PathPolicy,
    tools: Vec<ReadingTool>,
    index: OnceLock<Result<Index, String>>,
    pub(crate) writing: Option<Writing>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineRange {
    pub first: Option<usize>,
    pub last: Option<usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("cannot open the project folder {path}: {source}")]
    Unreadable { path: String, source: io::Error },
    #[error(transparent)]
    Policy(#[from] PolicyError),
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    BadPattern(#[from] PolicyError),
    #[error("`{0}` does not exist or is not a file")]
    NotAFile(String),
    #[error("`{path}` is larger than {limit} bytes")]
    TooLarge { path: String, limit: u64 },
    #[error("`{0}` is not a text file")]
    NotText(String),
    #[error("`{0}` is not a valid regular expression")]
    BadRegex(String),
    #[error("arguments do not fit the tool: {0}")]
    BadArguments(String),
    #[error("search is not available: {0}")]
    SearchUnavailable(String),
    #[error("`{path}` was not written: {reason}")]
    NotWritten { path: String, reason: String },
    #[error("`old` is empty: give the exact text to replace in `{0}`")]
    EmptyEdit(String),
    #[error("`old` text is not in `{0}`: read the file and copy the text exactly")]
    EditNotFound(String),
    #[error(
        "`old` text appears {count} times in `{path}`: add surrounding lines so it appears once"
    )]
    EditAmbiguous { path: String, count: usize },
}

impl Workspace {
    pub fn open(
        root: &Path,
        readable: &[String],
        tools: &[ReadingTool],
    ) -> Result<Self, WorkspaceError> {
        let unreadable = |source| WorkspaceError::Unreadable {
            path: root.display().to_string(),
            source,
        };
        let root = root.canonicalize().map_err(unreadable)?;
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(unreadable)?;
        let policy = PathPolicy::reading(readable)?;
        Ok(Self {
            root,
            dir,
            policy,
            tools: tools.to_vec(),
            index: OnceLock::new(),
            writing: None,
        })
    }

    pub fn read_file(
        &self,
        raw_path: &str,
        lines: LineRange,
        given: Option<&[String]>,
    ) -> Result<String, ToolError> {
        let path = ProjectPath::parse(raw_path)?;
        if given.is_some_and(|given| !is_among(&path, given)) {
            return Err(Refusal::NotGiven(path.as_str().to_owned()).into());
        }
        Ok(numbered(&self.text_of(&path)?, lines))
    }

    pub(crate) fn tools(&self) -> &[ReadingTool] {
        &self.tools
    }

    pub fn list_files(&self, raw_pattern: &str) -> Result<String, ToolError> {
        let found = self.readable_files(&pattern(raw_pattern)?.compile_matcher());
        let names: Vec<String> = found.iter().map(|path| path.as_str().to_owned()).collect();
        Ok(capped(names, MAX_LISTED, "files"))
    }

    pub(crate) fn index(&self) -> Result<&Index, ToolError> {
        let built = self.index.get_or_init(|| Index::build(&self.root));
        built
            .as_ref()
            .map_err(|reason| ToolError::SearchUnavailable(reason.clone()))
    }

    pub(crate) fn readable(&self, raw_path: &str) -> Option<ProjectPath> {
        let path = ProjectPath::parse(raw_path).ok()?;
        self.policy.permits(&path).ok()?;
        self.resolved(&path).ok()?;
        Some(path)
    }

    pub(crate) fn text_of(&self, path: &ProjectPath) -> Result<String, ToolError> {
        self.policy.permits(path)?;
        let resolved = self.resolved(path)?;
        let not_a_file = || ToolError::NotAFile(path.as_str().to_owned());
        let file = self.dir.open(resolved.as_str()).map_err(|_| not_a_file())?;
        let metadata = file.metadata().map_err(|_| not_a_file())?;
        if !metadata.is_file() {
            return Err(not_a_file());
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(ToolError::TooLarge {
                path: path.as_str().to_owned(),
                limit: MAX_FILE_BYTES,
            });
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|_| not_a_file())?;
        text(bytes).ok_or_else(|| ToolError::NotText(path.as_str().to_owned()))
    }

    fn resolved(&self, path: &ProjectPath) -> Result<ProjectPath, ToolError> {
        let not_a_file = || ToolError::NotAFile(path.as_str().to_owned());
        let canonical = self
            .dir
            .canonicalize(path.as_str())
            .map_err(|_| not_a_file())?;
        let resolved = ProjectPath::parse(&canonical.to_string_lossy())?;
        self.policy.permits(&resolved)?;
        Ok(resolved)
    }

    fn readable_files(&self, wanted: &GlobMatcher) -> Vec<ProjectPath> {
        let walk = WalkBuilder::new(&self.root)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != GIT_FOLDER)
            .build();
        let mut files: Vec<ProjectPath> = walk
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .filter_map(|entry| self.project_path(entry.path()))
            .filter(|path| wanted.is_match(path.as_str()) && self.policy.permits(path).is_ok())
            .collect();
        files.sort();
        files
    }

    fn project_path(&self, absolute: &Path) -> Option<ProjectPath> {
        let relative = absolute.strip_prefix(&self.root).ok()?;
        ProjectPath::parse(&relative.to_string_lossy()).ok()
    }
}

fn is_among(path: &ProjectPath, given: &[String]) -> bool {
    given
        .iter()
        .filter_map(|raw| ProjectPath::parse(raw).ok())
        .any(|listed| listed == *path)
}

pub(crate) fn shortened(line: &str) -> String {
    line.trim().chars().take(MAX_LINE_CHARS).collect()
}

pub(crate) fn capped(mut lines: Vec<String>, limit: usize, what: &str) -> String {
    if lines.is_empty() {
        return format!("no {what}");
    }
    let hidden = lines.len().saturating_sub(limit);
    lines.truncate(limit);
    if hidden > 0 {
        lines.push(format!("more {what} exist, narrow the request"));
    }
    lines.join("\n")
}

fn text(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    (!text.contains('\0')).then_some(text)
}

fn numbered(text: &str, lines: LineRange) -> String {
    let first = lines.first.unwrap_or(1);
    let last = lines.last.unwrap_or(usize::MAX);
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .filter(|(number, _)| (first..=last).contains(number))
        .map(|(number, line)| format!("{number:>5}  {line}\n"))
        .collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{fs, path::Path};

    use blind_core::ReadingTool;
    use tempfile::TempDir;

    use super::Workspace;

    pub const SECRET: &str = "TOP-SECRET-VALUE";

    pub fn project(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for (path, content) in files {
            write(dir.path(), path, content.as_bytes());
        }
        dir
    }

    pub fn sample() -> TempDir {
        project(&[
            ("src/main.rs", "fn main() {\n    run();\n}\n"),
            ("src/tui/app.rs", "pub struct App;\n"),
            ("docs/DESIGN.md", "# design\n"),
            (".env", SECRET),
            (".git/config", SECRET),
            (".blind/agents/coder/contract.json", SECRET),
            ("certs/server.pem", SECRET),
        ])
    }

    pub fn write(root: &Path, path: &str, content: &[u8]) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().expect("parent")).expect("folders");
        fs::write(file, content).expect("write file");
    }

    pub fn reading(dir: &TempDir, patterns: &[&str]) -> Workspace {
        reading_with(dir, patterns, &ReadingTool::ALL)
    }

    pub fn reading_with(dir: &TempDir, patterns: &[&str], tools: &[ReadingTool]) -> Workspace {
        let patterns: Vec<String> = patterns.iter().map(|raw| (*raw).to_owned()).collect();
        Workspace::open(dir.path(), &patterns, tools).expect("workspace opens")
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::test_support::{SECRET, project, reading, sample, write};
    use super::*;

    fn whole(workspace: &Workspace, path: &str) -> Result<String, ToolError> {
        workspace.read_file(path, LineRange::default(), None)
    }

    fn refusal(workspace: &Workspace, path: &str) -> String {
        whole(workspace, path)
            .expect_err("must be refused")
            .to_string()
    }

    #[test]
    fn file_in_scope_is_read_with_line_numbers() {
        let dir = sample();
        let text = whole(&reading(&dir, &["src/**"]), "src/main.rs").expect("readable");
        assert_eq!(text, "    1  fn main() {\n    2      run();\n    3  }\n");
    }

    #[test]
    fn line_range_limits_what_is_read() {
        let dir = sample();
        let lines = LineRange {
            first: Some(2),
            last: Some(2),
        };
        let text = reading(&dir, &["src/**"])
            .read_file("src/main.rs", lines, None)
            .expect("readable");
        assert_eq!(text, "    2      run();\n");
    }

    #[test]
    fn file_outside_scope_is_refused() {
        let dir = sample();
        assert_eq!(
            refusal(&reading(&dir, &["src/**"]), "docs/DESIGN.md"),
            "`docs/DESIGN.md` is outside the paths this agent may read"
        );
    }

    #[test]
    fn protected_files_are_refused_even_with_full_scope() {
        let dir = sample();
        let workspace = reading(&dir, &["**"]);
        for path in [".env", ".git/config", ".GIT/config", "certs/server.pem"] {
            let refused = refusal(&workspace, path);
            assert!(refused.contains("protected"), "{path}: {refused}");
            assert!(!refused.contains(SECRET));
        }
    }

    #[test]
    fn paths_that_leave_the_project_are_refused() {
        let dir = sample();
        let workspace = reading(&dir, &["**"]);
        for path in ["../outside.txt", "/etc/passwd", "src/../../outside.txt"] {
            assert!(refusal(&workspace, path).contains("not a path inside the project"));
        }
    }

    #[test]
    fn link_that_points_outside_the_project_cannot_be_read() {
        let outside = project(&[("secret.txt", SECRET)]);
        let dir = sample();
        let target = outside.path().join("secret.txt");
        symlink(target, dir.path().join("src/link.rs")).expect("link");
        let read = whole(&reading(&dir, &["**"]), "src/link.rs");
        assert!(matches!(read, Err(ToolError::NotAFile(_))), "{read:?}");
    }

    #[test]
    fn link_that_points_at_a_protected_file_cannot_be_read() {
        let dir = sample();
        symlink("../.env", dir.path().join("src/settings.rs")).expect("link");
        let read = whole(&reading(&dir, &["**"]), "src/settings.rs");
        let protected = matches!(read, Err(ToolError::Refused(Refusal::Protected(_))));
        assert!(protected, "{read:?}");
    }

    #[test]
    fn link_with_an_absolute_target_cannot_be_read_even_inside_the_project() {
        let dir = sample();
        symlink(dir.path().join(".env"), dir.path().join("src/settings.rs")).expect("link");
        let read = whole(&reading(&dir, &["**"]), "src/settings.rs");
        assert!(matches!(read, Err(ToolError::NotAFile(_))), "{read:?}");
    }

    #[test]
    fn link_that_points_out_of_scope_cannot_be_read() {
        let dir = sample();
        symlink("../docs/DESIGN.md", dir.path().join("src/design.rs")).expect("link");
        let read = whole(&reading(&dir, &["src/**"]), "src/design.rs");
        let out_of_scope = matches!(read, Err(ToolError::Refused(Refusal::OutOfScope(_))));
        assert!(out_of_scope, "{read:?}");
    }

    #[test]
    fn folder_missing_file_binary_and_huge_file_are_errors() {
        let dir = sample();
        write(dir.path(), "src/image.bin", &[0, 159, 146, 150]);
        write(dir.path(), "src/zeros.txt", b"a\0b");
        let huge = vec![b'x'; usize::try_from(MAX_FILE_BYTES).expect("fits") + 1];
        write(dir.path(), "src/huge.txt", &huge);
        let workspace = reading(&dir, &["src/**"]);
        let read = |path| whole(&workspace, path);
        assert!(matches!(read("src/tui"), Err(ToolError::NotAFile(_))));
        assert!(matches!(read("src/absent.rs"), Err(ToolError::NotAFile(_))));
        assert!(matches!(read("src/image.bin"), Err(ToolError::NotText(_))));
        assert!(matches!(read("src/zeros.txt"), Err(ToolError::NotText(_))));
        assert!(matches!(
            read("src/huge.txt"),
            Err(ToolError::TooLarge { .. })
        ));
    }

    #[test]
    fn listing_shows_only_readable_files_sorted() {
        let dir = sample();
        let listed = reading(&dir, &["**"]).list_files("**").expect("listed");
        assert_eq!(listed, "docs/DESIGN.md\nsrc/main.rs\nsrc/tui/app.rs");
    }

    #[test]
    fn excluded_files_are_neither_listed_nor_read() {
        let dir = sample();
        let workspace = reading(&dir, &["**", "!src/tui/**"]);
        let listed = workspace.list_files("**").expect("listed");
        assert_eq!(listed, "docs/DESIGN.md\nsrc/main.rs");
        let refused = workspace.read_file("src/tui/app.rs", LineRange::default(), None);
        assert!(matches!(
            refused,
            Err(ToolError::Refused(Refusal::Excluded(_)))
        ));
    }

    #[test]
    fn listing_respects_scope_pattern_and_gitignore() {
        let dir = sample();
        write(dir.path(), ".gitignore", b"target/\n");
        write(dir.path(), "target/debug/build.rs", b"generated");
        let workspace = reading(&dir, &["src/**", "target/**"]);
        assert_eq!(
            workspace.list_files("**/*.rs").expect("listed"),
            "src/main.rs\nsrc/tui/app.rs"
        );
        assert_eq!(workspace.list_files("docs/**").expect("listed"), "no files");
    }

    #[test]
    fn links_are_not_listed() {
        let dir = sample();
        symlink("../.env", dir.path().join("src/settings.rs")).expect("link");
        let listed = reading(&dir, &["**"]).list_files("src/*").expect("listed");
        assert_eq!(listed, "src/main.rs");
    }

    fn given(workspace: &Workspace, path: &str, given: &[&str]) -> Result<String, ToolError> {
        let given: Vec<String> = given.iter().map(|raw| (*raw).to_owned()).collect();
        workspace.read_file(path, LineRange::default(), Some(&given))
    }

    #[test]
    fn only_given_files_can_be_read_when_a_list_is_given() {
        let dir = sample();
        let workspace = reading(&dir, &["**"]);
        assert!(given(&workspace, "src/main.rs", &["src/main.rs"]).is_ok());
        let refused = given(&workspace, "src/tui/app.rs", &["src/main.rs"]).expect_err("refused");
        assert_eq!(
            refused.to_string(),
            "`src/tui/app.rs` is not among the files given to this agent"
        );
    }

    #[test]
    fn given_path_is_matched_after_normalizing() {
        let dir = sample();
        let workspace = reading(&dir, &["**"]);
        assert!(given(&workspace, "src/main.rs", &["./src//main.rs"]).is_ok());
        assert!(given(&workspace, "./src/main.rs", &["src/main.rs"]).is_ok());
    }

    #[test]
    fn empty_list_means_nothing_can_be_read() {
        let dir = sample();
        assert!(given(&reading(&dir, &["**"]), "src/main.rs", &[]).is_err());
    }

    #[test]
    fn given_list_cannot_widen_the_scope_or_open_protected_files() {
        let dir = sample();
        let narrow = reading(&dir, &["src/**"]);
        let out_of_scope = given(&narrow, "docs/DESIGN.md", &["docs/DESIGN.md"]);
        assert!(matches!(
            out_of_scope,
            Err(ToolError::Refused(Refusal::OutOfScope(_)))
        ));
        let protected = given(&reading(&dir, &["**"]), ".env", &[".env"]);
        assert!(matches!(
            protected,
            Err(ToolError::Refused(Refusal::Protected(_)))
        ));
        let outside = given(&reading(&dir, &["**"]), "../x", &["../x"]);
        assert!(matches!(
            outside,
            Err(ToolError::Refused(Refusal::NotInsideProject(_)))
        ));
    }

    #[test]
    fn bad_pattern_is_an_error() {
        let dir = sample();
        let listed = reading(&dir, &["**"]).list_files("../**");
        assert!(matches!(listed, Err(ToolError::BadPattern(_))));
    }

    #[test]
    fn long_results_are_capped() {
        let lines: Vec<String> = (0..MAX_LISTED + 3)
            .map(|index| format!("f{index}"))
            .collect();
        let shown = capped(lines, MAX_LISTED, "files");
        assert_eq!(shown.lines().count(), MAX_LISTED + 1);
        assert!(shown.ends_with("more files exist, narrow the request"));
    }
}
