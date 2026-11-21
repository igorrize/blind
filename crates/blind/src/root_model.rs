use std::path::PathBuf;

use blind_core::ModelRef;

use crate::providers::config;

pub struct RootModel {
    config_path: PathBuf,
    current: ModelRef,
}

impl RootModel {
    pub fn new(config_path: PathBuf, current: ModelRef) -> Self {
        Self {
            config_path,
            current,
        }
    }

    pub fn current(&self) -> &ModelRef {
        &self.current
    }

    pub fn choose(&mut self, model_id: &str) -> anyhow::Result<()> {
        let chosen: ModelRef = format!("{}/{model_id}", self.current.provider()).parse()?;
        config::save_default_model(&self.config_path, &chosen)?;
        self.current = chosen;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;

    use tempfile::TempDir;

    use super::RootModel;
    use crate::providers::config::SAMPLE;

    pub fn saved_in(dir: &TempDir) -> RootModel {
        let path = dir.path().join("providers.toml");
        fs::write(&path, SAMPLE).expect("write config");
        RootModel::new(path, "ollama-cloud/glm-5.3".parse().expect("valid"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::test_support::saved_in;
    use super::*;

    #[test]
    fn choosing_a_model_switches_and_saves_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut root_model = saved_in(&dir);
        root_model.choose("glm-5.3-flash").expect("saved");
        assert_eq!(
            root_model.current().to_string(),
            "ollama-cloud/glm-5.3-flash"
        );
        let saved = fs::read_to_string(dir.path().join("providers.toml")).expect("read config");
        assert!(
            saved.contains(r#"model = "ollama-cloud/glm-5.3-flash""#),
            "{saved}"
        );
    }

    #[test]
    fn model_stays_when_the_config_cannot_be_saved() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("absent").join("providers.toml");
        let mut root_model =
            RootModel::new(missing, "ollama-cloud/glm-5.3".parse().expect("valid"));
        assert!(root_model.choose("glm-5.3-flash").is_err());
        assert_eq!(root_model.current().to_string(), "ollama-cloud/glm-5.3");
    }
}
