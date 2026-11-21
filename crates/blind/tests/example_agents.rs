use std::{path::Path, process::Command};

fn check(dir: &Path) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_blind"))
        .args(["agent", "check", "--dir"])
        .arg(dir)
        .output()
        .expect("blind runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn example_agents_pass_the_check() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/agents");
    let (passed, report) = check(&examples);
    assert!(passed, "{report}");
    assert_eq!(
        report,
        "ok     explorer  ollama-cloud/gemma4:31b\n\
         ok     fixer  ollama-cloud/gemma4:31b\n\
         ok     reviewer  ollama-cloud/gemma4:31b\n"
    );
}

#[test]
fn broken_agent_fails_the_check() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("ghost")).expect("create agent folder");
    let (passed, report) = check(dir.path());
    assert!(!passed);
    assert!(
        report.starts_with("error  ghost  cannot read contract.json: "),
        "{report}"
    );
}
