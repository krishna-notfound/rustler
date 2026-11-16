//! clips.auto follow-ups after v0.2.1:
//! 1. clips auto never adds a zoom itself; the template's own punch-ins are re-checked against
//!    the cuts the pause removal made (>= 0.5 s from every cut) and re-planned with the template's
//!    pattern when one breaks the rule (undo with `history restore`).
//! 2. whisper fallback for a URL without captions: the "no captions" error is recognised, and the
//!    audio-only transcription it runs gives a source clips auto can rank (a local file stands in
//!    for the URL download; needs the engine, skips otherwise).
//! 3. clip.cover --auto leaves out frames where a mask is shown, and reuses the QC contact sheet.
//!
//! Tests that need ffmpeg or the engine skip with a printed reason when they are missing.

mod common;
mod support;

use common::{add_source, new_project, probe_1080p, word};
use rustler_commands::auto_cmds::{self, punch_violations};
use rustler_commands::template_cmds::BY_TEMPLATE;
use rustler_core::model::CaptionScript;
use rustler_core::timeline::EffectKind;
use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError, SourceId, Transcript, Word, WordTiming};
use rustler_registry::{Ctx, Registry};
use serde_json::{Value, json};
use std::path::Path;

fn transcript(store: &ProjectStore, source: &SourceId, words: Vec<Word>) {
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    store.write_json(&Path::new("analysis").join(source.as_str()).join("transcript.json"), &t).unwrap();
}

/// Speech 0.5-12.0 s (a word every 0.5 s), a 2.5 s pause, speech 14.5-20.0 s, silence to 26 s.
fn speech_with_pause() -> Vec<Word> {
    let mut out = Vec::new();
    let mut i = 0u32;
    let mut t: f64 = 0.5;
    while t < 20.0 - 1e-9 {
        if !(12.0..14.5).contains(&t) {
            out.push(word(i, if i % 3 == 0 { "money" } else { "word" }, t, t + 0.45));
            i += 1;
        }
        t += 0.5;
    }
    out
}

/// A project template with only a planned punch-in pattern.
fn write_punch_template(ctx: &Ctx) {
    let dir = ctx.project_dir.as_ref().unwrap().join("templates");
    std::fs::create_dir_all(&dir).unwrap();
    let tpl = json!({
        "schema": "rustler.template@1", "name": "test-punch", "version": 1,
        "effects": [{"kind": "punch-in", "at": "emphasis", "duration": 2.0, "amount": 1.08, "amount_max": 1.2, "per_10s": 5.0}]
    });
    std::fs::write(dir.join("test-punch@1.json"), serde_json::to_string_pretty(&tpl).unwrap()).unwrap();
}

/// (id, start, end, template-tagged) of the zoom-type effects of a clip.
fn zooms(store: &ProjectStore, id: &ClipId) -> Vec<(String, f64, f64, bool)> {
    let (t, _) = rustler_commands::timeline_doc::assemble(store, id).unwrap();
    t.tracks.effects.iter().filter(|e| e.kind != EffectKind::Flash).map(|e| (e.id.clone(), e.start, e.end, e.extra.contains_key(BY_TEMPLATE))).collect()
}

/// Template punch-ins of the clip that break the planner rules on its current timeline.
fn violations(store: &ProjectStore, id: &ClipId) -> Vec<(String, String)> {
    let (mut t, _) = rustler_commands::timeline_doc::assemble(store, id).unwrap();
    let punches: Vec<(String, f64, f64)> =
        t.tracks.effects.iter().filter(|e| e.kind == EffectKind::PunchIn && e.extra.contains_key(BY_TEMPLATE)).map(|e| (e.id.clone(), e.start, e.end)).collect();
    t.tracks.effects.retain(|e| !(e.kind == EffectKind::PunchIn && e.extra.contains_key(BY_TEMPLATE)));
    let (input, _) = rustler_commands::timeline_doc::punch_input(store, &t);
    punch_violations(&punches, &input, 0.5)
}

fn clip_id(v: &Value) -> ClipId {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn violations_are_near_or_across_cuts_and_on_blocked_spans() {
    let input = rustler_media::punchin::PunchInput { duration: 30.0, cuts: vec![10.0, 20.0], blocked: vec![(25.0, 28.0)], ..Default::default() };
    let p = |id: &str, s: f64, e: f64| (id.to_string(), s, e);
    let bad = punch_violations(&[p("fx_1", 2.0, 4.0), p("fx_2", 8.0, 9.7), p("fx_3", 9.0, 11.0), p("fx_4", 10.3, 12.0), p("fx_5", 14.0, 16.0), p("fx_6", 24.0, 25.5)], &input, 0.5);
    let ids: Vec<&str> = bad.iter().map(|b| b.0.as_str()).collect();
    assert_eq!(ids, vec!["fx_2", "fx_3", "fx_4", "fx_6"], "{bad:#?}");
    assert!(bad[1].1.contains("across the cut at 10.00"), "{}", bad[1].1);
    assert!(bad[0].1.contains("0.30 s from the cut"), "{}", bad[0].1);
}

#[test]
fn no_captions_is_recognised_and_other_errors_are_not() {
    let v = |m: &str| RustlerError::new(ErrorCode::Validation, "source.fetch_transcript", m);
    assert!(auto_cmds::no_captions(&v("this video has no captions")));
    assert!(auto_cmds::no_captions(&v("no captions for language `de`; available: en, hi")));
    assert!(auto_cmds::no_captions(&RustlerError::new(ErrorCode::Provider, "source.fetch_transcript", "yt-dlp wrote no json3 captions for `en`")));
    assert!(!auto_cmds::no_captions(&RustlerError::new(ErrorCode::Provider, "yt-dlp", "yt-dlp failed: HTTP Error 403")));
    assert!(!auto_cmds::no_captions(&RustlerError::new(ErrorCode::DependencyMissing, "yt-dlp", "cannot start yt-dlp")));
    assert!(!auto_cmds::no_captions(&v("give exactly one of url or source_id")));
}

/// A template punch-in that ends just before a pause is < 0.5 s from the cut once the pause is
/// removed; the re-check re-plans the template punch-ins (all >= 0.5 s from cuts) and the undo
/// command brings the clip back.
#[tokio::test]
async fn template_punch_ins_are_rechecked_after_pause_cuts() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    write_punch_template(&ctx);
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(40.0, true, true));
    transcript(&store, &src.id, speech_with_pause());
    let v = common::call(&reg, &ctx, "clip.create", json!({"name": "t", "source_id": src.id, "start": 0.0, "end": 20.5, "raw": true, "template": "test-punch"})).await.unwrap();
    let id = clip_id(&v["id"]);
    // A template punch-in that ends 0.1 s before the pause (as the planner may place it on the uncut clip).
    let (t, _) = rustler_commands::timeline_doc::assemble(&store, &id).unwrap();
    let tag = t.tracks.effects.iter().find(|e| e.extra.contains_key(BY_TEMPLATE)).map(|e| e.extra.clone());
    let mut keep: Vec<Value> = t.tracks.effects.iter().filter(|e| e.end < 9.4 || e.start > 15.0).map(|e| serde_json::to_value(e).unwrap()).collect();
    let mut fx = json!({"id": "fx_test", "kind": "punch-in", "start": 10.0, "end": 11.9, "amount": 1.1, BY_TEMPLATE: "test-punch@1", "template_role": "effect"});
    if let Some(tag) = tag {
        fx[BY_TEMPLATE] = tag[BY_TEMPLATE].clone();
    }
    keep.push(fx);
    common::call(&reg, &ctx, "timeline.apply", json!({"clip_id": id, "base_revision": t.revision, "patch": [{"op": "replace", "path": "/tracks/effects", "value": keep}]})).await.unwrap();
    assert!(violations(&store, &id).is_empty(), "no cut yet");

    let cut = common::call(&reg, &ctx, "edit.remove_pauses", json!({"clip_id": id})).await.unwrap();
    let before = violations(&store, &id);
    assert!(!before.is_empty(), "the pause cut lands next to the punch-in: {cut:#}\n{:#?}", zooms(&store, &id));
    let rev_cut = store.load_clip(&id).unwrap().revision;

    let chk = auto_cmds::recheck_template_punch_ins_for(&ctx, &id).await.unwrap().expect("template punch-ins checked");
    let chk = serde_json::to_value(&chk).unwrap();
    assert_eq!(chk["action"], auto_cmds::TEMPLATE_PUNCH_CHECK);
    assert_eq!(chk["status"], "applied", "{chk:#}");
    assert!(chk["note"].as_str().unwrap().contains("fx_test"), "{chk:#}");
    assert!(violations(&store, &id).is_empty(), "after the re-plan: {:#?}", zooms(&store, &id));
    let after = zooms(&store, &id);
    assert!(after.iter().all(|z| z.3), "only template zooms: {after:#?}");
    assert!(!after.is_empty(), "the pattern placed punch-ins again");
    // Checked again: nothing to do.
    let again = serde_json::to_value(auto_cmds::recheck_template_punch_ins_for(&ctx, &id).await.unwrap().unwrap()).unwrap();
    assert_eq!(again["status"], "no_change", "{again:#}");
    // The undo brings back the clip as it was after the cut.
    assert!(chk["undo"].as_str().unwrap().contains(&format!("history restore {id} {rev_cut}")), "{chk:#}");
    common::call(&reg, &ctx, "history.restore", json!({"clip_id": id, "revision": rev_cut})).await.unwrap();
    assert!(zooms(&store, &id).iter().any(|z| z.0 == "fx_test"), "restored");
}

/// clips auto applies the silence fixes but never a zoom: every zoom effect on the clip is the
/// template's, each >= 0.5 s from every cut; retention punch-ins are only suggested.
#[tokio::test]
async fn clips_auto_adds_no_zooms_and_keeps_template_punch_ins_off_cuts() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    write_punch_template(&ctx);
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(40.0, true, true));
    transcript(&store, &src.id, speech_with_pause());
    for template in [Some("test-punch"), None] {
        let mut input = json!({"source_id": src.id, "raw": true, "cover": false, "ranges": [{"start": 0.0, "end": 26.0}]});
        if let Some(t) = template {
            input["template"] = json!(t);
        }
        let out = common::call(&reg, &ctx, "clips.auto", input).await.unwrap();
        let c = &out["clips"][0];
        let id = clip_id(&c["clip_id"]);
        let z = zooms(&store, &id);
        assert!(z.iter().all(|z| z.3), "no zoom without by_template ({template:?}): {z:#?}");
        assert!(violations(&store, &id).is_empty(), "{template:?}: {z:#?}");
        let applied = c["fixes_applied"].as_array().unwrap();
        assert!(!applied.iter().any(|a| a["action"] == "punch_in"), "{applied:#?}");
        if template.is_none() {
            assert!(z.is_empty(), "{z:#?}");
        } else if !z.is_empty() {
            assert!(applied.iter().any(|a| a["action"] == auto_cmds::TEMPLATE_PUNCH_CHECK), "{applied:#?}");
        }
        for s in c["fixes_suggested"].as_array().unwrap().iter().filter(|s| s["action"] == "punch_in") {
            assert!(s["command"].as_str().unwrap().contains("effect add"), "{s:#}");
            assert!(s["why"].as_str().unwrap().contains("never adds zooms"), "{s:#}");
        }
        assert_eq!(out["source"]["transcript_via"], "source");
    }
}

/// The whisper fallback on a local file standing in for the URL's audio download.
#[tokio::test]
async fn whisper_fallback_transcribes_audio_that_clips_auto_ranks() {
    if !support::have_media_tools() || !support::have_engine() {
        return;
    }
    let media = support::home().join("test-media").join("boyle_twoshot_105s.mp4");
    if !media.is_file() {
        eprintln!("SKIP: {} missing", media.display());
        return;
    }
    let reg: Registry = support::registry();
    let (_dir, ctx) = support::project();
    let t = std::time::Instant::now();
    let (sid, warnings) = auto_cmds::transcribe_for_ranking(&reg, &ctx, media.to_str().unwrap(), "talk (audio for ranking)").await.unwrap();
    let took = t.elapsed().as_secs_f64();
    eprintln!("whisper fallback (import + analyze quick --transcribe always --model base) on 105 s: {took:.1} s; warnings {warnings:?}");
    let store = ctx.store().unwrap();
    let tr: Transcript = store.read_json(&Path::new("analysis").join(&sid).join("transcript.json")).unwrap();
    assert!(tr.words.len() > 50, "{} words", tr.words.len());
    assert_eq!(tr.word_timing, WordTiming::Exact);
    let plan = support::ok(&reg, "clips.auto", &ctx, json!({"source_id": sid, "count": 2, "min_len": 15, "max_len": 35, "dry_run": true})).await;
    assert!(!plan["clips"].as_array().unwrap().is_empty(), "{plan:#}");
}

/// cover --auto: one ffmpeg call over the render, frames with a mask shown are left out; with a
/// QC contact sheet the sheet's tiles are scored instead (no decode).
#[tokio::test]
async fn cover_leaves_out_masked_frames_and_reuses_the_qc_sheet() {
    let Some((_tools, home)) = common::tools_home() else { return };
    let reg = common::registry();
    let (_tmp, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(60.0, true, true));
    transcript(&store, &src.id, speech_with_pause());
    let v = common::call(&reg, &ctx, "clip.create", json!({"name": "c", "source_id": src.id, "start": 10.0, "end": 16.0, "raw": true})).await.unwrap();
    let id = clip_id(&v["id"]);
    // A 6 s render (testsrc2) recorded as render.run does.
    let rid = rustler_core::RenderId::new();
    let rel = std::path::PathBuf::from("clips").join(id.as_str()).join("renders").join(format!("{rid}.mp4"));
    std::fs::create_dir_all(store.resolve(&rel).parent().unwrap()).unwrap();
    common::make_video(&common::ffmpeg_in(&home), &store.resolve(&rel), 6, 1080, 1920, true);
    let mut c = store.load_clip(&id).unwrap();
    let base = c.revision;
    c.renders.push(rustler_core::RenderRecord { id: rid, path: rel, quality: rustler_core::RenderQuality::Final, encoder: "libx264".into(), created_at: chrono::Utc::now(), qc: None });
    store.save_clip(&mut c, base).unwrap();
    common::call(&reg, &ctx, "mask.add", json!({"clip_id": id, "kind": "blur", "rect": {"x": 0.1, "y": 0.1, "w": 0.8, "h": 0.6}, "start": 0.0, "end": 4.0})).await.unwrap();

    let out = common::call(&reg, &ctx, "clip.cover", json!({"clip_id": id, "auto": true})).await.unwrap();
    eprintln!("cover (render frames): {:.2} s, {} frames, picked {}", out["elapsed_s"], out["frames_scored"], out["out_t"]);
    assert_eq!(out["frames_from"], "render");
    assert_eq!(out["frames_scored"], auto_cmds_samples());
    assert!(out["out_t"].as_f64().unwrap() >= 4.0, "a frame without the mask: {out:#}");
    let left: Vec<&str> = out["left_out"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(left.iter().any(|l| l.contains("mask")), "{left:?}");
    for cand in out["candidates"].as_array().unwrap() {
        assert!(!cand["overlays"].as_array().is_some_and(|o| o.iter().any(|k| k == "mask")), "{cand:#}");
    }

    // QC makes a contact sheet; cover then scores its tiles.
    match common::call(&reg, &ctx, "qc.run", json!({"clip_id": id, "allow_fail": true})).await {
        Ok(_) => {
            let out = common::call(&reg, &ctx, "clip.cover", json!({"clip_id": id, "auto": true})).await.unwrap();
            eprintln!("cover (QC sheet): {:.2} s, {} frames, picked {}", out["elapsed_s"], out["frames_scored"], out["out_t"]);
            assert_eq!(out["frames_from"], "qc_contact_sheet", "{out:#}");
            assert_eq!(out["frames_scored"], 12);
            assert!(out["out_t"].as_f64().unwrap() >= 4.0, "{out:#}");
        }
        Err(e) => eprintln!("SKIP qc sheet part: {}", e.message),
    }
}

fn auto_cmds_samples() -> usize {
    rustler_commands::cover_cmds::SAMPLES
}
