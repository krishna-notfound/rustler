use rustler_cli::argmap::args_to_json;
use rustler_core::ErrorCode;
use rustler_registry::{Annotations, CommandSpec};
use serde_json::{Value, json};

fn spec_with(positional: &'static [&'static str], schema: Value) -> CommandSpec {
    CommandSpec {
        name: "test.cmd",
        summary: "test",
        positional,
        input_schema: schema,
        output_schema: json!({}),
        annotations: Annotations::default(),
        examples: vec![],
    }
}

fn spec(positional: &'static [&'static str]) -> CommandSpec {
    spec_with(positional, json!({}))
}

fn flags(items: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.map(String::from)))
        .collect()
}

#[test]
fn kebab_to_snake_and_types() {
    let v = args_to_json(
        &spec(&[]),
        &[],
        &flags(&[("start-unit", Some("12")), ("pretty-name", Some("a b")), ("loud", None)]),
    )
    .unwrap();
    assert_eq!(v, json!({"start_unit": 12, "pretty_name": "a b", "loud": true}));
}

#[test]
fn leading_dashes_are_tolerated() {
    let v = args_to_json(&spec(&[]), &[], &flags(&[("--start-unit", Some("3"))])).unwrap();
    assert_eq!(v, json!({"start_unit": 3}));
}

#[test]
fn json_values_parse() {
    let v = args_to_json(
        &spec(&[]),
        &[],
        &flags(&[("r", Some("[1,2]")), ("o", Some("{\"a\":true}")), ("f", Some("1.5")), ("b", Some("false"))]),
    )
    .unwrap();
    assert_eq!(v, json!({"r": [1, 2], "o": {"a": true}, "f": 1.5, "b": false}));
}

#[test]
fn repeated_becomes_array() {
    let v = args_to_json(
        &spec(&[]),
        &[],
        &flags(&[("sections", Some("1:00-2:00")), ("sections", Some("5:00-6:00"))]),
    )
    .unwrap();
    assert_eq!(v, json!({"sections": ["1:00-2:00", "5:00-6:00"]}));
}

#[test]
fn positional_mapping() {
    let v = args_to_json(&spec(&["source_id"]), &["src_x".to_string()], &[]).unwrap();
    assert_eq!(v, json!({"source_id": "src_x"}));

    let err = args_to_json(&spec(&["source_id"]), &["a".to_string(), "b".to_string()], &[]).unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.hint.is_some());
}

#[test]
fn flag_overrides_positional() {
    let v = args_to_json(
        &spec(&["source_id"]),
        &["src_x".to_string()],
        &flags(&[("source-id", Some("src_y"))]),
    )
    .unwrap();
    assert_eq!(v, json!({"source_id": "src_y"}));
}

#[test]
fn input_json_file_merge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("in.json");
    std::fs::write(&path, r#"{"a":1,"b":2}"#).unwrap();
    let at = format!("@{}", path.display());
    let v = args_to_json(
        &spec(&[]),
        &[],
        &flags(&[("input-json", Some(&at)), ("b", Some("3"))]),
    )
    .unwrap();
    assert_eq!(v, json!({"a": 1, "b": 3}));
}

#[test]
fn input_json_inline_and_errors() {
    let v = args_to_json(&spec(&[]), &[], &flags(&[("input-json", Some("{\"a\":1}"))])).unwrap();
    assert_eq!(v, json!({"a": 1}));

    for bad in ["[1]", "not json", "@/definitely/not/here.json"] {
        let e = args_to_json(&spec(&[]), &[], &flags(&[("input-json", Some(bad))])).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{bad}");
    }
    let e = args_to_json(&spec(&[]), &[], &flags(&[("input-json", None)])).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
}

#[test]
fn schema_keeps_strings_and_wraps_arrays() {
    let schema = json!({"type":"object","properties":{
        "name": {"type":"string"},
        "maybe": {"type":["string","null"]},
        "count": {"type":"integer"},
        "sections": {"type":"array","items":{"type":"string"}},
        "ids": {"type":"array","items":{"type":"integer"}},
        "loud": {"type":"boolean"}
    }});
    let s = spec_with(&[], schema);
    let v = args_to_json(
        &s,
        &[],
        &flags(&[
            ("name", Some("123")),
            ("maybe", Some("true")),
            ("count", Some("7")),
            ("sections", Some("1")),
            ("ids", Some("4")),
            ("no-loud", None),
        ]),
    )
    .unwrap();
    assert_eq!(
        v,
        json!({"name":"123","maybe":"true","count":7,"sections":["1"],"ids":[4],"loud":false})
    );
}

#[test]
fn trailing_array_positional_collects_rest() {
    let schema = json!({"type":"object","properties":{"files":{"type":"array","items":{"type":"string"}}}});
    let s = spec_with(&["files"], schema);
    let v = args_to_json(&s, &["a.mp4".into(), "b.mp4".into()], &[]).unwrap();
    assert_eq!(v, json!({"files": ["a.mp4", "b.mp4"]}));
}
