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
    let ctx = rustler_registry::Ctx::new(None, home());

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

    let pctx = rustler_registry::Ctx::new(Some(target.clone()), home());
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
    let proj = ok(&reg, "project.show", &ctx, json!({})).await;
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
