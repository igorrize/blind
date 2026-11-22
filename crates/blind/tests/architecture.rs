use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const TEST_MARKER: &str = "#[cfg(test)]";
const FILES_WHOSE_MODULES_LIVE_BESIDE_THEM: [&str; 3] = ["lib", "main", "mod"];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).expect("readable dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files
}

fn production_files(dir: &Path, workspace: &Path) -> Vec<PathBuf> {
    let test_only = test_only_modules(&rust_files(workspace));
    rust_files(dir)
        .into_iter()
        .filter(|file| !test_only.iter().any(|module| is_part_of(file, module)))
        .collect()
}

fn is_part_of(file: &Path, module: &Path) -> bool {
    file.with_extension("") == module || file.starts_with(module)
}

fn test_only_modules(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut modules = Vec::new();
    for file in files {
        let source = fs::read_to_string(file).expect("readable source");
        let folder = folder_of_child_modules(file);
        let declared = modules_declared_for_tests_only(&source);
        modules.extend(declared.into_iter().map(|name| folder.join(name)));
    }
    modules
}

fn folder_of_child_modules(file: &Path) -> PathBuf {
    let folder = file.parent().expect("file lives in a folder");
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    if FILES_WHOSE_MODULES_LIVE_BESIDE_THEM.contains(&stem) {
        folder.to_owned()
    } else {
        folder.join(stem)
    }
}

fn modules_declared_for_tests_only(source: &str) -> Vec<&str> {
    source
        .match_indices(TEST_MARKER)
        .filter_map(|(start, marker)| {
            module_kept_in_its_own_file(declaration_after(source, start + marker.len()))
        })
        .collect()
}

fn declaration_after(source: &str, position: usize) -> &str {
    let rest = source[position..].trim_start();
    rest.lines().next().unwrap_or_default().trim_end()
}

fn module_kept_in_its_own_file(declaration: &str) -> Option<&str> {
    let without_visibility = declaration
        .strip_prefix("pub(crate) ")
        .or_else(|| declaration.strip_prefix("pub "))
        .unwrap_or(declaration);
    let name = without_visibility.strip_prefix("mod ")?.strip_suffix(';')?;
    let is_a_name = !name.is_empty()
        && name
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || letter == '_');
    is_a_name.then_some(name)
}

fn production_code(source: &str) -> &str {
    let test_block = source
        .match_indices(TEST_MARKER)
        .find(|(start, marker)| declaration_after(source, start + marker.len()).ends_with('{'));
    match test_block {
        Some((start, _)) => &source[..start],
        None => source,
    }
}

fn offenders(files: &[PathBuf], forbidden: &[&str]) -> Vec<String> {
    files
        .iter()
        .flat_map(|file| {
            let source = fs::read_to_string(file).expect("readable source");
            let code = production_code(&source).to_owned();
            forbidden
                .iter()
                .filter(move |word| code.contains(**word))
                .map(move |word| format!("{} uses {word}", file.display()))
        })
        .collect()
}

fn dependencies(krate: &str) -> BTreeSet<String> {
    let manifest =
        fs::read_to_string(crates_dir().join(krate).join("Cargo.toml")).expect("manifest");
    let manifest: toml::Table = manifest.parse().expect("valid manifest");
    let dependencies = manifest["dependencies"].as_table().expect("dependencies");
    dependencies.keys().cloned().collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn assert_clean(found: &[String]) {
    assert!(found.is_empty(), "{found:#?}");
}

fn source_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (path, source) in files {
        let file = dir.path().join(path);
        fs::create_dir_all(file.parent().expect("parent")).expect("folders");
        fs::write(file, source).expect("write");
    }
    dir
}

fn scanned(tree: &tempfile::TempDir) -> Vec<String> {
    let mut files: Vec<String> = production_files(tree.path(), tree.path())
        .iter()
        .map(|file| {
            let inside = file.strip_prefix(tree.path()).expect("inside the tree");
            inside.to_string_lossy().into_owned()
        })
        .collect();
    files.sort();
    files
}

#[test]
fn scanner_sees_production_code_and_skips_test_blocks() {
    let source = "#[cfg(test)]\nmod test_support;\nuse std::fs;\n#[cfg(test)]\nmod tests {\n    use std::process;\n}\n";
    let code = production_code(source);
    assert!(code.contains("std::fs"));
    assert!(!code.contains("std::process"));
}

#[test]
fn scanner_reports_a_forbidden_word() {
    let tree = source_tree(&[("leak.rs", "use std::fs;\n")]);
    let files = production_files(tree.path(), tree.path());
    let found = offenders(&files, &["std::fs", "std::net"]);
    let leak = tree.path().join("leak.rs");
    assert_eq!(found, [format!("{} uses std::fs", leak.display())]);
}

#[test]
fn file_is_skipped_only_when_it_is_declared_for_tests() {
    let tree = source_tree(&[
        ("lib.rs", "mod step;\nmod guarantees;\n"),
        ("guarantees.rs", "use std::fs;\n"),
        (
            "step.rs",
            "#[cfg(test)]\nmod guarantees;\nmod properties;\n",
        ),
        ("step/guarantees.rs", "use std::fs;\n"),
        ("step/properties.rs", "use std::fs;\n"),
    ]);
    assert_eq!(
        scanned(&tree),
        ["guarantees.rs", "lib.rs", "step.rs", "step/properties.rs"]
    );
}

#[test]
fn name_that_looks_like_a_test_file_does_not_hide_production_code() {
    let tree = source_tree(&[
        ("lib.rs", "mod test_support;\n"),
        ("test_support.rs", "use std::fs;\n"),
    ]);
    let files = production_files(tree.path(), tree.path());
    assert_eq!(offenders(&files, &["std::fs"]).len(), 1);
}

#[test]
fn everything_under_a_test_only_module_is_skipped() {
    let tree = source_tree(&[
        ("main.rs", "#[cfg(test)]\npub(crate) mod fixtures;\n"),
        ("fixtures.rs", "mod deep;\n"),
        ("fixtures/deep.rs", "use std::fs;\n"),
        ("fixtures_too.rs", "use std::fs;\n"),
    ]);
    assert_eq!(scanned(&tree), ["fixtures_too.rs", "main.rs"]);
}

#[test]
fn test_only_declaration_may_live_outside_the_scanned_folder() {
    let tree = source_tree(&[
        ("tui.rs", "#[cfg(test)]\nmod test_support;\n"),
        ("tui/test_support.rs", "use std::fs;\n"),
        ("tui/app.rs", "fn draw() {}\n"),
    ]);
    let inside_tui = production_files(&tree.path().join("tui"), tree.path());
    assert_eq!(inside_tui, [tree.path().join("tui/app.rs")]);
}

#[test]
fn inline_test_block_is_not_taken_for_a_file() {
    let source = "#[cfg(test)]\nmod tests {\n}\n#[cfg(test)]\nfn helper() {}\n";
    assert!(modules_declared_for_tests_only(source).is_empty());
}

#[test]
fn test_only_files_of_this_workspace_are_recognised() {
    let step_tests = crates_dir().join("blind/src/step");
    assert!(rust_files(&step_tests).len() >= 3);
    assert!(production_files(&step_tests, &crates_dir()).is_empty());
}

#[test]
fn core_does_no_io() {
    let files = production_files(&crates_dir().join("blind-core/src"), &crates_dir());
    assert!(files.len() > 5);
    let forbidden = [
        "std::fs",
        "std::net",
        "std::process",
        "std::env::var",
        "tokio",
        "reqwest",
    ];
    assert_clean(&offenders(&files, &forbidden));
}

#[test]
fn core_depends_only_on_serde_and_thiserror() {
    assert_eq!(
        dependencies("blind-core"),
        names(&["serde", "serde_json", "thiserror"])
    );
}

#[test]
fn executor_has_no_network_client() {
    let network_crates = names(&["reqwest", "hyper", "ureq", "tokio", "rustls", "native-tls"]);
    let used: Vec<String> = dependencies("blind-exec")
        .intersection(&network_crates)
        .cloned()
        .collect();
    assert!(used.is_empty(), "{used:?}");
}

#[test]
fn screen_cannot_reach_the_network_the_filesystem_or_the_worker_loop() {
    let blind = crates_dir().join("blind/src");
    let mut files = production_files(&blind.join("tui"), &crates_dir());
    files.push(blind.join("tui.rs"));
    assert!(files.len() > 8);
    let forbidden = [
        "crate::providers",
        "crate::step",
        "crate::chain_run",
        "crate::agents",
        "reqwest",
        "tokio",
        "std::fs",
        "std::net",
        "std::process",
    ];
    assert_clean(&offenders(&files, &forbidden));
}

#[test]
fn root_path_cannot_use_tools_files_or_the_worker_loop() {
    let blind = crates_dir().join("blind/src");
    let files = [blind.join("backend.rs"), blind.join("conversation.rs")];
    let forbidden = [
        "crate::step",
        "crate::chain_run",
        "crate::agents",
        "ToolSpec",
        ".turn(",
        "std::fs",
        "std::process",
    ];
    assert_clean(&offenders(&files, &forbidden));
}
