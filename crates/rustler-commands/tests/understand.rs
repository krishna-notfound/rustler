//! analyze.ocr / analyze.events / analyze.faces / analyze.deep (understanding pack v2).
//! Need ffmpeg + uv + the engine (RapidOCR, YAMNet, YuNet); each test skips with a printed reason
//! when a tool is missing. The YAMNet model downloads on first use; if that fails the engine falls
//! back to its heuristic and the assertions below accept `model: heuristic`.
mod support;

use rustler_core::ErrorCode;
use rustler_registry::Ctx;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use support::*;

const CAPTIONS: [&str; 6] = [
    "we choose to go to the moon",
    "not because it is easy",
    "but because it is hard",
    "that goal will serve",
    "to organize and measure",
    "the best of our energies",
];

fn font_dir() -> Option<PathBuf> {
    let d = home().join("fonts").join("Poppins");
    if d.join("Poppins-Bold.ttf").is_file() {
        Some(d)
    } else {
        eprintln!("SKIP: Poppins font missing under {} (run `rustler setup`)", d.display());
        None
    }
}

/// 960x540 grey video of 11 s with six burned-in captions near the bottom and a static handle on
/// top; audio is the JFK sample (speech).
fn make_captioned_video(path: &Path, wav: &Path, font_dir: &Path) {
    let mut vf = vec!["drawtext=fontfile=Poppins-Bold.ttf:text='@rustlertest':fontsize=40:fontcolor=white:x=(w-text_w)/2:y=h*0.06".to_string()];
    for (i, c) in CAPTIONS.iter().enumerate() {
        let (a, b) = (i as f64 * 1.8, (i + 1) as f64 * 1.8 - 0.05);
        vf.push(format!(
            "drawtext=fontfile=Poppins-Bold.ttf:text='{c}':fontsize=46:fontcolor=white:x=(w-text_w)/2:y=h*0.78:enable='between(t,{a:.2},{b:.2})'"
        ));
    }
    let out = Command::new(ffmpeg())
        .current_dir(font_dir)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "color=c=0x2a2a2a:s=960x540:r=10:d=11"])
        .args(["-i", wav.to_str().unwrap(), "-t", "11", "-vf", &vf.join(",")])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", path.to_str().unwrap()])
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg drawtext failed: {}", String::from_utf8_lossy(&out.stderr));
}

async fn import(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, path: &Path) -> String {
    let v = ok(reg, "source.import", ctx, json!({"input": path})).await;
    v["sources"][0]["id"].as_str().unwrap().to_string()
}

fn fuzzy_has(texts: &[&str], want: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let w = norm(want);
    texts.iter().any(|t| {
        let n = norm(t);
        // allow a missed or misread character or two
        n.contains(&w) || w.contains(&n) && n.len() * 10 >= w.len() * 8 || common_prefix(&n, &w) * 10 >= w.len() * 8
    })
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

#[tokio::test]
async fn ocr_reads_captions_and_flags_the_burned_in_band() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let (Some(jfk), Some(fonts)) = (jfk(), font_dir()) else { return };
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("captioned.mp4");
    make_captioned_video(&video, &jfk, &fonts);
    let id = import(&reg, &ctx, &video).await;

    let v = ok(&reg, "analyze.ocr", &ctx, json!({"source_id": id, "every": 1.0})).await;
    let texts: Vec<&str> = v["texts"].as_array().unwrap().iter().map(|t| t["text"].as_str().unwrap()).collect();
    assert!(v["unique_texts"].as_u64().unwrap() >= 5, "{v}");
    for want in ["we choose to go to the moon", "but because it is hard", "the best of our energies"] {
        assert!(fuzzy_has(&texts, want), "missing `{want}` in {texts:?}");
    }
    assert!(fuzzy_has(&texts, "@rustlertest"), "handle missing: {texts:?}");
    // changing text in one lower band = burned-in captions; the static handle is a persistent overlay
    assert_eq!(v["burned_in_captions"], true, "{v}");
    let y = v["caption_band_y"].as_f64().unwrap();
    assert!((0.7..0.9).contains(&y), "band y {y}");
    assert!(v["advice"].as_str().unwrap().contains("do not stack a second caption"));
    let regions = v["regions"].as_array().unwrap();
    assert!(regions.iter().any(|r| r["kind"] == "burned-in-captions" && r["description"].as_str().unwrap().contains("burned-in captions band at y")), "{regions:?}");
    assert!(regions.iter().any(|r| r["kind"] == "persistent-overlay" && r["y"].as_f64().unwrap() < 0.25), "{regions:?}");
    // first/last seen of one caption are sensible
    let moon = v["texts"].as_array().unwrap().iter().find(|t| fuzzy_has(&[t["text"].as_str().unwrap()], "we choose to go to the moon")).unwrap();
    assert!(moon["first_seen"].as_f64().unwrap() <= 1.0 && moon["last_seen"].as_f64().unwrap() <= 2.5, "{moon}");
    assert!(ctx.project_dir.clone().unwrap().join("analysis").join(&id).join("ocr.json").is_file());
}

#[tokio::test]
async fn events_on_speech_and_audio_only() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let Some(jfk) = jfk() else { return };
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("jfk.mp4");
    make_video_with_wav(&video, &jfk);
    let id = import(&reg, &ctx, &video).await;
    let v = ok(&reg, "analyze.events", &ctx, json!({"source_id": id})).await;
    let model = v["model"].as_str().unwrap();
    assert!(model == "yamnet" || model == "heuristic", "{v}");
    assert!(v["duration"].as_f64().unwrap() > 10.0);
    let speech = &v["counts"]["speech"];
    assert!(speech["seconds"].as_f64().unwrap() >= 4.0, "speech sample should be mostly speech: {v}");
    assert!(ctx.project_dir.clone().unwrap().join("analysis").join(&id).join("events.json").is_file());
    if model == "heuristic" {
        assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("heuristic")));
    }

    // audio-only (a tone) works as well; silent video is refused with a hint
    let audio = dir.path().join("tone.m4a");
    make_audio_only(&audio, 6);
    let aid = import(&reg, &ctx, &audio).await;
    let a = ok(&reg, "analyze.events", &ctx, json!({"source_id": aid})).await;
    assert!(a["counts"].is_object());
    let silent = dir.path().join("silent.mp4");
    make_video(&silent, 4, false);
    let sid = import(&reg, &ctx, &silent).await;
    let e = expect_code(run(&reg, "analyze.events", &ctx, json!({"source_id": sid})).await, ErrorCode::Validation);
    assert!(e.message.contains("no audio"), "{}", e.message);
}

#[tokio::test]
async fn faces_timeline_on_a_video_without_faces() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("testsrc.mp4");
    make_video(&video, 6, false);
    let id = import(&reg, &ctx, &video).await;
    let v = ok(&reg, "analyze.faces", &ctx, json!({"source_id": id, "fps": 1.0})).await;
    if v["warnings"].as_array().is_some_and(|w| w.iter().any(|w| w.as_str().unwrap().contains("could be read"))) {
        return;
    }
    assert!(v["samples"].as_u64().unwrap() >= 4, "{v}");
    assert_eq!(v["face_time_pct"], 0.0);
    assert_eq!(v["typical_faces"], 0);
    assert!(v["dominant_box"].is_null());
    assert_eq!(v["suggestion"]["layout"], "blur-fit");
    let f = ctx.project_dir.clone().unwrap().join("analysis").join(&id).join("faces.json");
    let saved: Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
    assert!(saved["samples"].as_array().unwrap().len() >= 4);
    let bad = expect_code(run(&reg, "analyze.faces", &ctx, json!({"source_id": id, "fps": 0})).await, ErrorCode::Validation);
    assert!(bad.message.contains("fps"));
}

#[tokio::test]
async fn deep_combines_everything_and_degrades_per_part() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let (Some(jfk), Some(fonts)) = (jfk(), font_dir()) else { return };
    let reg = registry();
    let (dir, ctx) = project();
    let video = dir.path().join("captioned.mp4");
    make_captioned_video(&video, &jfk, &fonts);
    let id = import(&reg, &ctx, &video).await;

    let dry = ok(&reg, "analyze.deep", &Ctx { dry_run: true, ..ctx.clone() }, json!({"source_id": id})).await;
    assert_eq!(dry["dry_run"], true);

    let v = ok(&reg, "analyze.deep", &ctx, json!({"source_id": id, "every": 1.0})).await;
    assert!(v["ocr"].is_object() && v["events"].is_object() && v["faces"].is_object(), "{v}");
    let lines: Vec<&str> = v["understanding"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert!(lines.iter().any(|l| l.starts_with("On-screen text:")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("burned-in captions band")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("Audio (")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("Faces:") && l.contains("Suggested layout")), "{lines:?}");
    let root = ctx.project_dir.clone().unwrap().join("analysis").join(&id);
    for f in ["ocr.json", "events.json", "faces.json", "understanding.json"] {
        assert!(root.join(f).is_file(), "missing {f}");
    }

    // silent video: events are skipped with a warning, text + faces still run
    let silent = dir.path().join("silent.mp4");
    make_video(&silent, 4, false);
    let sid = import(&reg, &ctx, &silent).await;
    let s = ok(&reg, "analyze.deep", &ctx, json!({"source_id": sid})).await;
    assert!(s["events"].is_null() && s["ocr"].is_object() && s["faces"].is_object(), "{s}");
    assert!(s["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("no audio track")), "{s}");

    // audio-only: only events run
    let audio = dir.path().join("tone.m4a");
    make_audio_only(&audio, 5);
    let aid = import(&reg, &ctx, &audio).await;
    let a = ok(&reg, "analyze.deep", &ctx, json!({"source_id": aid})).await;
    assert!(a["ocr"].is_null() && a["faces"].is_null() && a["events"].is_object(), "{a}");
    assert!(a["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("no video stream")));

    // OCR/faces on audio-only are refused individually
    expect_code(run(&reg, "analyze.ocr", &ctx, json!({"source_id": aid})).await, ErrorCode::Validation);
    expect_code(run(&reg, "analyze.faces", &ctx, json!({"source_id": aid})).await, ErrorCode::Validation);
}

#[tokio::test]
async fn commands_are_registered_with_schemas() {
    let reg = registry();
    for name in ["analyze.ocr", "analyze.events", "analyze.faces", "analyze.deep"] {
        assert!(reg.describe(name).is_some(), "{name} not registered");
    }
}
