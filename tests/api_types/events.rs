//! `api.events.js`: a JSDoc type per `/ws/events` event and `SERVER_EVENTS`, the topic each
//! states and how a sync restates it, which the frontend's guard tests hold its handlers to
//! (`useEventStream.catalogue.test.js`). The topic is read off `ServerEvent::examples()`, one
//! per variant: the schema lists every variant on its own, so one without a sample fails here.

use std::collections::BTreeSet;

use night_amplifier::session::events::{ServerEvent, Topic, TopicKind};
use schemars::generate::SchemaSettings;
use serde_json::{json, Map, Value};

use super::{js_name, type_definitions, typedef};

const HEADER: &str = "\
/**
 * The events `/ws/events` sends: a JSDoc type of each, and the topic each states, generated
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

fn strings(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
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

/// The variant's schema as one flat object, `type` first.
fn flat_variant(variant: &Value, definitions: &Map<String, Value>) -> Value {
    let (mut properties, required) = fields(variant, definitions);
    let mut ordered = Map::new();
    ordered.insert("type".into(), properties.shift_remove("type").expect("a tagged variant"));
    ordered.extend(properties);
    let mut flat = json!({ "type": "object", "properties": ordered, "required": required });
    if let Some(description) = variant.get("description") {
        flat["description"] = description.clone();
    }
    flat
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

pub fn generate() -> String {
    let (roots, type_definitions) = type_definitions();
    let mut generator = SchemaSettings::draft2020_12().for_serialize().into_generator();
    generator.subschema_for::<ServerEvent>();
    let definitions = generator.take_definitions(true);
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

    let mut out = String::from(HEADER);
    let mut names = Vec::new();
    for variant in variants {
        let tag = tag_of(variant);
        let flat = flat_variant(variant, &definitions);
        assert_described(&tag, &examples[&tag]["wire"], &flat);
        let name = format!("{}Event", pascal(&tag));
        out.push('\n');
        out.push_str(&typedef(&name, &flat, &roots));
        names.push(name);
    }
    let union = names.join("|");
    out.push_str(&format!("\n/**\n * Any event `/ws/events` sends.\n * @typedef {{{union}}} ServerEvent\n */\n"));

    // Types `api.types.js` holds already are imported from it rather than written twice.
    let mut nested: Vec<&String> = definitions.keys().filter(|key| *key != "ServerEvent").collect();
    nested.sort_by_key(|key| js_name(key, &roots));
    for key in nested {
        let name = js_name(key, &roots);
        out.push('\n');
        if type_definitions.contains_key(key) {
            out.push_str(&format!("/** @typedef {{import('./api.types.js').{name}}} {name} */\n"));
        } else {
            out.push_str(&typedef(&name, &definitions[key], &roots));
        }
    }

    out.push_str(
        "\n/**\n \
         * Every event by `type`, in declaration order: the topic it states or announces, and how a\n \
         * sync restates that topic (`TOPICS`); `notice` for a one-off a lost copy of goes without.\n \
         * @type {Readonly<Object<string, Readonly<{topic: string|null, kind: 'value'|'refetch'|'notice'}>>>}\n \
         */\n\
         export const SERVER_EVENTS = Object.freeze({\n",
    );
    for variant in variants {
        let tag = tag_of(variant);
        let entry = &examples[&tag];
        let topic = entry["topic"].as_str().map_or("null".to_owned(), |topic| format!("'{topic}'"));
        let kind = entry["kind"].as_str().unwrap();
        out.push_str(&format!("    {tag}: Object.freeze({{topic: {topic}, kind: '{kind}'}}),\n"));
    }
    out.push_str(
        "})\n\n/**\n \
         * How a sync restates each topic: `value` by the events of the snapshot before `synced`,\n \
         * `refetch` by the client reading its REST endpoint again on `synced`.\n \
         * @type {Readonly<Object<string, 'value'|'refetch'>>}\n \
         */\n\
         export const TOPICS = Object.freeze({\n",
    );
    for &topic in Topic::ALL {
        out.push_str(&format!("    {}: '{}',\n", snake(&format!("{topic:?}")), kind_name(topic.kind())));
    }
    out.push_str("})\n");
    out
}

#[test]
fn names_convert_between_the_wire_and_js() {
    assert_eq!(pascal("push_direction_updated"), "PushDirectionUpdated");
    assert_eq!(snake("PushToDirection"), "push_to_direction");
}
