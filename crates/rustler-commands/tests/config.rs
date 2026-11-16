//! config.* tests: scopes, validation, and the commands that read settings.
mod common;

use common::*;
use rustler_commands::config_cmds::setting;
use rustler_core::ErrorCode;
use serde_json::json;

#[tokio::test]
async fn project_overrides_global() {
    let (_p, _store, ctx) = new_project();
    let reg = registry();

    // Nothing set: the documented default, source "default".
    let v = call(&reg, &ctx, "config.get", json!({"key": "download.quality"})).await.unwrap();
    assert_eq!(v["value"], "720");
    assert_eq!(v["source"], "default");

    call(&reg, &ctx, "config.set", json!({"key": "download.quality", "value": "480", "scope": "global"})).await.unwrap();
    let v = call(&reg, &ctx, "config.get", json!({"key": "download.quality"})).await.unwrap();
    assert_eq!((v["value"].clone(), v["source"].clone()), (json!("480"), json!("global")));
    assert!(ctx.home.join("config.json").is_file());

    // The project scope wins.
    let s = call(&reg, &ctx, "config.set", json!({"key": "download.quality", "value": 1080, "scope": "project"})).await.unwrap();
    assert_eq!(s["value"], "1080", "numbers are normalized to the string option");
    let v = call(&reg, &ctx, "config.get", json!({"key": "download.quality"})).await.unwrap();
    assert_eq!((v["value"].clone(), v["source"].clone()), (json!("1080"), json!("project")));
    assert_eq!(setting::<String>(&ctx, "download.quality").as_deref(), Some("1080"));

    // Scope-only reads do not fall back.
    let g = call(&reg, &ctx, "config.get", json!({"key": "download.quality", "scope": "global"})).await.unwrap();
    assert_eq!(g["value"], "480");

    // Another project without its own value sees the global one.
    let (_p2, _s2, mut other) = new_project();
    other.home = ctx.home.clone();
    assert_eq!(setting::<String>(&other, "download.quality").as_deref(), Some("480"));

    // Unset the project value: the global one applies again.
    let u = call(&reg, &ctx, "config.unset", json!({"key": "download.quality", "scope": "project"})).await.unwrap();
    assert_eq!(u["was_set"], true);
    assert_eq!(u["effective"]["source"], "global");
    let u = call(&reg, &ctx, "config.unset", json!({"key": "download.quality", "scope": "global"})).await.unwrap();
    assert_eq!(u["effective"]["source"], "default");

    // config.set defaults to the project scope when a project is open.
    let s = call(&reg, &ctx, "config.set", json!({"key": "render.parallel", "value": "3"})).await.unwrap();
    assert_eq!((s["scope"].clone(), s["value"].clone()), (json!("project"), json!(3)));
    assert_eq!(setting::<u32>(&ctx, "render.parallel"), Some(3));
}

#[tokio::test]
async fn unknown_key_validation_lists_keys() {
    let (_p, _store, ctx) = new_project();
    let reg = registry();
    for name in ["config.get", "config.set", "config.unset"] {
        let e = call(&reg, &ctx, name, json!({"key": "nope.key", "value": 1})).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "{name}");
        let hint = e.hint.unwrap();
        for k in [
            "download.quality",
            "render.parallel",
            "render.quality",
            "captions.style",
            "captions.words_per_line",
            "template.default",
            "clip.smart_defaults",
            "qc.faces",
        ] {
            assert!(hint.contains(k), "{name}: hint lacks {k}: {hint}");
        }
    }
    let l = call(&reg, &ctx, "config.list", json!({})).await.unwrap();
    assert_eq!(l["settings"].as_array().unwrap().len(), 8);
}

#[tokio::test]
async fn bad_value_validation() {
    let (_p, _store, ctx) = new_project();
    let reg = registry();
    for (k, v) in [
        ("download.quality", json!("4k")),
        ("render.parallel", json!(0)),
        ("render.parallel", json!(5)),
        ("render.parallel", json!(2.5)),
        ("render.parallel", json!("many")),
        ("render.quality", json!("ultra")),
        ("qc.faces", json!("maybe")),
        ("clip.smart_defaults", json!(3)),
        ("captions.style", json!("  ")),
        ("captions.words_per_line", json!(11)),
    ] {
        let e = call(&reg, &ctx, "config.set", json!({"key": k, "value": v.clone()})).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "{k}={v}");
        assert!(e.hint.unwrap().contains(k));
    }
    // Nothing was written.
    assert!(!ctx.project_dir.as_ref().unwrap().join("config.json").exists());
    // Valid coercions.
    let s = call(&reg, &ctx, "config.set", json!({"key": "qc.faces", "value": "true"})).await.unwrap();
    assert_eq!(s["value"], true);
    let s = call(&reg, &ctx, "config.set", json!({"key": "captions.style", "value": "hormozi-pop"})).await.unwrap();
    assert_eq!(s["value"], "hormozi-pop");
    // Dry run writes nothing.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    call(&reg, &dry, "config.set", json!({"key": "render.quality", "value": "draft"})).await.unwrap();
    assert_eq!(setting::<String>(&ctx, "render.quality"), None);
}

#[tokio::test]
async fn import_uses_configured_quality() {
    let (_p, _store, ctx) = new_project();
    let reg = registry();
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let url = "https://example.com/v";
    let v = call(&reg, &dry, "source.import", json!({"input": url})).await.unwrap();
    assert_eq!(v["would"]["format"], "bv*[height<=720]+ba/b[height<=720]");

    call(&reg, &ctx, "config.set", json!({"key": "download.quality", "value": "480"})).await.unwrap();
    let v = call(&reg, &dry, "source.import", json!({"input": url})).await.unwrap();
    assert_eq!(v["would"]["format"], "bv*[height<=480]+ba/b[height<=480]");

    // An explicit flag always wins over the setting.
    let v = call(&reg, &dry, "source.import", json!({"input": url, "quality": "best"})).await.unwrap();
    assert_eq!(v["would"]["format"], "bv*+ba/b");
}
