//! analyze.speakers, speaker.rename, caption.speaker_colors, crop.track_speaker.
//!
//! Tests with synthetic `speakers.json` need no external tools. The `real_*` tests need ffmpeg,
//! uv, the engine and the diarization / YuNet models and skip with a printed reason when anything
//! is missing (offline included).
mod common;
mod support;

use rustler_captions::CaptionWord;
use rustler_commands::speaker_cmds::{SpeakerTurn, SpeakersFile, assign_words, label_transcript, load_speakers, write_clip_ass};
use rustler_core::{Clip, ErrorCode, ProjectStore, Source, SourceId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn turn(start: f64, end: f64, speaker: &str) -> SpeakerTurn {
    SpeakerTurn { start, end, speaker: speaker.to_string() }
}

fn write_speakers(store: &ProjectStore, source: &SourceId, turns: Vec<SpeakerTurn>) {
    let file = SpeakersFile {
        schema: "rustler.speakers@1".into(),
        source_id: source.clone(),
        model: "test".into(),
        num_speakers: 2,
        duration: 60.0,
        segments: turns,
        labels: BTreeMap::new(),
    };
    store.write_json(&Path::new("analysis").join(source.as_str()).join("speakers.json"), &file).unwrap();
}

struct Fx {
    _tmp: tempfile::TempDir,
    store: ProjectStore,
    ctx: rustler_registry::Ctx,
    src: Source,
    reg: rustler_registry::Registry,
}

/// Source with the 12-word sample transcript (1 s - 7 s): S1 talks until 4.2 s, S2 after.
fn fx() -> Fx {
    let (tmp, store, ctx) = common::new_project();
    let src = common::add_source(&store, Path::new("C:/nonexistent/video.mp4"), common::probe_1080p(60.0, true, true));
    common::write_transcript(&store, &src.id, common::sample_words());
    write_speakers(&store, &src.id, vec![turn(0.9, 4.2, "S1"), turn(4.2, 7.1, "S2")]);
    Fx { _tmp: tmp, store, ctx, src, reg: common::registry() }
}

async fn clip(f: &Fx, start: f64, end: f64) -> Clip {
    let v = common::call(&f.reg, &f.ctx, "clip.create", json!({"name": "c", "source_id": f.src.id, "start": start, "end": end}))
        .await
        .unwrap();
    f.store.load_clip(&v["id"].as_str().unwrap().parse().unwrap()).unwrap()
}

fn cw(text: &str, s: f64, e: f64) -> CaptionWord {
    CaptionWord { text: text.into(), out_start: s, out_end: e }
}

// --- word assignment ---------------------------------------------------------------------------------

#[test]
fn transcript_words_get_speakers_and_the_raw_copy_is_untouched() {
    let f = fx();
    let raw = f.store.resolve(&Path::new("analysis").join(f.src.id.as_str()).join("transcript.raw.json"));
    std::fs::write(&raw, b"{\"keep\":\"me\"}").unwrap();
    let turns = load_speakers(&f.store, &f.src.id).unwrap().unwrap().segments;
    let (n, total) = label_transcript(&f.store, &f.src.id, &turns).unwrap().unwrap();
    assert_eq!((n, total), (12, 12));
    let tr = f.store.read_json::<rustler_core::Transcript>(&Path::new("analysis").join(f.src.id.as_str()).join("transcript.json")).unwrap();
    let who: Vec<&str> = tr.words.iter().map(|w| w.speaker.as_deref().unwrap()).collect();
    // "ask" (4.05-4.3) overlaps S1 by 0.15 s and S2 by 0.10 s -> S1; "not." (4.35-5.0) is S2
    assert_eq!(&who[..6], &["S1"; 6]);
    assert_eq!(&who[6..], &["S2"; 6]);
    assert_eq!(std::fs::read(&raw).unwrap(), b"{\"keep\":\"me\"}");
}

#[test]
fn label_transcript_without_a_transcript_is_none() {
    let (_tmp, store, _ctx) = common::new_project();
    let src = common::add_source(&store, Path::new("C:/nonexistent/v.mp4"), common::probe_1080p(10.0, true, true));
    assert!(label_transcript(&store, &src.id, &[turn(0.0, 1.0, "S1")]).unwrap().is_none());
}

#[test]
fn overlapping_diarization_and_gaps() {
    let mut words = vec![common::word(0, "a", 1.0, 1.5), common::word(1, "b", 3.0, 3.4)];
    // S2 only overlaps word 1; word 0 lies 0.45 s before S1 -> nearest turn
    assign_words(&mut words, &[turn(1.7, 2.5, "S1"), turn(2.9, 4.0, "S2")]);
    assert_eq!(words[0].speaker.as_deref(), Some("S1"));
    assert_eq!(words[1].speaker.as_deref(), Some("S2"));
}

// --- speaker.rename ---------------------------------------------------------------------------------

#[tokio::test]
async fn rename_stores_labels_and_validates_the_speaker() {
    let f = fx();
    let v = common::call(&f.reg, &f.ctx, "speaker.rename", json!({"source_id": f.src.id, "speaker": "s1", "name": "  Host "})).await.unwrap();
    assert_eq!(v["speaker"], "S1");
    assert_eq!(v["name"], "Host");
    assert_eq!(v["labels"], json!({"S1": "Host"}));
    let file = load_speakers(&f.store, &f.src.id).unwrap().unwrap();
    assert_eq!(file.name_of("S1"), "Host");
    assert_eq!(file.name_of("S2"), "S2");
    assert_eq!(file.segments.len(), 2, "segments are untouched");

    let e = common::call(&f.reg, &f.ctx, "speaker.rename", json!({"source_id": f.src.id, "speaker": "S9", "name": "X"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("S1, S2"), "{}", e.message);

    // empty name resets
    common::call(&f.reg, &f.ctx, "speaker.rename", json!({"source_id": f.src.id, "speaker": "S1", "name": ""})).await.unwrap();
    assert!(load_speakers(&f.store, &f.src.id).unwrap().unwrap().labels.is_empty());

    // a source that was never diarized
    let other = common::add_source(&f.store, Path::new("C:/nonexistent/o.mp4"), common::probe_1080p(5.0, true, true));
    let e = common::call(&f.reg, &f.ctx, "speaker.rename", json!({"source_id": other.id, "speaker": "S1", "name": "X"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("analyze speakers"));
}

// --- caption.speaker_colors + ASS ---------------------------------------------------------------------

#[tokio::test]
async fn speaker_colors_are_stored_in_extras_without_clobbering_other_keys() {
    let f = fx();
    let c = clip(&f, 0.0, 7.5).await;
    let extras_path = Path::new("clips").join(c.id.as_str()).join("extras.json");
    f.store.write_json(&extras_path, &json!({"music": {"keep": true}, "crop_paths": {"seg": [{"t": 0.0, "x": 0.1}]}})).unwrap();

    let v = common::call(&f.reg, &f.ctx, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": true})).await.unwrap();
    assert_eq!(v["enabled"], true);
    assert_eq!(v["speakers"], json!(["S1", "S2"]));
    assert_eq!(v["colors"], json!({"S1": "#FFE000", "S2": "#5EE0FF"}));

    let v = common::call(&f.reg, &f.ctx, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": true, "colors": {"s2": "7cff6b", "S7": "#112233"}})).await.unwrap();
    assert_eq!(v["colors"]["S2"], "#7CFF6B");
    assert_eq!(v["colors"]["S1"], "#FFE000", "unspecified speakers keep their colour");
    assert!(v["warnings"][0].as_str().unwrap().contains("S7"), "{v}");

    let stored: Value = f.store.read_json(&extras_path).unwrap();
    assert_eq!(stored["speaker_colors"]["enabled"], true);
    assert_eq!(stored["speaker_colors"]["colors"]["S2"], "#7CFF6B");
    assert_eq!(stored["music"], json!({"keep": true}));
    assert_eq!(stored["crop_paths"]["seg"][0]["x"], 0.1);

    // disabling keeps the palette for later
    let v = common::call(&f.reg, &f.ctx, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": false})).await.unwrap();
    assert_eq!(v["enabled"], false);
    let stored: Value = f.store.read_json(&extras_path).unwrap();
    assert_eq!(stored["speaker_colors"]["enabled"], false);
    assert_eq!(stored["speaker_colors"]["colors"]["S1"], "#FFE000");

    // dry run writes nothing
    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    let v = common::call(&f.reg, &dry, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": true, "colors": {"S1": "#000000"}})).await.unwrap();
    assert_eq!(v["dry_run"], true);
    let stored: Value = f.store.read_json(&extras_path).unwrap();
    assert_eq!(stored["speaker_colors"]["enabled"], false);
}

#[tokio::test]
async fn speaker_colors_errors() {
    let f = fx();
    let c = clip(&f, 0.0, 7.5).await;
    let e = common::call(&f.reg, &f.ctx, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": true, "colors": {"S1": "yellow"}})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("yellow"));

    // a clip whose source was never diarized
    let (_t, store, ctx) = common::new_project();
    let src = common::add_source(&store, Path::new("C:/nonexistent/video.mp4"), common::probe_1080p(60.0, true, true));
    common::write_transcript(&store, &src.id, common::sample_words());
    let v = common::call(&f.reg, &ctx, "clip.create", json!({"name": "c", "source_id": src.id, "start": 0.0, "end": 5.0})).await.unwrap();
    let e = common::call(&f.reg, &ctx, "caption.speaker_colors", json!({"clip_id": v["id"], "enabled": true})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("analyze speakers"));
    // switching it off needs no analysis
    common::call(&f.reg, &ctx, "caption.speaker_colors", json!({"clip_id": v["id"], "enabled": false})).await.unwrap();
}

/// Caption words in output time for a clip starting at source second `offset` (sample words 1-7 s).
fn clip_words(offset: f64) -> Vec<CaptionWord> {
    common::sample_words().iter().filter(|w| w.start >= offset).map(|w| cw(&w.text, w.start - offset, w.end - offset)).collect()
}

#[tokio::test]
async fn ass_output_is_speaker_coloured_through_the_clip_timeline() {
    let f = fx();
    // The clip starts at source 3.0 s: output time 0 = source 3.0. S1 talks until source 4.2 (out 1.2).
    let c = clip(&f, 3.0, 7.5).await;
    let words = clip_words(3.0);
    let mut warnings = Vec::new();

    // not enabled: identical to the plain writer
    let plain = write_clip_ass(&f.store, &c, &words, "Anton", &mut warnings).unwrap();
    assert!(!plain.contains("&H00563412&") && !plain.contains("&H00214365&"));

    common::call(&f.reg, &f.ctx, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": true, "colors": {"S1": "#123456", "S2": "#654321"}})).await.unwrap();
    let ass = write_clip_ass(&f.store, &c, &words, "Anton", &mut warnings).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let dialogue: Vec<&str> = ass.lines().filter(|l| l.starts_with("Dialogue:")).collect();
    assert!(!dialogue.is_empty());
    // S1 (#123456) = &H00563412&, S2 (#654321) = &H00214365&. Words "Fellow Americans" (out 0-1.0) are S1,
    // "ask not." partly; "What your country can do." (source 5.05+) are S2.
    let line_with = |needle: &str| *dialogue.iter().find(|l| l.to_lowercase().contains(needle)).unwrap_or_else(|| panic!("no event with {needle}: {ass}"));
    let s1_line = line_with("americans");
    assert!(s1_line.contains(r"{\c&H00563412&}"), "{s1_line}");
    let s2_line = dialogue.iter().rev().find(|l| l.to_lowercase().contains("country")).unwrap();
    assert!(s2_line.contains(r"{\c&H00214365&}"), "{s2_line}");
    assert!(!s2_line.contains("&H00563412&"), "{s2_line}");
}

#[tokio::test]
async fn missing_speaker_analysis_warns_and_falls_back_to_plain_captions() {
    let f = fx();
    let c = clip(&f, 0.0, 7.5).await;
    common::call(&f.reg, &f.ctx, "caption.speaker_colors", json!({"clip_id": c.id, "enabled": true, "colors": {"S1": "#123456", "S2": "#654321"}})).await.unwrap();
    std::fs::remove_file(f.store.resolve(&Path::new("analysis").join(f.src.id.as_str()).join("speakers.json"))).unwrap();
    let words = clip_words(0.0);
    let mut warnings = Vec::new();
    let ass = write_clip_ass(&f.store, &c, &words, "Anton", &mut warnings).unwrap();
    assert!(warnings.iter().any(|w| w.contains("analyze speakers")), "{warnings:?}");
    assert!(!ass.contains("&H00563412&"));
}

// --- real runs ---------------------------------------------------------------------------------------

fn lincoln() -> Option<PathBuf> {
    let p = support::home().join("test-media").join("lincoln.jpg");
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: {} missing", p.display());
        None
    }
}

/// 640x360 video: Lincoln left and mirrored right; the mouth area of the face that "speaks" flickers
/// (left 0-3 s and 6-9 s, right 3-6 s).
fn make_two_face_video(path: &Path, portrait: &Path) {
    let fg = "color=c=gray:s=640x360:r=10:d=9[bg];\
        [0:v]scale=-2:320[l];[1:v]scale=-2:320,hflip[r];\
        [bg][l]overlay=10:20[t1];[t1][r]overlay=W-w-10:20[t2];\
        [t2]drawbox=x=92:y=186:w=74:h=16:color=black:t=fill:enable='(between(t,0,3)+between(t,6,9))*lt(mod(n,2),1)'[t3];\
        [t3]drawbox=x=464:y=186:w=74:h=16:color=black:t=fill:enable='between(t,3,6)*lt(mod(n,2),1)'";
    let p = portrait.to_str().unwrap();
    let out = std::process::Command::new(support::ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-loop", "1", "-framerate", "10", "-t", "9", "-i", p])
        .args(["-loop", "1", "-framerate", "10", "-t", "9", "-i", p, "-filter_complex", fg])
        .args(["-t", "9", "-r", "10", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "16", "-pix_fmt", "yuv420p", path.to_str().unwrap()])
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
}

#[tokio::test]
async fn real_track_speaker_follows_the_talking_face() {
    if !support::have_media_tools() || !support::have_engine() {
        return;
    }
    let Some(portrait) = lincoln() else { return };
    let reg = support::registry();
    let (dir, ctx) = support::project();
    let video = dir.path().join("two_faces.mp4");
    make_two_face_video(&video, &portrait);
    let v = support::ok(&reg, "source.import", &ctx, json!({"input": video})).await;
    let id = v["sources"][0]["id"].as_str().unwrap().to_string();
    let store = ctx.store().unwrap();
    let sid: SourceId = id.parse().unwrap();
    write_speakers(&store, &sid, vec![turn(0.0, 3.0, "S1"), turn(3.0, 6.0, "S2"), turn(6.0, 9.0, "S1")]);
    let c = support::ok(&reg, "clip.create", &ctx, json!({"name": "two", "source_id": id, "start": 0.0, "end": 9.0, "captions": false})).await;
    let cid = c["id"].as_str().unwrap().to_string();

    let out = match support::run(&reg, "crop.track_speaker", &ctx, json!({"clip_id": cid})).await {
        Ok(v) => v,
        Err(e) => {
            // YuNet model download needs the network on first use
            eprintln!("SKIP: crop.track_speaker could not run (offline?): {e}");
            return;
        }
    };
    let seg = &out["segments"][0];
    let turns = seg["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 3, "{out}");
    let faces: Vec<i64> = turns.iter().map(|t| t["face_id"].as_i64().unwrap()).collect();
    assert_eq!(faces, vec![0, 1, 0], "{out}");
    assert!(turns.iter().all(|t| t["reason"] == "mouth-motion"), "{out}");
    assert_eq!(turns[1]["speaker"], "S2");
    assert!(turns[1]["face_x"].as_f64().unwrap() > 0.5);

    let keys = seg["keys"].as_array().unwrap();
    let at = |t: f64| keys.iter().rev().find(|k| k["t"].as_f64().unwrap() <= t).map_or(keys[0]["x"].as_f64().unwrap(), |k| k["x"].as_f64().unwrap());
    assert!(at(1.5) < 0.2, "{keys:?}");
    assert!(at(4.8) > 0.5, "{keys:?}");
    assert!(at(8.0) < 0.2, "{keys:?}");

    // stored exactly like crop.track_face does
    let extras: Value = store.read_json(&Path::new("clips").join(&cid).join("extras.json")).unwrap();
    let seg_id = seg["segment_id"].as_str().unwrap();
    assert_eq!(extras["crop_paths"][seg_id].as_array().unwrap().len(), keys.len());
}

#[tokio::test]
async fn real_analyze_speakers_on_the_jfk_sample() {
    if !support::have_media_tools() || !support::have_engine() {
        return;
    }
    let Some(jfk) = support::jfk() else { return };
    let reg = support::registry();
    let (dir, ctx) = support::project();
    let video = dir.path().join("jfk.mp4");
    support::make_video_with_wav(&video, &jfk);
    let v = support::ok(&reg, "source.import", &ctx, json!({"input": video})).await;
    let id = v["sources"][0]["id"].as_str().unwrap().to_string();
    support::ok(&reg, "analyze.transcribe", &ctx, json!({"source_id": id, "model": "tiny", "language": "en"})).await;

    let out = match support::run(&reg, "analyze.speakers", &ctx, json!({"source_id": id})).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("SKIP: analyze.speakers could not run (models need a download): {e}");
            return;
        }
    };
    assert_eq!(out["num_speakers"], 1, "{out}");
    assert_eq!(out["speakers"][0]["id"], "S1");
    assert!(out["speakers"][0]["talk_pct"].as_f64().unwrap() > 99.0);
    assert!(out["words_labelled"].as_u64().unwrap() >= 10, "{out}");
    let store = ctx.store().unwrap();
    let sid: SourceId = id.parse().unwrap();
    assert!(load_speakers(&store, &sid).unwrap().unwrap().segments.len() >= 2);
    let tr = store.read_json::<rustler_core::Transcript>(&Path::new("analysis").join(&id).join("transcript.json")).unwrap();
    assert!(tr.words.iter().filter(|w| w.speaker.as_deref() == Some("S1")).count() >= 10);

    // second run with a forced split keeps working and relabels the transcript
    let two = support::ok(&reg, "analyze.speakers", &ctx, json!({"source_id": id, "num_speakers": 2})).await;
    assert_eq!(two["num_speakers"], 2, "{two}");
    let named = support::ok(&reg, "speaker.rename", &ctx, json!({"source_id": id, "speaker": "S1", "name": "Kennedy"})).await;
    assert_eq!(named["labels"]["S1"], "Kennedy");
}
