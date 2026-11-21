use std::process::ExitCode;

use crate::providers::{self, config};
use crate::runtime;

pub fn run(provider: Option<String>) -> anyhow::Result<ExitCode> {
    let config = config::load()?;
    let provider = provider.unwrap_or_else(|| config.default_model().provider().to_owned());
    let client = providers::connect(&config, &provider)?;
    let ids = runtime::block_on(client.model_ids())??;
    for id in ids {
        println!("{provider}/{id}");
    }
    Ok(ExitCode::SUCCESS)
}
