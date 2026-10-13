//! The frontend's `api.types.js`, written from the Rust wire types so the two cannot drift
//! (the hand-kept file once described 8 of ~60 settings and push directions by field
//! names the server never sent), and `api.settings.js`: the default settings and the
//! ranges the server clamps them to, read off the code that enforces them.
//!
//! `cargo test --features api-schema --test api_types` fails when either file is stale;
//! `UPDATE_API_TYPES=1` on the same command rewrites them. Run this binary alone: the feature
//! turns on `serde_json/preserve_order` (declaration order in the output) for the build.
#![cfg(feature = "api-schema")]

use night_amplifier::push_to::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, DatabaseTypeResponse,
    PushToStatusResponse,
};
use night_amplifier::server::{
    CapabilitiesResponse, CaptureStatusResponse, SettingsResponse, SimulatorConfigResponse,
};
use night_amplifier::session::capture::BLACK_FLOOR_RANGE;
use night_amplifier::session::events::{CameraInfoResponse, CameraListEntry};
use night_amplifier::session::state::CaptureSettings;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::JsonSchema;
use serde_json::{json, Map, Value};
use std::ops::RangeInclusive;

const TYPES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/composables/api.types.js");
const SETTINGS_FILE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/composables/api.settings.js");

const HEADER: &str = "\
/**
 * JSDoc types of the server's JSON answers, generated from the Rust wire types by
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

/// The answers `api.js` documents, by the name it uses for each.
fn roots(generator: &mut SchemaGenerator) -> Vec<(&'static str, Value)> {
    fn root<T: JsonSchema>(generator: &mut SchemaGenerator, name: &'static str) -> (&'static str, Value) {
        let reference = generator.subschema_for::<T>().to_value();
        let key = reference["$ref"].as_str().expect("a named type").rsplit('/').next().unwrap();
        (name, Value::String(key.to_string()))
    }
    vec![
        root::<SettingsResponse>(generator, "Settings"),
        root::<CapabilitiesResponse>(generator, "Capabilities"),
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
         /** @type {{import('./api.types.js').Settings}} */\n\
         export const SERVER_DEFAULT_SETTINGS = {}\n\n\
         /**\n \
         * The range the server clamps each ranged setting to, by its dotted path in `Settings`;\n \
         * `null` is an open end.\n \
         * @type {{Object<string, {{min: number|null, max: number|null}}>}}\n \
         */\n\
         export const SERVER_SETTING_RANGES = {}\n",
        defaults.unwrap(),
        serde_json::to_string_pretty(&ranges).unwrap(),
    )
}

/// Rewrites `path` under `UPDATE_API_TYPES`, else fails when it differs from `generated`.
fn assert_current(path: &str, generated: &str) {
    let name = path.rsplit('/').next().unwrap();
    if std::env::var_os("UPDATE_API_TYPES").is_some() {
        std::fs::write(path, generated).unwrap_or_else(|e| panic!("write {name}: {e}"));
        return;
    }
    let committed = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    assert!(
        committed == generated,
        "web/src/composables/{name} is stale; regenerate it with\n  \
         UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types"
    );
}

#[test]
fn api_types_js_is_current() {
    assert_current(TYPES_FILE, &generate());
}

#[test]
fn api_settings_js_is_current() {
    assert_current(SETTINGS_FILE, &generate_settings());
}
