//! Generic CLI flags/positionals -> command input JSON. (Task 8)
//!
//! The mapping is schema-aware where that makes agents' lives easier (a `--name 123` for a
//! string field stays a string; a single `--sections x` for an array field becomes `["x"]`),
//! but never *requires* a schema: with an empty `input_schema` the plain rules apply.

use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::CommandSpec;
use serde_json::{Map, Value};
use std::io::Read;

fn usage(message: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Usage, "cli", message)
}

/// `--kebab-key` / `kebab-key` -> `snake_key`.
pub fn normalize_key(key: &str) -> String {
    key.trim_start_matches('-').replace('-', "_")
}

/// `snake_key` -> `--kebab-key`.
pub fn flag_name(field: &str) -> String {
    format!("--{}", field.replace('_', "-"))
}

/// Schema of the input property `key` (snake_case), if the spec declares one.
pub fn prop_schema<'a>(spec: &'a CommandSpec, key: &str) -> Option<&'a Value> {
    spec.input_schema.get("properties")?.get(key)
}

/// JSON-schema `type`s of a (sub)schema, including those of `anyOf`/`oneOf` branches.
/// Empty when unknown (e.g. a bare `$ref`).
pub fn schema_types(schema: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match schema.get("type") {
        Some(Value::String(s)) => out.push(s.clone()),
        Some(Value::Array(a)) => out.extend(a.iter().filter_map(|v| v.as_str().map(String::from))),
        _ => {}
    }
    for k in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = schema.get(k) {
            for b in branches {
                out.extend(schema_types(b));
            }
        }
    }
    out
}

/// The property names the input schema declares, if it declares a *closed, simple* set
/// (no `allOf`/`anyOf`/`oneOf` composition, no `additionalProperties: true`).
pub fn known_properties(spec: &CommandSpec) -> Option<Vec<String>> {
    let s = &spec.input_schema;
    for k in ["allOf", "anyOf", "oneOf"] {
        if s.get(k).is_some() {
            return None;
        }
    }
    if s.get("additionalProperties") == Some(&Value::Bool(true)) {
        return None;
    }
    let props = s.get("properties")?.as_object()?;
    if props.is_empty() {
        return None;
    }
    Some(props.keys().cloned().collect())
}

/// True when the property `key` is declared boolean (so a bare `--flag` must not swallow
/// the next token).
pub fn is_bool_flag(spec: &CommandSpec, key: &str) -> bool {
    let key = normalize_key(key);
    if let Some(s) = prop_schema(spec, &key) {
        return schema_types(s).iter().any(|t| t == "boolean");
    }
    // `--no-foo` negates a boolean `foo`.
    if let Some(rest) = key.strip_prefix("no_") {
        if let Some(s) = prop_schema(spec, rest) {
            return schema_types(s).iter().any(|t| t == "boolean");
        }
    }
    false
}

fn parse_scalar(raw: &str, types: &[String]) -> Value {
    let has = |t: &str| types.iter().any(|x| x == t);
    let non_string = has("number") || has("integer") || has("boolean") || has("array") || has("object");
    if has("string") && !non_string {
        return Value::String(raw.to_string());
    }
    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// Parses one raw CLI value according to the (optional) property schema.
fn parse_value(raw: &str, schema: Option<&Value>) -> Value {
    let Some(schema) = schema else {
        return serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
    };
    let types = schema_types(schema);
    if types.iter().any(|t| t == "array") && !types.iter().any(|t| t == "string") {
        let item_types = schema.get("items").map(schema_types).unwrap_or_default();
        // A bare element of an array field, e.g. `--sections 1:00-2:00`.
        return match serde_json::from_str::<Value>(raw) {
            Ok(v @ Value::Array(_)) => v,
            _ => parse_scalar(raw, &item_types),
        };
    }
    parse_scalar(raw, &types)
}

fn wants_array(schema: Option<&Value>) -> bool {
    schema
        .map(schema_types)
        .map(|t| t.iter().any(|x| x == "array") && !t.iter().any(|x| x == "string"))
        .unwrap_or(false)
}

fn read_input_json(raw: &str) -> rustler_core::Result<Map<String, Value>> {
    let text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).map_err(|e| {
            usage(format!("cannot read --input-json file `{path}`: {e}"))
                .hint("pass a path that exists, e.g. --input-json @args.json")
        })?
    } else if raw == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| usage(format!("cannot read --input-json from stdin: {e}")))?;
        s
    } else {
        raw.to_string()
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(usage("--input-json must be a JSON object")
            .hint("e.g. --input-json '{\"source_id\":\"src_x\"}'")),
        Err(e) => Err(usage(format!("--input-json is not valid JSON: {e}"))
            .hint("pass a JSON object, `@file.json`, or `-` for stdin")),
    }
}

/// `--kebab-key` -> `snake_key`; values parsed as JSON when they parse (numbers, bools,
/// arrays, objects) else strings; flag without value -> `true`; repeated key -> array;
/// positionals assigned to `spec.positional` in order (extra -> `Usage`);
/// `--input-json` merged first, flags override.
///
/// Flag keys may be given with or without leading dashes, kebab or snake case.
/// Extras beyond the contract: when the input schema is known, string-typed fields keep
/// their raw text (`--name 123` stays `"123"`), a single value for an array field becomes a
/// one-element array, `--no-foo` sets a boolean `foo` to `false`, and a trailing array-typed
/// positional collects the remaining positionals.
pub fn args_to_json(
    spec: &CommandSpec,
    positional: &[String],
    flags: &[(String, Option<String>)],
) -> rustler_core::Result<serde_json::Value> {
    let mut obj: Map<String, Value> = Map::new();

    // 1. --input-json (first, so everything else overrides it).
    for (k, v) in flags {
        if normalize_key(k) == "input_json" {
            let raw = v
                .as_deref()
                .ok_or_else(|| usage("--input-json needs a value").hint("pass a JSON object or @file"))?;
            for (mk, mv) in read_input_json(raw)? {
                obj.insert(mk, mv);
            }
        }
    }

    // 2. positionals, in order.
    let slots = spec.positional;
    if positional.len() > slots.len() {
        let last_is_array = slots
            .last()
            .map(|k| wants_array(prop_schema(spec, k)))
            .unwrap_or(false);
        if !last_is_array {
            let expected = if slots.is_empty() {
                "this command takes no positional arguments".to_string()
            } else {
                format!("expected at most {}: <{}>", slots.len(), slots.join("> <"))
            };
            return Err(usage(format!(
                "unexpected positional argument `{}` for `{}`",
                positional[slots.len()],
                spec.name
            ))
            .hint(format!(
                "{expected}; pass everything else as --flag value (see `rustler commands describe {}`)",
                spec.name
            )));
        }
    }
    for (i, slot) in slots.iter().enumerate() {
        let is_last = i + 1 == slots.len();
        let schema = prop_schema(spec, slot);
        if is_last && wants_array(schema) && positional.len() > i {
            let items: Vec<Value> = positional[i..]
                .iter()
                .map(|p| parse_value(p, schema))
                .flat_map(|v| match v {
                    Value::Array(a) => a,
                    other => vec![other],
                })
                .collect();
            obj.insert((*slot).to_string(), Value::Array(items));
        } else if let Some(p) = positional.get(i) {
            obj.insert((*slot).to_string(), parse_value(p, schema));
        }
    }

    // 3. flags, repeated keys collected in order.
    let mut order: Vec<String> = Vec::new();
    let mut grouped: Map<String, Value> = Map::new(); // key -> Array of values
    for (k, v) in flags {
        let mut key = normalize_key(k);
        if key.is_empty() {
            return Err(usage("empty flag name `--`"));
        }
        if key == "input_json" {
            continue;
        }
        let mut negate = false;
        if prop_schema(spec, &key).is_none() {
            if let Some(rest) = key.strip_prefix("no_") {
                if prop_schema(spec, rest).is_some_and(|s| schema_types(s).iter().any(|t| t == "boolean")) {
                    key = rest.to_string();
                    negate = true;
                }
            }
        }
        let schema = prop_schema(spec, &key);
        let value = match v {
            None => Value::Bool(!negate),
            Some(raw) => parse_value(raw, schema),
        };
        if !grouped.contains_key(&key) {
            order.push(key.clone());
            grouped.insert(key.clone(), Value::Array(Vec::new()));
        }
        if let Some(Value::Array(items)) = grouped.get_mut(&key) {
            items.push(value);
        }
    }
    for key in order {
        let Some(Value::Array(mut items)) = grouped.remove(&key) else {
            continue;
        };
        let value = if items.len() == 1 {
            let only = items.remove(0);
            if wants_array(prop_schema(spec, &key)) && !only.is_array() {
                Value::Array(vec![only])
            } else {
                only
            }
        } else {
            // Repeated: flatten array-typed repeats (`--x [1] --x [2]`) no further; keep as given.
            Value::Array(items)
        };
        obj.insert(key, value);
    }

    Ok(Value::Object(obj))
}
