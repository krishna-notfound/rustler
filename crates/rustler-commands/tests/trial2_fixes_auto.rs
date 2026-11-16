//! Trial-2 fixes (docs/research/trial-2-2026-11-16.md, bugs 1, 2, 6, 7, 9, 10):
//! - `clips auto` names never collide, and `export clip` refuses another clip's export name;
//! - the cheap trims never cut a word and are skipped on approximate (captions) word timings,
//!   pause cuts too, and every fix reports its `status` / `changed`;
//! - `fixes_suggested` is always in the report;
//! - `export clip --render-id <older>` writes the srt of that render's state and leaves out a
//!   cover taken from another render;
//! - `clip cover` does not make the render stale for `clip retention`.
//!
//! Tests that need ffmpeg skip with a printed reason when it is missing.

mod common;

use chrono::Utc;
use common::{add_source, new_project, probe_1080p, word};
use rustler_core::{ClipId, ProjectStore, RenderId, RenderQuality, RenderRecord, SourceId, Transcript, Word, WordTiming};
use rustler_core::model::CaptionScript;
use rustler_registry::Ctx;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn transcript(store: &ProjectStore, source: &SourceId, words: Vec<Word>, timing: WordTiming) {
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: timing,
    };
    store.write_json(&Path::new("analysis").join(source.as_str()).join("transcript.json"), &t).unwrap();
}

/// Speech 0.5-12.0 s (one word every 0.5 s), a 2.5 s pause, speech 14.5-20.0 s, then silence
/// to the end of the 26 s range: retention suggests `remove_pauses` and `trim_tail`.
fn speech_with_pause_and_silent_tail() -> Vec<Word> {
    let mut out = Vec::new();
    let mut i = 0u32;
    let mut t: f64 = 0.5;
    while t < 20.0 - 1e-9 {
        if !(12.0..14.5).contains(&t) {
            let text = if (t - 19.5).abs() < 1e-9 { "have." } else { "word" };
            out.push(word(i, text, t, t + 0.45));
            i += 1;
        }
        t += 0.5;
    }
    out
}

fn fixes(c: &Value) -> Vec<Value> {
    c["fixes_applied"].as_array().cloned().unwrap_or_default()
}

/// Bugs 2, 7, 10 (no media needed: raw clips, the render stage fails without ffmpeg/media).
#[tokio::test]
async fn approximate_timings_skip_trims_and_pause_cuts_and_the_report_lists_suggestions() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(40.0, true, true));
    transcript(&store, &src.id, speech_with_pause_and_silent_tail(), WordTiming::Approximate);
    let out = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "raw": true, "ranges": [{"start": 0.0, "end": 26.0}]})).await.unwrap();
    let c = &out["clips"][0];
    assert!(c["clip_id"].is_string(), "{c:#}");
    let f = fixes(c);
    // Every silence fix that was suggested is reported as skipped with the reason, nothing cut.
    let silence: Vec<&Value> = f.iter().filter(|a| ["trim_head", "trim_tail", "remove_pauses"].contains(&a["action"].as_str().unwrap())).collect();
    assert!(silence.iter().any(|a| a["action"] == "trim_tail"), "trim_tail suggested: {c:#}");
    assert!(silence.iter().any(|a| a["action"] == "remove_pauses"), "remove_pauses suggested: {c:#}");
    for a in &silence {
        assert_eq!(a["status"], "skipped", "{a:#}");
        assert_eq!(a["changed"], false);
        assert_eq!(a["ok"], true);
        assert!(a["note"].as_str().unwrap().contains("approximate"), "{a:#}");
    }
    let clip = store.load_clip(&serde_json::from_value::<ClipId>(c["clip_id"].clone()).unwrap()).unwrap();
    assert!((clip.segments.last().unwrap().end - 26.0).abs() < 1e-6, "the tail is untouched");
    // fixes_suggested is always present; the skipped trim is still listed, saying why.
    let sugg = c["fixes_suggested"].as_array().expect("fixes_suggested present");
    assert!(sugg.iter().any(|s| s["action"] == "trim_tail" && s["why"].as_str().unwrap().contains("skipped")), "{sugg:#?}");
}

/// Bug 2 with exact timings: the tail trim stops WORD_PAD after the last word and is applied.
#[tokio::test]
async fn exact_timings_trim_the_tail_but_keep_the_last_word() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(40.0, true, true));
    let words = speech_with_pause_and_silent_tail();
    let last_end = words.last().unwrap().end;
    transcript(&store, &src.id, words, WordTiming::Exact);
    let out = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "raw": true, "ranges": [{"start": 0.0, "end": 26.0}]})).await.unwrap();
    let c = &out["clips"][0];
    let tail = fixes(c).into_iter().find(|a| a["action"] == "trim_tail").unwrap_or_else(|| panic!("trim_tail tried: {c:#}"));
    let clip = store.load_clip(&serde_json::from_value::<ClipId>(c["clip_id"].clone()).unwrap()).unwrap();
    let end = clip.segments.last().unwrap().end;
    assert!(end >= last_end + rustler_commands::auto_cmds::WORD_PAD - 1e-6, "tail {end} cuts into the last word (ends {last_end}): {tail:#}");
    match tail["status"].as_str().unwrap() {
        "applied" => assert!(end < 26.0 - 0.05 && tail["changed"] == true, "{tail:#}"),
        "rolled_back" => assert!((end - 26.0).abs() < 1e-6, "{tail:#}"),
        s => panic!("unexpected status {s}: {tail:#}"),
    }
    // remove_pauses ran on exact timings: it either cut the 2.5 s pause or says it changed nothing.
    if let Some(p) = fixes(c).into_iter().find(|a| a["action"] == "remove_pauses") {
        assert!(["applied", "no_change", "rolled_back"].contains(&p["status"].as_str().unwrap()), "{p:#}");
        assert_eq!(p["changed"], p["status"] == "applied");
    }
}

/// Bug 1: two runs on the same moment of two sources never share a clip name.
#[tokio::test]
async fn auto_names_are_unique_across_runs_and_sources() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let mut names = Vec::new();
    for _ in 0..2 {
        let src = add_source(&store, Path::new("same title.mp4"), probe_1080p(40.0, true, true));
        transcript(&store, &src.id, speech_with_pause_and_silent_tail(), WordTiming::Exact);
        let out = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "raw": true, "fixes": false, "ranges": [{"start": 3.07, "end": 20.0}]}))
            .await
            .unwrap();
        names.push(out["clips"][0]["name"].as_str().unwrap().to_string());
    }
    assert_eq!(names[0], "auto-1-00m03s-same-title-mp4");
    assert_eq!(names[1], "auto-1-00m03s-same-title-mp4-2");
}

// ---------------------------------------------------------------------------------------------
// export.clip (needs ffmpeg for the renders and QC)
// ---------------------------------------------------------------------------------------------

fn ffmpeg_render(ffmpeg: &Path, out: &Path) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let st = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "testsrc2=size=1080x1920:rate=30:duration=6"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=6", "-t", "6", "-c:v", "libx264", "-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-ar", "48000"])
        .arg(out)
        .status()
        .unwrap();
    assert!(st.success());
}

/// Adds a render record (with a real mp4) to the clip as `render.run` does: a new revision.
fn add_render(store: &ProjectStore, ffmpeg: &Path, clip: &ClipId) -> RenderId {
    let id = RenderId::new();
    let rel = PathBuf::from("clips").join(clip.as_str()).join("renders").join(format!("{id}.mp4"));
    ffmpeg_render(ffmpeg, &store.resolve(&rel));
    let mut c = store.load_clip(clip).unwrap();
    let base = c.revision;
    c.renders.push(RenderRecord { id: id.clone(), path: rel, quality: RenderQuality::Final, encoder: "libx264".into(), created_at: Utc::now(), qc: None });
    store.save_clip(&mut c, base).unwrap();
    id
}

struct ExportFx {
    _tmp: tempfile::TempDir,
    _tools: tempfile::TempDir,
    store: ProjectStore,
    ctx: Ctx,
    ffmpeg: PathBuf,
    src: SourceId,
}

fn export_fx() -> Option<ExportFx> {
    let (tools, home) = common::tools_home()?;
    let (tmp, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(60.0, true, true));
    let words = vec![word(0, "Hello", 10.2, 10.6), word(1, "there", 10.65, 11.0), word(2, "my", 11.05, 11.2), word(3, "friends.", 11.25, 11.9), word(4, "Bye", 15.0, 15.4)];
    transcript(&store, &src.id, words, WordTiming::Exact);
    Some(ExportFx { _tmp: tmp, _tools: tools, store, ctx, ffmpeg: common::ffmpeg_in(&home), src: src.id })
}

impl ExportFx {
    async fn clip(&self, reg: &rustler_registry::Registry, name: &str) -> ClipId {
        let v = common::call(reg, &self.ctx, "clip.create", json!({"name": name, "source_id": self.src, "start": 10.0, "end": 16.0, "raw": true})).await.unwrap();
        serde_json::from_value(v["id"].clone()).unwrap()
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Bug 1 (export side): another clip's export name is refused unless --overwrite; the same clip
/// re-exports freely.
#[tokio::test]
async fn export_refuses_a_name_exported_from_another_clip() {
    let Some(fx) = export_fx() else { return };
    let reg = common::registry();
    let a = fx.clip(&reg, "a").await;
    let b = fx.clip(&reg, "b").await;
    add_render(&fx.store, &fx.ffmpeg, &a);
    add_render(&fx.store, &fx.ffmpeg, &b);
    common::call(&reg, &fx.ctx, "export.clip", json!({"clip_id": a, "name": "shared", "allow_fail": true})).await.unwrap();
    // Same clip again: fine.
    common::call(&reg, &fx.ctx, "export.clip", json!({"clip_id": a, "name": "shared", "allow_fail": true})).await.unwrap();
    let report = fx.store.exports_dir().join("shared.report.json");
    let e = common::call(&reg, &fx.ctx, "export.clip", json!({"clip_id": b, "name": "shared", "allow_fail": true})).await.unwrap_err();
    assert_eq!(common::code(&e), rustler_core::ErrorCode::Validation);
    assert!(e.message.contains(a.as_str()) && e.hint.clone().unwrap_or_default().contains("--overwrite"), "{e:?}");
    assert!(read(&report).contains(a.as_str()), "a's export is untouched");
    let out = common::call(&reg, &fx.ctx, "export.clip", json!({"clip_id": b, "name": "shared", "allow_fail": true, "overwrite": true})).await.unwrap();
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("replaced")), "{out:#}");
    assert!(read(&report).contains(b.as_str()));
}

/// Bugs 6 and 9: an older render exports with its own captions and without the newer render's
/// cover; picking a cover does not make the render stale for retention.
#[tokio::test]
async fn older_render_exports_its_own_srt_and_no_foreign_cover() {
    let Some(fx) = export_fx() else { return };
    let reg = common::registry();
    let id = fx.clip(&reg, "talk").await;
    let old = add_render(&fx.store, &fx.ffmpeg, &id);
    // Edit after the first render: the clip now starts at 11.0 s ("Hello there" is gone).
    common::call(&reg, &fx.ctx, "edit.trim", json!({"clip_id": id, "start_delta": 1.0})).await.unwrap();
    let new = add_render(&fx.store, &fx.ffmpeg, &id);
    common::call(&reg, &fx.ctx, "clip.cover", json!({"clip_id": id, "out_t": 1.0})).await.unwrap();

    // Bug 9: the cover is not an edit; retention still measures the render.
    let r = common::call(&reg, &fx.ctx, "clip.retention", json!({"clip_id": id, "per_second": false})).await.unwrap();
    assert_eq!(r["measured_on"], "render", "{:#}", r["warnings"]);
    assert_eq!(r["render_id"], json!(new));

    let dir = fx.store.exports_dir();
    let out = common::call(&reg, &fx.ctx, "export.clip", json!({"clip_id": id, "render_id": old, "name": "old", "allow_fail": true})).await.unwrap();
    let srt = read(&dir.join("old.srt"));
    assert!(srt.contains("Hello"), "the old render's captions: {srt}");
    assert!(!dir.join("old.cover.jpg").exists(), "the cover of the newer render is left out");
    let post: Value = serde_json::from_str(&read(&dir.join("old.post.json"))).unwrap();
    assert!(post["cover"].is_null(), "{post:#}");
    let warns: Vec<&str> = out["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(warns.iter().any(|w| w.contains("no cover exported")), "{warns:?}");
    assert!(warns.iter().any(|w| w.contains("made before later edits")), "{warns:?}");
    let report: Value = serde_json::from_str(&read(&dir.join("old.report.json"))).unwrap();
    assert!((report["clip"]["segments"][0]["start"].as_f64().unwrap() - 10.0).abs() < 1e-6, "report describes the rendered clip");

    // The latest render: current captions and its cover.
    common::call(&reg, &fx.ctx, "export.clip", json!({"clip_id": id, "name": "new", "allow_fail": true})).await.unwrap();
    let srt = read(&dir.join("new.srt"));
    assert!(!srt.contains("Hello") && srt.contains("my"), "{srt}");
    assert!(dir.join("new.cover.jpg").is_file());
}
