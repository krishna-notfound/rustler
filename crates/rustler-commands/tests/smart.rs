//! Smart defaults at clip.create, layout.plan and crop.track_active.
//!
//! The engine tests need uv + the repo engine, ffmpeg and `~/.rustler/test-media/lincoln.jpg`
//! (public domain); they skip with a printed reason otherwise.

mod common;
mod support;

use rustler_core::{ProjectStore, SourceId, Word};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// --- no engine needed ---------------------------------------------------------------------------

#[test]
fn filler_files_load() {
    let en = rustler_commands::smart_cmds::builtin_fillers(false);
    assert!(en.plain.len() >= 7, "{en:?}");
    assert!(en.guarded.iter().any(|p| p.join(" ") == "you know"));
    let hi = rustler_commands::smart_cmds::builtin_fillers(true);
    assert!(hi.guarded.iter().any(|p| p.join(" ") == "matlab"));
    assert!(hi.guarded.len() > en.guarded.len());
}

fn words_with_filler_and_pause() -> Vec<Word> {
    let w = common::word;
    vec![
        w(0, "So", 1.0, 1.2),
        w(1, "um,", 1.25, 1.6),
        w(2, "this", 1.65, 1.9),
        w(3, "works.", 1.95, 2.4),
        // 2.1 s pause
        w(4, "Then", 4.5, 4.8),
        w(5, "it", 4.85, 5.0),
        w(6, "ends.", 5.05, 5.6),
    ]
}

fn fake_source(store: &ProjectStore, dir: &Path) -> SourceId {
    // the media file does not exist: layout analysis is skipped, cleanup still runs
    let src = common::add_source(store, &dir.join("missing.mp4"), common::probe_1080p(60.0, true, true));
    common::write_transcript(store, &src.id, words_with_filler_and_pause());
    src.id
}

fn seg_ranges(v: &Value) -> Vec<(f64, f64)> {
    v["segments"].as_array().unwrap().iter().map(|s| (s["start"].as_f64().unwrap(), s["end"].as_f64().unwrap())).collect()
}

#[tokio::test]
async fn clip_create_trims_fillers_and_pauses_and_lists_undo_commands() {
    let (tmp, store, ctx) = common::new_project();
    let reg = common::registry();
    let src = fake_source(&store, tmp.path());
    let v = common::call(&reg, &ctx, "clip.create", json!({"name": "c", "source_id": src, "start": 1.0, "end": 5.6})).await.unwrap();
    let smart = &v["smart"];
    assert_eq!(smart["applied"], true, "{v}");
    let stages: Vec<&str> = smart["steps"].as_array().unwrap().iter().map(|s| s["stage"].as_str().unwrap()).collect();
    assert_eq!(stages, vec!["fillers", "pauses"], "{smart}");
    assert_eq!(v["revision"], 3);
    // "um," cut out, the 2.1 s pause cut to 0.15 s
    let r = seg_ranges(&v);
    assert_eq!(r.len(), 3, "{r:?}");
    assert!((r[0].1 - 1.25).abs() < 1e-6 && (r[1].0 - 1.6).abs() < 1e-6, "{r:?}");
    assert!((r[1].1 - 2.475).abs() < 1e-6 && (r[2].0 - 4.425).abs() < 1e-6, "{r:?}");
    let d = smart["decisions"].as_array().unwrap();
    let kinds: Vec<&str> = d.iter().map(|x| x["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"filler") && kinds.contains(&"pause") && kinds.contains(&"skipped"), "{kinds:?}");
    for x in d.iter().filter(|x| x["kind"] == "filler" || x["kind"] == "pause") {
        let cmds: Vec<&str> = x["override_with"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        assert!(cmds.iter().any(|c| c.starts_with("rustler history restore ")), "{x}");
        assert!(cmds.iter().any(|c| c.starts_with("rustler edit trim ") && c.contains("--segment-id seg_")), "{x}");
    }
    // the per-item override really puts the pause back: extend the piece before it
    let pause = d.iter().find(|x| x["kind"] == "pause").unwrap();
    let seg = pause["segment_ids"][0].as_str().unwrap();
    let end = pause["source_end"].as_f64().unwrap();
    let t = common::call(&reg, &ctx, "edit.trim", json!({"clip_id": v["id"], "segment_id": seg, "end": end})).await.unwrap();
    let pieces = t["segments"].as_array().unwrap();
    assert!(pieces.iter().any(|p| (p["end"].as_f64().unwrap() - 4.425).abs() < 1e-6), "{t}");
    let undo_all = smart["undo_all"].as_str().unwrap();
    assert!(undo_all.ends_with("--revision 1"), "{undo_all}");
    // the undo command really restores the raw clip
    let id = v["id"].as_str().unwrap();
    common::call(&reg, &ctx, "history.restore", json!({"clip_id": id, "revision": 1})).await.unwrap();
    let shown = common::call(&reg, &ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(seg_ranges(&shown["clip"]), vec![(1.0, 5.6)], "{shown}");
}

#[tokio::test]
async fn raw_flag_and_setting_off_keep_the_raw_clip() {
    let (tmp, store, ctx) = common::new_project();
    let reg = common::registry();
    let src = fake_source(&store, tmp.path());
    let v = common::call(&reg, &ctx, "clip.create", json!({"name": "c", "source_id": src, "start": 1.0, "end": 5.6, "raw": true})).await.unwrap();
    assert!(v.get("smart").is_none(), "{v}");
    assert_eq!(seg_ranges(&v), vec![(1.0, 5.6)]);
    assert_eq!(v["revision"], 1);

    common::call(&reg, &ctx, "config.set", json!({"key": "clip.smart_defaults", "value": false})).await.unwrap();
    let v = common::call(&reg, &ctx, "clip.create", json!({"name": "d", "source_id": src, "start": 1.0, "end": 5.6})).await.unwrap();
    assert!(v.get("smart").is_none(), "{v}");
    assert_eq!(seg_ranges(&v), vec![(1.0, 5.6)]);
}

#[tokio::test]
async fn approximate_word_timings_are_not_trimmed() {
    let (tmp, store, ctx) = common::new_project();
    let reg = common::registry();
    let src = common::add_source(&store, &tmp.path().join("missing.mp4"), common::probe_1080p(60.0, true, true));
    let t = rustler_core::Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: src.id.clone(),
        language: "en".into(),
        engine: "youtube:auto".into(),
        script: rustler_core::CaptionScript::Latin,
        words: words_with_filler_and_pause(),
        word_timing: rustler_core::WordTiming::Approximate,
    };
    store.write_json(&Path::new("analysis").join(src.id.as_str()).join("transcript.json"), &t).unwrap();
    let v = common::call(&reg, &ctx, "clip.create", json!({"name": "c", "source_id": src.id, "start": 1.0, "end": 5.6})).await.unwrap();
    assert_eq!(seg_ranges(&v), vec![(1.0, 5.6)]);
    assert_eq!(v["smart"]["applied"], false);
    assert!(v["smart"]["decisions"].as_array().unwrap().iter().any(|d| d["why"].as_str().unwrap().contains("approximate")), "{v}");
}

// --- real engine runs -----------------------------------------------------------------------------

fn lincoln() -> Option<PathBuf> {
    let p = support::home().join("test-media").join("lincoln.jpg");
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: {} missing", p.display());
        None
    }
}

fn ffmpeg_in(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new(support::ffmpeg())
        .current_dir(dir)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// 960x540, 6 s: Lincoln moving from left to right.
fn make_moving_face(path: &Path, portrait: &Path) {
    let fg = "color=c=gray:s=960x540:r=25:d=6[bg];[0:v]scale=-2:400[p];[bg][p]overlay=x='40+(W-w-80)*t/6':y=70:shortest=1";
    ffmpeg_in(path.parent().unwrap(), &["-loop", "1", "-framerate", "25", "-t", "6", "-i", portrait.to_str().unwrap(), "-filter_complex", fg, "-t", "6", "-r", "25", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "16", "-pix_fmt", "yuv420p", path.to_str().unwrap()]);
}

/// 640x360, 9 s: Lincoln left and mirrored right; the mouth of the "talking" face flickers
/// (left 0-3 s and 6-9 s, right 3-6 s). Same fixture as the crop.track_speaker test.
fn make_two_faces(path: &Path, portrait: &Path) {
    let fg = "color=c=gray:s=640x360:r=10:d=9[bg];\
        [0:v]scale=-2:320[l];[1:v]scale=-2:320,hflip[r];\
        [bg][l]overlay=10:20[t1];[t1][r]overlay=W-w-10:20[t2];\
        [t2]drawbox=x=92:y=186:w=74:h=16:color=black:t=fill:enable='(between(t,0,3)+between(t,6,9))*lt(mod(n,2),1)'[t3];\
        [t3]drawbox=x=464:y=186:w=74:h=16:color=black:t=fill:enable='between(t,3,6)*lt(mod(n,2),1)'";
    let p = portrait.to_str().unwrap();
    ffmpeg_in(path.parent().unwrap(), &["-loop", "1", "-framerate", "10", "-t", "9", "-i", p, "-loop", "1", "-framerate", "10", "-t", "9", "-i", p, "-filter_complex", fg, "-t", "9", "-r", "10", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "16", "-pix_fmt", "yuv420p", path.to_str().unwrap()]);
}

/// 960x540, 8 s: Lincoln (0-4 s), then a text-only title card (4-8 s).
fn make_face_then_card(path: &Path, portrait: &Path) -> bool {
    let dir = path.parent().unwrap();
    let Some(font) = ["C:\\Windows\\Fonts\\arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/dejavu/DejaVuSans.ttf"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
    else {
        eprintln!("SKIP: no system font for the title card");
        return false;
    };
    std::fs::copy(&font, dir.join("font.ttf")).unwrap();
    let fg = "color=c=gray:s=960x540:r=25:d=8[bg];[0:v]scale=-2:420[p];[bg][p]overlay=x=(W-w)/2:y=60:enable='lt(t,4)'[a];\
        [a]drawbox=x=0:y=0:w=iw:h=ih:color=0x0A1E5A:t=fill:enable='gte(t,4)'[b];\
        [b]drawtext=fontfile=font.ttf:text='THREE RULES':fontsize=72:fontcolor=white:x=(w-text_w)/2:y=200:enable='gte(t,4)'[c];\
        [c]drawtext=fontfile=font.ttf:text='for speaking well':fontsize=44:fontcolor=white:x=(w-text_w)/2:y=300:enable='gte(t,4)'";
    ffmpeg_in(dir, &["-loop", "1", "-framerate", "25", "-t", "8", "-i", portrait.to_str().unwrap(), "-filter_complex", fg, "-t", "8", "-r", "25", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-pix_fmt", "yuv420p", path.to_str().unwrap()]);
    true
}

async fn import(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, video: &Path) -> String {
    let v = support::ok(reg, "source.import", ctx, json!({"input": video})).await;
    v["sources"][0]["id"].as_str().unwrap().to_string()
}

fn ready() -> Option<PathBuf> {
    if !support::have_media_tools() || !support::have_engine() {
        return None;
    }
    lincoln()
}

fn crop_paths(ctx: &rustler_registry::Ctx, clip: &str) -> serde_json::Map<String, Value> {
    let store = ctx.store().unwrap();
    let id: rustler_core::ClipId = clip.parse().unwrap();
    rustler_commands::extras::load(&store, &id).unwrap().get("crop_paths").and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn x_at(keys: &[Value], t: f64) -> f64 {
    keys.iter().rev().find(|k| k["t"].as_f64().unwrap() <= t).map_or(keys[0]["x"].as_f64().unwrap(), |k| k["x"].as_f64().unwrap())
}

#[tokio::test]
async fn real_one_moving_face_gets_center_crop_and_a_smooth_track() {
    let Some(portrait) = ready() else { return };
    let reg = support::registry();
    let (dir, ctx) = support::project();
    let video = dir.path().join("moving.mp4");
    make_moving_face(&video, &portrait);
    let id = import(&reg, &ctx, &video).await;
    let v = support::ok(&reg, "clip.create", &ctx, json!({"name": "m", "source_id": id, "start": 0.0, "end": 6.0, "captions": false})).await;
    let smart = &v["smart"];
    let layout: Vec<&Value> = smart["decisions"].as_array().unwrap().iter().filter(|d| d["kind"] == "layout").collect();
    assert_eq!(layout.len(), 1, "{smart}");
    assert!(layout[0]["what"].as_str().unwrap().contains("smooth track"), "{smart}");
    assert_eq!(v["segments"][0]["layout"]["preset"], "center-crop");
    let cid = v["id"].as_str().unwrap();
    let paths = crop_paths(&ctx, cid);
    let seg = v["segments"][0]["id"].as_str().unwrap();
    let keys = paths.get(seg).and_then(|k| k.as_array()).cloned().unwrap_or_default();
    assert!(!keys.is_empty(), "no crop path: {paths:?}");
    // the face travels left -> right; the crop follows
    assert!(x_at(&keys, 5.5) > x_at(&keys, 0.5) + 0.3, "{keys:?}");
    // override commands are real registry commands
    let cmds: Vec<&str> = layout[0]["override_with"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert!(cmds.iter().any(|c| c.starts_with("rustler crop track-face ")), "{cmds:?}");

    // --raw: no analysis, no path
    let r = support::ok(&reg, "clip.create", &ctx, json!({"name": "r", "source_id": id, "start": 0.0, "end": 6.0, "captions": false, "raw": true})).await;
    assert!(r.get("smart").is_none());
    assert!(crop_paths(&ctx, r["id"].as_str().unwrap()).is_empty());
}

#[tokio::test]
async fn real_two_faces_alternate_with_the_light_active_speaker() {
    let Some(portrait) = ready() else { return };
    let reg = support::registry();
    let (dir, ctx) = support::project();
    let video = dir.path().join("two.mp4");
    make_two_faces(&video, &portrait);
    let id = import(&reg, &ctx, &video).await;
    let v = support::ok(&reg, "clip.create", &ctx, json!({"name": "two", "source_id": id, "start": 0.0, "end": 9.0, "captions": false})).await;
    let smart = &v["smart"];
    let d = smart["decisions"].as_array().unwrap().iter().find(|d| d["kind"] == "layout").cloned().unwrap();
    assert!(d["what"].as_str().unwrap().contains("active speaker"), "{smart}");
    assert_eq!(v["segments"].as_array().unwrap().len(), 1);
    let cid = v["id"].as_str().unwrap().to_string();

    let out = support::ok(&reg, "crop.track_active", &ctx, json!({"clip_id": cid})).await;
    let seg = &out["segments"][0];
    let turns = seg["turns"].as_array().unwrap();
    let xs: Vec<f64> = turns.iter().map(|t| t["face_x"].as_f64().unwrap()).collect();
    assert_eq!(turns.len(), 3, "{out}");
    assert!(xs[0] < 0.5 && xs[1] > 0.5 && xs[2] < 0.5, "{out}");
    assert!((turns[1]["start"].as_f64().unwrap() - 3.0).abs() <= 0.7, "{out}");
    assert!((turns[2]["start"].as_f64().unwrap() - 6.0).abs() <= 0.7, "{out}");
    assert_eq!(seg["switches"], 2);
    let keys = seg["keys"].as_array().unwrap();
    assert!(x_at(keys, 1.5) < 0.25, "{keys:?}");
    assert!(x_at(keys, 4.8) > 0.4, "{keys:?}");
    assert!(x_at(keys, 8.0) < 0.25, "{keys:?}");
    // stored for the renderer
    let paths = crop_paths(&ctx, &cid);
    assert_eq!(paths.get(seg["segment_id"].as_str().unwrap()).and_then(|k| k.as_array()).map(|k| k.len()), Some(keys.len()));
}

#[tokio::test]
async fn real_graphics_shot_gets_its_own_segment_and_layout_plan_proposes_first() {
    let Some(portrait) = ready() else { return };
    let reg = support::registry();
    let (dir, ctx) = support::project();
    let video = dir.path().join("card.mp4");
    if !make_face_then_card(&video, &portrait) {
        return;
    }
    let id = import(&reg, &ctx, &video).await;
    // a raw clip, then layout.plan proposes without applying
    let r = support::ok(&reg, "clip.create", &ctx, json!({"name": "raw", "source_id": id, "start": 0.0, "end": 8.0, "captions": false, "raw": true})).await;
    let cid = r["id"].as_str().unwrap().to_string();
    let plan = support::ok(&reg, "layout.plan", &ctx, json!({"clip_id": cid})).await;
    assert_eq!(plan["applied"], false);
    let pieces = plan["pieces"].as_array().unwrap();
    assert_eq!(pieces.len(), 2, "{plan}");
    assert_eq!(pieces[0]["layout"]["kind"], "center-crop", "{plan}");
    let second = pieces[1]["layout"]["kind"].as_str().unwrap();
    assert!(second == "zoom-fit" || second == "blur-fit", "{plan}");
    assert_eq!(pieces[1]["class"], "graphics", "{plan}");
    assert!((pieces[1]["start"].as_f64().unwrap() - 4.0).abs() < 0.1, "{plan}");
    let shown = support::ok(&reg, "clip.show", &ctx, json!({"clip_id": cid})).await;
    assert_eq!(shown["clip"]["segments"].as_array().unwrap().len(), 1, "proposing must not change the clip");
    assert_eq!(shown["clip"]["revision"], 1);

    // apply
    let applied = support::ok(&reg, "layout.plan", &ctx, json!({"clip_id": cid, "apply": true})).await;
    assert_eq!(applied["applied"], true);
    assert!(applied["undo"].as_str().unwrap().starts_with("rustler history restore "), "{applied}");

    // clip.create with smart defaults gives the same two segments
    let v = support::ok(&reg, "clip.create", &ctx, json!({"name": "smart", "source_id": id, "start": 0.0, "end": 8.0, "captions": false})).await;
    let segs = v["segments"].as_array().unwrap();
    assert_eq!(segs.len(), 2, "{v}");
    assert_eq!(segs[0]["layout"]["preset"], "center-crop");
    assert_eq!(segs[1]["layout"]["preset"].as_str().unwrap(), second);
    assert!((segs[0]["end"].as_f64().unwrap() - 4.0).abs() < 0.1, "{v}");
}
