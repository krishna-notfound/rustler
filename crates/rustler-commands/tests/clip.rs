//! clip.*, caption.*, layout.*, audio.* command tests (no external tools needed).
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};
use std::path::Path;

fn approx(a: &Value, b: f64) {
    let a = a.as_f64().unwrap();
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

struct Fx {
    _tmp: tempfile::TempDir,
    store: rustler_core::ProjectStore,
    ctx: rustler_registry::Ctx,
    src: rustler_core::Source,
    reg: rustler_registry::Registry,
}

fn fx() -> Fx {
    let (tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("C:/nonexistent/video.mp4"), probe_1080p(60.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    Fx { _tmp: tmp, store, ctx, src, reg: registry() }
}

async fn create(f: &Fx, extra: Value) -> rustler_core::Result<Value> {
    let mut input = json!({"name": "c1", "source_id": f.src.id});
    input.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    call(&f.reg, &f.ctx, "clip.create", input).await
}

#[tokio::test]
async fn create_by_units_pads_and_snaps() {
    let f = fx();
    // unit 1 = words 3..6 (3.0 - 5.0); previous word ends 2.3, next word starts 5.05
    let v = create(&f, json!({"start_unit": 1, "end_unit": 1})).await.unwrap();
    let seg = &v["segments"][0];
    approx(&seg["start"], 2.92); // 3.0 - 0.08
    approx(&seg["end"], 5.05); // 5.0 + 0.15 would pass the next word -> snapped
    approx(&v["duration"], 5.05 - 2.92);
    assert_eq!(v["captions"]["enabled"], true);
    assert_eq!(v["captions"]["script"], "latin");
    assert_eq!(seg["layout"]["preset"], "center-crop");
    let next = v["next"].as_array().unwrap();
    assert!(next.iter().any(|n| n.as_str().unwrap().contains("render preview-frame")));
    assert!(next.iter().any(|n| n.as_str().unwrap().contains("render run")));

    // first unit: lead-in is limited by 0 / previous word, tail gets the full 0.15
    let v = create(&f, json!({"start_unit": 0, "end_unit": 0})).await.unwrap();
    approx(&v["segments"][0]["start"], 0.92);
    approx(&v["segments"][0]["end"], 2.45);

    // persisted: project lists it, clip.show / clip.list work, event logged
    let clip_id = v["id"].as_str().unwrap().to_string();
    let p = f.store.load_project().unwrap();
    assert_eq!(p.clips.len(), 2);
    let shown = call(&f.reg, &f.ctx, "clip.show", json!({"clip_id": clip_id})).await.unwrap();
    assert_eq!(shown["timeline"][0]["out_start"], 0.0);
    assert!(shown["summary"].as_str().unwrap().contains("1 segment"));
    let list = call(&f.reg, &f.ctx, "clip.list", json!({})).await.unwrap();
    assert_eq!(list["clips"].as_array().unwrap().len(), 2);
    let events = std::fs::read_to_string(f.store.root.join("events.ndjson")).unwrap();
    assert_eq!(events.lines().filter(|l| l.contains("\"clip.create\"")).count(), 2);
}

#[tokio::test]
async fn create_options_caption_style_and_layout_params() {
    let f = fx();
    let v = create(
        &f,
        json!({"start_unit": 1, "layout": "center-crop", "layout_params": {"focus_x": 0.25}, "caption_style": "hormozi-pop"}),
    )
    .await
    .unwrap();
    assert_eq!(v["captions"]["preset"], "hormozi-pop");
    let src = &v["segments"][0]["layout"]["regions"][0]["src"];
    // 606/1920 wide crop centered at 25% => left edge ~0.0936
    assert!(src["x"].as_f64().unwrap() < 0.12 && src["x"].as_f64().unwrap() > 0.05, "{src}");

    let e = create(&f, json!({"start_unit": 1, "caption_style": "nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("hormozi-pop"));

    let v = create(
        &f,
        json!({"start": 10.0, "end": 20.0, "layout": "split-vertical",
               "layout_params": {"top": {"x":0.25,"y":0.0,"w":0.5,"h":0.6}, "bottom": {"x":0.0,"y":0.6,"w":1.0,"h":0.4}}}),
    )
    .await
    .unwrap();
    assert_eq!(v["segments"][0]["layout"]["regions"].as_array().unwrap().len(), 2);
    let e = create(&f, json!({"start": 10.0, "end": 20.0, "layout": "split-vertical"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn expect_text_mismatch_validation() {
    let f = fx();
    create(&f, json!({"start_unit": 1, "expect_text": "fellow americans"})).await.unwrap();
    let e = create(&f, json!({"start_unit": 1, "expect_text": "hello there"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("opening words do not match"), "{}", e.message);
    assert!(e.message.contains("Fellow Americans"), "{}", e.message);
    // nothing was created by the failed call
    assert_eq!(f.store.load_project().unwrap().clips.len(), 1);
}

#[tokio::test]
async fn create_without_units_hints_transcribe() {
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("C:/nonexistent/v.mp4"), probe_1080p(60.0, true, true));
    let reg = registry();
    let e = call(&reg, &ctx, "clip.create", json!({"name": "x", "source_id": src.id, "start_unit": 0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.as_deref().unwrap().contains("analyze transcribe"), "{e:?}");
    assert!(e.hint.as_deref().unwrap().contains("--start"), "{e:?}");

    // empty transcript behaves the same for unit mode ...
    write_transcript(&store, &src.id, vec![]);
    let e = call(&reg, &ctx, "clip.create", json!({"name": "x", "source_id": src.id, "start_unit": 0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    // ... but start/end works and captions are auto-disabled with a warning
    let v = call(&reg, &ctx, "clip.create", json!({"name": "x", "source_id": src.id, "start": 1.0, "end": 5.0})).await.unwrap();
    assert_eq!(v["captions"]["enabled"], false);
    assert!(v["warnings"][0].as_str().unwrap().contains("captions disabled"));
    // and no range at all is a validation error
    let e = call(&reg, &ctx, "clip.create", json!({"name": "x", "source_id": src.id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn time_range_validation() {
    let f = fx();
    for (s, e) in [(5.0, 5.0), (10.0, 5.0), (-1.0, 5.0), (50.0, 70.0)] {
        let err = create(&f, json!({"start": s, "end": e})).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{s}..{e}");
    }
    let err = create(&f, json!({"start_unit": 99})).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    let err = create(&f, json!({"end_unit": 1})).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

#[tokio::test]
async fn audio_only_source_rejected() {
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("C:/nonexistent/podcast.mp3"), probe_1080p(120.0, false, true));
    write_transcript(&store, &src.id, sample_words());
    let e = call(&registry(), &ctx, "clip.create", json!({"name": "x", "source_id": src.id, "start_unit": 0}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("no video"), "{}", e.message);
}

#[tokio::test]
async fn stale_revision_conflict() {
    let f = fx();
    let v = create(&f, json!({"start_unit": 1})).await.unwrap();
    let id = v["id"].as_str().unwrap();
    let rev = v["revision"].as_u64().unwrap();
    let a = call(&f.reg, &f.ctx, "clip.set_captions", json!({"clip_id": id, "enabled": false, "base_revision": rev}))
        .await
        .unwrap();
    assert_eq!(a["revision"].as_u64().unwrap(), rev + 1);
    assert_eq!(a["captions"]["enabled"], false);
    let e = call(&f.reg, &f.ctx, "clip.set_captions", json!({"clip_id": id, "position_y": 0.5, "base_revision": rev}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::RevisionConflict);
    // the losing change was not applied
    let c = f.store.load_clip(&v["id"].as_str().unwrap().parse().unwrap()).unwrap();
    assert!((c.captions.position_y - 0.70).abs() < 1e-9);
}

#[tokio::test]
async fn caption_set_alias_styles_and_srt() {
    let f = fx();
    let v = create(&f, json!({"start_unit": 1, "end_unit": 2})).await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();

    let styles = call(&f.reg, &f.ctx, "caption.list_styles", json!({})).await.unwrap();
    let names: Vec<&str> = styles["styles"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"bold-karaoke") && names.contains(&"hormozi-pop"), "{names:?}");

    // both names work and `preset` is an alias of `style`
    let a = call(&f.reg, &f.ctx, "caption.set", json!({"clip_id": id, "style": "clean-white", "max_words_per_line": 2}))
        .await
        .unwrap();
    assert_eq!(a["captions"]["preset"], "clean-white");
    assert_eq!(a["captions"]["max_words_per_line"], 2);
    let b = call(&f.reg, &f.ctx, "clip.set_captions", json!({"clip_id": id, "preset": "boxed-highlight", "position_y": 0.6}))
        .await
        .unwrap();
    assert_eq!(b["captions"]["preset"], "boxed-highlight");
    approx(&b["captions"]["position_y"], 0.6);
    for bad in [json!({"style": "zzz"}), json!({"position_y": 2.0}), json!({"max_words_per_line": 0})] {
        let mut input = json!({"clip_id": id});
        input.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        assert_eq!(call(&f.reg, &f.ctx, "caption.set", input).await.unwrap_err().code, ErrorCode::Validation);
    }

    let out = call(&f.reg, &f.ctx, "caption.export_srt", json!({"clip_id": id})).await.unwrap();
    let srt = std::fs::read_to_string(out["path"].as_str().unwrap()).unwrap();
    assert!(srt.contains("-->"), "{srt}");
    assert!(srt.contains("Fellow"), "{srt}");
    assert!(!srt.contains("Hello"), "words outside the clip must not appear: {srt}");
    assert!(out["path"].as_str().unwrap().ends_with("captions.srt"));
}

#[tokio::test]
async fn layout_presets_set_layout_and_add_segment() {
    let f = fx();
    let presets = call(&f.reg, &f.ctx, "layout.list_presets", json!({})).await.unwrap();
    let names: Vec<&str> = presets["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["center-crop", "blur-fit", "split-vertical"]);
    assert!(presets["presets"][2]["params"].as_array().unwrap().len() >= 2);

    let v = create(&f, json!({"start_unit": 1})).await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let w = call(&f.reg, &f.ctx, "clip.add_segment", json!({"clip_id": id, "source_id": f.src.id, "start_unit": 2})).await.unwrap();
    assert_eq!(w["segments"].as_array().unwrap().len(), 2);
    let d1 = w["segments"][0]["end"].as_f64().unwrap() - w["segments"][0]["start"].as_f64().unwrap();
    let d2 = w["segments"][1]["end"].as_f64().unwrap() - w["segments"][1]["start"].as_f64().unwrap();
    approx(&w["duration"], d1 + d2);

    // all segments -> blur-fit
    let s = call(&f.reg, &f.ctx, "clip.set_layout", json!({"clip_id": id, "preset": "blur-fit"})).await.unwrap();
    assert!(s["segments"].as_array().unwrap().iter().all(|g| g["layout"]["preset"] == "blur-fit"));
    // one segment -> center-crop
    let seg0 = s["segments"][0]["id"].as_str().unwrap().to_string();
    let s = call(&f.reg, &f.ctx, "clip.set_layout", json!({"clip_id": id, "segment_id": seg0, "preset": "center-crop"})).await.unwrap();
    assert_eq!(s["segments"][0]["layout"]["preset"], "center-crop");
    assert_eq!(s["segments"][1]["layout"]["preset"], "blur-fit");
    let e = call(&f.reg, &f.ctx, "clip.set_layout", json!({"clip_id": id, "preset": "nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = call(&f.reg, &f.ctx, "clip.set_layout", json!({"clip_id": id, "segment_id": "seg_missing", "preset": "blur-fit"}))
        .await
        .unwrap_err();
    assert!(matches!(e.code, ErrorCode::NotFound | ErrorCode::Validation), "{e:?}");
}

#[tokio::test]
async fn music_bed_is_stored_in_extras_and_crop_commands_validate() {
    let f = fx();
    let v = create(&f, json!({"start_unit": 1})).await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let music = f._tmp.path().join("bed.mp3");
    std::fs::write(&music, b"not really audio").unwrap();

    let m = call(&f.reg, &f.ctx, "audio.add_music", json!({"clip_id": id, "path": music, "volume_db": -22.0})).await.unwrap();
    approx(&m["music"]["volume_db"], -22.0);
    assert_eq!(m["music"]["duck"], true);
    let extras: Value = serde_json::from_slice(&std::fs::read(f.store.root.join("clips").join(&id).join("extras.json")).unwrap()).unwrap();
    assert!(extras["music"]["path"].as_str().unwrap().ends_with("bed.mp3"));
    assert!(extras.get("crop_paths").is_none(), "no empty crop_paths written");
    // clip.show surfaces it; adding music is one new revision (history undo removes it)
    let shown = call(&f.reg, &f.ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    assert!(shown["extras"]["music"].is_object());
    assert_eq!(shown["clip"]["revision"].as_u64(), v["revision"].as_u64().map(|r| r + 1));

    let e = call(&f.reg, &f.ctx, "audio.add_music", json!({"clip_id": id, "path": f._tmp.path().join("missing.mp3")})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = call(&f.reg, &f.ctx, "audio.add_music", json!({"clip_id": id, "path": music, "volume_db": 40.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    call(&f.reg, &f.ctx, "audio.remove_music", json!({"clip_id": id})).await.unwrap();
    let shown = call(&f.reg, &f.ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    assert!(shown["extras"]["music"].is_null());

    // crop.track_face needs a cropping center-crop layout; the 16:9 -> 9:16 default qualifies, blur-fit does not
    call(&f.reg, &f.ctx, "clip.set_layout", json!({"clip_id": id, "preset": "blur-fit"})).await.unwrap();
    let e = call(&f.reg, &f.ctx, "crop.track_face", json!({"clip_id": id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("center-crop"));
    // A clip mixing center-crop and blur-fit segments: the whole-clip call tracks the center-crop
    // ones and skips the rest instead of failing (dry run: no engine needed).
    let w = call(&f.reg, &f.ctx, "clip.add_segment", json!({"clip_id": id, "source_id": f.src.id, "start_unit": 2, "layout": "center-crop"}))
        .await
        .unwrap();
    assert_eq!(w["segments"][1]["layout"]["preset"], "center-crop");
    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    let t = call(&f.reg, &dry, "crop.track_face", json!({"clip_id": id})).await.unwrap();
    assert_eq!(t["dry_run"], true);
    let c = call(&f.reg, &f.ctx, "crop.clear", json!({"clip_id": id})).await.unwrap();
    assert_eq!(c["cleared"], 0);
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let mut f = fx();
    f.ctx.dry_run = true;
    let v = create(&f, json!({"start_unit": 1})).await.unwrap();
    assert_eq!(v["dry_run"], true);
    assert!(f.store.load_project().unwrap().clips.is_empty());
    assert!(!f.store.root.join("clips").join(v["id"].as_str().unwrap()).exists());
}
