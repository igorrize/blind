use std::{path::Path, process::ExitCode};

use anyhow::Context;
use blind_core::{ObjectSchema, RootAgent};

use crate::agents;
use crate::backend::Backend;
use crate::chain_backend::ChainBackend;
use crate::chain_command::{self, Prepared};
use crate::cli::{AGENTS_DIR, ROOT_DIR};
use crate::crew::Crew;
use crate::lineup::Lineup;
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
    let root = agents::load_root(Path::new(ROOT_DIR))
        .with_context(|| format!("cannot load root from {ROOT_DIR}"))?;
    let root_input = root
        .as_ref()
        .map_or_else(ObjectSchema::empty, |root| root.contract().input().clone());
    let about = about_the_project(root.as_ref(), agents_dir, &root_input);
    let crew = Crew::of(agents::load_all(agents_dir).unwrap_or_default());
    let crew = match &root {
        Some(root) => crew.with_root(root.contract().clone()),
        None => crew,
    };
    let services = Services {
        backend: Backend::start(client, model.model().to_owned(), &about)?,
        chain_backend: ChainBackend::start(Box::new(links_in_this_project), root_input)?,
        crew,
    };
    tui::run(services, RootModel::new(config_path, model), ui)?;
    Ok(ExitCode::SUCCESS)
}

fn about_the_project(
    root: Option<&RootAgent>,
    agents_dir: &Path,
    root_input: &ObjectSchema,
) -> String {
    let own = root.map(|root| root.prompt().trim().to_owned());
    let known = [roster::of(agents_dir), roster::delivery(root_input)];
    own.into_iter()
        .chain(known)
        .collect::<Vec<String>>()
        .join("\n\n")
}

fn links_in_this_project(lineup: &Lineup) -> anyhow::Result<Prepared> {
    chain_command::prepare(Path::new(AGENTS_DIR), Path::new(PROJECT_DIR), lineup)
}
