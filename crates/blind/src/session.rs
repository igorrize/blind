use std::process::ExitCode;

use crate::backend::Backend;
use crate::providers::{self, config};
use crate::root_model::RootModel;
use crate::tui;
use crate::ui_config::{self, UiSettings};

pub fn run() -> anyhow::Result<ExitCode> {
    let config = config::load()?;
    let config_path = config::default_path()?;
    let ui = UiSettings::load(config_path.with_file_name(ui_config::FILE_NAME))?;
    let model = config.default_model().clone();
    let client = providers::connect(&config, model.provider())?;
    let backend = Backend::start(client, model.model().to_owned())?;
    tui::run(backend, RootModel::new(config_path, model), ui)?;
    Ok(ExitCode::SUCCESS)
}
