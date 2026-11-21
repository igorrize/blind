use std::{path::Path, process::ExitCode};

use anyhow::Context;
use blind_core::{Agent, Violation};
use serde_json::{Value, json};

use crate::agents;
use crate::providers::{self, config};
use crate::runtime;
use crate::step::{self, Progress, StepOutcome, StepReport};

pub fn run(
    dir: &Path,
    project: &Path,
    name: &str,
    task: &str,
    input: Option<&str>,
) -> anyhow::Result<ExitCode> {
    let agent = agents::load_one(dir, name)
        .with_context(|| format!("cannot load agent `{name}` from {}", dir.display()))?;
    let input = parsed_input(input)?;
    let violations = agent.contract().input().violations(&input);
    if !violations.is_empty() {
        eprintln!("{}", input_rejection(&violations));
        return Ok(ExitCode::FAILURE);
    }
    let workspace = step::workspace_for(&agent, project)?;
    let config = config::load()?;
    let client = providers::connect(&config, agent.settings().model().provider())?;
    let mut show = show_progress;
    let step = step::run_step(workspace.as_ref(), &client, &agent, task, &input, &mut show);
    let outcome = runtime::block_on(step)??;
    Ok(report(&agent, outcome))
}

fn parsed_input(input: Option<&str>) -> anyhow::Result<Value> {
    match input {
        Some(text) => serde_json::from_str(text).context("--input is not valid JSON"),
        None => Ok(json!({})),
    }
}

pub fn show_progress(progress: Progress<'_>) {
    match progress {
        Progress::Text(text) => eprint!("{text}"),
        Progress::ToolCalled {
            name,
            arguments,
            refusal: None,
        } => eprintln!("· {name} {arguments}"),
        Progress::ToolCalled {
            name,
            arguments,
            refusal: Some(refusal),
        } => eprintln!("✗ {name} {arguments} — {refusal}"),
        Progress::Rejected(reason) => eprintln!("\n{reason}"),
        Progress::Reminded => eprintln!("\nno `complete` call, reminding the agent"),
        Progress::ToldToFinish(reason) => {
            eprintln!("\n{reason}: tools are taken away, the agent must finish now");
        }
    }
}

fn report(agent: &Agent, outcome: StepOutcome) -> ExitCode {
    match outcome {
        StepOutcome::Completed { output, report } => {
            println!("{output:#}");
            eprintln!("{}", summary(agent, &report));
            ExitCode::SUCCESS
        }
        StepOutcome::InputRejected(violations) => {
            eprintln!("{}", input_rejection(&violations));
            ExitCode::FAILURE
        }
        StepOutcome::Failed { report, stop } => {
            eprintln!("failed: {}", stop.explained());
            eprintln!("{}", summary(agent, &report));
            ExitCode::FAILURE
        }
    }
}

fn input_rejection(violations: &[Violation]) -> String {
    let problems: Vec<String> = violations.iter().map(Violation::to_string).collect();
    format!(
        "input rejected, the model was not called: {}",
        problems.join("; ")
    )
}

fn summary(agent: &Agent, report: &StepReport) -> String {
    format!(
        "{} · {} · {report}",
        agent.contract().name(),
        agent.settings().model()
    )
}

#[cfg(test)]
mod tests {
    use blind_core::Usage;

    use super::*;

    fn planner() -> Agent {
        let contract = r#"{"name":"planner","kind":"worker",
            "input":{"type":"object","properties":{"focus":{"type":"string"}}},
            "output":{"type":"object","properties":{}}}"#;
        let settings = r#"{"description":"Plans.","model":"ollama-cloud/glm-5.3-flash"}"#;
        Agent::from_parts(contract, settings, "Plan.".to_owned()).expect("valid agent")
    }

    #[test]
    fn missing_input_is_an_empty_object() {
        assert_eq!(parsed_input(None).expect("valid"), json!({}));
        assert_eq!(
            parsed_input(Some(r#"{"focus":"tests"}"#)).expect("valid"),
            json!({"focus": "tests"})
        );
        assert!(parsed_input(Some("{oops")).is_err());
    }

    #[test]
    fn summary_names_the_agent_the_model_and_the_counts() {
        let report = StepReport {
            usage: Usage {
                input_tokens: 120,
                output_tokens: 45,
            },
            turns: 2,
            rejections: 1,
            tool_calls: 3,
            forced_to_finish: false,
        };
        assert_eq!(
            summary(&planner(), &report),
            "planner · ollama-cloud/glm-5.3-flash · turns: 2, tool calls: 3, rejected: 1 · tokens: 120 in, 45 out"
        );
    }

    #[test]
    fn input_rejection_says_the_model_was_not_called() {
        let violations = planner()
            .contract()
            .input()
            .violations(&json!({"focus": 7}));
        assert_eq!(
            input_rejection(&violations),
            "input rejected, the model was not called: `focus` must be a string"
        );
    }
}
