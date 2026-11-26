use serde_json::{Map, Value, map::Entry};

use super::{ObjectSchema, Schema};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` comes from both, and only lists of the same kind can be put together")]
pub struct Clash(pub String);

impl ObjectSchema {
    pub fn merged_with(&self, other: &ObjectSchema) -> Result<ObjectSchema, Clash> {
        let mut properties = self.properties.clone();
        for (name, schema) in &other.properties {
            let merged = match properties.get(name) {
                Some(mine) => joined_lists(mine, schema).ok_or_else(|| Clash(name.clone()))?,
                None => schema.clone(),
            };
            properties.insert(name.clone(), merged);
        }
        let mut required = self.required.clone();
        let more = other.required.iter().filter(|name| !self.requires(name));
        required.extend(more.cloned());
        Ok(ObjectSchema {
            properties,
            required,
        })
    }
}

fn joined_lists(first: &Schema, second: &Schema) -> Option<Schema> {
    let Schema::Array {
        items,
        max_items: first_limit,
    } = first
    else {
        return None;
    };
    let Schema::Array {
        items: other_items,
        max_items: second_limit,
    } = second
    else {
        return None;
    };
    if items != other_items {
        return None;
    }
    let limits = first_limit.zip(*second_limit);
    Some(Schema::Array {
        items: items.clone(),
        max_items: limits.and_then(|(first, second)| first.checked_add(second)),
    })
}

pub fn merged_outputs(outputs: &[Value]) -> Value {
    let mut merged = Map::new();
    let fields = outputs.iter().filter_map(Value::as_object).flatten();
    for (name, value) in fields {
        match merged.entry(name.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(value.clone());
            }
            Entry::Occupied(mut slot) => {
                if let (Value::Array(list), Value::Array(more)) = (slot.get_mut(), value) {
                    list.extend(more.iter().cloned());
                }
            }
        }
    }
    Value::Object(merged)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(schema: Value) -> ObjectSchema {
        let Schema::Object(object) = serde_json::from_value(schema).expect("valid schema") else {
            panic!("must be an object");
        };
        object
    }

    fn files(limit: Option<u32>) -> ObjectSchema {
        let mut list = json!({"type": "array", "items": {"type": "string"}});
        if let Some(limit) = limit {
            list["maxItems"] = limit.into();
        }
        object(json!({"type": "object", "required": ["files"], "properties": {"files": list}}))
    }

    fn summary() -> ObjectSchema {
        object(json!({"type": "object", "properties": {"summary": {"type": "string"}}}))
    }

    #[test]
    fn different_fields_are_all_kept_and_required_ones_stay_required() {
        let merged = files(Some(5)).merged_with(&summary()).expect("no clash");
        assert!(merged.declares("files") && merged.declares("summary"));
        assert_eq!(merged.required(), ["files"]);
    }

    #[test]
    fn same_lists_are_joined_and_their_limits_add_up() {
        let merged = files(Some(5))
            .merged_with(&files(Some(7)))
            .expect("no clash");
        assert_eq!(merged, files(Some(12)));
    }

    #[test]
    fn joined_list_is_unlimited_when_one_side_is_unlimited() {
        let merged = files(Some(5)).merged_with(&files(None)).expect("no clash");
        assert_eq!(merged, files(None));
    }

    #[test]
    fn field_required_by_one_side_is_required_in_the_result() {
        let optional = object(json!({"type": "object", "properties": {
            "files": {"type": "array", "items": {"type": "string"}}}}));
        let merged = optional.merged_with(&files(None)).expect("no clash");
        assert!(merged.requires("files"));
    }

    #[test]
    fn same_name_that_is_not_a_list_of_the_same_kind_clashes() {
        let numbers = object(json!({"type": "object", "properties": {
            "files": {"type": "array", "items": {"type": "integer"}}}}));
        let text = object(json!({"type": "object", "properties": {"files": {"type": "string"}}}));
        for other in [numbers, text] {
            assert_eq!(
                files(None).merged_with(&other),
                Err(Clash("files".to_owned()))
            );
        }
        assert_eq!(
            summary().merged_with(&summary()),
            Err(Clash("summary".to_owned()))
        );
    }

    #[test]
    fn outputs_are_merged_and_lists_joined_in_order() {
        let merged = merged_outputs(&[
            json!({"files": ["a.rs"], "summary": "first"}),
            json!({"files": ["b.rs"], "steps": ["read"]}),
        ]);
        assert_eq!(
            merged,
            json!({"files": ["a.rs", "b.rs"], "summary": "first", "steps": ["read"]})
        );
    }

    #[test]
    fn single_output_is_returned_as_it_is() {
        let output = json!({"files": ["a.rs"]});
        assert_eq!(merged_outputs(std::slice::from_ref(&output)), output);
    }
}
