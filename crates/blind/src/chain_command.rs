use std::{path::Path, process::ExitCode};

use anyhow::Context;
use blind_core::{Agent, ChainError, Contract, Usage, Violation, check_chain};

use crate::agents;
use crate::chain_run::{self, ChainEvent, ChainOutcome, Link, StepRecord, StopReason};
use crate::lineup::Lineup;
use crate::providers::{self, config, config::ProvidersConfig};
use crate::run_agent::show_progress;
use crate::runtime;
use crate::step::{self, Ground, Progress};
use crate::workplace::Workplace;

pub fn check(dir: &Path, arguments: &[String]) -> anyhow::Result<ExitCode> {
    let lineup = Lineup::parse(arguments);
    let agents = load(dir, &lineup)?;
    Ok(match compatibility(&agents) {
        Ok(()) => {
            println!("ok: {lineup}");
            ExitCode::SUCCESS
        }
        Err(error) => refuse(&error),
    })
}

pub fn run(
    dir: &Path,
    project: &Path,
    arguments: &[String],
    task: &str,
) -> anyhow::Result<ExitCode> {
    let lineup = Lineup::parse(arguments);
    let agents = load(dir, &lineup)?;
    if let Err(error) = compatibility(&agents) {
        return Ok(refuse(&error));
    }
    let prepared = connected(agents, project)?;
    let mut printer = Printer::default();
    let mut show = |event: ChainEvent<'_>| printer.show(event);
    let chain = chain_run::run_chain(&prepared.steps, task, &mut show);
    let outcome = runtime::block_on(chain)?;
    if let Some(changes) = prepared.changes(&lineup, task) {
        eprintln!("\n{changes}");
    }
    Ok(report(outcome?))
}

pub struct Prepared {
    pub steps: Vec<Vec<Link>>,
    workplace: Workplace,
}

impl Prepared {
    pub fn changes(&self, lineup: &Lineup, task: &str) -> Option<String> {
        match self.workplace.changes(&lineup.to_string(), task) {
            Ok(changes) => changes.map(|changes| changes.described()),
            Err(error) => Some(format!("the copy could not be finished: {error:#}")),
        }
    }
}

pub fn prepare(dir: &Path, project: &Path, lineup: &Lineup) -> anyhow::Result<Prepared> {
    let agents = load(dir, lineup)?;
    if let Err(error) = compatibility(&agents) {
        anyhow::bail!("chain is not compatible, no model was called: {error}");
    }
    connected(agents, project)
}

fn connected(agents: Vec<Vec<Agent>>, project: &Path) -> anyhow::Result<Prepared> {
    let workplace = Workplace::for_agents(agents.iter().flatten(), project)?;
    let steps = connect(agents, workplace.ground())?;
    Ok(Prepared { steps, workplace })
}

fn load(dir: &Path, lineup: &Lineup) -> anyhow::Result<Vec<Vec<Agent>>> {
    lineup
        .steps()
        .iter()
        .map(|step| step.iter().map(|name| load_agent(dir, name)).collect())
        .collect()
}

fn load_agent(dir: &Path, name: &str) -> anyhow::Result<Agent> {
    agents::load_one(dir, name)
        .with_context(|| format!("cannot load agent `{name}` from {}", dir.display()))
}

fn compatibility(agents: &[Vec<Agent>]) -> Result<(), ChainError> {
    let contracts: Vec<Vec<&Contract>> = agents
        .iter()
        .map(|step| step.iter().map(Agent::contract).collect())
        .collect();
    check_chain(&contracts)
}

fn refuse(error: &ChainError) -> ExitCode {
    eprintln!("chain is not compatible, no model was called: {error}");
    ExitCode::FAILURE
}

fn connect(agents: Vec<Vec<Agent>>, ground: Ground<'_>) -> anyhow::Result<Vec<Vec<Link>>> {
    let config = config::load()?;
    agents
        .into_iter()
        .map(|step| {
            step.into_iter()
                .map(|agent| link(&config, agent, ground))
                .collect()
        })
        .collect()
}

fn link(config: &ProvidersConfig, agent: Agent, ground: Ground<'_>) -> anyhow::Result<Link> {
    let client = providers::connect(config, agent.settings().model().provider())?;
    let workspace = step::workspace_for(&agent, ground)?;
    Ok(Link {
        agent,
        client,
        workspace,
    })
}

#[cfg(test)]
impl Prepared {
    pub fn in_place(steps: Vec<Vec<Link>>) -> Self {
        Self {
            steps,
            workplace: crate::workplace::test_support::in_place(Path::new(".")),
        }
    }
}

#[derive(Default)]
struct Printer {
    side_by_side: bool,
}

impl Printer {
    fn show(&mut self, event: ChainEvent<'_>) {
        match event {
            ChainEvent::StepStarted(agents) => {
                self.side_by_side = agents.len() > 1;
                eprintln!("→ {}", agents.join(" + "));
            }
            ChainEvent::Step { agent, progress } if self.side_by_side => {
                show_signed(agent, &progress);
            }
            ChainEvent::Step { progress, .. } => show_progress(progress),
            ChainEvent::StepResult(output) => eprintln!("{output:#}"),
            ChainEvent::StepFinished(record) => eprintln!("\n{}", step_line(record)),
        }
    }
}

fn show_signed(agent: &str, progress: &Progress<'_>) {
    if !matches!(progress, Progress::Text(_)) {
        eprintln!("{agent}: {}", progress.described());
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

pub fn explained(reason: &StopReason) -> String {
    match reason {
        StopReason::StepFailed(stop) => stop.explained().to_owned(),
        StopReason::InputRejected(violations) => {
            let problems: Vec<String> = violations.iter().map(Violation::to_string).collect();
            format!("its input was rejected: {}", problems.join("; "))
        }
    }
}

pub fn step_line(record: &StepRecord) -> String {
    format!("{} · {}", record.agent, record.report)
}

pub fn total_line(steps: &[StepRecord]) -> String {
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
