//! The frontend's `api.types.js`, written from the Rust wire types so the two cannot drift
//! (the hand-kept file once described 8 of ~60 settings and push directions by field
//! names the server never sent).
//!
//! `cargo test --features api-schema --test api_types` fails when the file is stale;
//! `UPDATE_API_TYPES=1` on the same command rewrites it. Run this binary alone: the feature
//! turns on `serde_json/preserve_order` (declaration order in the output) for the build.
#![cfg(feature = "api-schema")]

use night_amplifier::push_to::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, DatabaseTypeResponse,
    PushToStatusResponse,
};
use night_amplifier::server::{CaptureStatusResponse, SettingsResponse, SimulatorConfigResponse};
use night_amplifier::session::events::{CameraInfoResponse, CameraListEntry};
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::JsonSchema;
use serde_json::{Map, Value};

const TYPES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/composables/api.types.js");

const HEADER: &str = "\
/**
 * JSDoc types of the server's JSON answers, generated from the Rust wire types by
 * `tests/api_types.rs`. Do not edit: change the Rust type, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */
";

/// The answers `api.js` documents, by the name it uses for each.
fn roots(generator: &mut SchemaGenerator) -> Vec<(&'static str, Value)> {
    fn root<T: JsonSchema>(generator: &mut SchemaGenerator, name: &'static str) -> (&'static str, Value) {
        let reference = generator.subschema_for::<T>().to_value();
        let key = reference["$ref"].as_str().expect("a named type").rsplit('/').next().unwrap();
        (name, Value::String(key.to_string()))
    }
    vec![
        root::<SettingsResponse>(generator, "Settings"),
        root::<CaptureStatusResponse>(generator, "CaptureStatus"),
        root::<CameraListEntry>(generator, "Camera"),
        root::<CameraInfoResponse>(generator, "CameraInfo"),
        root::<SimulatorConfigResponse>(generator, "SimulatorConfig"),
        root::<PushToStatusResponse>(generator, "PushToStatus"),
        root::<CatalogEntryResponse>(generator, "CatalogEntry"),
        root::<AstapStatusResponse>(generator, "AstapStatus"),
        root::<DatabaseTypeResponse>(generator, "DatabaseType"),
        root::<CatalogStatusResponse>(generator, "CatalogStatus"),
    ]
}

/// A definition's JSDoc name: the root's name, else the Rust name less `Response`/`Dto`.
fn js_name(rust: &str, roots: &[(&str, Value)]) -> String {
    if let Some((name, _)) = roots.iter().find(|(_, key)| key == rust) {
        return name.to_string();
    }
    rust.trim_end_matches("Response").trim_end_matches("Dto").to_string()
}

/// The first paragraph of a doc comment, on one line.
fn summary(schema: &Value) -> Option<String> {
    let text = schema.get("description")?.as_str()?;
    let first = text.split("\n\n").next()?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!first.is_empty()).then_some(first)
}

fn type_expr(schema: &Value, roots: &[(&str, Value)]) -> String {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return js_name(reference.rsplit('/').next().unwrap(), roots);
    }
    if let Some(constant) = schema.get("const") {
        return constant.to_string().replace('"', "'");
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.iter().map(|v| v.to_string().replace('"', "'")).collect::<Vec<_>>().join("|");
    }
    for union in ["anyOf", "oneOf"] {
        if let Some(variants) = schema.get(union).and_then(Value::as_array) {
            let mut parts: Vec<String> = variants.iter().map(|v| type_expr(v, roots)).collect();
            parts.dedup();
            return parts.join("|");
        }
    }
    match schema.get("type") {
        Some(Value::Array(types)) => types
            .iter()
            .map(|t| {
                let mut single = schema.clone();
                single["type"] = t.clone();
                type_expr(&single, roots)
            })
            .collect::<Vec<_>>()
            .join("|"),
        Some(Value::String(t)) => match t.as_str() {
            "integer" | "number" => "number".into(),
            "string" => "string".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => match schema.get("items") {
                Some(items) => format!("Array<{}>", type_expr(items, roots)),
                None => "Array<*>".into(),
            },
            "object" => match schema.get("additionalProperties") {
                Some(values) if values.is_object() => {
                    format!("Object<string, {}>", type_expr(values, roots))
                }
                _ => "object".into(),
            },
            other => panic!("unhandled JSON Schema type {other}"),
        },
        _ => "*".into(),
    }
}

fn typedef(name: &str, schema: &Value, roots: &[(&str, Value)]) -> String {
    let mut out = String::from("/**\n");
    if let Some(text) = summary(schema) {
        out.push_str(&format!(" * {text}\n"));
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        out.push_str(&format!(" * @typedef {{{}}} {name}\n */\n", type_expr(schema, roots)));
        return out;
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    out.push_str(&format!(" * @typedef {{object}} {name}\n"));
    for (field, property) in properties {
        let field = if required.contains(&field.as_str()) { field.clone() } else { format!("[{field}]") };
        let description = summary(property).map(|d| format!(" - {d}")).unwrap_or_default();
        out.push_str(&format!(" * @property {{{}}} {field}{description}\n", type_expr(property, roots)));
    }
    out.push_str(" */\n");
    out
}

fn generate() -> String {
    let mut generator = SchemaSettings::draft2020_12().for_serialize().into_generator();
    let roots = roots(&mut generator);
    let definitions: Map<String, Value> = generator.take_definitions(true);

    let mut order: Vec<&String> = roots
        .iter()
        .map(|(_, key)| definitions.get_key_value(key.as_str().unwrap()).unwrap().0)
        .collect();
    let mut nested: Vec<&String> = definitions.keys().filter(|k| !order.contains(k)).collect();
    nested.sort_by_key(|k| js_name(k, &roots));
    order.extend(nested);

    let mut out = String::from(HEADER);
    for key in order {
        out.push('\n');
        out.push_str(&typedef(&js_name(key, &roots), &definitions[key], &roots));
    }
    out
}

#[test]
fn api_types_js_is_current() {
    let generated = generate();
    if std::env::var_os("UPDATE_API_TYPES").is_some() {
        std::fs::write(TYPES_FILE, &generated).expect("write api.types.js");
        return;
    }
    let committed = std::fs::read_to_string(TYPES_FILE).expect("read api.types.js");
    assert!(
        committed == generated,
        "web/src/composables/api.types.js is stale; regenerate it with\n  \
         UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types"
    );
}
