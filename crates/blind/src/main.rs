mod agent_check;
mod agents;
mod ask;
mod backend;
mod chain_backend;
mod chain_command;
mod chain_log;
mod chain_run;
mod cli;
mod conversation;
mod crew;
mod model_catalog;
mod model_list;
mod providers;
mod root_model;
mod roster;
mod run_agent;
mod runtime;
mod session;
mod step;
mod tui;
mod ui_config;

use std::process::ExitCode;

use clap::Parser;
use cli::{AgentCommand, ChainCommand, Cli, Command};

fn main() -> anyhow::Result<ExitCode> {
    match Cli::parse().command {
        None => session::run(),
        Some(Command::Agent {
            command: AgentCommand::Check { dir },
        }) => agent_check::run(&dir),
        Some(Command::Run {
            agent,
            task,
            input,
            project,
            dir,
        }) => run_agent::run(&dir, &project, &agent, &task, input.as_deref()),
        Some(Command::Chain { command }) => chain(command),
        Some(Command::Ask { prompt, model }) => ask::run(&prompt, model),
        Some(Command::Models { provider }) => model_list::run(provider),
    }
}

fn chain(command: ChainCommand) -> anyhow::Result<ExitCode> {
    match command {
        ChainCommand::Check { agents, dir } => chain_command::check(&dir, &agents),
        ChainCommand::Run {
            agents,
            task,
            project,
            dir,
        } => chain_command::run(&dir, &project, &agents, &task),
    }
}
