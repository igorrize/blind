use std::{path::Path, process::ExitCode};

use anyhow::Context;
use blind_core::{Agent, ChainError, Contract, Usage, Violation, check_chain};

use crate::agents;
use crate::chain_run::{self, ChainEvent, ChainOutcome, Link, StepRecord, StopReason};
use crate::providers::{self, config};
use crate::run_agent::show_progress;
use crate::runtime;
use crate::step;

const ARROW: &str = " → ";

pub fn check(dir: &Path, names: &[String]) -> anyhow::Result<ExitCode> {
    let agents = load(dir, names)?;
    Ok(match compatibility(&agents) {
        Ok(()) => {
            println!("ok: {}", names.join(ARROW));
            ExitCode::SUCCESS
        }
        Err(error) => refuse(&error),
    })
}

pub fn run(dir: &Path, project: &Path, names: &[String], task: &str) -> anyhow::Result<ExitCode> {
    let agents = load(dir, names)?;
    if let Err(error) = compatibility(&agents) {
        return Ok(refuse(&error));
    }
    let links = connect(agents, project)?;
    let mut show = show_event;
    let chain = chain_run::run_chain(&links, task, &mut show);
    let outcome = runtime::block_on(chain)??;
    Ok(report(outcome))
}

fn load(dir: &Path, names: &[String]) -> anyhow::Result<Vec<Agent>> {
    names
        .iter()
        .map(|name| {
            agents::load_one(dir, name)
                .with_context(|| format!("cannot load agent `{name}` from {}", dir.display()))
        })
        .collect()
}

fn compatibility(agents: &[Agent]) -> Result<(), ChainError> {
    let contracts: Vec<&Contract> = agents.iter().map(Agent::contract).collect();
    check_chain(&contracts)
}

fn refuse(error: &ChainError) -> ExitCode {
    eprintln!("chain is not compatible, no model was called: {error}");
    ExitCode::FAILURE
}

fn connect(agents: Vec<Agent>, project: &Path) -> anyhow::Result<Vec<Link>> {
    let config = config::load()?;
    agents
        .into_iter()
        .map(|agent| {
            let client = providers::connect(&config, agent.settings().model().provider())?;
            let workspace = step::workspace_for(&agent, project)?;
            Ok(Link {
                agent,
                client,
                workspace,
            })
        })
        .collect()
}

fn show_event(event: ChainEvent<'_>) {
    match event {
        ChainEvent::StepStarted(name) => eprintln!("→ {name}"),
        ChainEvent::Step(progress) => show_progress(progress),
        ChainEvent::StepResult(output) => eprintln!("{output:#}"),
        ChainEvent::StepFinished(record) => eprintln!("\n{}", step_line(record)),
    }
}

fn report(outcome: ChainOutcome) -> ExitCode {
    match outcome {
        ChainOutcome::Completed { output, steps } => {
            println!("{output:#}");
            eprintln!("{}", total_line(&steps));
            ExitCode::SUCCESS
        }
        ChainOutcome::Incompatible(error) => refuse(&error),
        ChainOutcome::Stopped { at, reason, steps } => {
            eprintln!("stopped at `{at}`: {}", explained(&reason));
            eprintln!("{}", total_line(&steps));
            ExitCode::FAILURE
        }
    }
}

fn explained(reason: &StopReason) -> String {
    match reason {
        StopReason::StepFailed(stop) => stop.explained().to_owned(),
        StopReason::InputRejected(violations) => {
            let problems: Vec<String> = violations.iter().map(Violation::to_string).collect();
            format!("its input was rejected: {}", problems.join("; "))
        }
    }
}

fn step_line(record: &StepRecord) -> String {
    format!("{} · {}", record.agent, record.report)
}

fn total_line(steps: &[StepRecord]) -> String {
    let usage = steps.iter().fold(Usage::default(), |total, step| {
        total.plus(step.report.usage)
    });
    format!(
        "chain · steps: {} · tokens: {} in, {} out",
        steps.len(),
        usage.input_tokens,
        usage.output_tokens
    )
}

#[cfg(test)]
mod tests {
    use crate::step::{StepReport, Stop};

    use super::*;

    fn record(agent: &str, input: u64, output: u64) -> StepRecord {
        StepRecord {
            agent: agent.to_owned(),
            report: StepReport {
                usage: Usage {
                    input_tokens: input,
                    output_tokens: output,
                },
                turns: 2,
                rejections: 1,
                tool_calls: 3,
                forced_to_finish: false,
            },
        }
    }

    #[test]
    fn step_line_names_the_agent_and_its_counts() {
        assert_eq!(
            step_line(&record("explorer", 120, 45)),
            "explorer · turns: 2, tool calls: 3, rejected: 1 · tokens: 120 in, 45 out"
        );
    }

    #[test]
    fn total_line_adds_up_the_steps() {
        let steps = [record("explorer", 120, 45), record("reviewer", 30, 5)];
        assert_eq!(
            total_line(&steps),
            "chain · steps: 2 · tokens: 150 in, 50 out"
        );
    }

    #[test]
    fn stop_reasons_are_explained() {
        assert_eq!(
            explained(&StopReason::StepFailed(Stop::OutOfTokens)),
            "it used more tokens than one step may"
        );
    }
}
