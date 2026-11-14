//! source.* and project.* command tests.
mod support;

use rustler_commands::source_cmds::{parse_json3, parse_section};
use rustler_core::{ErrorCode, ProjectStore};
use serde_json::json;
use support::*;

#[test]
fn parse_json3_words() {
    // Auto-caption style: one seg per word with tOffsetMs; plus a manual-style multi-token seg
    // and a "\n" append event that must be ignored.
    let fixture = r#"{"events":[
      {"tStartMs":1000,"dDurationMs":3000,"segs":[{"utf8":"hello"},{"utf8":" big","tOffsetMs":500},{"utf8":" world","tOffsetMs":1200}]},
      {"tStartMs":4000,"dDurationMs":100,"segs":[{"utf8":"\n"}]},
      {"tStartMs":5000,"dDurationMs":2000,"segs":[{"utf8":"two words"}]}
    ]}"#;
    let w = parse_json3(fixture).unwrap();
    let texts: Vec<_> = w.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["hello", "big", "world", "two", "words"]);
    assert!((w[0].start - 1.0).abs() < 1e-9);
    assert!((w[1].start - 1.5).abs() < 1e-9, "start = tStartMs + tOffsetMs/1000");
    assert!((w[2].start - 2.2).abs() < 1e-9);
    assert!((w[0].end - 1.5).abs() < 1e-9, "end = next word start");
    assert!((w[2].end - 4.0).abs() < 1e-9, "last word ends at the event end");
    // multi-token seg is split across its span in proportion to length
    assert!((w[3].start - 5.0).abs() < 1e-9 && w[3].end > 5.0 && w[3].end < 7.0);
    assert!((w[4].end - 7.0).abs() < 1e-9);
    assert!(w.iter().enumerate().all(|(i, w)| w.i == i as u32 && w.end >= w.start));
    assert!(w.windows(2).all(|p| p[0].end <= p[1].start + 1e-9));
}

#[test]
fn sections_parse_both_formats() {
    let a = parse_section("47:00-49:30").unwrap();
    let b = parse_section("0:47:00-0:49:30").unwrap();
    assert_eq!((a.start, a.end), (b.start, b.end));
    assert_eq!(parse_section("5:00-4:00").unwrap_err().code, ErrorCode::Validation);
}

#[tokio::test]
async fn project_create_show_and_dry_run() {
    let reg = registry();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("p1");
    let ctx = rustler_registry::Ctx::new(None, dir.path().join("home"));

    let mut dry = ctx.clone();
    dry.dry_run = true;
    let v = ok(&reg, "project.create", &dry, json!({"name": "Demo", "dir": target})).await;
    assert_eq!(v["dry_run"], true);
    assert!(!target.exists(), "dry run must not write");

    let v = ok(&reg, "project.create", &ctx, json!({"name": "Demo", "dir": target})).await;
    assert_eq!(v["project"]["name"], "Demo");
    assert!(target.join("project.json").is_file());
    let e = expect_code(run(&reg, "project.create", &ctx, json!({"name": "Demo", "dir": target})).await, ErrorCode::Validation);
    assert!(e.hint.is_some());

    let pctx = rustler_registry::Ctx::new(Some(target.clone()), dir.path().join("home"));
    let v = ok(&reg, "project.show", &pctx, json!({})).await;
    assert_eq!(v["counts"]["sources"], 0);
    assert!(v["next"][0].as_str().unwrap().contains("source import"));

    // events were appended with the actor
    let ev = std::fs::read_to_string(target.join("events.ndjson")).unwrap();
    assert!(ev.contains("project.created") && ev.contains("\"agent\""));

    let v = ok(&reg, "project.set_permissions", &pctx, json!({"network": false, "fs_roots": "C:/nowhere"})).await;
    assert_eq!(v["permissions"]["network"], false);
    assert_eq!(v["project_revision"], 1);
}

#[tokio::test]
async fn no_project_is_not_found_with_hint() {
    let reg = registry();
    let ctx = rustler_registry::Ctx::new(None, home());
    let e = expect_code(run(&reg, "source.list", &ctx, json!({})).await, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("project create"));
}

#[tokio::test]
async fn import_respects_fs_roots() {
    let reg = registry();
    let (dir, ctx) = project();
    let allowed = dir.path().join("allowed");
    std::fs::create_dir_all(&allowed).unwrap();
    let outside = dir.path().join("outside.mp4");
    std::fs::write(&outside, b"not really media").unwrap();
    ok(&reg, "project.set_permissions", &ctx, json!({"fs_roots": [allowed]})).await;
    let e = expect_code(run(&reg, "source.import", &ctx, json!({"input": outside})).await, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("fs-roots"));
}

#[tokio::test]
async fn import_missing_file_is_not_found() {
    let reg = registry();
    let (dir, ctx) = project();
    let e = expect_code(run(&reg, "source.import", &ctx, json!({"input": dir.path().join("nope.mp4")})).await, ErrorCode::NotFound);
    assert!(e.hint.is_some());
}

#[tokio::test]
async fn sections_on_local_file_rejected() {
    let reg = registry();
    let (dir, ctx) = project();
    let f = dir.path().join("a.mp4");
    std::fs::write(&f, b"x").unwrap();
    expect_code(run(&reg, "source.import", &ctx, json!({"input": f, "sections": "0:10-0:20"})).await, ErrorCode::Validation);
}

#[tokio::test]
async fn url_import_requires_permission() {
    let reg = registry();
    let (_dir, ctx) = project();
    // No yt-dlp is needed: the permission check comes first.
    let store = ProjectStore::open(ctx.project_dir.as_ref().unwrap()).unwrap();
    let mut p = store.load_project().unwrap();
    p.permissions.network = false;
    store.save_project(&mut p, 0).unwrap();
    for (cmd, input) in [
        ("source.import", json!({"input": "https://example.com/v"})),
        ("source.info", json!({"url": "https://example.com/v"})),
        ("source.fetch_transcript", json!({"url": "https://example.com/v"})),
        ("source.search", json!({"query": "x"})),
    ] {
        let e = expect_code(run(&reg, cmd, &ctx, input).await, ErrorCode::Validation);
        assert!(e.hint.unwrap().contains("set-permissions"), "{cmd}");
    }
    // downloads off but network on: info is fine to reject only for import
    let mut p = store.load_project().unwrap();
    p.permissions.network = true;
    p.permissions.downloads = false;
    let base = p.revision;
    store.save_project(&mut p, base).unwrap();
    let e = expect_code(run(&reg, "source.import", &ctx, json!({"input": "https://example.com/v"})).await, ErrorCode::Validation);
    assert!(e.message.contains("downloads"));
}

#[tokio::test]
async fn bad_urls_and_sections_are_validation_errors() {
    let reg = registry();
    let (_dir, ctx) = project();
    expect_code(run(&reg, "source.info", &ctx, json!({"url": "not a url"})).await, ErrorCode::Validation);
    expect_code(
        run(&reg, "source.import", &ctx, json!({"input": "https://example.com/v", "sections": "garbage"})).await,
        ErrorCode::Validation,
    );
}

#[tokio::test]
async fn import_local_creates_proxy_and_wav() {
    if !have_media_tools() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let f = dir.path().join("my clip (1) – टेस्ट.mp4");
    make_video(&f, 6, true);
    let v = ok(&reg, "source.import", &ctx, json!({"input": f})).await;
    let s = &v["sources"][0];
    assert_eq!(s["name"], "my clip (1) – टेस्ट", "name defaults to the file stem");
    assert_eq!(s["probe"]["has_video"], true);
    assert_eq!(s["probe"]["has_audio"], true);
    let id = s["id"].as_str().unwrap();
    let root = ctx.project_dir.clone().unwrap();
    assert!(root.join("sources").join(id).join("proxy.mp4").is_file());
    assert!(root.join("sources").join(id).join("audio.wav").is_file());
    assert!(root.join("sources").join(id).join("source.json").is_file());
    assert!(v["next"][0].as_str().unwrap().contains("analyze quick"));
    let list = ok(&reg, "source.list", &ctx, json!({})).await;
    assert_eq!(list["sources"][0]["id"], id);
    let shown = ok(&reg, "source.show", &ctx, json!({"source_id": id})).await;
    assert_eq!(shown["analysis"]["transcript"], false);
    // (own home: project.show records the project in the recent-projects registry)
    let mut reg_ctx = ctx.clone();
    reg_ctx.home = dir.path().join("home");
    let proj = ok(&reg, "project.show", &reg_ctx, json!({})).await;
    assert_eq!(proj["counts"]["sources"], 1);
    // custom name
    let v = ok(&reg, "source.import", &ctx, json!({"input": f, "name": "Named"})).await;
    assert_eq!(v["sources"][0]["name"], "Named");
}

#[tokio::test]
async fn import_emits_progress_and_honours_dry_run() {
    if !have_media_tools() {
        return;
    }
    let reg = registry();
    let (dir, mut ctx) = project();
    let f = dir.path().join("p.mp4");
    make_video(&f, 4, true);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    ctx.progress = rustler_registry::ProgressSink::new(move |e| s2.lock().unwrap().push(e.stage));
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let v = ok(&reg, "source.import", &dry, json!({"input": f})).await;
    assert_eq!(v["dry_run"], true);
    assert!(seen.lock().unwrap().is_empty());
    ok(&reg, "source.import", &ctx, json!({"input": f})).await;
    let stages = seen.lock().unwrap().clone();
    for want in ["probe", "proxy", "audio_wav"] {
        assert!(stages.iter().any(|s| s == want), "missing progress stage {want}: {stages:?}");
    }
}

#[tokio::test]
async fn import_video_without_audio_and_audio_only() {
    if !have_media_tools() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let silent = dir.path().join("silent.mp4");
    make_video(&silent, 4, false);
    let v = ok(&reg, "source.import", &ctx, json!({"input": silent})).await;
    assert_eq!(v["sources"][0]["probe"]["has_audio"], false);
    assert!(v["sources"][0]["audio_wav"].is_null());
    assert!(v["warnings"][0].as_str().unwrap().contains("no audio"));

    let audio = dir.path().join("pod.m4a");
    make_audio_only(&audio, 4);
    let v = ok(&reg, "source.import", &ctx, json!({"input": audio})).await;
    assert_eq!(v["sources"][0]["probe"]["has_video"], false);
    assert!(v["sources"][0]["proxy"].is_null());
    assert!(v["warnings"][0].as_str().unwrap().contains("audio-only"));
}

/// Live network test: needs yt-dlp and YouTube reachable. Run with `--ignored`.
#[tokio::test]
#[ignore]
async fn live_info_and_fetch_transcript() {
    if !have_ytdlp() || !have_engine() {
        return;
    }
    let reg = registry();
    let (_dir, ctx) = project();
    let url = std::env::var("RUSTLER_LIVE_URL").unwrap_or_else(|_| "https://www.youtube.com/watch?v=aqz-KE-bpKQ".to_string());
    let info = ok(&reg, "source.info", &ctx, json!({"url": url})).await;
    eprintln!("INFO title={} duration={} peaks={} subtitles={} chapters={}", info["title"], info["duration"], info["heatmap_peaks"], info["subtitles"], info["chapters"]);
    assert!(!info["title"].as_str().unwrap().is_empty());
    assert!(info["duration"].as_f64().unwrap() > 0.0);
    let t = ok(&reg, "source.fetch_transcript", &ctx, json!({"url": url})).await;
    eprintln!("TRANSCRIPT: {}", serde_json::to_string_pretty(&t).unwrap().chars().take(2500).collect::<String>());
    assert!(t["word_count"].as_u64().unwrap() > 0);
    let id = t["pending_id"].as_str().unwrap();
    let got = ok(&reg, "transcript.get", &ctx, json!({"source_id": id, "limit": 5})).await;
    assert!(got["returned"].as_u64().unwrap() > 0);
}

// ---------------------------------------------------------------------------------------
// source_info.json: shifting / filtering of the yt-dlp info (no network)
// ---------------------------------------------------------------------------------------

use rustler_commands::source_cmds::{HeatWindow, build_source_info, shift_heatmap};
use rustler_core::{
    TimeRange,
    model::{CaptionScript, WordTiming},
};

/// What yt-dlp `-J` / `--write-info-json` gives for a 100 s video: heatmap in 10 s windows.
fn fake_ytdlp_info() -> serde_json::Value {
    let heat: Vec<_> = (0..10)
        .map(|i| json!({"start_time": i as f64 * 10.0, "end_time": (i + 1) as f64 * 10.0, "value": (i as f64 + 1.0) / 10.0}))
        .collect();
    json!({
        "title": "A talk", "duration": 100.0, "uploader": "Some Channel",
        "webpage_url": "https://www.youtube.com/watch?v=abc",
        "chapters": [{"start_time": 0.0, "end_time": 50.0, "title": "Intro"}, {"start_time": 50.0, "end_time": 100.0, "title": "Main"}],
        "heatmap": heat,
        "subtitles": {"en": []}, "automatic_captions": {"en": [], "hi": []},
    })
}

#[test]
fn heatmap_shift_keeps_only_points_inside_the_section() {
    let h = |s: f64, e: f64, v: f64| HeatWindow { start: s, end: e, value: v };
    let heat = vec![h(0.0, 10.0, 0.2), h(10.0, 20.0, 0.9), h(20.0, 30.0, 0.5), h(30.0, 40.0, 0.1)];
    // section 15..32: the first window is outside; the others are clipped to the section
    let local = shift_heatmap(&heat, 15.0, 32.0, 15.0);
    let got: Vec<(f64, f64, f64)> = local.iter().map(|w| (w.start, w.end, w.value)).collect();
    assert_eq!(got, vec![(0.0, 5.0, 0.9), (5.0, 15.0, 0.5), (15.0, 17.0, 0.1)]);
    // no shift: original-video times, still only the section
    let orig = shift_heatmap(&heat, 15.0, 32.0, 0.0);
    assert_eq!((orig[0].start, orig[0].end), (15.0, 20.0));
    assert_eq!((orig[2].start, orig[2].end), (30.0, 32.0));
    // a section outside every window leaves nothing; touching edges do not count
    assert!(shift_heatmap(&heat, 40.0, 50.0, 40.0).is_empty());
    assert_eq!(shift_heatmap(&heat, 20.0, 30.0, 0.0).len(), 1);
}

#[test]
fn source_info_for_a_section_and_for_the_whole_video() {
    let v = fake_ytdlp_info();
    let url = "https://youtu.be/abc";

    let whole = build_source_info(&v, url, None);
    assert_eq!(whole["title"], "A talk");
    assert_eq!(whole["uploader"], "Some Channel");
    assert_eq!(whole["url"], "https://www.youtube.com/watch?v=abc", "webpage_url");
    assert_eq!(whole["duration"], 100.0);
    assert_eq!(whole["chapters"].as_array().unwrap().len(), 2);
    assert_eq!(whole["heatmap"].as_array().unwrap().len(), 10);
    assert_eq!(whole["heatmap"], whole["heatmap_local"]);
    assert_eq!(whole["original_offset"], 0.0);
    assert!(whole["section"].is_null());

    let sec = build_source_info(&v, url, Some(TimeRange { start: 25.0, end: 47.0 }));
    assert_eq!(sec["original_offset"], 25.0);
    assert_eq!(sec["section"], json!({"start": 25.0, "end": 47.0}));
    // original-video time (what highlights.signals expects), clipped to the section
    let orig: Vec<(f64, f64)> =
        sec["heatmap"].as_array().unwrap().iter().map(|w| (w["start"].as_f64().unwrap(), w["end"].as_f64().unwrap())).collect();
    assert_eq!(orig, vec![(25.0, 30.0), (30.0, 40.0), (40.0, 47.0)]);
    // shifted by -original_offset: 0 is the start of the imported section
    let local: Vec<(f64, f64, f64)> = sec["heatmap_local"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["start"].as_f64().unwrap(), w["end"].as_f64().unwrap(), w["value"].as_f64().unwrap()))
        .collect();
    assert_eq!(local, vec![(0.0, 5.0, 0.3), (5.0, 15.0, 0.4), (15.0, 22.0, 0.5)]);
    // peaks come from the section only (best is 40-47, value 0.5)
    assert_eq!(sec["heatmap_peaks"][0]["value"], 0.5);
    assert_eq!(sec["heatmap_peaks"].as_array().unwrap().len(), 3);
    // video-level fields are kept
    assert_eq!(sec["title"], "A talk");
    assert_eq!(sec["chapters"].as_array().unwrap().len(), 2);

    // a video without a heatmap gives empty lists, not an error
    let none = build_source_info(&json!({"title": "x", "duration": 5.0}), url, None);
    assert_eq!(none["heatmap"], json!([]));
}

/// Synthetic URL-section source (no media needed): source.json, transcript and units.
fn url_section_source(store: &ProjectStore, offset: f64, len: f64) -> rustler_core::SourceId {
    use rustler_core::{MediaProbe, Source, SourceId, SourceOrigin, Transcript, Word};
    let id = SourceId::new();
    let section = TimeRange { start: offset, end: offset + len };
    let source = Source {
        id: id.clone(),
        name: "talk [section]".into(),
        origin: SourceOrigin::Url { url: "https://youtu.be/abc".into(), section: Some(section), original_offset: offset },
        media_path: std::path::PathBuf::from("sources").join(id.as_str()).join("media.mp4"),
        probe: MediaProbe { duration: len, has_video: true, has_audio: true, width: 320, height: 240, fps: 25.0, ..Default::default() },
        proxy: None,
        audio_wav: None,
        imported_at: chrono::Utc::now(),
    };
    store.save_source(&source).unwrap();
    let words: Vec<Word> = (0..40)
        .map(|i| Word {
            i,
            text: if i % 8 == 7 { "word.".into() } else { "word".into() },
            start: i as f64 * 0.5,
            end: i as f64 * 0.5 + 0.4,
            conf: None,
            speaker: None,
        })
        .collect();
    let units = rustler_core::units::build_units(&words);
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: id.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    let rel = std::path::Path::new("analysis").join(id.as_str());
    store.write_json(&rel.join("transcript.json"), &t).unwrap();
    store.write_json(&rel.join("units.json"), &units).unwrap();
    id
}

#[tokio::test]
async fn saved_source_info_feeds_highlights_signals_and_source_info_by_id() {
    let reg = registry();
    let (_dir, ctx) = project();
    let store = ProjectStore::open(ctx.project_dir.as_ref().unwrap()).unwrap();
    // a 20 s section starting at 25 s of the 100 s video
    let id = url_section_source(&store, 25.0, 20.0);
    let rel = std::path::Path::new("analysis").join(id.as_str()).join("source_info.json");

    // no saved info yet: heatmap is null, and source.info by id explains how to get it
    let v = ok(&reg, "highlights.signals", &ctx, json!({"source_id": id.as_str(), "window": 10})).await;
    assert!(v["rows"][0]["heatmap"].is_null(), "{v}");
    let e = expect_code(run(&reg, "source.info", &ctx, json!({"source_id": id.as_str()})).await, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("source info https://youtu.be/abc"));
    expect_code(run(&reg, "source.info", &ctx, json!({})).await, ErrorCode::Validation);

    store.write_json(&rel, &build_source_info(&fake_ytdlp_info(), "https://youtu.be/abc", Some(TimeRange { start: 25.0, end: 45.0 }))).unwrap();

    // section 0-10 s = video 25-35 s: 25-30 at 0.3 and 30-35 at 0.4 -> 0.35
    let v = ok(&reg, "highlights.signals", &ctx, json!({"source_id": id.as_str(), "window": 10})).await;
    let rows = v["rows"].as_array().unwrap();
    let h0 = rows[0]["heatmap"].as_f64().expect("heatmap is non-null once source_info.json is saved");
    assert!((h0 - 0.35).abs() < 1e-9, "{h0}");
    let h1 = rows[1]["heatmap"].as_f64().unwrap();
    assert!((h1 - 0.45).abs() < 1e-9, "video 35-45 s: 0.4 and 0.5 -> {h1}");

    // source.info by id returns the saved video info without network (even with the network forbidden)
    let mut p = store.load_project().unwrap();
    let rev = p.revision;
    p.permissions.network = false;
    store.save_project(&mut p, rev).unwrap();
    let i = ok(&reg, "source.info", &ctx, json!({"source_id": id.as_str()})).await;
    assert_eq!(i["title"], "A talk");
    assert_eq!(i["uploader"], "Some Channel");
    assert_eq!(i["heatmap"].as_array().unwrap().len(), 3);
}

/// Live: import a short section of a public captioned video, then `highlights signals` shows a heatmap.
/// Needs network, yt-dlp, ffmpeg and the engine (transcription). Run with `--ignored`.
#[tokio::test]
#[ignore]
async fn live_section_import_saves_source_info_and_highlights_shows_heatmap() {
    if !have_ytdlp() || !have_media_tools() || !have_engine() {
        return;
    }
    let reg = registry();
    let (_dir, ctx) = project();
    let url = std::env::var("RUSTLER_LIVE_URL").unwrap_or_else(|_| "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string());
    let sections = std::env::var("RUSTLER_LIVE_SECTION").unwrap_or_else(|_| "0:40-1:10".to_string());
    let v = ok(&reg, "source.import", &ctx, json!({"input": url, "sections": sections})).await;
    eprintln!("IMPORT warnings={}", v["warnings"]);
    let id = v["sources"][0]["id"].as_str().unwrap().to_string();
    let info_path = ctx.project_dir.clone().unwrap().join("analysis").join(&id).join("source_info.json");
    assert!(info_path.is_file(), "source_info.json was not written: warnings {}", v["warnings"]);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&info_path).unwrap()).unwrap();
    eprintln!("SAVED title={} offset={} heatmap={} local={}", saved["title"], saved["original_offset"], saved["heatmap"], saved["heatmap_local"]);
    assert!(!saved["title"].as_str().unwrap().is_empty());
    assert!(saved["original_offset"].as_f64().unwrap() > 0.0);
    let by_id = ok(&reg, "source.info", &ctx, json!({"source_id": id})).await;
    assert_eq!(by_id["title"], saved["title"]);

    ok(&reg, "analyze.quick", &ctx, json!({"source_id": id})).await;
    let sig = ok(&reg, "highlights.signals", &ctx, json!({"source_id": id, "window": 10})).await;
    let rows = sig["rows"].as_array().unwrap();
    eprintln!("SIGNALS heatmap per window: {:?}", rows.iter().map(|r| r["heatmap"].clone()).collect::<Vec<_>>());
    assert!(rows.iter().any(|r| !r["heatmap"].is_null()), "highlights.signals shows no heatmap: {sig}");
}
