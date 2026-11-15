//! analyze.* command tests (need ffmpeg; transcription tests also need uv + the engine and
//! download the whisper `tiny` model on first use). Skips with a printed reason otherwise.
mod support;

use rustler_core::ErrorCode;
use serde_json::{Value, json};
use support::*;

async fn import(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, path: &std::path::Path) -> String {
    let v = ok(reg, "source.import", ctx, json!({"input": path})).await;
    v["sources"][0]["id"].as_str().unwrap().to_string()
}

fn parts_snapshot(dir: &std::path::Path) -> Vec<(String, std::time::SystemTime)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().modified().unwrap()))
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn transcribe_resumes_from_parts() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let Some(jfk) = jfk() else { return };
    let reg = registry();
    let (dir, mut ctx) = project();
    let video = dir.path().join("jfk.mp4");
    make_video_with_wav(&video, &jfk);
    let id = import(&reg, &ctx, &video).await;
    let root = ctx.project_dir.clone().unwrap();
    let parts = root.join("analysis").join(&id).join("transcript.parts");

    ctx.max_seconds = Some(1);
    let first = ok(&reg, "analyze.transcribe", &ctx, json!({"source_id": id, "model": "tiny", "language": "en"})).await;
    if first["complete"] == false {
        assert!(first["resume"].as_str().unwrap().contains("rerun"));
        assert!(!root.join("analysis").join(&id).join("transcript.json").exists());
    }
    ctx.max_seconds = None;
    let second = ok(&reg, "analyze.transcribe", &ctx, json!({"source_id": id, "model": "tiny", "language": "en"})).await;
    assert_eq!(second["complete"], true);
    assert!(second["words"].as_u64().unwrap() > 15);
    assert!(second["units_preview"].as_array().unwrap().len() <= 30);
    assert!(second["units_preview"][0].as_str().unwrap().starts_with("#0 ["));
    let before = parts_snapshot(&parts);
    assert!(!before.is_empty(), "chunk parts were saved");

    // Third call reuses the parts (mtimes unchanged) and still completes.
    let third = ok(&reg, "analyze.transcribe", &ctx, json!({"source_id": id, "model": "tiny", "language": "en"})).await;
    assert_eq!(third["complete"], true);
    assert_eq!(parts_snapshot(&parts), before, "parts must be reused, not rewritten");

    let found = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "fellow americans"})).await;
    assert!(found["total_matches"].as_u64().unwrap() >= 1, "{found}");

    // --force deletes the parts and redoes the work.
    let forced = ok(&reg, "analyze.transcribe", &ctx, json!({"source_id": id, "model": "tiny", "language": "en", "force": true})).await;
    assert_eq!(forced["complete"], true);
    assert_ne!(parts_snapshot(&parts), before, "force rewrites the parts");
}

#[tokio::test]
async fn empty_transcript_ok() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("tone.mp4");
    make_video(&video, 8, true);
    let id = import(&reg, &ctx, &video).await;
    let v = ok(&reg, "analyze.transcribe", &ctx, json!({"source_id": id, "model": "tiny", "language": "en"})).await;
    assert_eq!(v["complete"], true);
    assert_eq!(v["words"], 0);
    assert_eq!(v["units"], 0);
    assert!(v["warnings"][0].as_str().unwrap().contains("no speech"));
    let g = ok(&reg, "transcript.get", &ctx, json!({"source_id": id})).await;
    assert_eq!(g["total_units"], 0);
    assert!(g["next"][0].as_str().unwrap().contains("--start/--end"));
}

#[tokio::test]
async fn quick_on_speech_has_highlight_signals() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let Some(jfk) = jfk() else { return };
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("jfk.mp4");
    make_video_with_wav(&video, &jfk);
    let id = import(&reg, &ctx, &video).await;
    ok(&reg, "project.set_settings", &ctx, json!({"default_whisper_model": "tiny"})).await;
    let q = ok(&reg, "analyze.quick", &ctx, json!({"source_id": id, "language": "en"})).await;
    assert_eq!(q["complete"], true);
    assert!(q["words"].as_u64().unwrap() > 15, "{q}");
    assert!(q["units"].as_u64().unwrap() >= 1);
    assert!(q["scenes"].as_u64().unwrap() >= 1);
    assert!(q["duration"].as_f64().unwrap() > 10.0);
    let root = ctx.project_dir.clone().unwrap().join("analysis").join(&id);
    for f in ["transcript.json", "units.json", "scenes.json", "audio_events.json"] {
        assert!(root.join(f).is_file(), "missing {f}");
    }
    // units.json is a bare array
    let units: Value = serde_json::from_slice(&std::fs::read(root.join("units.json")).unwrap()).unwrap();
    assert!(units.as_array().is_some_and(|a| !a.is_empty()));
    // every highlight signal is a loud moment; those during speech carry the unit text
    for h in q["highlight_signals"].as_array().unwrap() {
        assert!(h["lufs"].as_f64().is_some() && h["mmss"].as_str().is_some());
        if !h["unit_id"].is_null() {
            assert!(!h["text"].as_str().unwrap().is_empty());
        }
    }
    if let Some(p) = q["peaks"].as_array().and_then(|p| p.first()) {
        assert!(q["highlight_signals"].as_array().is_some_and(|h| h[0]["t"] == p["t"]));
    }
}

#[tokio::test]
async fn audio_and_scenes_commands_and_audio_only_rules() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("tone.mp4");
    make_video(&video, 12, true);
    let id = import(&reg, &ctx, &video).await;
    let a = ok(&reg, "analyze.audio", &ctx, json!({"source_id": id})).await;
    assert!(a["silences_total"].as_u64().unwrap() >= 2, "{a}");
    assert!(a["buckets"].as_u64().unwrap() >= 10);
    let s = ok(&reg, "analyze.scenes", &ctx, json!({"source_id": id})).await;
    assert!(s["scene_count"].as_u64().unwrap() >= 1);

    // audio-only: scenes rejected with a hint, transcription allowed
    let pod = dir.path().join("pod.m4a");
    make_audio_only(&pod, 4);
    let pid = import(&reg, &ctx, &pod).await;
    let e = expect_code(run(&reg, "analyze.scenes", &ctx, json!({"source_id": pid})).await, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("audio-only"));

    // silent video: transcribe rejected with a hint
    let silent = dir.path().join("silent.mp4");
    make_video(&silent, 3, false);
    let sid = import(&reg, &ctx, &silent).await;
    let e = expect_code(run(&reg, "analyze.transcribe", &ctx, json!({"source_id": sid})).await, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("--start/--end"));
    expect_code(run(&reg, "analyze.audio", &ctx, json!({"source_id": sid})).await, ErrorCode::Validation);
}

#[tokio::test]
async fn unknown_source_is_not_found() {
    let reg = registry();
    let (_d, ctx) = project();
    let e = expect_code(run(&reg, "analyze.quick", &ctx, json!({"source_id": "src_nope"})).await, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("source list"));
}

/// Turns an imported local file into a "URL section" source with fetched YouTube captions on disk
/// (what `source import URL --sections` + `source fetch-transcript` leave behind), without network.
fn fake_url_source_with_captions(root: &std::path::Path, id: &str, url: &str, section: (f64, f64)) {
    let sj = root.join("sources").join(id).join("source.json");
    let mut s: Value = serde_json::from_slice(&std::fs::read(&sj).unwrap()).unwrap();
    s["origin"] = json!({"kind": "url", "url": url, "section": {"start": section.0, "end": section.1}, "original_offset": section.0});
    std::fs::write(&sj, serde_json::to_vec_pretty(&s).unwrap()).unwrap();
    let pid = rustler_commands::source_cmds::pending_id(url);
    let dir = root.join("analysis").join(&pid);
    std::fs::create_dir_all(&dir).unwrap();
    let words: Vec<Value> = (0..20)
        .map(|i| json!({"i": i, "text": format!("w{i}"), "start": 95.0 + i as f64, "end": 96.0 + i as f64, "conf": null, "speaker": null}))
        .collect();
    let t = json!({
        "schema": "rustler.transcript@1", "source_id": id, "language": "en", "engine": "youtube:auto",
        "script": "latin", "words": words, "word_timing": "approximate"
    });
    std::fs::write(dir.join("transcript.json"), serde_json::to_vec_pretty(&t).unwrap()).unwrap();
}

#[tokio::test]
async fn quick_transcribe_auto_reuses_fetched_captions_and_never_runs_whisper() {
    if !have_media_tools() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("tone.mp4");
    make_video(&video, 12, true);
    let id = import(&reg, &ctx, &video).await;
    let root = ctx.project_dir.clone().unwrap();
    fake_url_source_with_captions(&root, &id, "https://youtu.be/captioned", (100.0, 108.0));

    let q = ok(&reg, "analyze.quick", &ctx, json!({"source_id": id})).await;
    assert_eq!(q["transcript_source"], "captions", "{q}");
    assert_eq!(q["words"], 8, "only the section's words, {q}");
    assert!(q["units"].as_u64().unwrap() >= 1);
    let w = q["warnings"].to_string();
    assert!(w.contains("fetched captions") && w.contains("approximate"), "{w}");
    let adir = root.join("analysis").join(&id);
    assert!(!adir.join("transcript.parts").exists(), "whisper must not have run");
    let t: Value = serde_json::from_slice(&std::fs::read(adir.join("transcript.json")).unwrap()).unwrap();
    assert_eq!(t["engine"], "youtube:auto");
    assert_eq!(t["word_timing"], "approximate");
    // shifted to the source's own timeline: the section started at 100 s
    assert_eq!(t["words"][0]["text"], "w5");
    assert_eq!(t["words"][0]["start"], 0.0);
    assert_eq!(t["words"][7]["end"], 8.0);
    assert!(adir.join("units.json").is_file());
}

#[tokio::test]
async fn quick_transcribe_never_skips_the_transcript_and_accepts_booleans() {
    if !have_media_tools() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("tone.mp4");
    make_video(&video, 12, true);
    let id = import(&reg, &ctx, &video).await;
    let root = ctx.project_dir.clone().unwrap();
    for arg in [json!("never"), json!(false), json!("false")] {
        let q = ok(&reg, "analyze.quick", &ctx, json!({"source_id": id, "transcribe": arg})).await;
        assert_eq!(q["transcript_source"], "none", "{q}");
        assert_eq!(q["words"], 0);
        assert!(q["warnings"].to_string().contains("--transcribe never"), "{q}");
        assert!(q["peaks"].is_array(), "audio analysis still runs: {q}");
    }
    let adir = root.join("analysis").join(&id);
    assert!(!adir.join("transcript.json").exists() && !adir.join("transcript.parts").exists());
    assert!(adir.join("audio_events.json").is_file());
    expect_code(run(&reg, "analyze.quick", &ctx, json!({"source_id": id, "transcribe": "sometimes"})).await, ErrorCode::Validation);
}
