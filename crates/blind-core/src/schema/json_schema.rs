use serde_json::{Map, Value, json};

use super::{ObjectSchema, Schema};

impl ObjectSchema {
    pub fn to_json_schema(&self) -> Value {
        let properties: Map<String, Value> = self
            .properties
            .iter()
            .map(|(name, schema)| (name.clone(), schema.to_json_schema()))
            .collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": self.required,
            "additionalProperties": false,
        })
    }
}

impl Schema {
    fn to_json_schema(&self) -> Value {
        match self {
            Schema::String { max_length } => with_limits("string", [("maxLength", *max_length)]),
            Schema::Integer { minimum, maximum } => {
                with_limits("integer", [("minimum", *minimum), ("maximum", *maximum)])
            }
            Schema::Boolean {} => json!({"type": "boolean"}),
            Schema::Array { items, max_items } => {
                let mut schema = with_limits("array", [("maxItems", *max_items)]);
                schema["items"] = items.to_json_schema();
                schema
            }
            Schema::Object(object) => object.to_json_schema(),
        }
    }
}

fn with_limits<T: Into<Value>, const N: usize>(
    kind: &str,
    limits: [(&str, Option<T>); N],
) -> Value {
    let mut schema = json!({"type": kind});
    for (keyword, limit) in limits {
        if let Some(limit) = limit {
            schema[keyword] = limit.into();
        }
    }
    schema
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::schema::test_support::object;

    #[test]
    fn json_schema_closes_objects_and_keeps_limits() {
        let schema = object(
            r#"{"type":"object","required":["files"],"properties":{
                "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}},
                "line":{"type":"integer","minimum":1},
                "done":{"type":"boolean"},
                "meta":{"type":"object","properties":{}}}}"#,
        );
        assert_eq!(
            schema.to_json_schema(),
            json!({
                "type": "object",
                "required": ["files"],
                "additionalProperties": false,
                "properties": {
                    "files": {"type": "array", "maxItems": 50,
                              "items": {"type": "string", "maxLength": 200}},
                    "line": {"type": "integer", "minimum": 1},
                    "done": {"type": "boolean"},
                    "meta": {"type": "object", "properties": {}, "required": [],
                             "additionalProperties": false}
                }
            })
        );
    }
}
