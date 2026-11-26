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
        "ok     explorer  ollama-cloud/glm-5.3-flash\n\
         ok     fixer  ollama-cloud/glm-5.3-flash\n\
         ok     planner  ollama-cloud/glm-5.3-flash\n\
         ok     reviewer  ollama-cloud/glm-5.3-flash\n\
         ok     root  model chosen in settings\n"
    );
}

#[test]
fn baseline_eval_agents_pass_the_check_with_their_shared_rules() {
    let baseline = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/setups/baseline/agents");
    let (passed, report) = check(&baseline);
    assert!(passed, "{report}");
    assert_eq!(report.lines().count(), 4, "{report}");
}

fn chain_check(dir: &Path, agents: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_blind"))
        .args(["chain", "check"])
        .args(agents)
        .arg("--dir")
        .arg(dir)
        .output()
        .expect("blind runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn daily_agents_pass_the_check_and_form_their_chains() {
    let daily = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/daily/agents");
    let (passed, report) = check(&daily);
    assert!(passed, "{report}");
    for chain in [
        vec!["scout"],
        vec!["writer", "critic"],
        vec!["scout", "red", "writer", "critic"],
    ] {
        let (compatible, shown) = chain_check(&daily, &chain);
        assert!(compatible, "{chain:?}: {shown}");
    }
    let (compatible, _) = chain_check(&daily, &["red", "critic"]);
    assert!(!compatible, "critic needs the files the writer changed");
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
