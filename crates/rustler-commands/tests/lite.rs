//! Lite install behaviour: heavy analyzers whose extra is not installed fail with a clear
//! `DependencyMissing` (exit code 4) and the exact `rustler setup run --with <extra>` hint, never a
//! traceback. `RUSTLER_HIDE_EXTRAS` makes the engine pretend the extras are absent (own test binary,
//! so the environment variable cannot leak into the full-engine tests).
mod support;

use rustler_core::ErrorCode;
use serde_json::json;
use support::*;

fn hide_all_extras() {
    // SAFETY: this test binary only ever sets this one variable, before spawning any engine.
    unsafe { std::env::set_var("RUSTLER_HIDE_EXTRAS", "ocr,audio-events,speakers") };
}

async fn lite_source() -> Option<(tempfile::TempDir, rustler_registry::Ctx, rustler_registry::Registry, String)> {
    if !have_media_tools() || !have_engine() {
        return None;
    }
    hide_all_extras();
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("v.mp4");
    make_video(&video, 6, true);
    let v = ok(&reg, "source.import", &ctx, json!({"input": video})).await;
    let id = v["sources"][0]["id"].as_str().unwrap().to_string();
    Some((dir, ctx, reg, id))
}

#[tokio::test]
async fn ocr_without_its_extra_is_dependency_missing_with_the_enable_command() {
    let Some((_d, ctx, reg, id)) = lite_source().await else { return };
    let e = expect_code(run(&reg, "analyze.ocr", &ctx, json!({"source_id": id})).await, ErrorCode::DependencyMissing);
    assert_eq!(e.code.exit_code(), 4);
    let hint = e.hint.clone().unwrap_or_default();
    assert!(hint.contains("rustler setup run --with ocr"), "hint: {hint}");
    assert!(e.message.contains("ocr"), "{}", e.message);
}

#[tokio::test]
async fn events_fall_back_to_the_heuristic_and_say_how_to_get_yamnet() {
    let Some((_d, ctx, reg, id)) = lite_source().await else { return };
    let v = ok(&reg, "analyze.events", &ctx, json!({"source_id": id})).await;
    assert_eq!(v["model"], "heuristic", "{v}");
    let warnings = v["warnings"].to_string();
    assert!(warnings.contains("rustler setup run --with audio-events"), "{warnings}");
}

#[tokio::test]
async fn deep_reports_the_skipped_heavy_parts_with_their_hints_and_still_succeeds() {
    let Some((_d, ctx, reg, id)) = lite_source().await else { return };
    let v = ok(&reg, "analyze.deep", &ctx, json!({"source_id": id})).await;
    assert!(v["ocr"].is_null(), "{v}");
    let warnings = v["warnings"].to_string();
    assert!(warnings.contains("on-screen text skipped") && warnings.contains("rustler setup run --with ocr"), "{warnings}");
    assert!(v["faces"].is_object(), "faces (YuNet, base install) must still work: {v}");
}

#[tokio::test]
async fn doctor_lists_optional_features_with_enable_commands() {
    let reg = registry();
    let tmp = tempfile::tempdir().unwrap();
    let ctx = rustler_registry::Ctx::new(None, tmp.path().to_path_buf());
    let v = ok(&reg, "doctor.run", &ctx, json!({})).await;
    let feats = v["optional_features"].as_array().expect("optional_features");
    let names: Vec<&str> = feats.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["ocr", "audio-events", "speakers", "cuda", "sfx-pack"]);
    for f in feats {
        assert_eq!(f["available"], false, "empty home has nothing installed: {f}");
        assert_eq!(f["enable"].as_str().unwrap(), format!("rustler setup run --with {}", f["name"].as_str().unwrap()));
    }
    assert_eq!(v["profile"], "lite");
}

#[tokio::test]
async fn setup_rejects_an_unknown_with_piece_before_downloading_anything() {
    let reg = registry();
    let tmp = tempfile::tempdir().unwrap();
    let ctx = rustler_registry::Ctx::new(None, tmp.path().to_path_buf());
    let e = expect_code(run(&reg, "setup.run", &ctx, json!({"with": ["ocr", "gpu-magic"]})).await, ErrorCode::Validation);
    assert!(e.hint.unwrap_or_default().contains("sfx-pack"));
    assert!(!tmp.path().join("bin").join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }).exists());
}
