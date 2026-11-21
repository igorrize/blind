use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

const PROTECTED: [&str; 9] = [
    "**/.git",
    "**/.git/**",
    "**/.blind",
    "**/.blind/**",
    "**/.env*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/id_ed25519*",
];
const SEPARATOR: char = '/';
const HERE: &str = ".";
const PARENT: &str = "..";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectPath(String);

pub struct PathPolicy {
    allowed: GlobSet,
    protected: GlobSet,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("`{0}` is not a path inside the project")]
    NotInsideProject(String),
    #[error("`{0}` is protected and cannot be accessed")]
    Protected(String),
    #[error("`{0}` is outside the paths this agent may read")]
    OutOfScope(String),
    #[error("`{0}` is not among the files given to this agent")]
    NotGiven(String),
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("`{0}` is not a pattern inside the project")]
    OutsideProject(String),
    #[error("`{pattern}` is not a valid pattern: {source}")]
    Invalid {
        pattern: String,
        source: globset::Error,
    },
}

impl ProjectPath {
    pub fn parse(raw: &str) -> Result<Self, Refusal> {
        match parts_inside_project(raw) {
            Some(parts) => Ok(Self(parts.join("/"))),
            None => Err(Refusal::NotInsideProject(raw.to_owned())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PathPolicy {
    pub fn reading(patterns: &[String]) -> Result<Self, PolicyError> {
        let allowed = patterns.iter().map(|raw| pattern(raw));
        let protected = PROTECTED.iter().map(|raw| protected_pattern(raw));
        Ok(Self {
            allowed: glob_set(allowed)?,
            protected: glob_set(protected)?,
        })
    }

    pub fn permits(&self, path: &ProjectPath) -> Result<(), Refusal> {
        let shown = || path.as_str().to_owned();
        if self.protected.is_match(path.as_str()) {
            return Err(Refusal::Protected(shown()));
        }
        if !self.allowed.is_match(path.as_str()) {
            return Err(Refusal::OutOfScope(shown()));
        }
        Ok(())
    }
}

pub fn pattern(raw: &str) -> Result<Glob, PolicyError> {
    if parts_inside_project(raw).is_none() {
        return Err(PolicyError::OutsideProject(raw.to_owned()));
    }
    built(GlobBuilder::new(raw).literal_separator(true), raw)
}

fn protected_pattern(raw: &str) -> Result<Glob, PolicyError> {
    let any_case = GlobBuilder::new(raw)
        .literal_separator(true)
        .case_insensitive(true)
        .clone();
    built(&any_case, raw)
}

fn built(builder: &GlobBuilder<'_>, raw: &str) -> Result<Glob, PolicyError> {
    builder.build().map_err(|source| PolicyError::Invalid {
        pattern: raw.to_owned(),
        source,
    })
}

fn glob_set(
    globs: impl Iterator<Item = Result<Glob, PolicyError>>,
) -> Result<GlobSet, PolicyError> {
    let mut set = GlobSetBuilder::new();
    for glob in globs {
        set.add(glob?);
    }
    set.build().map_err(|source| PolicyError::Invalid {
        pattern: "the whole set".to_owned(),
        source,
    })
}

fn parts_inside_project(raw: &str) -> Option<Vec<&str>> {
    if raw.starts_with(SEPARATOR) || raw.contains(['\\', '\0']) {
        return None;
    }
    let parts: Vec<&str> = raw
        .split(SEPARATOR)
        .filter(|part| !part.is_empty() && *part != HERE)
        .collect();
    let inside = !parts.is_empty() && !parts.contains(&PARENT);
    inside.then_some(parts)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn everything() -> PathPolicy {
        PathPolicy::reading(&["**".to_owned()]).expect("valid policy")
    }

    fn only(patterns: &[&str]) -> PathPolicy {
        let patterns: Vec<String> = patterns.iter().map(|raw| (*raw).to_owned()).collect();
        PathPolicy::reading(&patterns).expect("valid policy")
    }

    fn verdict(policy: &PathPolicy, raw: &str) -> Result<(), Refusal> {
        policy.permits(&ProjectPath::parse(raw)?)
    }

    #[test]
    fn path_is_normalized() {
        let path = ProjectPath::parse("./src//tui/./app.rs").expect("inside");
        assert_eq!(path.as_str(), "src/tui/app.rs");
    }

    #[test]
    fn paths_that_leave_the_project_are_refused() {
        for raw in [
            "",
            ".",
            "/etc/passwd",
            "../secret",
            "src/../../secret",
            "src\\..\\x",
            "a\0b",
        ] {
            assert_eq!(
                ProjectPath::parse(raw),
                Err(Refusal::NotInsideProject(raw.to_owned())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn protected_paths_are_refused_even_when_everything_is_allowed() {
        for raw in [
            ".git/config",
            ".GIT/config",
            "vendor/lib/.git/HEAD",
            ".blind/agents/coder/contract.json",
            ".env",
            "deploy/.env.production",
            "certs/server.pem",
            "certs/SERVER.KEY",
            "home/id_rsa",
            "home/id_ed25519.pub",
        ] {
            assert_eq!(
                verdict(&everything(), raw),
                Err(Refusal::Protected(raw.to_owned())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn only_paths_in_scope_are_permitted() {
        let policy = only(&["src/**", "README.md"]);
        assert_eq!(verdict(&policy, "src/tui/app.rs"), Ok(()));
        assert_eq!(verdict(&policy, "README.md"), Ok(()));
        assert_eq!(
            verdict(&policy, "docs/DESIGN.md"),
            Err(Refusal::OutOfScope("docs/DESIGN.md".to_owned()))
        );
    }

    #[test]
    fn single_star_does_not_cross_folders() {
        let policy = only(&["src/*.rs"]);
        assert_eq!(verdict(&policy, "src/main.rs"), Ok(()));
        assert!(verdict(&policy, "src/tui/app.rs").is_err());
    }

    #[test]
    fn agent_without_patterns_can_read_nothing() {
        assert!(verdict(&only(&[]), "README.md").is_err());
    }

    #[test]
    fn scope_patterns_that_leave_the_project_are_rejected() {
        for raw in ["/etc/**", "../**", "src/../../**"] {
            assert!(PathPolicy::reading(&[raw.to_owned()]).is_err(), "{raw:?}");
        }
    }

    proptest! {
        #[test]
        fn parsed_path_never_leaves_the_project(raw in ".{0,40}") {
            if let Ok(path) = ProjectPath::parse(&raw) {
                prop_assert!(!path.as_str().starts_with('/'));
                prop_assert!(path.as_str().split('/').all(|part| part != ".." && !part.is_empty()));
            }
        }

        #[test]
        fn nothing_under_a_protected_folder_is_ever_permitted(
            prefix in "([a-z]{1,5}/){0,3}",
            folder in prop_oneof![Just(".git"), Just(".blind"), Just(".Git"), Just(".BLIND")],
            rest in "[a-z]{1,8}(/[a-z]{1,8}){0,2}",
        ) {
            let raw = format!("{prefix}{folder}/{rest}");
            prop_assert!(verdict(&everything(), &raw).is_err());
        }

        #[test]
        fn path_outside_every_pattern_is_never_permitted(name in "[a-z]{1,8}\\.[a-z]{1,3}") {
            let policy = only(&["src/**"]);
            let outside = format!("docs/{}", name);
            prop_assert!(verdict(&policy, &outside).is_err());
        }
    }
}
