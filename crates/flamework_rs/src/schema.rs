//! Validation against the transformer's JSON schemas, as Ajv 8 (default options) validates and words it.
//!
//! Hand-written rather than a schema crate, because the build reports Ajv's first error exactly, and which error
//! comes first follows how Ajv orders its checks: the type first when no keyword of that type's group is present,
//! then the keyword groups in the order untyped (`enum`, `oneOf`), number, string, array, object; inside the object
//! group `required`, then `additionalProperties` (data keys in JavaScript order), then `properties` (schema order),
//! each stopping at its first error. Errors inside `oneOf` branches are collected and reported with the `oneOf`
//! error. Only the keywords the two schemas use are implemented (`type`, `enum`, `oneOf`, `minimum`, `items`,
//! `required`, `additionalProperties`, `properties`, `$ref` to an `$id`); others are ignored.
//!
//! The schemas themselves are the transformer's files, embedded, so the validation cannot drift from them.

use std::sync::OnceLock;

use crate::js;
use crate::json::{self, JsObject, JsValue};

/// `flamework.config.schema.json`, as the transformer ships it.
pub const PROJECT_CONFIG_SCHEMA_TEXT: &str = include_str!("../schema/flamework.config.schema.json");
/// `flamework-schema.json`: the build info's shape.
pub const FLAMEWORK_SCHEMA_TEXT: &str = include_str!("../schema/flamework-schema.json");

/// The parsed `flamework.config.schema.json`.
pub fn project_config_schema() -> &'static JsValue {
    static SCHEMA: OnceLock<JsValue> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        json::parse(PROJECT_CONFIG_SCHEMA_TEXT).expect("the config schema is valid JSON")
    })
}

/// The parsed `flamework-schema.json`.
pub fn flamework_schema() -> &'static JsValue {
    static SCHEMA: OnceLock<JsValue> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        json::parse(FLAMEWORK_SCHEMA_TEXT).expect("the flamework schema is valid JSON")
    })
}

/// The build info's schema: `root#/properties/buildInfo`.
pub fn build_info_schema() -> &'static JsValue {
    flamework_schema()
        .get("properties")
        .and_then(|p| p.get("buildInfo"))
        .expect("flamework-schema.json has buildInfo")
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaError {
    /// A JSON pointer into the value: `""` for the root.
    pub instance_path: String,
    pub message: String,
    pub additional_property: Option<String>,
    pub allowed_values: Option<Vec<JsValue>>,
}

impl SchemaError {
    /// The transformer's `describeSchemaErrors` line: where (`/` for the root), what, and the property or the
    /// allowed values when the error names them.
    pub fn describe(&self) -> String {
        let location = if self.instance_path.is_empty() {
            "/"
        } else {
            &self.instance_path
        };
        let extra = if let Some(key) = &self.additional_property {
            format!(" '{key}'")
        } else if let Some(values) = &self.allowed_values {
            format!(
                ": {}",
                values
                    .iter()
                    .map(JsValue::to_json)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        format!("{location} {}{extra}", self.message)
    }
}

/// Validates `value` against `schema` (a node of `root`, whose `$id`s `$ref`s resolve to). `Err` holds what Ajv's
/// `errors` would: the first error, or the branch errors and the `oneOf` error.
pub fn validate(root: &JsValue, schema: &JsValue, value: &JsValue) -> Result<(), Vec<SchemaError>> {
    let mut v = Validator {
        root,
        errors: Vec::new(),
        composite: 0,
    };
    match v.node(schema, value, "") {
        Ok(()) if v.errors.is_empty() => Ok(()),
        _ => Err(v.errors),
    }
}

/// Validation stopped at an error (outside a composite rule, the first error ends it).
struct Stop;

struct Validator<'a> {
    root: &'a JsValue,
    errors: Vec<SchemaError>,
    /// Inside `oneOf`: errors are collected rather than ending the validation.
    composite: usize,
}

const NUMBER_KEYWORDS: &[&str] = &[
    "maximum",
    "minimum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "multipleOf",
];
const STRING_KEYWORDS: &[&str] = &["maxLength", "minLength", "pattern", "format"];
const ARRAY_KEYWORDS: &[&str] = &[
    "maxItems",
    "minItems",
    "uniqueItems",
    "additionalItems",
    "items",
    "contains",
];
const OBJECT_KEYWORDS: &[&str] = &[
    "maxProperties",
    "minProperties",
    "required",
    "propertyNames",
    "additionalProperties",
    "dependencies",
    "properties",
    "patternProperties",
];
const UNTYPED_KEYWORDS: &[&str] = &["const", "enum", "not", "anyOf", "oneOf", "allOf", "if"];

fn has_any(schema: &JsObject, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| schema.contains_key(k))
}

fn type_matches(ty: &str, value: &JsValue) -> bool {
    match (ty, value) {
        ("string", JsValue::String(_)) => true,
        ("number", JsValue::Number(n)) => n.is_finite(),
        ("integer", JsValue::Number(n)) => n.is_finite() && n.fract() == 0.0,
        ("boolean", JsValue::Bool(_)) => true,
        ("null", JsValue::Null) => true,
        ("object", JsValue::Object(_)) => true,
        ("array", JsValue::Array(_)) => true,
        _ => false,
    }
}

/// One JSON pointer segment, escaped.
fn pointer(path: &str, segment: &str) -> String {
    format!("{path}/{}", segment.replace('~', "~0").replace('/', "~1"))
}

/// Deep equality as Ajv's `enum` uses it.
fn equal(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Object(x), JsValue::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| equal(v, w)))
        }
        (JsValue::Array(x), JsValue::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| equal(v, w))
        }
        _ => a == b,
    }
}

impl<'a> Validator<'a> {
    fn report(&mut self, error: SchemaError) -> Result<(), Stop> {
        self.errors.push(error);
        if self.composite == 0 {
            Err(Stop)
        } else {
            Ok(())
        }
    }

    fn error(&mut self, path: &str, message: impl Into<String>) -> Result<(), Stop> {
        self.report(SchemaError {
            instance_path: path.to_owned(),
            message: message.into(),
            additional_property: None,
            allowed_values: None,
        })
    }

    fn find_id(&self, id: &str) -> Option<&'a JsValue> {
        fn walk<'v>(node: &'v JsValue, id: &str) -> Option<&'v JsValue> {
            match node {
                JsValue::Object(o) => {
                    if o.get("$id").and_then(JsValue::as_str) == Some(id) {
                        return Some(node);
                    }
                    o.iter().find_map(|(_, v)| walk(v, id))
                }
                JsValue::Array(a) => a.iter().find_map(|v| walk(v, id)),
                _ => None,
            }
        }
        walk(self.root, id)
    }

    /// Validates one subschema. Returns `Err(Stop)` once an error ends the validation; inside a composite rule,
    /// errors are added and it carries on as Ajv's generated code does.
    fn node(&mut self, schema: &JsValue, data: &JsValue, path: &str) -> Result<(), Stop> {
        let Some(schema) = schema.as_object() else {
            return Ok(()); // `true` or anything that is not a schema object
        };
        if let Some(reference) = schema.get("$ref").and_then(JsValue::as_str) {
            let target = self
                .find_id(reference)
                .unwrap_or_else(|| panic!("no schema with $id {reference}"));
            return self.node(target, data, path);
        }
        let errs_count = self.errors.len();
        let types: Vec<&str> = match schema.get("type") {
            Some(JsValue::String(t)) => vec![t.as_str()],
            Some(JsValue::Array(ts)) => ts.iter().filter_map(JsValue::as_str).collect(),
            _ => Vec::new(),
        };
        let group_for = |t: &str| -> Option<&[&str]> {
            match t {
                "number" => Some(NUMBER_KEYWORDS),
                "string" => Some(STRING_KEYWORDS),
                "array" => Some(ARRAY_KEYWORDS),
                "object" => Some(OBJECT_KEYWORDS),
                _ => None,
            }
        };
        let type_error = |types: &[&str]| format!("must be {}", types.join(","));

        // The type is checked first unless the one type has keywords of its own group present.
        let check_types = !types.is_empty()
            && !(types.len() == 1 && group_for(types[0]).is_some_and(|g| has_any(schema, g)));
        if check_types && !types.iter().any(|t| type_matches(t, data)) {
            self.error(path, type_error(&types))?;
        }

        // Untyped keywords.
        if has_any(schema, UNTYPED_KEYWORDS) {
            if let Some(JsValue::Array(values)) = schema.get("enum")
                && !values.iter().any(|v| equal(v, data))
            {
                self.report(SchemaError {
                    instance_path: path.to_owned(),
                    message: "must be equal to one of the allowed values".into(),
                    additional_property: None,
                    allowed_values: Some(values.clone()),
                })?;
            }
            if let Some(JsValue::Array(branches)) = schema.get("oneOf") {
                self.one_of(branches, data, path)?;
            }
            if self.errors.len() > errs_count {
                return Ok(());
            }
        }

        for (group, keywords) in [
            ("number", NUMBER_KEYWORDS),
            ("string", STRING_KEYWORDS),
            ("array", ARRAY_KEYWORDS),
            ("object", OBJECT_KEYWORDS),
        ] {
            if !has_any(schema, keywords) {
                continue;
            }
            if type_matches(group, data) {
                match group {
                    "number" => self.number_keywords(schema, data, path)?,
                    "array" => self.array_keywords(schema, data, path)?,
                    "object" => self.object_keywords(schema, data, path)?,
                    _ => {}
                }
            } else if types.len() == 1 && types[0] == group && !check_types {
                self.error(path, type_error(&types))?;
            }
            if self.errors.len() > errs_count {
                return Ok(());
            }
        }
        Ok(())
    }

    fn one_of(&mut self, branches: &[JsValue], data: &JsValue, path: &str) -> Result<(), Stop> {
        let before = self.errors.len();
        let mut passing = 0;
        self.composite += 1;
        for branch in branches {
            let count = self.errors.len();
            let _ = self.node(branch, data, path);
            if self.errors.len() == count {
                passing += 1;
            }
        }
        self.composite -= 1;
        if passing == 1 {
            self.errors.truncate(before);
            return Ok(());
        }
        self.error(path, "must match exactly one schema in oneOf")
    }

    fn number_keywords(
        &mut self,
        schema: &JsObject,
        data: &JsValue,
        path: &str,
    ) -> Result<(), Stop> {
        let n = data.as_f64().unwrap();
        for (keyword, op, ok) in [
            ("maximum", "<=", (|n, l| n <= l) as fn(f64, f64) -> bool),
            ("minimum", ">=", |n, l| n >= l),
            ("exclusiveMaximum", "<", |n, l| n < l),
            ("exclusiveMinimum", ">", |n, l| n > l),
        ] {
            if let Some(limit) = schema.get(keyword).and_then(JsValue::as_f64)
                && !ok(n, limit)
            {
                self.error(
                    path,
                    format!("must be {op} {}", js::number_to_string(limit)),
                )?;
            }
        }
        Ok(())
    }

    fn array_keywords(
        &mut self,
        schema: &JsObject,
        data: &JsValue,
        path: &str,
    ) -> Result<(), Stop> {
        let items = data.as_array().unwrap();
        if let Some(item_schema) = schema.get("items")
            && item_schema.is_plain_object()
        {
            for (i, item) in items.iter().enumerate() {
                let count = self.errors.len();
                self.node(item_schema, item, &pointer(path, &i.to_string()))?;
                if self.errors.len() > count {
                    break;
                }
            }
        }
        Ok(())
    }

    fn object_keywords(
        &mut self,
        schema: &JsObject,
        data: &JsValue,
        path: &str,
    ) -> Result<(), Stop> {
        let object = data.as_object().unwrap();
        if let Some(JsValue::Array(required)) = schema.get("required")
            && let Some(missing) = required
                .iter()
                .filter_map(JsValue::as_str)
                .find(|p| !object.contains_key(p))
        {
            // The rest of the object group is in the `else` of this check.
            return self.error(path, format!("must have required property '{missing}'"));
        }
        let properties = schema.get("properties").and_then(JsValue::as_object);
        if let Some(additional) = schema.get("additionalProperties") {
            let count = self.errors.len();
            for key in object.keys() {
                if properties.is_some_and(|p| p.contains_key(key)) {
                    continue;
                }
                match additional {
                    JsValue::Bool(false) => {
                        self.report(SchemaError {
                            instance_path: path.to_owned(),
                            message: "must NOT have additional properties".into(),
                            additional_property: Some(key.clone()),
                            allowed_values: None,
                        })?;
                        break;
                    }
                    JsValue::Object(_) => {
                        let before = self.errors.len();
                        self.node(additional, object.get(key).unwrap(), &pointer(path, key))?;
                        if self.errors.len() > before {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if self.errors.len() > count {
                return Ok(());
            }
        }
        if let Some(properties) = properties {
            for (key, property_schema) in properties.iter() {
                let Some(value) = object.get(key) else {
                    continue;
                };
                let count = self.errors.len();
                self.node(property_schema, value, &pointer(path, key))?;
                if self.errors.len() > count {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

/// The keywords [`validate`] implements. Any other keyword in a schema would be ignored silently, so a test fails
/// when the embedded schemas use one.
pub const IMPLEMENTED_KEYWORDS: &[&str] = &[
    "type",
    "enum",
    "oneOf",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "items",
    "required",
    "additionalProperties",
    "properties",
    "$ref",
];

/// Keywords that only annotate, which Ajv (without `useDefaults`) does not act on either.
pub const ANNOTATION_KEYWORDS: &[&str] = &["$schema", "$id", "title", "description", "default"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schemas_parse() {
        assert!(project_config_schema().get("properties").is_some());
        assert!(build_info_schema().get("required").is_some());
    }

    /// Every keyword the embedded schemas use is one `validate` implements or an annotation.
    #[test]
    fn the_schemas_use_only_implemented_keywords() {
        fn walk(node: &JsValue, at: &str, unknown: &mut Vec<String>) {
            let Some(object) = node.as_object() else {
                return;
            };
            for (key, value) in object.iter() {
                if !IMPLEMENTED_KEYWORDS.contains(&key.as_str())
                    && !ANNOTATION_KEYWORDS.contains(&key.as_str())
                {
                    unknown.push(format!("{at}/{key}"));
                }
                match key.as_str() {
                    "properties" => {
                        for (name, schema) in value.as_object().into_iter().flat_map(JsObject::iter)
                        {
                            walk(schema, &format!("{at}/properties/{name}"), unknown);
                        }
                    }
                    "oneOf" => {
                        for (i, schema) in value.as_array().into_iter().flatten().enumerate() {
                            walk(schema, &format!("{at}/oneOf/{i}"), unknown);
                        }
                    }
                    "items" | "additionalProperties" => {
                        walk(value, &format!("{at}/{key}"), unknown)
                    }
                    _ => {}
                }
            }
        }
        let mut unknown = Vec::new();
        walk(
            project_config_schema(),
            "flamework.config.schema.json#",
            &mut unknown,
        );
        walk(flamework_schema(), "flamework-schema.json#", &mut unknown);
        assert!(
            unknown.is_empty(),
            "keywords validate does not implement: {unknown:?}"
        );
    }

    /// The embedded schemas are the transformer's files (line endings aside). Skipped, with a note, when the monorepo
    /// is not at `FLAMEWORK_MONOREPO` (default `E:/Projects/TS/Flamework/ExperimentalV2Monorepo`).
    #[test]
    fn the_embedded_schemas_match_the_transformers() {
        let monorepo = std::env::var("FLAMEWORK_MONOREPO")
            .unwrap_or_else(|_| "E:/Projects/TS/Flamework/ExperimentalV2Monorepo".to_owned());
        let transformer = std::path::Path::new(&monorepo).join("packages/transformer");
        for (name, embedded) in [
            ("flamework.config.schema.json", PROJECT_CONFIG_SCHEMA_TEXT),
            ("flamework-schema.json", FLAMEWORK_SCHEMA_TEXT),
        ] {
            let Ok(text) = std::fs::read_to_string(transformer.join(name)) else {
                println!("[schema] {name}: no transformer at {monorepo}, drift not checked");
                continue;
            };
            assert_eq!(
                embedded.replace("\r\n", "\n"),
                text.replace("\r\n", "\n"),
                "schema/{name} differs from the transformer's; copy it again"
            );
        }
    }
}
