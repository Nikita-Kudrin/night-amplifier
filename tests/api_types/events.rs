//! `api.events.ts`: an interface per `/ws/events` event, their union `ServerEvent`, and
//! `SERVER_EVENTS`, the topic each states and how a sync restates it, which the frontend's
//! guard tests hold its handlers to (`useEventStream.catalogue.test.ts`). The topic is read off
//! `ServerEvent::examples()`, one per variant: the schema lists every variant on its own, so one
//! without a sample fails here.

use std::collections::BTreeSet;

use night_amplifier::session::events::{ServerEvent, Topic, TopicKind};
use schemars::generate::SchemaSettings;
use serde_json::{json, Map, Value};

use super::{declaration, doc_comment, js_name, members, snake, strings, type_definitions};

const HEADER: &str = "\
/**
 * The events `/ws/events` sends: a type of each, and the topic each states, generated
 * from the Rust `ServerEvent` by `tests/api_types.rs`. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */
";

fn pascal(snake: &str) -> String {
    snake
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

/// The definitions `schema` names, by Rust name.
fn references(schema: &Value, out: &mut BTreeSet<String>) {
    match schema {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                out.insert(reference.rsplit('/').next().unwrap().to_owned());
            }
            map.values().for_each(|value| references(value, out));
        }
        Value::Array(items) => items.iter().for_each(|value| references(value, out)),
        _ => {}
    }
}

/// A variant's fields as the client sees one object: its own, a newtype's struct's, and
/// every branch of a flattened enum, required only where every branch requires it.
fn fields(schema: &Value, definitions: &Map<String, Value>) -> (Map<String, Value>, BTreeSet<String>) {
    let mut properties = schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut required = strings(schema.get("required"));
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let (more, more_required) = fields(&definitions[reference.rsplit('/').next().unwrap()], definitions);
        properties.extend(more);
        required.extend(more_required);
    }
    let Some(branches) = schema.get("oneOf").and_then(Value::as_array) else {
        return (properties, required);
    };
    let branches: Vec<_> = branches.iter().map(|branch| fields(branch, definitions)).collect();
    let everywhere = branches.iter().map(|(_, r)| r.clone()).reduce(|a, b| &a & &b).unwrap_or_default();
    for (branch, _) in branches {
        for (name, property) in branch {
            match properties.get_mut(&name) {
                None => {
                    properties.insert(name, property);
                }
                Some(existing) if *existing == property => {}
                Some(existing) => match existing.get_mut("anyOf").and_then(Value::as_array_mut) {
                    Some(union) => union.push(property),
                    None => *existing = json!({ "anyOf": [existing.clone(), property] }),
                },
            }
        }
    }
    required.extend(everywhere);
    (properties, required)
}

/// `type` first, the rest in declaration order.
fn type_first(mut properties: Map<String, Value>) -> Map<String, Value> {
    let mut ordered = Map::new();
    ordered.insert("type".into(), properties.shift_remove("type").expect("a tagged variant"));
    ordered.extend(properties);
    ordered
}

/// The variant's schema as one flat object, `type` first.
fn flat_variant(variant: &Value, definitions: &Map<String, Value>) -> Value {
    let (properties, required) = fields(variant, definitions);
    let ordered = type_first(properties);
    let mut flat = json!({ "type": "object", "properties": ordered, "required": required });
    if let Some(description) = variant.get("description") {
        flat["description"] = description.clone();
    }
    flat
}

/// The branches of the enum a variant flattens (`AstapInstallStatus`'s `InstallStatus`), if any.
fn branches(variant: &Value) -> Option<&Vec<Value>> {
    variant.get("oneOf").and_then(Value::as_array)
}

/// A variant that flattens an enum, as its own fields and a union of the enum's branches, so a
/// field only some branches carry (`error` of `failed`) is typed by the branch that sends it.
fn union_declaration(name: &str, variant: &Value, definitions: &Map<String, Value>, roots: &[super::Root]) -> String {
    let mut common = variant.clone();
    let branches = common.as_object_mut().unwrap().remove("oneOf").expect("a flattened enum");
    let (properties, required) = fields(&common, definitions);
    let mut out = doc_comment(variant, "");
    out.push_str(&format!("export type {name} = {{\n"));
    out.push_str(&members(&type_first(properties), &required, roots, "    "));
    out.push_str("} & (\n");
    for branch in branches.as_array().unwrap() {
        let (properties, required) = fields(branch, definitions);
        // The discriminant first: `state: 'failed'`, then what that state carries.
        let (mut properties, rest): (Map<String, Value>, Map<String, Value>) =
            properties.into_iter().partition(|(_, property)| property.get("const").is_some());
        properties.extend(rest);
        out.push_str(&format!("    | {{\n{}    }}\n", members(&properties, &required, roots, "        ")));
    }
    out.push_str(")\n");
    out
}

fn tag_of(variant: &Value) -> String {
    variant["properties"]["type"]["const"].as_str().expect("a tagged variant").to_owned()
}

/// The topic, kind and wire form of each example, by its `type`.
fn classified() -> Map<String, Value> {
    ServerEvent::examples()
        .iter()
        .map(|event| {
            let wire = serde_json::to_value(event).unwrap();
            let (topic, kind) = match event.topic() {
                None => (Value::Null, "notice"),
                Some(topic) => (json!(snake(&format!("{topic:?}"))), kind_name(topic.kind())),
            };
            (wire["type"].as_str().unwrap().to_owned(), json!({ "topic": topic, "kind": kind, "wire": wire }))
        })
        .collect()
}

fn kind_name(kind: TopicKind) -> &'static str {
    match kind {
        TopicKind::Value => "value",
        TopicKind::Refetch => "refetch",
    }
}

/// Fails an example its variant's typedef does not describe, so neither drifts from the wire.
fn assert_described(tag: &str, wire: &Value, flat: &Value) {
    let keys: BTreeSet<String> = wire.as_object().unwrap().keys().cloned().collect();
    let known: BTreeSet<String> = flat["properties"].as_object().unwrap().keys().cloned().collect();
    let required = strings(flat.get("required"));
    assert!(keys.is_subset(&known), "{tag}: sends {:?}, not in its schema", &keys - &known);
    assert!(required.is_subset(&keys), "{tag}: lacks required {:?}", &required - &keys);
}

/// `ServerEvent`'s definitions by Rust name, its own (whose `oneOf` is the variants) among them.
fn event_definitions() -> Map<String, Value> {
    let mut generator = SchemaSettings::draft2020_12().for_serialize().into_generator();
    generator.subschema_for::<ServerEvent>();
    generator.take_definitions(true)
}

pub fn generate() -> String {
    let (roots, type_definitions) = type_definitions();
    let definitions = event_definitions();
    let variants = definitions["ServerEvent"]["oneOf"].as_array().expect("a tagged enum");

    let examples = classified();
    let tags: BTreeSet<String> = variants.iter().map(tag_of).collect();
    let sampled: BTreeSet<String> = examples.keys().cloned().collect();
    assert!(
        tags == sampled && examples.len() == ServerEvent::examples().len(),
        "ServerEvent::examples() (crates/session/src/events/examples.rs) needs exactly one of each \
         variant; missing {:?}, not variants {:?}",
        &tags - &sampled,
        &sampled - &tags,
    );

    let mut body = String::new();
    let mut names = Vec::new();
    let mut named = BTreeSet::new();
    for variant in variants {
        let tag = tag_of(variant);
        let flat = flat_variant(variant, &definitions);
        assert_described(&tag, &examples[&tag]["wire"], &flat);
        references(&flat, &mut named);
        let name = format!("{}Event", pascal(&tag));
        body.push('\n');
        body.push_str(&match branches(variant) {
            Some(_) => union_declaration(&name, variant, &definitions, &roots),
            None => declaration(&name, &flat, &roots),
        });
        names.push(name);
    }
    body.push_str("\n/** Any event `/ws/events` sends. */\nexport type ServerEvent =\n");
    for name in &names {
        body.push_str(&format!("    | {name}\n"));
    }
    body.push_str(
        "\nexport type ServerEventType = ServerEvent['type']\n\n\
         /** The event `/ws/events` sends as `type`. */\n\
         export type EventOf<K extends ServerEventType> = Extract<ServerEvent, {type: K}>\n",
    );

    // Types `api.types.ts` holds already are imported from it rather than written twice.
    let mut nested: Vec<&String> = definitions.keys().filter(|key| *key != "ServerEvent").collect();
    nested.sort_by_key(|key| js_name(key, &roots));
    for key in nested.into_iter().filter(|key| !type_definitions.contains_key(*key)) {
        references(&definitions[key], &mut named);
        body.push('\n');
        body.push_str(&declaration(&js_name(key, &roots), &definitions[key], &roots));
    }
    let mut imported: Vec<String> = named
        .iter()
        .filter(|key| type_definitions.contains_key(*key))
        .map(|key| js_name(key, &roots))
        .collect();
    imported.sort();

    let mut out = String::from(HEADER);
    let imported = imported.join(",\n    ");
    out.push_str(&format!("\nimport type {{\n    {imported},\n}} from './api.types'\n"));
    out.push_str(&body);
    out.push_str(
        "\n/**\n \
         * Every event by `type`, in declaration order: the topic it states or announces, and how a\n \
         * sync restates that topic (`TOPICS`); `notice` for a one-off a lost copy of goes without.\n \
         */\n\
         export const SERVER_EVENTS = {\n",
    );
    for variant in variants {
        let tag = tag_of(variant);
        let entry = &examples[&tag];
        let topic = entry["topic"].as_str().map_or("null".to_owned(), |topic| format!("'{topic}'"));
        let kind = entry["kind"].as_str().unwrap();
        out.push_str(&format!("    {tag}: {{topic: {topic}, kind: '{kind}'}},\n"));
    }
    out.push_str(
        "} as const satisfies \
         Record<ServerEventType, {topic: Topic | null; kind: 'value' | 'refetch' | 'notice'}>\n\n\
         /**\n \
         * How a sync restates each topic: `value` by the events of the snapshot before `synced`,\n \
         * `refetch` by the client reading its REST endpoint again on `synced`.\n \
         */\n\
         export const TOPICS = {\n",
    );
    for &topic in Topic::ALL {
        out.push_str(&format!("    {}: '{}',\n", snake(&format!("{topic:?}")), kind_name(topic.kind())));
    }
    out.push_str("} as const satisfies Record<string, 'value' | 'refetch'>\n\nexport type Topic = keyof typeof TOPICS\n");
    out
}

const EXAMPLES_HEADER: &str = "\
/**
 * One event of each type, generated from `ServerEvent::examples()` by `tests/api_types.rs`, for
 * tests only. Do not edit: change the Rust, then run
 *   UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types
 */
";

/// Whether `schema` lets a value be `null`.
fn admits_null(schema: &Value, definitions: &Map<String, Value>) -> bool {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return admits_null(&definitions[reference.rsplit('/').next().unwrap()], definitions);
    }
    let null_type = match schema.get("type") {
        Some(Value::String(t)) => t == "null",
        Some(Value::Array(types)) => types.iter().any(|t| t == "null"),
        _ => false,
    };
    null_type
        || schema.get("const").is_some_and(Value::is_null)
        || ["anyOf", "oneOf"].iter().any(|union| {
            schema.get(union).and_then(Value::as_array).is_some_and(|v| v.iter().any(|s| admits_null(s, definitions)))
        })
}

/// The branch of a flattened enum `wire` was sent as: the one whose constants it carries.
fn branch_of(variant: &Value, wire: &Value, definitions: &Map<String, Value>) -> Option<(Map<String, Value>, BTreeSet<String>)> {
    branches(variant)?.iter().map(|branch| fields(branch, definitions)).find(|(properties, _)| {
        properties.iter().all(|(name, property)| property.get("const").is_none_or(|c| wire.get(name) == Some(c)))
    })
}

/// `wire` cut to the fields its event (and branch) requires, a nullable one `null` and a list
/// empty: a complete event that says nothing a test did not ask for.
fn minimal(variant: &Value, wire: &Value, definitions: &Map<String, Value>) -> Value {
    let (mut properties, mut required) = fields(variant, definitions);
    if let Some((branch, branch_required)) = branch_of(variant, wire, definitions) {
        properties.extend(branch);
        required.extend(branch_required);
    }
    let mut out = Map::new();
    for (name, value) in wire.as_object().unwrap() {
        if !required.contains(name) {
            continue;
        }
        let neutral = if name != "type" && admits_null(&properties[name], definitions) {
            Value::Null
        } else if value.is_array() {
            json!([])
        } else {
            value.clone()
        };
        out.insert(name.clone(), neutral);
    }
    Value::Object(out)
}

/// `web/src/test/api.examples.ts`: what the frontend's `serverEvent(type, fields)` starts from.
pub fn generate_examples() -> String {
    let definitions = event_definitions();
    let variants = definitions["ServerEvent"]["oneOf"].as_array().expect("a tagged enum");
    let examples = classified();
    let mut out = String::from(EXAMPLES_HEADER);
    out.push_str(
        "\nimport type {EventOf, ServerEventType} from '../composables/api.events'\n\n\
         /**\n \
         * Each event with only the fields it requires, a nullable one `null`: complete, yet saying\n \
         * nothing a test did not ask for.\n \
         */\n\
         export const SERVER_EVENT_EXAMPLES: {readonly [K in ServerEventType]: EventOf<K>} = {\n",
    );
    for variant in variants {
        let tag = tag_of(variant);
        let event = minimal(variant, &examples[&tag]["wire"], &definitions);
        out.push_str(&format!("    {tag}: {},\n", serde_json::to_string(&event).unwrap()));
    }
    out.push_str("}\n");
    out
}

#[test]
fn a_minimal_example_keeps_required_fields_and_nulls_the_nullable_ones() {
    let variant = json!({
        "properties": {
            "type": { "const": "thing" },
            "name": { "type": "string" },
            "reason": { "type": ["string", "null"] },
            "note": { "type": "string" },
            "items": { "type": "array" },
        },
        "required": ["type", "name", "reason", "items"],
        "oneOf": [
            { "properties": { "state": { "const": "idle" } }, "required": ["state"] },
            { "properties": { "state": { "const": "failed" }, "error": { "type": "string" } }, "required": ["state", "error"] },
        ],
    });
    let wire = json!({ "type": "thing", "name": "A", "reason": "why", "note": "n", "items": [1], "state": "failed", "error": "e" });
    assert_eq!(
        minimal(&variant, &wire, &Map::new()),
        json!({ "type": "thing", "name": "A", "reason": null, "items": [], "state": "failed", "error": "e" })
    );
}

#[test]
fn a_flattened_enum_is_a_union_of_its_branches_discriminant_first() {
    let variant = json!({
        "description": "Where it stands.",
        "properties": { "type": { "const": "thing_status" } },
        "required": ["type"],
        "oneOf": [
            { "properties": { "state": { "const": "idle" } }, "required": ["state"] },
            { "properties": { "error": { "type": "string" }, "state": { "const": "failed" } }, "required": ["state", "error"] },
        ],
    });
    assert_eq!(
        union_declaration("ThingStatusEvent", &variant, &Map::new(), &[]),
        "/** Where it stands. */\nexport type ThingStatusEvent = {\n    type: 'thing_status'\n} & (\n    | {\n        \
         state: 'idle'\n    }\n    | {\n        state: 'failed'\n        error: string\n    }\n)\n"
    );
}

#[test]
fn names_convert_between_the_wire_and_js() {
    assert_eq!(pascal("push_direction_updated"), "PushDirectionUpdated");
    assert_eq!(snake("PushToDirection"), "push_to_direction");
}
