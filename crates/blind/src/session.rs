use std::{path::Path, process::ExitCode};

use crate::agents;
use crate::backend::Backend;
use crate::chain_backend::ChainBackend;
use crate::chain_command;
use crate::chain_run::Link;
use crate::cli::AGENTS_DIR;
use crate::crew::Crew;
use crate::providers::{self, config};
use crate::root_model::RootModel;
use crate::roster;
use crate::tui::{self, Services};
use crate::ui_config::{self, UiSettings};

const PROJECT_DIR: &str = ".";

pub fn run() -> anyhow::Result<ExitCode> {
    let config = config::load()?;
    let config_path = config::default_path()?;
    let ui = UiSettings::load(config_path.with_file_name(ui_config::FILE_NAME))?;
    let model = config.default_model().clone();
    let client = providers::connect(&config, model.provider())?;
    let agents_dir = Path::new(AGENTS_DIR);
    let services = Services {
        backend: Backend::start(client, model.model().to_owned(), &roster::of(agents_dir))?,
        chain_backend: ChainBackend::start(Box::new(links_in_this_project))?,
        crew: Crew::of(agents::load_all(agents_dir).unwrap_or_default()),
    };
    tui::run(services, RootModel::new(config_path, model), ui)?;
    Ok(ExitCode::SUCCESS)
}

fn links_in_this_project(agents: &[String]) -> anyhow::Result<Vec<Link>> {
    chain_command::links(Path::new(AGENTS_DIR), Path::new(PROJECT_DIR), agents)
}
