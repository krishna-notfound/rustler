//! Task 2 registry contract tests.

use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Deserialize, JsonSchema)]
struct In {
    a: u32,
}

#[derive(Serialize, JsonSchema)]
struct Out {
    b: u32,
}

fn registry() -> Registry {
    let mut reg = Registry::new();
    reg.register(typed::<In, Out, _, _>(
        "test.double",
        "Double a number",
        &["a"],
        Annotations {
            read_only: true,
            ..Default::default()
        },
        vec![Example {
            description: "double 2",
            cli: "rustler test double 2",
        }],
        |_ctx: Ctx, i: In| async move { Ok(Out { b: i.a * 2 }) },
    ));
    reg
}

fn ctx() -> Ctx {
    Ctx::new(None, std::env::temp_dir())
}

#[tokio::test]
async fn typed_command_validates_input() {
    let reg = registry();
    let err = reg
        .dispatch("test.double", json!({"a": "x"}), ctx())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(!err.message.is_empty());
    let ok = reg
        .dispatch("test.double", json!({"a": 2}), ctx())
        .await
        .unwrap();
    assert_eq!(ok, json!({"b": 4}));
}

#[tokio::test]
async fn list_and_describe() {
    let reg = registry();
    let names: Vec<&str> = reg.list().iter().map(|s| s.name).collect();
    assert!(names.contains(&"test.double"));
    let spec = reg.describe("test.double").unwrap();
    assert!(
        spec.input_schema["properties"]["a"].is_object(),
        "{}",
        spec.input_schema
    );
    assert!(
        spec.output_schema["properties"]["b"].is_object(),
        "{}",
        spec.output_schema
    );
    assert_eq!(spec.positional, &["a"]);
    assert!(spec.annotations.read_only);
    assert!(reg.describe("nope").is_none());
    let err = reg.dispatch("nope", json!({}), ctx()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(err.hint.unwrap().contains("rustler commands list"));
    // CommandSpec is serializable for `rustler commands describe`.
    let v = serde_json::to_value(&spec).unwrap();
    assert_eq!(v["name"], "test.double");
}

#[test]
fn envelope_shapes() {
    let env = Envelope::ok("source.import", json!({"x": 1, "warnings": ["w"]}));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
    assert_eq!(
        v,
        json!({"ok": true, "schema": "rustler.source.import@1", "data": {"x": 1}, "warnings": ["w"]})
    );
    assert_eq!(env.exit_code(), 0);

    let plain = Envelope::ok("project.show", json!({"x": 1}));
    let v: serde_json::Value = serde_json::from_str(&plain.to_json()).unwrap();
    assert_eq!(v["warnings"], json!([]));

    let e =
        RustlerError::new(ErrorCode::RevisionConflict, "clip.set_captions", "stale").hint("reload");
    let env = Envelope::Err { error: e.clone() };
    assert_eq!(env.exit_code(), e.code.exit_code());
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "revision_conflict");
    assert_eq!(v["error"]["stage"], "clip.set_captions");
    assert_eq!(v["error"]["hint"], "reload");
    assert_eq!(v["error"]["retryable"], false);
}

#[test]
fn ctx_store_without_project_is_not_found() {
    let err = ctx().store().unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(err.hint.unwrap().contains("rustler project create"));
}

#[test]
fn progress_sink_delivers_events() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let sink = ProgressSink::new(move |e: ProgressEvent| s2.lock().unwrap().push(e.stage));
    sink.emit(ProgressEvent {
        stage: "probe".into(),
        pct: Some(0.5),
        message: None,
    });
    ProgressSink::noop().emit(ProgressEvent {
        stage: "x".into(),
        pct: None,
        message: None,
    });
    assert_eq!(*seen.lock().unwrap(), vec!["probe".to_string()]);
}
