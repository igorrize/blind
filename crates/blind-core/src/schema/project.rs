use serde_json::Value;

use super::{ObjectSchema, Schema};

impl ObjectSchema {
    pub fn project(&self, value: &Value) -> Value {
        let Value::Object(fields) = value else {
            return value.clone();
        };
        let declared = fields.iter().filter_map(|(name, field)| {
            let schema = self.properties.get(name)?;
            Some((name.clone(), schema.project(field)))
        });
        Value::Object(declared.collect())
    }
}

impl Schema {
    fn project(&self, value: &Value) -> Value {
        match self {
            Schema::Object(object) => object.project(value),
            Schema::Array { items, .. } => match value.as_array() {
                Some(elements) => elements
                    .iter()
                    .map(|element| items.project(element))
                    .collect(),
                None => value.clone(),
            },
            Schema::String { .. } | Schema::Integer { .. } | Schema::Boolean {} => value.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::schema::test_support::object;

    const REVIEW_INPUT: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"}}}},
        "focus":{"type":"string"}}}"#;

    #[test]
    fn undeclared_fields_are_dropped_at_every_level() {
        let handed_over = json!({
            "files": [{"path": "a.rs", "blame": "someone"}, {"path": "b.rs"}],
            "notes": "private",
        });
        assert_eq!(
            object(REVIEW_INPUT).project(&handed_over),
            json!({"files": [{"path": "a.rs"}, {"path": "b.rs"}]})
        );
    }

    #[test]
    fn values_of_declared_fields_are_kept_as_they_are() {
        let handed_over = json!({"files": "not a list", "focus": 7});
        assert_eq!(object(REVIEW_INPUT).project(&handed_over), handed_over);
    }

    #[test]
    fn value_that_is_not_an_object_is_returned_unchanged() {
        assert_eq!(object(REVIEW_INPUT).project(&json!("text")), json!("text"));
    }
}
