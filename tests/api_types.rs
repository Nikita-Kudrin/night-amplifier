//! The frontend's `api.types.ts`, written from the Rust wire types so the two cannot drift
//! (the hand-kept file once described 8 of ~60 settings and push directions by field
//! names the server never sent), `api.settings.ts`: the default settings and the ranges
//! the server clamps them to, read off the code that enforces them, and `api.events.ts`
//! (`events`): every `/ws/events` event and the topic it states, and the tests' `api.examples.ts`:
//! one complete event of each type.
//!
//! `cargo test --features api-schema --test api_types` fails when any file is stale;
//! `UPDATE_API_TYPES=1` on the same command rewrites them. Run this binary alone: the feature
//! turns on `serde_json/preserve_order` (declaration order in the output) for the build.
#![cfg(feature = "api-schema")]

#[path = "api_types/events.rs"]
mod events;

use night_amplifier::license::LicenseStatus;
use night_amplifier::push_to::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, DatabaseTypeResponse,
};
use night_amplifier::render::denoise::AiComputeReport;
use night_amplifier::server::{
    AstapInstallRequest, CapabilitiesResponse, CaptureStatusResponse, CatalogInstallRequest,
    ConfigureSimulatorRequest, ConnectCameraRequest, DisconnectCameraRequest, DisconnectResponse,
    MessageResponse, PushToConfigRequest, SearchCatalogRequest, SetTargetRequest, SettingsResponse,
    SimulatorConfigResponse, SoftwareLicensesResponse, StartCaptureRequest, StopCaptureRequest,
    UpdateLicenseRequest, ViewedCameraBody,
};
use night_amplifier::session::capture::BLACK_FLOOR_RANGE;
use night_amplifier::session::events::{CameraInfoResponse, CameraListEntry};
use night_amplifier::session::services::{PushToStatus, UpdateSettingsRequest};
use night_amplifier::session::state::{CaptureSettings, StackingTypeInfo};
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::JsonSchema;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::ops::RangeInclusive;

const TYPES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/composables/api.types.ts");
const SETTINGS_FILE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/composables/api.settings.ts");
const EVENTS_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/composables/api.events.ts");
const EXAMPLES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/test/api.examples.ts");

const HEADER: &str = "\
/**
 * TypeScript types of the server's JSON answers, generated from the Rust wire types by
 * `tests/api_types.rs`. Do not edit: change the Rust type, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */
";

const SETTINGS_HEADER: &str = "\
/**
 * The server's default settings and the ranges it clamps settings to, generated from the
 * Rust code that enforces them by `tests/api_types.rs`. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */
";

/// A root's TypeScript name and the Rust name of its definition.
type Root = (String, Value);

fn root<T: JsonSchema>(generator: &mut SchemaGenerator, name: &str) -> Root {
    let reference = generator.subschema_for::<T>().to_value();
    let key = reference["$ref"].as_str().expect("a named type").rsplit('/').next().unwrap();
    (name.to_string(), Value::String(key.to_string()))
}

/// The answers `api.ts` documents, by the name it uses for each.
fn roots(generator: &mut SchemaGenerator) -> Vec<Root> {
    vec![
        root::<SettingsResponse>(generator, "Settings"),
        root::<StackingTypeInfo>(generator, "StackingTypeInfo"),
        root::<CapabilitiesResponse>(generator, "Capabilities"),
        root::<CaptureStatusResponse>(generator, "CaptureStatus"),
        root::<CameraListEntry>(generator, "Camera"),
        root::<CameraInfoResponse>(generator, "CameraInfo"),
        root::<SimulatorConfigResponse>(generator, "SimulatorConfig"),
        root::<PushToStatus>(generator, "PushToStatus"),
        root::<CatalogEntryResponse>(generator, "CatalogEntry"),
        root::<AstapStatusResponse>(generator, "AstapStatus"),
        root::<DatabaseTypeResponse>(generator, "DatabaseType"),
        root::<CatalogStatusResponse>(generator, "CatalogStatus"),
        root::<AiComputeReport>(generator, "AiComputeReport"),
        root::<LicenseStatus>(generator, "LicenseStatus"),
        root::<SoftwareLicensesResponse>(generator, "SoftwareLicenses"),
        root::<DisconnectResponse>(generator, "Disconnection"),
        root::<MessageResponse>(generator, "Message"),
        root::<ViewedCameraBody>(generator, "ViewedCamera"),
    ]
}

/// The bodies (and the one query) `api.ts` sends, as the server reads them.
fn request_roots(generator: &mut SchemaGenerator) -> Vec<Root> {
    vec![
        root::<UpdateSettingsRequest>(generator, "UpdateSettingsRequest"),
        root::<StartCaptureRequest>(generator, "StartCaptureRequest"),
        root::<StopCaptureRequest>(generator, "StopCaptureRequest"),
        root::<ConnectCameraRequest>(generator, "ConnectCameraRequest"),
        root::<DisconnectCameraRequest>(generator, "DisconnectCameraRequest"),
        root::<ViewedCameraBody>(generator, "ViewedCameraRequest"),
        root::<ConfigureSimulatorRequest>(generator, "ConfigureSimulatorRequest"),
        root::<SetTargetRequest>(generator, "SetTargetRequest"),
        root::<SearchCatalogRequest>(generator, "SearchCatalogRequest"),
        root::<PushToConfigRequest>(generator, "PushToConfigRequest"),
        root::<AstapInstallRequest>(generator, "AstapInstallRequest"),
        root::<CatalogInstallRequest>(generator, "CatalogInstallRequest"),
        root::<UpdateLicenseRequest>(generator, "UpdateLicenseRequest"),
    ]
}

/// A definition's TypeScript name: the root's name, else the Rust name less `Response`/`Dto`.
fn js_name(rust: &str, roots: &[Root]) -> String {
    if let Some((name, _)) = roots.iter().find(|(_, key)| key == rust) {
        return name.to_string();
    }
    rust.trim_end_matches("Response").trim_end_matches("Dto").to_string()
}

fn snake(pascal: &str) -> String {
    let mut out = String::new();
    for c in pascal.chars() {
        if c.is_uppercase() && !out.is_empty() {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// A doc comment at `indent`, every paragraph and line of the Rust doc kept as written.
fn doc_comment(schema: &Value, indent: &str) -> String {
    let Some(text) = schema.get("description").and_then(Value::as_str).map(str::trim) else {
        return String::new();
    };
    if text.is_empty() {
        return String::new();
    }
    let text = text.replace("*/", "*\\/");
    if !text.contains('\n') {
        return format!("{indent}/** {text} */\n");
    }
    let mut out = format!("{indent}/**\n");
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str(&format!("{indent} *\n"));
        } else {
            out.push_str(&format!("{indent} * {line}\n"));
        }
    }
    out.push_str(&format!("{indent} */\n"));
    out
}

fn string_literal(value: &Value) -> String {
    format!("'{}'", value.as_str().expect("a string literal").replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The values of a union of string literals, or `None` for any other type.
fn string_union(schema: &Value) -> Option<Vec<Value>> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.iter().all(Value::is_string).then(|| values.clone());
    }
    let variants = ["oneOf", "anyOf"].iter().find_map(|union| schema.get(union)?.as_array())?;
    let values: Vec<Value> = variants
        .iter()
        .map(|variant| match variant.get("const") {
            Some(constant) if constant.is_string() => Some(vec![constant.clone()]),
            _ => string_union(variant),
        })
        .collect::<Option<Vec<_>>>()?
        .concat();
    Some(values)
}

fn type_expr(schema: &Value, roots: &[Root]) -> String {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return js_name(reference.rsplit('/').next().unwrap(), roots);
    }
    if let Some(constant) = schema.get("const") {
        return if constant.is_string() { string_literal(constant) } else { constant.to_string() };
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(|v| if v.is_string() { string_literal(v) } else { v.to_string() })
            .collect::<Vec<_>>()
            .join(" | ");
    }
    for union in ["anyOf", "oneOf"] {
        if let Some(variants) = schema.get(union).and_then(Value::as_array) {
            let mut parts: Vec<String> = variants.iter().map(|v| type_expr(v, roots)).collect();
            parts.dedup();
            return parts.join(" | ");
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
            .join(" | "),
        Some(Value::String(t)) => match t.as_str() {
            "integer" | "number" => "number".into(),
            "string" => "string".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => match schema.get("items") {
                Some(items) => {
                    let item = type_expr(items, roots);
                    if item.contains(' ') { format!("({item})[]") } else { format!("{item}[]") }
                }
                None => "unknown[]".into(),
            },
            "object" => match schema.get("additionalProperties") {
                // A map holds only the keys it was sent: a lookup may miss.
                Some(values) if values.is_object() => {
                    format!("Partial<Record<string, {}>>", type_expr(values, roots))
                }
                _ => "Record<string, unknown>".into(),
            },
            other => panic!("unhandled JSON Schema type {other}"),
        },
        _ => "unknown".into(),
    }
}

fn property_key(field: &str) -> String {
    let mut chars = field.chars();
    let identifier = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if identifier { field.to_string() } else { string_literal(&Value::String(field.to_string())) }
}

/// An object's fields, one per line at `indent`, each with its doc comment.
fn members(properties: &Map<String, Value>, required: &BTreeSet<String>, roots: &[Root], indent: &str) -> String {
    let mut out = String::new();
    for (field, property) in properties {
        let optional = if required.contains(field) { "" } else { "?" };
        out.push_str(&doc_comment(property, indent));
        out.push_str(&format!("{indent}{}{optional}: {}\n", property_key(field), type_expr(property, roots)));
    }
    out
}

/// An `interface` for an object, a values list and the type derived from it for a string
/// union (so the list is exhaustive by construction), else a `type` alias.
fn declaration(name: &str, schema: &Value, roots: &[Root]) -> String {
    let mut out = doc_comment(schema, "");
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let required = strings(schema.get("required"));
        out.push_str(&format!("export interface {name} {{\n"));
        out.push_str(&members(properties, &required, roots, "    "));
        out.push_str("}\n");
        return out;
    }
    if let Some(values) = string_union(schema) {
        let constant = format!("{}_VALUES", snake(name).to_uppercase());
        let values: Vec<String> = values.iter().map(string_literal).collect();
        let line = format!("export const {constant} = [{}] as const\n", values.join(", "));
        if line.len() <= 110 {
            out.push_str(&line);
        } else {
            out.push_str(&format!("export const {constant} = [\n    {},\n] as const\n", values.join(",\n    ")));
        }
        out.push_str(&format!("export type {name} = (typeof {constant})[number]\n"));
        return out;
    }
    out.push_str(&format!("export type {name} = {}\n", type_expr(schema, roots)));
    out
}

fn strings(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// `api.types.ts`'s answers by Rust name, and the roots that name some of them.
fn type_definitions() -> (Vec<Root>, Map<String, Value>) {
    let mut generator = SchemaSettings::draft2020_12().for_serialize().into_generator();
    let roots = roots(&mut generator);
    (roots, generator.take_definitions(true))
}

/// The request bodies' definitions, read as the server deserializes them: a field with a
/// `#[serde(default)]` may be left out.
fn request_definitions() -> (Vec<Root>, Map<String, Value>) {
    let mut generator = SchemaSettings::draft2020_12().for_deserialize().into_generator();
    let roots = request_roots(&mut generator);
    (roots, generator.take_definitions(true))
}

fn generate() -> String {
    let (roots, definitions) = type_definitions();

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
        out.push_str(&declaration(&js_name(key, &roots), &definitions[key], &roots));
    }

    // What reads exactly as it is sent is written once, under its answer's name; a type the
    // server reads more leniently than it sends (defaults) is written again as `…Input`.
    let (request_roots, requests) = request_definitions();
    let differs = |key: &str| definitions.get(key) != Some(&requests[key]);
    let mut names: Vec<Root> =
        request_roots.iter().filter(|(_, key)| differs(key.as_str().unwrap())).cloned().collect();
    let mut nested: Vec<&String> = requests
        .keys()
        .filter(|key| differs(key) && !request_roots.iter().any(|(_, root)| root == *key))
        .collect();
    nested.sort_by_key(|key| js_name(key, &roots));
    for key in nested {
        let name = js_name(key, &roots);
        let name = if definitions.contains_key(key) { format!("{name}Input") } else { name };
        names.push((name, json!(key)));
    }
    let emitted: Vec<String> = names.iter().map(|(_, key)| key.as_str().unwrap().to_owned()).collect();
    names.extend(roots.iter().cloned());

    out.push_str("\n// Request bodies, as the server reads them: a field it defaults may be left out.\n");
    for key in emitted {
        out.push('\n');
        out.push_str(&declaration(&js_name(&key, &names), &requests[&key], &names));
    }
    out
}

/// A setting `CaptureSettings::sanitized` clamps: its dotted path in the `/api/settings`
/// answer, and how to set it from an `f64`.
type Probe = (&'static str, fn(&mut CaptureSettings, f64));

const CLAMPED: &[Probe] = &[
    ("rejection_sigma", |s, v| s.rejection_sigma = v as f32),
    ("auto_stretch_intensity", |s, v| s.auto_stretch_intensity = v as f32),
    ("saturation_boost_strength", |s, v| s.saturation_boost_strength = v as f32),
    ("simulated_preload_images", |s, v| s.simulated_preload_images = v as usize),
    ("target_temp_c", |s, v| s.target_temp_c = Some(v)),
    ("dew_heater_power", |s, v| s.dew_heater_power = v as i32),
    ("guide_camera.target_temp_c", |s, v| s.guide_camera.target_temp_c = Some(v)),
    ("guide_camera.dew_heater_power", |s, v| s.guide_camera.dew_heater_power = v as i32),
    ("sensor_correction.hot_pixel_sigma", |s, v| s.sensor_correction.hot_pixel_sigma = v as f32),
    ("denoise.chroma_strength", |s, v| s.denoise.chroma_strength = v as f32),
    ("denoise.background_grain", |s, v| s.denoise.background_grain = v as f32),
    ("denoise.luma_strength", |s, v| s.denoise.luma_strength = v as f32),
    ("denoise.detail", |s, v| s.denoise.detail = v as f32),
];

/// Settings as `/api/settings` sends them. Parsed back from the text, not `to_value`, which
/// widens an f32 to its f64 digits (`-0.04500000178813934` for `-0.045`).
fn wire(settings: &CaptureSettings) -> Value {
    let text = serde_json::to_string(&SettingsResponse::from(settings)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn at(wire: &Value, path: &str) -> Value {
    let value = path.split('.').fold(wire, |value, key| &value[key]);
    assert!(!value.is_null(), "{path} is not in the /api/settings answer");
    value.clone()
}

/// Where `sanitized` puts a value far past one end, or `null` when it lets it through.
fn clamp_end((path, set): &Probe, far: f64) -> Value {
    let mut settings = CaptureSettings::default();
    set(&mut settings, far);
    let kept = at(&wire(&settings.clone().sanitized()), path);
    if kept == at(&wire(&settings), path) { Value::Null } else { kept }
}

/// An f32 as the literal a human would write, not its f64 widening.
fn shortest(value: f32) -> f64 {
    value.to_string().parse().unwrap()
}

fn range(range: RangeInclusive<f32>) -> Value {
    json!({ "min": shortest(*range.start()), "max": shortest(*range.end()) })
}

fn generate_settings() -> String {
    // Without a Pro plugin `rejection_method` is Community's `best_available`.
    let defaults = serde_json::to_string_pretty(&SettingsResponse::from(&CaptureSettings::default()));
    let mut ranges = Map::new();
    for probe in CLAMPED {
        let (min, max) = (clamp_end(probe, -1e9), clamp_end(probe, 1e9));
        // Else a probe that stopped clamping (or sets the wrong field) silently drops its bound.
        assert!(
            !(min.is_null() && max.is_null()),
            "{}: `sanitized` clamps neither end; fix its probe or drop it from CLAMPED",
            probe.0
        );
        ranges.insert(probe.0.to_string(), json!({ "min": min, "max": max }));
    }
    // Clamped where the renderer reads it, not in `sanitized`.
    ranges.insert("eyepiece.black_floor".to_string(), range(BLACK_FLOOR_RANGE));
    format!(
        "{SETTINGS_HEADER}\n\
         import type {{Settings}} from './api.types'\n\n\
         export const SERVER_DEFAULT_SETTINGS = {} satisfies Settings\n\n\
         /**\n \
         * The range the server clamps each ranged setting to, by its dotted path in `Settings`;\n \
         * `null` is an open end.\n \
         */\n\
         export const SERVER_SETTING_RANGES = {} as const satisfies \
         Record<string, {{min: number | null; max: number | null}}>\n\n\
         /** A setting the server clamps, by its dotted path in `Settings`. */\n\
         export type ClampedPath = keyof typeof SERVER_SETTING_RANGES\n",
        defaults.unwrap(),
        serde_json::to_string_pretty(&ranges).unwrap(),
    )
}

/// Rewrites `path` under `UPDATE_API_TYPES`, else fails when it differs from `generated`.
fn assert_current(path: &str, generated: &str) {
    let name = path.strip_prefix(concat!(env!("CARGO_MANIFEST_DIR"), "/")).unwrap_or(path);
    if std::env::var_os("UPDATE_API_TYPES").is_some() {
        std::fs::write(path, generated).unwrap_or_else(|e| panic!("write {name}: {e}"));
        return;
    }
    let committed = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    assert!(
        committed == generated,
        "{name} is stale; regenerate it with\n  \
         UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types"
    );
}

#[test]
fn api_types_ts_is_current() {
    assert_current(TYPES_FILE, &generate());
}

#[test]
fn api_settings_ts_is_current() {
    assert_current(SETTINGS_FILE, &generate_settings());
}

#[test]
fn api_events_ts_is_current() {
    assert_current(EVENTS_FILE, &events::generate());
}

#[test]
fn api_examples_ts_is_current() {
    assert_current(EXAMPLES_FILE, &events::generate_examples());
}

#[test]
fn an_optional_field_and_a_nullable_one_stay_apart() {
    let schema = json!({
        "description": "A thing.",
        "properties": {
            "always": { "type": "string" },
            "maybe_null": { "type": ["number", "null"] },
            "left_out": { "type": "boolean" },
            "not-an-identifier": { "type": "integer" },
        },
        "required": ["always", "maybe_null", "not-an-identifier"],
    });
    assert_eq!(
        declaration("Thing", &schema, &[]),
        "/** A thing. */\nexport interface Thing {\n    always: string\n    maybe_null: number | null\n    \
         left_out?: boolean\n    'not-an-identifier': number\n}\n"
    );
}

#[test]
fn a_string_union_is_its_values_list() {
    let schema = json!({ "oneOf": [{ "const": "deep_sky", "description": "Stars" }, { "enum": ["comet"] }] });
    assert_eq!(
        declaration("StackingType", &schema, &[]),
        "export const STACKING_TYPE_VALUES = ['deep_sky', 'comet'] as const\n\
         export type StackingType = (typeof STACKING_TYPE_VALUES)[number]\n"
    );
}

#[test]
fn a_union_inside_an_array_or_a_map_keeps_its_meaning() {
    let union = json!({ "anyOf": [{ "$ref": "#/$defs/CatalogEntryResponse" }, { "type": "null" }] });
    let roots = [("CatalogEntry".to_string(), json!("CatalogEntryResponse"))];
    assert_eq!(type_expr(&json!({ "type": "array", "items": union }), &roots), "(CatalogEntry | null)[]");
    assert_eq!(
        type_expr(&json!({ "type": "object", "additionalProperties": union }), &roots),
        "Partial<Record<string, CatalogEntry | null>>"
    );
    assert_eq!(type_expr(&json!({}), &roots), "unknown");
}

#[test]
fn a_doc_comment_keeps_its_paragraphs_and_cannot_be_closed_early() {
    let schema = json!({ "description": "First line\nsecond line.\n\nSee `a/*/b` and */ here." });
    assert_eq!(
        doc_comment(&schema, "    "),
        "    /**\n     * First line\n     * second line.\n     *\n     * See `a/*\\/b` and *\\/ here.\n     */\n"
    );
}
