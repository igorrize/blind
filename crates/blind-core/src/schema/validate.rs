use serde_json::Value;

use super::{ObjectSchema, Schema, child_path, item_path};

const ROOT: &str = "value";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Violation {
    #[error("`{0}` is required but missing")]
    Missing(String),
    #[error("`{0}` is not declared in the contract")]
    Unexpected(String),
    #[error("`{path}` must be {expected}")]
    WrongType {
        path: String,
        expected: &'static str,
    },
    #[error("`{path}` is {length} characters, at most {limit} allowed")]
    TooLong {
        path: String,
        length: usize,
        limit: u32,
    },
    #[error("`{path}` has {count} items, at most {limit} allowed")]
    TooMany {
        path: String,
        count: usize,
        limit: u32,
    },
    #[error("`{0}` is outside the allowed range")]
    OutOfRange(String),
}

impl ObjectSchema {
    pub fn violations(&self, value: &Value) -> Vec<Violation> {
        self.violations_at(value, "")
    }

    fn violations_at(&self, value: &Value, path: &str) -> Vec<Violation> {
        let Value::Object(fields) = value else {
            return wrong_type(path, "an object");
        };
        let missing = self
            .required
            .iter()
            .filter(|name| !fields.contains_key(*name))
            .map(|name| Violation::Missing(child_path(path, name)));
        let present = fields
            .iter()
            .flat_map(|(name, field)| self.field_violations(name, field, path));
        missing.chain(present).collect()
    }

    fn field_violations(&self, name: &str, field: &Value, path: &str) -> Vec<Violation> {
        let field_path = child_path(path, name);
        match self.properties.get(name) {
            Some(schema) => schema.violations_at(field, &field_path),
            None => vec![Violation::Unexpected(field_path)],
        }
    }
}

impl Schema {
    fn violations_at(&self, value: &Value, path: &str) -> Vec<Violation> {
        match self {
            Schema::String { max_length } => {
                let Value::String(text) = value else {
                    return wrong_type(path, "a string");
                };
                let length = text.chars().count();
                match exceeded(length, *max_length) {
                    Some(limit) => vec![Violation::TooLong {
                        path: path.to_owned(),
                        length,
                        limit,
                    }],
                    None => Vec::new(),
                }
            }
            Schema::Integer { minimum, maximum } => {
                let Some(number) = value.as_i64() else {
                    return wrong_type(path, "an integer");
                };
                let too_small = minimum.is_some_and(|minimum| number < minimum);
                let too_large = maximum.is_some_and(|maximum| number > maximum);
                if too_small || too_large {
                    vec![Violation::OutOfRange(path.to_owned())]
                } else {
                    Vec::new()
                }
            }
            Schema::Boolean {} => {
                if value.is_boolean() {
                    Vec::new()
                } else {
                    wrong_type(path, "a boolean")
                }
            }
            Schema::Array { items, max_items } => {
                let Value::Array(elements) = value else {
                    return wrong_type(path, "an array");
                };
                let too_many =
                    exceeded(elements.len(), *max_items).map(|limit| Violation::TooMany {
                        path: path.to_owned(),
                        count: elements.len(),
                        limit,
                    });
                let inside = elements
                    .iter()
                    .flat_map(|element| items.violations_at(element, &item_path(path)));
                too_many.into_iter().chain(inside).collect()
            }
            Schema::Object(object) => object.violations_at(value, path),
        }
    }
}

fn wrong_type(path: &str, expected: &'static str) -> Vec<Violation> {
    let path = if path.is_empty() { ROOT } else { path };
    vec![Violation::WrongType {
        path: path.to_owned(),
        expected,
    }]
}

fn exceeded(actual: usize, limit: Option<u32>) -> Option<u32> {
    let limit = limit?;
    let within = u32::try_from(actual).is_ok_and(|actual| actual <= limit);
    (!within).then_some(limit)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::schema::test_support::object;

    const REVIEW: &str = r#"{"type":"object","required":["findings","approved"],"properties":{
        "findings":{"type":"array","maxItems":2,"items":{"type":"string","maxLength":5}},
        "approved":{"type":"boolean"},
        "line":{"type":"integer","minimum":1,"maximum":10}}}"#;

    fn problems(value: &Value) -> Vec<String> {
        let found = object(REVIEW).violations(value);
        found.iter().map(Violation::to_string).collect()
    }

    #[test]
    fn value_that_follows_the_schema_has_no_violations() {
        let value = json!({"findings": ["short"], "approved": true, "line": 3});
        assert_eq!(problems(&value), [] as [&str; 0]);
    }

    #[test]
    fn optional_field_may_be_absent() {
        assert!(problems(&json!({"findings": [], "approved": false})).is_empty());
    }

    #[test]
    fn missing_required_field_is_reported() {
        assert_eq!(
            problems(&json!({"findings": []})),
            ["`approved` is required but missing"]
        );
    }

    #[test]
    fn field_outside_the_contract_is_reported() {
        let value = json!({"findings": [], "approved": true, "secret": "x"});
        assert_eq!(
            problems(&value),
            ["`secret` is not declared in the contract"]
        );
    }

    #[test]
    fn wrong_types_are_reported_with_their_path() {
        let value = json!({"findings": [1], "approved": "yes"});
        assert_eq!(
            problems(&value),
            [
                "`approved` must be a boolean",
                "`findings[]` must be a string"
            ]
        );
    }

    #[test]
    fn limits_are_enforced() {
        let value = json!({"findings": ["toolong", "ok", "ok"], "approved": true, "line": 11});
        assert_eq!(
            problems(&value),
            [
                "`findings` has 3 items, at most 2 allowed",
                "`findings[]` is 7 characters, at most 5 allowed",
                "`line` is outside the allowed range"
            ]
        );
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        let value = json!({"findings": ["пять5"], "approved": true});
        assert!(problems(&value).is_empty());
    }

    #[test]
    fn fractional_number_is_not_an_integer() {
        let value = json!({"findings": [], "approved": true, "line": 1.5});
        assert_eq!(problems(&value), ["`line` must be an integer"]);
    }

    #[test]
    fn value_that_is_not_an_object_is_reported() {
        assert_eq!(problems(&json!("text")), ["`value` must be an object"]);
    }
}
