mod agent_check;
mod agents;
mod ask;
mod backend;
mod cli;
mod conversation;
mod model_catalog;
mod model_list;
mod providers;
mod root_model;
mod runtime;
mod session;
mod tui;
mod ui_config;

use std::process::ExitCode;

use clap::Parser;
use cli::{AgentCommand, Cli, Command};

fn main() -> anyhow::Result<ExitCode> {
    match Cli::parse().command {
        None => session::run(),
        Some(Command::Agent {
            command: AgentCommand::Check { dir },
        }) => agent_check::run(&dir),
        Some(Command::Ask { prompt, model }) => ask::run(&prompt, model),
        Some(Command::Models { provider }) => model_list::run(provider),
    }
}
