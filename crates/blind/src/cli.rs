use std::path::PathBuf;

use blind_core::ModelRef;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "blind",
    version,
    about = "TUI harness with super-strict agents and sub-agents"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(about = "Work with agent and contract pairs")]
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    #[command(about = "Send one prompt to a model and print the answer")]
    Ask {
        prompt: String,
        #[arg(long, help = "provider/model, defaults to the one in providers.toml")]
        model: Option<ModelRef>,
    },
    #[command(about = "List the models a provider offers")]
    Models {
        #[arg(
            long,
            help = "Provider name, defaults to the provider of the default model"
        )]
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AgentCommand {
    #[command(about = "Check every agent folder: contract.json, agent.json, prompt.md")]
    Check {
        #[arg(
            long,
            default_value = ".blind/agents",
            help = "Folder with one subfolder per agent"
        )]
        dir: PathBuf,
    },
}
