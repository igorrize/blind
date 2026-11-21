use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Deserialize;

pub const FILE_NAME: &str = "ui.toml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flavor {
    Latte,
    Frappe,
    Macchiato,
    #[default]
    Mocha,
}

pub struct UiSettings {
    path: PathBuf,
    flavor: Flavor,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiFile {
    #[serde(default)]
    theme: ThemeSection,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeSection {
    #[serde(default)]
    flavor: Flavor,
}

impl Flavor {
    pub const ALL: [Flavor; 4] = [
        Flavor::Latte,
        Flavor::Frappe,
        Flavor::Macchiato,
        Flavor::Mocha,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Flavor::Latte => "latte",
            Flavor::Frappe => "frappe",
            Flavor::Macchiato => "macchiato",
            Flavor::Mocha => "mocha",
        }
    }
}

impl UiSettings {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let flavor = match read_if_present(&path)? {
            Some(text) => saved_flavor(&text).with_context(|| format!("in {}", path.display()))?,
            None => Flavor::default(),
        };
        Ok(Self { path, flavor })
    }

    pub fn flavor(&self) -> Flavor {
        self.flavor
    }

    pub fn choose(&mut self, flavor: Flavor) -> anyhow::Result<()> {
        let current = read_if_present(&self.path)?.unwrap_or_default();
        let updated = with_flavor(&current, flavor)?;
        fs::write(&self.path, updated)
            .with_context(|| format!("cannot write {}", self.path.display()))?;
        self.flavor = flavor;
        Ok(())
    }
}

fn read_if_present(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn saved_flavor(text: &str) -> Result<Flavor, toml::de::Error> {
    let file: UiFile = toml::from_str(text)?;
    Ok(file.theme.flavor)
}

fn with_flavor(text: &str, flavor: Flavor) -> anyhow::Result<String> {
    let mut document: toml_edit::DocumentMut = text.parse()?;
    let theme = document.entry("theme").or_insert(toml_edit::table());
    theme["flavor"] = toml_edit::value(flavor.name());
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavor_indexes_match_their_order() {
        for flavor in Flavor::ALL {
            assert_eq!(Flavor::ALL[flavor.index()], flavor);
        }
    }

    #[test]
    fn every_flavor_name_is_read_back_from_the_file() {
        for flavor in Flavor::ALL {
            let text = with_flavor("", flavor).expect("valid toml");
            assert_eq!(saved_flavor(&text).expect("valid ui config"), flavor);
        }
    }

    #[test]
    fn missing_file_means_mocha() {
        let dir = tempfile::tempdir().expect("temp dir");
        let settings = UiSettings::load(dir.path().join(FILE_NAME)).expect("defaults");
        assert_eq!(settings.flavor(), Flavor::Mocha);
    }

    #[test]
    fn chosen_flavor_is_saved_and_loaded_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(FILE_NAME);
        let mut settings = UiSettings::load(path.clone()).expect("defaults");
        settings.choose(Flavor::Latte).expect("saved");
        assert_eq!(
            fs::read_to_string(&path).expect("ui.toml"),
            "[theme]\nflavor = \"latte\"\n"
        );
        assert_eq!(
            UiSettings::load(path).expect("loaded").flavor(),
            Flavor::Latte
        );
    }

    #[test]
    fn choosing_a_flavor_keeps_the_rest_of_the_file() {
        let text = "# my look\n[theme]\nflavor = \"mocha\"\n";
        let updated = with_flavor(text, Flavor::Frappe).expect("valid toml");
        assert_eq!(updated, "# my look\n[theme]\nflavor = \"frappe\"\n");
    }

    #[test]
    fn unknown_flavor_is_rejected() {
        let error = saved_flavor("[theme]\nflavor = \"dracula\"\n").expect_err("must be rejected");
        assert!(
            error.to_string().contains("unknown variant `dracula`"),
            "{error}"
        );
    }
}
