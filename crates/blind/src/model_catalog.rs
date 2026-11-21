#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Catalog {
    #[default]
    NotLoaded,
    Loading,
    Loaded(Vec<String>),
    Failed(String),
}

impl Catalog {
    pub fn needs_loading(&self) -> bool {
        *self == Catalog::NotLoaded
    }

    pub fn models(&self) -> &[String] {
        match self {
            Catalog::Loaded(models) => models,
            Catalog::NotLoaded | Catalog::Loading | Catalog::Failed(_) => &[],
        }
    }
}

impl From<Result<Vec<String>, String>> for Catalog {
    fn from(outcome: Result<Vec<String>, String>) -> Self {
        match outcome {
            Ok(models) => Catalog::Loaded(models),
            Err(reason) => Catalog::Failed(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_new_catalog_needs_loading() {
        assert!(Catalog::default().needs_loading());
        assert!(!Catalog::Loading.needs_loading());
        assert!(!Catalog::Failed("offline".to_owned()).needs_loading());
    }

    #[test]
    fn loaded_catalog_lists_its_models() {
        let catalog = Catalog::from(Ok(vec!["beam".to_owned(), "kimi".to_owned()]));
        assert_eq!(catalog.models(), ["beam", "kimi"]);
    }

    #[test]
    fn failed_catalog_has_no_models() {
        let catalog = Catalog::from(Err("provider answered 401".to_owned()));
        assert_eq!(catalog, Catalog::Failed("provider answered 401".to_owned()));
        assert!(catalog.models().is_empty());
    }
}
