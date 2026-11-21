mod covers;

use std::collections::BTreeMap;

use serde::Deserialize;

pub use covers::Mismatch;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Schema {
    String {
        max_length: Option<u32>,
    },
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Boolean {},
    Array {
        items: Box<Schema>,
        max_items: Option<u32>,
    },
    Object(ObjectSchema),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectSchema {
    properties: BTreeMap<String, Schema>,
    #[serde(default)]
    required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaProblem {
    #[error("`{0}` is required but not declared in properties")]
    UnknownRequired(String),
    #[error("`{field}`: minimum {minimum} is greater than maximum {maximum}")]
    EmptyRange {
        field: String,
        minimum: i64,
        maximum: i64,
    },
}

impl ObjectSchema {
    pub fn check(&self) -> Result<(), SchemaProblem> {
        self.check_at("")
    }

    pub fn required(&self) -> &[String] {
        &self.required
    }

    pub fn requires(&self, name: &str) -> bool {
        self.required.iter().any(|required| required == name)
    }

    pub fn declares(&self, name: &str) -> bool {
        self.properties.contains_key(name)
    }

    fn check_at(&self, path: &str) -> Result<(), SchemaProblem> {
        if let Some(undeclared) = self.required.iter().find(|name| !self.declares(name)) {
            return Err(SchemaProblem::UnknownRequired(child_path(path, undeclared)));
        }
        self.properties
            .iter()
            .try_for_each(|(name, schema)| schema.check_at(&child_path(path, name)))
    }
}

impl Schema {
    fn check_at(&self, path: &str) -> Result<(), SchemaProblem> {
        match self {
            Schema::Integer {
                minimum: Some(minimum),
                maximum: Some(maximum),
            } if minimum > maximum => Err(SchemaProblem::EmptyRange {
                field: path.to_owned(),
                minimum: *minimum,
                maximum: *maximum,
            }),
            Schema::Array { items, .. } => items.check_at(&item_path(path)),
            Schema::Object(object) => object.check_at(path),
            Schema::String { .. } | Schema::Integer { .. } | Schema::Boolean {} => Ok(()),
        }
    }
}

fn child_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}.{name}")
    }
}

fn item_path(parent: &str) -> String {
    format!("{parent}[]")
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{ObjectSchema, Schema};

    pub fn object(json: &str) -> ObjectSchema {
        let schema = serde_json::from_str(json).expect("schema parses");
        let Schema::Object(object) = schema else {
            panic!("expected an object schema, got {schema:?}");
        };
        object
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::object;
    use super::*;

    fn rejection(json: &str) -> String {
        serde_json::from_str::<Schema>(json)
            .expect_err("schema must be rejected")
            .to_string()
    }

    #[test]
    fn parses_nested_schema() {
        let schema = object(
            r#"{"type":"object","required":["files"],"properties":{
                "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}},
                "strict":{"type":"boolean"}}}"#,
        );
        assert!(schema.requires("files"));
        assert!(schema.declares("strict"));
        assert_eq!(schema.check(), Ok(()));
    }

    #[test]
    fn rejects_misspelled_keyword() {
        let error = rejection(r#"{"type":"object","properties":{},"requred":["rubric"]}"#);
        assert!(error.contains("unknown field `requred`"), "{error}");
    }

    #[test]
    fn rejects_keyword_of_another_type() {
        let error = rejection(r#"{"type":"integer","maxLength":5}"#);
        assert!(error.contains("unknown field `maxLength`"), "{error}");
    }

    #[test]
    fn rejects_extra_keyword_on_boolean() {
        let error = rejection(r#"{"type":"boolean","maxLength":5}"#);
        assert!(error.contains("unknown field `maxLength`"), "{error}");
    }

    #[test]
    fn rejects_negative_limit() {
        let error = rejection(r#"{"type":"string","maxLength":-1}"#);
        assert!(error.contains("expected u32"), "{error}");
    }

    #[test]
    fn rejects_array_without_items() {
        let error = rejection(r#"{"type":"array"}"#);
        assert!(error.contains("missing field `items`"), "{error}");
    }

    #[test]
    fn rejects_schema_without_type() {
        let error = rejection(r#"{"maxLength":10}"#);
        assert!(error.contains("missing field `type`"), "{error}");
    }

    #[test]
    fn rejects_unknown_type() {
        let error = rejection(r#"{"type":"anything"}"#);
        assert!(error.contains("unknown variant `anything`"), "{error}");
    }

    #[test]
    fn check_reports_required_field_missing_from_properties() {
        let schema = object(
            r#"{"type":"object","properties":{"review":{"type":"object","required":["filse"],
                "properties":{"files":{"type":"boolean"}}}}}"#,
        );
        assert_eq!(
            schema.check(),
            Err(SchemaProblem::UnknownRequired("review.filse".to_owned()))
        );
    }

    #[test]
    fn check_reports_empty_integer_range() {
        let schema = object(
            r#"{"type":"object","properties":{"lines":{"type":"array",
                "items":{"type":"integer","minimum":10,"maximum":1}}}}"#,
        );
        assert_eq!(
            schema.check(),
            Err(SchemaProblem::EmptyRange {
                field: "lines[]".to_owned(),
                minimum: 10,
                maximum: 1
            })
        );
    }
}
