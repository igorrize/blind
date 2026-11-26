use std::{path::Path, time::Duration};

use fff_search::file_picker::FilePicker;
use fff_search::{
    FFFMode, FilePickerOptions, FuzzySearchOptions, GrepMode, GrepResult, GrepSearchOptions,
    PaginationArgs, QueryParser, SharedFilePicker, SharedFrecency, parse_grep_query,
};

use crate::workspace::{ToolError, Workspace, capped, shortened};

pub const MAX_FOUND: usize = 30;
pub const MAX_MATCHES: usize = 100;

const SCAN_PATIENCE: Duration = Duration::from_secs(20);
const PAGE: usize = 100;
const NOT_READY: &str = "the project index is not ready";

pub struct Index {
    picker: SharedFilePicker,
}

impl Index {
    pub fn build(root: &Path) -> Result<Self, String> {
        let picker = SharedFilePicker::default();
        let options = FilePickerOptions {
            base_path: root.to_string_lossy().into_owned(),
            mode: FFFMode::Ai,
            watch: false,
            ..Default::default()
        };
        FilePicker::new_with_shared_state(picker.clone(), SharedFrecency::default(), options)
            .map_err(|error| error.to_string())?;
        if picker.wait_for_scan(SCAN_PATIENCE) {
            Ok(Self { picker })
        } else {
            Err("the project is too large to index in time".to_owned())
        }
    }
}

impl Workspace {
    pub fn find_files(&self, query: &str) -> Result<String, ToolError> {
        let guard = self.index()?.picker.read().map_err(unavailable)?;
        let picker = guard.as_ref().ok_or_else(|| unavailable(NOT_READY))?;
        let parser = QueryParser::default();
        let parsed = parser.parse(query);
        let mut found = Vec::new();
        let mut offset = 0;
        loop {
            let options = FuzzySearchOptions {
                pagination: PaginationArgs {
                    offset,
                    limit: PAGE,
                },
                ..Default::default()
            };
            let page = picker.fuzzy_search(&parsed, None, options);
            let candidates = page.items.iter().map(|item| item.relative_path(picker));
            found.extend(candidates.filter_map(|path| self.readable(&path)));
            offset += page.items.len();
            if page.items.is_empty() || offset >= page.total_matched || found.len() > MAX_FOUND {
                break;
            }
        }
        let names = found.iter().map(|path| path.as_str().to_owned()).collect();
        Ok(capped(names, MAX_FOUND, "files"))
    }

    pub fn grep(&self, query: &str, regex: bool) -> Result<String, ToolError> {
        let guard = self.index()?.picker.read().map_err(unavailable)?;
        let picker = guard.as_ref().ok_or_else(|| unavailable(NOT_READY))?;
        let parsed = parse_grep_query(query);
        let mut lines = Vec::new();
        let mut file_offset = 0;
        loop {
            let options = GrepSearchOptions {
                mode: if regex {
                    GrepMode::Regex
                } else {
                    GrepMode::PlainText
                },
                file_offset,
                page_limit: PAGE,
                trim_whitespace: true,
                ..Default::default()
            };
            let page = picker.grep(&parsed, &options);
            if regex && page.regex_fallback_error.is_some() {
                return Err(ToolError::BadRegex(query.to_owned()));
            }
            lines.extend(self.readable_matches(&page, picker));
            file_offset = page.next_file_offset;
            if file_offset == 0 || lines.len() > MAX_MATCHES {
                break;
            }
        }
        Ok(capped(lines, MAX_MATCHES, "matches"))
    }

    fn readable_matches(&self, page: &GrepResult<'_>, picker: &FilePicker) -> Vec<String> {
        let files: Vec<Option<String>> = page
            .files
            .iter()
            .map(|file| self.readable(&file.relative_path(picker)))
            .map(|path| path.map(|path| path.as_str().to_owned()))
            .collect();
        page.matches
            .iter()
            .filter_map(|found| {
                let path = files.get(found.file_index)?.as_ref()?;
                let line = shortened(&found.line_content);
                Some(format!("{path}:{}: {line}", found.line_number))
            })
            .collect()
    }
}

fn unavailable(reason: impl ToString) -> ToolError {
    ToolError::SearchUnavailable(reason.to_string())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::workspace::test_support::{SECRET, project, reading, sample, write};

    fn leaky() -> tempfile::TempDir {
        let dir = sample();
        write(dir.path(), "docs/notes.md", SECRET.as_bytes());
        write(dir.path(), "server.pem", SECRET.as_bytes());
        write(dir.path(), "deploy/prod.key", SECRET.as_bytes());
        dir
    }

    #[test]
    fn files_are_found_by_a_fuzzy_name() {
        let dir = sample();
        let found = reading(&dir, &["**"]).find_files("app").expect("searched");
        assert_eq!(found.lines().next(), Some("src/tui/app.rs"));
    }

    #[test]
    fn found_files_stay_inside_the_read_scope() {
        let dir = sample();
        let found = reading(&dir, &["docs/**"])
            .find_files("rs")
            .expect("searched");
        assert!(!found.contains("src/"), "{found}");
        let found = reading(&dir, &["docs/**"])
            .find_files("design")
            .expect("searched");
        assert_eq!(found, "docs/DESIGN.md");
    }

    #[test]
    fn excluded_files_are_neither_found_nor_searched() {
        let dir = sample();
        let workspace = reading(&dir, &["**", "!docs/**"]);
        let found = workspace.find_files("design").expect("searched");
        assert!(!found.contains("docs/"), "{found}");
        let matched = workspace.grep("design", false).expect("searched");
        assert!(!matched.contains("docs/"), "{matched}");
    }

    #[test]
    fn protected_files_are_never_found_even_with_full_scope() {
        let dir = leaky();
        let workspace = reading(&dir, &["**"]);
        for query in ["pem", "key", "env", "server", "prod", "config"] {
            let found = workspace.find_files(query).expect("searched");
            for protected in ["server.pem", "prod.key", ".env", ".git"] {
                assert!(!found.contains(protected), "{query}: {found}");
            }
        }
    }

    #[test]
    fn grep_finds_lines_with_file_and_line_number() {
        let dir = sample();
        let found = reading(&dir, &["**"]).grep("run", false).expect("searched");
        assert_eq!(found, "src/main.rs:2: run();");
    }

    #[test]
    fn grep_never_shows_protected_content_even_with_full_scope() {
        let dir = leaky();
        let found = reading(&dir, &["**"])
            .grep("SECRET", false)
            .expect("searched");
        assert_eq!(found, format!("docs/notes.md:1: {SECRET}"));
    }

    #[test]
    fn grep_never_shows_content_outside_the_read_scope() {
        let dir = leaky();
        let found = reading(&dir, &["src/**"])
            .grep("SECRET", false)
            .expect("searched");
        assert_eq!(found, "no matches");
    }

    #[test]
    fn grep_does_not_read_a_protected_file_through_a_link() {
        let dir = sample();
        symlink("../.env", dir.path().join("src/settings.rs")).expect("link");
        let found = reading(&dir, &["**"])
            .grep("SECRET", false)
            .expect("searched");
        assert_eq!(found, "no matches");
    }

    #[test]
    fn grep_can_use_a_regular_expression() {
        let dir = sample();
        let workspace = reading(&dir, &["**"]);
        let found = workspace
            .grep("pub\\s+struct\\s+\\w+", true)
            .expect("searched");
        assert_eq!(found, "src/tui/app.rs:1: pub struct App;");
    }

    #[test]
    fn broken_regular_expression_is_an_error() {
        let dir = sample();
        let found = reading(&dir, &["**"]).grep("(unclosed", true);
        assert!(matches!(found, Err(ToolError::BadRegex(_))), "{found:?}");
    }

    #[test]
    fn grep_can_be_narrowed_with_a_file_constraint() {
        let dir = project(&[
            ("src/a.rs", "let shared = 1;\n"),
            ("docs/a.md", "shared notes\n"),
        ]);
        let workspace = reading(&dir, &["**"]);
        let found = workspace.grep("*.md shared", false).expect("searched");
        assert_eq!(found, "docs/a.md:1: shared notes");
    }
}
