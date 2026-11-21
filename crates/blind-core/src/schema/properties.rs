use proptest::{
    collection::{btree_map, vec},
    option,
    prelude::*,
};
use serde_json::{Map, Value, json};

use super::{ObjectSchema, Schema, Violation};

const UNDECLARED: &str = "undeclared";
const DEFAULT_TEXT_LENGTH: usize = 12;
const DEFAULT_ITEMS: usize = 3;
const DEFAULT_MINIMUM: i64 = -20;
const DEFAULT_MAXIMUM: i64 = 20;

fn limited<T: Into<Value>>(mut schema: Value, keyword: &str, limit: Option<T>) -> Value {
    if let Some(limit) = limit {
        schema[keyword] = limit.into();
    }
    schema
}

fn leaf_schema() -> impl Strategy<Value = Value> {
    prop_oneof![
        option::of(0u32..12).prop_map(|limit| limited(
            json!({"type": "string"}),
            "maxLength",
            limit
        )),
        (option::of(-9i64..0), option::of(0i64..9)).prop_map(|(minimum, maximum)| {
            let integer = limited(json!({"type": "integer"}), "minimum", minimum);
            limited(integer, "maximum", maximum)
        }),
        Just(json!({"type": "boolean"})),
    ]
}

fn any_schema() -> impl Strategy<Value = Value> {
    leaf_schema().prop_recursive(3, 24, 4, |inner| {
        let array = (inner.clone(), option::of(0u32..4)).prop_map(|(items, limit)| {
            limited(json!({"type": "array", "items": items}), "maxItems", limit)
        });
        prop_oneof![array, object_schema_of(inner)]
    })
}

fn object_schema_of(field: impl Strategy<Value = Value>) -> impl Strategy<Value = Value> {
    btree_map("[a-e]", (field, any::<bool>()), 0..4).prop_map(|fields| {
        let required: Vec<&String> = fields
            .iter()
            .filter(|(_, (_, required))| *required)
            .map(|(name, _)| name)
            .collect();
        let properties: Map<String, Value> = fields
            .iter()
            .map(|(name, (schema, _))| (name.clone(), schema.clone()))
            .collect();
        json!({"type": "object", "properties": properties, "required": required})
    })
}

fn object_schema() -> impl Strategy<Value = Value> {
    object_schema_of(any_schema())
}

fn parsed(schema: &Value) -> ObjectSchema {
    let schema = serde_json::from_value(schema.clone()).expect("generated schema parses");
    let Schema::Object(object) = schema else {
        panic!("generated schema must be an object, got {schema:?}");
    };
    object
}

fn conforming(schema: &Value) -> BoxedStrategy<Value> {
    match schema["type"].as_str().unwrap_or_default() {
        "string" => text_up_to(count(schema, "maxLength", DEFAULT_TEXT_LENGTH)),
        "integer" => {
            let minimum = schema["minimum"].as_i64().unwrap_or(DEFAULT_MINIMUM);
            let maximum = schema["maximum"].as_i64().unwrap_or(DEFAULT_MAXIMUM);
            (minimum..=maximum).prop_map(Value::from).boxed()
        }
        "boolean" => any::<bool>().prop_map(Value::from).boxed(),
        "array" => {
            let most = count(schema, "maxItems", DEFAULT_ITEMS);
            vec(conforming(&schema["items"]), 0..=most)
                .prop_map(Value::from)
                .boxed()
        }
        _ => conforming_object(schema),
    }
}

fn count(schema: &Value, keyword: &str, default: usize) -> usize {
    let limit = schema[keyword].as_u64();
    limit.map_or(default, |limit| usize::try_from(limit).unwrap_or(default))
}

fn text_up_to(longest: usize) -> BoxedStrategy<Value> {
    let character = prop_oneof![Just('a'), Just('я'), Just(' '), Just('"')];
    vec(character, 0..=longest)
        .prop_map(|characters| Value::from(characters.into_iter().collect::<String>()))
        .boxed()
}

fn conforming_object(schema: &Value) -> BoxedStrategy<Value> {
    let required: Vec<String> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect();
    let empty = Just(Map::new()).boxed();
    let fields = schema["properties"].as_object().into_iter().flatten();
    let built = fields.fold(empty, |built, (name, field_schema)| {
        let name = name.clone();
        let always = required.contains(&name);
        let included = prop_oneof![Just(true), Just(always)];
        (built, conforming(field_schema), included)
            .prop_map(move |(mut object, value, included)| {
                if included {
                    object.insert(name.clone(), value);
                }
                object
            })
            .boxed()
    });
    built.prop_map(Value::Object).boxed()
}

fn schema_with_conforming_value() -> impl Strategy<Value = (ObjectSchema, Value)> {
    object_schema().prop_flat_map(|schema| (Just(parsed(&schema)), conforming(&schema)))
}

fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "[a-e ]{0,6}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::from),
            btree_map("[a-e]", inner, 0..4)
                .prop_map(|fields| Value::Object(fields.into_iter().collect())),
        ]
    })
}

fn every_object_is_closed(schema: &Value) -> bool {
    match schema["type"].as_str().unwrap_or_default() {
        "object" => {
            let closed = schema["additionalProperties"] == json!(false);
            let fields = schema["properties"].as_object().into_iter().flatten();
            closed
                && fields
                    .into_iter()
                    .all(|(_, field)| every_object_is_closed(field))
        }
        "array" => every_object_is_closed(&schema["items"]),
        _ => true,
    }
}

proptest! {
    #[test]
    fn generated_schemas_are_well_formed(schema in object_schema()) {
        prop_assert_eq!(parsed(&schema).check(), Ok(()));
    }

    #[test]
    fn value_built_from_a_schema_has_no_violations(
        (schema, value) in schema_with_conforming_value()
    ) {
        prop_assert_eq!(schema.violations(&value), vec![]);
    }

    #[test]
    fn value_without_violations_has_only_declared_fields_and_every_required_one(
        schema in object_schema(),
        value in any_json(),
    ) {
        let schema = parsed(&schema);
        if schema.violations(&value).is_empty() {
            let fields = value.as_object().expect("only an object can pass");
            prop_assert!(fields.keys().all(|name| schema.declares(name)));
            prop_assert!(schema.required().iter().all(|name| fields.contains_key(name)));
        }
    }

    #[test]
    fn dropping_a_required_field_is_always_caught(
        (schema, value) in schema_with_conforming_value()
    ) {
        for name in schema.required() {
            let mut broken = value.clone();
            broken.as_object_mut().expect("object").remove(name);
            let missing = Violation::Missing(name.clone());
            prop_assert!(schema.violations(&broken).contains(&missing));
        }
    }

    #[test]
    fn undeclared_field_is_always_caught(
        (schema, value) in schema_with_conforming_value(),
        extra in any_json(),
    ) {
        let mut smuggling = value;
        smuggling.as_object_mut().expect("object").insert(UNDECLARED.to_owned(), extra);
        let unexpected = Violation::Unexpected(UNDECLARED.to_owned());
        prop_assert!(schema.violations(&smuggling).contains(&unexpected));
    }

    #[test]
    fn null_in_place_of_any_field_is_always_caught(
        (schema, value) in schema_with_conforming_value()
    ) {
        for name in value.as_object().expect("object").keys() {
            let mut broken = value.clone();
            broken[name] = Value::Null;
            prop_assert!(!schema.violations(&broken).is_empty());
        }
    }

    #[test]
    fn schema_covers_itself(schema in object_schema()) {
        let schema = parsed(&schema);
        prop_assert_eq!(schema.mismatches_with(&schema), vec![]);
    }

    #[test]
    fn json_schema_for_the_provider_closes_every_object(schema in object_schema()) {
        prop_assert!(every_object_is_closed(&parsed(&schema).to_json_schema()));
    }
}

const NEW_OPTIONAL_FIELD: &str = "optional_extra";

fn with_looser_bound(schema: Value, keyword: &'static str, direction: i64) -> BoxedStrategy<Value> {
    let Some(bound) = schema[keyword].as_i64() else {
        return Just(schema).boxed();
    };
    let looser = prop_oneof![
        Just(None),
        (0i64..5).prop_map(move |distance| Some(bound + direction * distance)),
    ];
    looser
        .prop_map(move |looser| {
            let mut loosened = schema.clone();
            loosened[keyword] = looser.map_or(Value::Null, Value::from);
            without_nulls(loosened)
        })
        .boxed()
}

fn without_nulls(schema: Value) -> Value {
    let Value::Object(keywords) = schema else {
        return schema;
    };
    let set = keywords.into_iter().filter(|(_, value)| !value.is_null());
    Value::Object(set.collect())
}

fn weakened(schema: &Value) -> BoxedStrategy<Value> {
    match schema["type"].as_str().unwrap_or_default() {
        "string" => with_looser_bound(schema.clone(), "maxLength", 1),
        "integer" => with_looser_bound(schema.clone(), "minimum", -1)
            .prop_flat_map(|loosened| with_looser_bound(loosened, "maximum", 1))
            .boxed(),
        "boolean" => Just(schema.clone()).boxed(),
        "array" => {
            let array = schema.clone();
            weakened(&schema["items"])
                .prop_flat_map(move |items| {
                    let mut loosened = array.clone();
                    loosened["items"] = items;
                    with_looser_bound(loosened, "maxItems", 1)
                })
                .boxed()
        }
        _ => weakened_object(schema),
    }
}

fn weakened_object(schema: &Value) -> BoxedStrategy<Value> {
    let guaranteed: Vec<String> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect();
    let nothing = Just((Map::new(), Vec::<String>::new())).boxed();
    let fields = schema["properties"].as_object().into_iter().flatten();
    let kept = fields.fold(nothing, |built, (name, field)| {
        let name = name.clone();
        let may_demand = guaranteed.contains(&name);
        (built, weakened(field), any::<bool>(), any::<bool>())
            .prop_map(
                move |((mut properties, mut required), field, kept, demanded)| {
                    if kept {
                        properties.insert(name.clone(), field);
                    }
                    if kept && demanded && may_demand {
                        required.push(name.clone());
                    }
                    (properties, required)
                },
            )
            .boxed()
    });
    (kept, option::of(leaf_schema()))
        .prop_map(|((mut properties, required), extra)| {
            properties.extend(extra.map(|schema| (NEW_OPTIONAL_FIELD.to_owned(), schema)));
            json!({"type": "object", "properties": properties, "required": required})
        })
        .boxed()
}

fn handoff() -> impl Strategy<Value = (ObjectSchema, ObjectSchema, Value)> {
    object_schema().prop_flat_map(|given| {
        let wanted = weakened(&given).prop_map(|wanted| parsed(&wanted));
        (Just(parsed(&given)), wanted, conforming(&given))
    })
}

proptest! {
    #[test]
    fn schema_covers_every_schema_that_asks_for_less((given, wanted, _) in handoff()) {
        prop_assert_eq!(given.mismatches_with(&wanted), vec![]);
    }

    #[test]
    fn value_handed_to_a_covered_schema_always_fits_it((given, wanted, value) in handoff()) {
        prop_assert_eq!(given.violations(&value), vec![]);
        prop_assert_eq!(wanted.violations(&wanted.project(&value)), vec![]);
    }

    #[test]
    fn projection_leaves_only_declared_fields(schema in object_schema(), value in any_json()) {
        let schema = parsed(&schema);
        if let Value::Object(fields) = schema.project(&value) {
            prop_assert!(fields.keys().all(|name| schema.declares(name)));
        }
    }

    #[test]
    fn projection_does_not_change_a_value_that_fits(
        (schema, value) in schema_with_conforming_value()
    ) {
        prop_assert_eq!(schema.project(&value), value);
    }
}
