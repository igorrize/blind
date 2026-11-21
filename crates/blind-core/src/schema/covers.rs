use super::{ObjectSchema, Schema, child_path, item_path};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Mismatch {
    #[error("`{0}` is required but the previous step does not guarantee it")]
    NotGuaranteed(String),
    #[error("`{0}` has a different type in the previous step")]
    DifferentType(String),
    #[error("`{0}` may exceed the limit the next step accepts")]
    LooserLimit(String),
}

impl ObjectSchema {
    pub fn mismatches_with(&self, wanted: &ObjectSchema) -> Vec<Mismatch> {
        self.mismatches_at(wanted, "")
    }

    fn mismatches_at(&self, wanted: &ObjectSchema, path: &str) -> Vec<Mismatch> {
        wanted
            .properties
            .iter()
            .flat_map(|(name, wanted_field)| {
                self.field_mismatches(wanted, name, wanted_field, path)
            })
            .collect()
    }

    fn field_mismatches(
        &self,
        wanted: &ObjectSchema,
        name: &str,
        wanted_field: &Schema,
        path: &str,
    ) -> Vec<Mismatch> {
        let field = child_path(path, name);
        let guaranteed = self.declares(name) && self.requires(name);
        if wanted.requires(name) && !guaranteed {
            return vec![Mismatch::NotGuaranteed(field)];
        }
        match self.properties.get(name) {
            Some(given_field) => given_field.mismatches_at(wanted_field, &field),
            None => Vec::new(),
        }
    }
}

impl Schema {
    fn mismatches_at(&self, wanted: &Schema, path: &str) -> Vec<Mismatch> {
        match wanted {
            Schema::String { max_length: limit } => {
                let Schema::String { max_length } = self else {
                    return different_type(path);
                };
                looser_limit_unless(fits_under(*max_length, *limit), path)
            }
            Schema::Integer {
                minimum: lowest,
                maximum: highest,
            } => {
                let Schema::Integer { minimum, maximum } = self else {
                    return different_type(path);
                };
                let in_range = fits_over(*minimum, *lowest) && fits_under(*maximum, *highest);
                looser_limit_unless(in_range, path)
            }
            Schema::Boolean {} => {
                let Schema::Boolean {} = self else {
                    return different_type(path);
                };
                Vec::new()
            }
            Schema::Array {
                items: wanted_items,
                max_items: limit,
            } => {
                let Schema::Array { items, max_items } = self else {
                    return different_type(path);
                };
                let mut mismatches = looser_limit_unless(fits_under(*max_items, *limit), path);
                mismatches.extend(items.mismatches_at(wanted_items, &item_path(path)));
                mismatches
            }
            Schema::Object(wanted_object) => {
                let Schema::Object(object) = self else {
                    return different_type(path);
                };
                object.mismatches_at(wanted_object, path)
            }
        }
    }
}

fn different_type(path: &str) -> Vec<Mismatch> {
    vec![Mismatch::DifferentType(path.to_owned())]
}

fn looser_limit_unless(fits: bool, path: &str) -> Vec<Mismatch> {
    if fits {
        Vec::new()
    } else {
        vec![Mismatch::LooserLimit(path.to_owned())]
    }
}

fn fits_under<T: PartialOrd>(actual: Option<T>, limit: Option<T>) -> bool {
    match (actual, limit) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(actual), Some(limit)) => actual <= limit,
    }
}

fn fits_over<T: PartialOrd>(actual: Option<T>, limit: Option<T>) -> bool {
    match (actual, limit) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(actual), Some(limit)) => actual >= limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::test_support::object;

    fn mismatches(given: &str, wanted: &str) -> Vec<Mismatch> {
        object(given).mismatches_with(&object(wanted))
    }

    const FILES_UP_TO_200: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}}}}"#;

    #[test]
    fn identical_schemas_fit() {
        assert_eq!(mismatches(FILES_UP_TO_200, FILES_UP_TO_200), vec![]);
    }

    #[test]
    fn extra_given_fields_are_ignored() {
        let given = r#"{"type":"object","required":["files","summary"],"properties":{
            "files":{"type":"array","maxItems":10,"items":{"type":"string","maxLength":100}},
            "summary":{"type":"string"}}}"#;
        assert_eq!(mismatches(given, FILES_UP_TO_200), vec![]);
    }

    #[test]
    fn missing_required_field_is_not_guaranteed() {
        let given = r#"{"type":"object","properties":{}}"#;
        assert_eq!(
            mismatches(given, FILES_UP_TO_200),
            vec![Mismatch::NotGuaranteed("files".to_owned())]
        );
    }

    #[test]
    fn optional_given_field_does_not_guarantee_a_required_one() {
        let given = r#"{"type":"object","properties":{
            "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}}}}"#;
        assert_eq!(
            mismatches(given, FILES_UP_TO_200),
            vec![Mismatch::NotGuaranteed("files".to_owned())]
        );
    }

    #[test]
    fn missing_optional_field_fits() {
        let given = r#"{"type":"object","properties":{}}"#;
        let wanted = r#"{"type":"object","properties":{"focus":{"type":"string"}}}"#;
        assert_eq!(mismatches(given, wanted), vec![]);
    }

    #[test]
    fn different_type_is_reported() {
        let given =
            r#"{"type":"object","required":["files"],"properties":{"files":{"type":"string"}}}"#;
        assert_eq!(
            mismatches(given, FILES_UP_TO_200),
            vec![Mismatch::DifferentType("files".to_owned())]
        );
    }

    #[test]
    fn longer_items_and_longer_list_are_looser_limits() {
        let given = r#"{"type":"object","required":["files"],"properties":{
            "files":{"type":"array","maxItems":500,"items":{"type":"string","maxLength":300}}}}"#;
        assert_eq!(
            mismatches(given, FILES_UP_TO_200),
            vec![
                Mismatch::LooserLimit("files".to_owned()),
                Mismatch::LooserLimit("files[]".to_owned())
            ]
        );
    }

    #[test]
    fn unlimited_value_does_not_fit_a_limited_one() {
        let given =
            r#"{"type":"object","required":["line"],"properties":{"line":{"type":"integer"}}}"#;
        let wanted = r#"{"type":"object","required":["line"],"properties":{
            "line":{"type":"integer","minimum":1,"maximum":100000}}}"#;
        assert_eq!(
            mismatches(given, wanted),
            vec![Mismatch::LooserLimit("line".to_owned())]
        );
    }

    #[test]
    fn nested_object_fields_are_checked_with_their_path() {
        let given = r#"{"type":"object","required":["review"],"properties":{
            "review":{"type":"object","properties":{}}}}"#;
        let wanted = r#"{"type":"object","required":["review"],"properties":{
            "review":{"type":"object","required":["verdict"],"properties":{"verdict":{"type":"boolean"}}}}}"#;
        assert_eq!(
            mismatches(given, wanted),
            vec![Mismatch::NotGuaranteed("review.verdict".to_owned())]
        );
    }
}
