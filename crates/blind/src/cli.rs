use std::path::PathBuf;

use blind_core::ModelRef;
use clap::{Parser, Subcommand};

const AGENTS_DIR: &str = ".blind/agents";
const AGENTS_DIR_HELP: &str = "Folder with one subfolder per agent";
const PROJECT_HELP: &str = "Project folder the agents may read, within their contract rights";

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
    #[command(about = "Run one agent on a task and print its checked result as JSON")]
    Run {
        agent: String,
        #[arg(long, help = "What the agent has to do")]
        task: String,
        #[arg(long, help = "JSON object with the fields the agent's contract takes")]
        input: Option<String>,
        #[arg(long, default_value = ".", help = PROJECT_HELP)]
        project: PathBuf,
        #[arg(long, default_value = AGENTS_DIR, help = AGENTS_DIR_HELP)]
        dir: PathBuf,
    },
    #[command(about = "Check or run several agents one after another")]
    Chain {
        #[command(subcommand)]
        command: ChainCommand,
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
pub enum ChainCommand {
    #[command(about = "Check that each agent's result fits the next agent's input")]
    Check {
        #[arg(required = true, help = "Agent names in the order they run")]
        agents: Vec<String>,
        #[arg(long, default_value = AGENTS_DIR, help = AGENTS_DIR_HELP)]
        dir: PathBuf,
    },
    #[command(about = "Run the agents in order and print the last checked result as JSON")]
    Run {
        #[arg(required = true, help = "Agent names in the order they run")]
        agents: Vec<String>,
        #[arg(long, help = "What the chain has to do; every agent gets it")]
        task: String,
        #[arg(long, default_value = ".", help = PROJECT_HELP)]
        project: PathBuf,
        #[arg(long, default_value = AGENTS_DIR, help = AGENTS_DIR_HELP)]
        dir: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum AgentCommand {
    #[command(about = "Check every agent folder: contract.json, agent.json, prompt.md")]
    Check {
        #[arg(long, default_value = AGENTS_DIR, help = AGENTS_DIR_HELP)]
        dir: PathBuf,
    },
}
