use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const TEST_ONLY_FILES: [&str; 3] = ["test_support.rs", "guarantees.rs", "properties.rs"];
const TEST_BLOCK_MARKER: &str = "#[cfg(test)]";

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).expect("readable dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if is_production_source(&path) {
            files.push(path);
        }
    }
    files
}

fn is_production_source(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.ends_with(".rs") && !TEST_ONLY_FILES.contains(&name)
}

fn production_code(source: &str) -> &str {
    let test_block = source
        .match_indices(TEST_BLOCK_MARKER)
        .find(|(start, marker)| {
            let after = &source[start + marker.len()..];
            let declaration = after.trim_start().lines().next().unwrap_or_default();
            declaration.trim_end().ends_with('{')
        });
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

#[test]
fn scanner_sees_production_code_and_skips_test_blocks() {
    let source = "#[cfg(test)]\nmod test_support;\nuse std::fs;\n#[cfg(test)]\nmod tests {\n    use std::process;\n}\n";
    let code = production_code(source);
    assert!(code.contains("std::fs"));
    assert!(!code.contains("std::process"));
}

#[test]
fn scanner_reports_a_forbidden_word() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("leak.rs");
    fs::write(&file, "use std::fs;\n").expect("write");
    let found = offenders(&rust_files(dir.path()), &["std::fs", "std::net"]);
    assert_eq!(found, [format!("{} uses std::fs", file.display())]);
}

#[test]
fn core_does_no_io() {
    let files = rust_files(&crates_dir().join("blind-core/src"));
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
    let mut files = rust_files(&blind.join("tui"));
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
