mod support;

use rustler_core::{Clip, ErrorCode, QcReport, QcStatus, RenderRecord};
use rustler_media::qc::{QcView, names_with, verdict, view};
use rustler_media::run_qc;
use rustler_tools::ToolPaths;
use std::path::Path;
use std::process::Command;

fn ffmpeg(t: &ToolPaths, args: &[&str]) {
    let out = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg fixture failed: {}", String::from_utf8_lossy(&out.stderr));
}

const ENC: [&str; 8] = ["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac"];

/// 1080x1920 30 fps testsrc2 + a constant 440 Hz tone, loudnormalised like a real render.
fn make_good(t: &ToolPaths, path: &Path, secs: u32) {
    let v = format!("testsrc2=size=1080x1920:rate=30:duration={secs}");
    let a = format!("sine=frequency=440:sample_rate=48000:duration={secs}");
    let s = secs.to_string();
    let mut args = vec!["-f", "lavfi", "-i", &v, "-f", "lavfi", "-i", &a];
    args.extend(["-af", "loudnorm=I=-14:TP=-2:LRA=11,aresample=48000", "-t", &s]);
    args.extend(ENC);
    args.extend(["-b:a", "160k", "-ar", "48000", path.to_str().unwrap()]);
    ffmpeg(t, &args);
}

/// All-black 1080x1920 video with a normal tone.
fn make_black(t: &ToolPaths, path: &Path, secs: u32) {
    let v = format!("color=c=black:size=1080x1920:rate=30:duration={secs}");
    let a = format!("sine=frequency=440:sample_rate=48000:duration={secs}");
    let s = secs.to_string();
    let mut args = vec!["-f", "lavfi", "-i", &v, "-f", "lavfi", "-i", &a, "-t", &s];
    args.extend(ENC);
    args.push(path.to_str().unwrap());
    ffmpeg(t, &args);
}

/// testsrc2 video; audio is silent until `silent_until`, then tone, then silent from `silent_from`.
fn make_gated(t: &ToolPaths, path: &Path, secs: u32, silent_until: f64, silent_from: f64) {
    let v = format!("testsrc2=size=1080x1920:rate=30:duration={secs}");
    let a = format!(
        "sine=frequency=440:sample_rate=48000:duration={secs},volume='if(between(t,{silent_until},{silent_from}),0.3,0)':eval=frame"
    );
    let s = secs.to_string();
    let mut args = vec!["-f", "lavfi", "-i", &v, "-f", "lavfi", "-i", &a, "-t", &s];
    args.extend(ENC);
    args.push(path.to_str().unwrap());
    ffmpeg(t, &args);
}

fn make_sized(t: &ToolPaths, path: &Path, w: u32, h: u32, secs: u32) {
    let v = format!("testsrc2=size={w}x{h}:rate=30:duration={secs}");
    let a = format!("sine=frequency=440:sample_rate=48000:duration={secs}");
    let s = secs.to_string();
    let mut args = vec!["-f", "lavfi", "-i", &v, "-f", "lavfi", "-i", &a, "-t", &s];
    args.extend(ENC);
    args.push(path.to_str().unwrap());
    ffmpeg(t, &args);
}

fn make_video_only(t: &ToolPaths, path: &Path, secs: u32) {
    let v = format!("testsrc2=size=1080x1920:rate=30:duration={secs}");
    let s = secs.to_string();
    ffmpeg(t, &["-f", "lavfi", "-i", &v, "-t", &s, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", path.to_str().unwrap()]);
}

/// One-segment clip whose timeline is `secs` long.
fn clip_for(secs: f64, loudnorm: bool, position_y: f64) -> (Clip, RenderRecord) {
    let clip: Clip = serde_json::from_value(serde_json::json!({
        "schema": "rustler.clip@1",
        "id": "clp_00000000000000000000000001",
        "name": "qc fixture",
        "revision": 1,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": [{
            "id": "seg_00000000000000000000000001",
            "source_id": "src_00000000000000000000000001",
            "start": 10.0, "end": 10.0 + secs,
            "layout": {"preset": "center-crop", "regions": [], "background": "black"}
        }],
        "captions": {"enabled": true, "preset": "bold-karaoke", "script": "latin", "position_y": position_y, "max_words_per_line": 3},
        "audio": {"loudnorm": loudnorm, "target_lufs": -14.0, "true_peak": -2.0, "lra": 11.0},
        "renders": [],
        "created_at": "2026-11-15T00:00:00Z"
    }))
    .unwrap();
    let render: RenderRecord = serde_json::from_value(serde_json::json!({
        "id": "rnd_00000000000000000000000001",
        "path": "renders/rnd_1.mp4",
        "quality": "final",
        "encoder": "libx264",
        "created_at": "2026-11-15T00:00:00Z",
        "qc": null
    }))
    .unwrap();
    (clip, render)
}

fn status(r: &QcReport, name: &str) -> QcStatus {
    r.checks.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no check {name}")).status
}

fn hint<'a>(r: &'a QcReport, name: &str) -> &'a str {
    r.checks.iter().find(|c| c.name == name).unwrap().fix_hint.as_deref().unwrap_or_else(|| panic!("{name} has no fix_hint"))
}

#[tokio::test]
async fn qc_passes_good_render() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("good (1) – टेस्ट.mp4");
    make_good(&t, &f, 6);
    let (clip, render) = clip_for(6.0, true, 0.70);
    let qc = dir.path().join("qc");
    let r = run_qc(&t, &clip, &render, &f, &font, &qc).await.unwrap();
    for c in &r.checks {
        eprintln!("{:<22} {:?} {} | {}", c.name, c.status, c.value, c.fix_hint.as_deref().unwrap_or(""));
    }
    assert!(r.passed, "{}", verdict(&r));
    for name in ["streams", "dimensions", "duration", "av_sync", "black_frames", "frozen_video", "long_silence", "first_frame_not_black", "hook_window", "caption_safe_zone", "contact_sheet"] {
        assert_eq!(status(&r, name), QcStatus::Pass, "{name}");
    }
    assert_eq!(status(&r, "loudness"), QcStatus::Pass);
    assert!(r.contact_sheet.is_file());
    if let Some(keep) = std::env::var_os("RUSTLER_QC_KEEP") {
        // manual inspection: RUSTLER_QC_KEEP=<dir> keeps a copy of the contact sheet
        let _ = std::fs::create_dir_all(&keep);
        std::fs::copy(&r.contact_sheet, Path::new(&keep).join("good_sheet.jpg")).unwrap();
    }
    assert_eq!(r.contact_sheet, qc.join("rnd_00000000000000000000000001.jpg"));
    assert_eq!(image::image_dimensions(&r.contact_sheet).unwrap().0, 4 * 270);
    assert!(std::fs::metadata(&r.contact_sheet).unwrap().len() > 5_000);
    // the scratch first-frame jpg is cleaned up
    assert_eq!(std::fs::read_dir(&qc).unwrap().count(), 1);

    let v: QcView = view(r.clone());
    assert_eq!(v.look_at, vec![r.contact_sheet.clone()]);
    assert!(v.verdict.starts_with("PASS"));
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["schema"], "rustler.qc@1");
    assert_eq!(json["passed"], true);
    assert!(json["look_at"][0].is_string());
    let back: QcView = serde_json::from_value(json).unwrap();
    assert_eq!(back, v);
    let plain: QcReport = serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
    assert_eq!(plain, r);
}

#[tokio::test]
async fn qc_flags_black() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("black.mp4");
    make_black(&t, &f, 4);
    let (clip, render) = clip_for(4.0, false, 0.70);
    let r = run_qc(&t, &clip, &render, &f, &font, &dir.path().join("qc")).await.unwrap();
    assert!(!r.passed);
    assert_eq!(status(&r, "black_frames"), QcStatus::Fail);
    assert!(hint(&r, "black_frames").contains("rustler clip set-layout clp_00000000000000000000000001 --preset blur-fit"));
    assert_eq!(status(&r, "first_frame_not_black"), QcStatus::Warn);
    assert_eq!(status(&r, "frozen_video"), QcStatus::Warn);
    assert!(names_with(&r, QcStatus::Fail).contains(&"black_frames".to_string()));
    let v = verdict(&r);
    assert!(v.starts_with("FAIL") && v.contains("black_frames"), "{v}");
    assert!(r.contact_sheet.is_file());
}

#[tokio::test]
async fn qc_duration_mismatch_fails() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("ten.mp4");
    make_good(&t, &f, 10);
    let (clip, render) = clip_for(12.0, true, 0.70); // TimeMap says 12 s, file is 10 s
    let r = run_qc(&t, &clip, &render, &f, &font, &dir.path().join("qc")).await.unwrap();
    assert!(!r.passed);
    assert_eq!(status(&r, "duration"), QcStatus::Fail);
    assert!(hint(&r, "duration").contains("rustler render run"));
    assert_eq!(status(&r, "av_sync"), QcStatus::Pass);
}

#[tokio::test]
async fn qc_warns_on_silence_and_late_hook() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("gated.mp4");
    // silent 0-2 s, tone 2-4 s, silent 4-10 s
    make_gated(&t, &f, 10, 2.0, 4.0);
    let (clip, render) = clip_for(10.0, false, 0.70);
    let r = run_qc(&t, &clip, &render, &f, &font, &dir.path().join("qc")).await.unwrap();
    assert!(r.passed, "warnings must not fail QC: {}", verdict(&r));
    assert_eq!(status(&r, "long_silence"), QcStatus::Warn);
    assert!(hint(&r, "long_silence").contains("rustler edit remove-pauses clp_"));
    assert_eq!(status(&r, "hook_window"), QcStatus::Warn);
    assert!(hint(&r, "hook_window").contains("rustler edit trim clp_00000000000000000000000001 --start-delta 0.5"));
    assert!(verdict(&r).starts_with("PASS with "), "{}", verdict(&r));
    for n in ["long_silence", "hook_window"] {
        assert!(names_with(&r, QcStatus::Warn).contains(&n.to_string()), "{n}");
    }
}

#[tokio::test]
async fn qc_checks_caption_safe_zone() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("good.mp4");
    make_good(&t, &f, 4);
    for (y, expect) in [(0.95, QcStatus::Warn), (0.05, QcStatus::Warn), (0.70, QcStatus::Pass), (0.85, QcStatus::Pass)] {
        let (clip, render) = clip_for(4.0, true, y);
        let r = run_qc(&t, &clip, &render, &f, &font, &dir.path().join("qc")).await.unwrap();
        assert_eq!(status(&r, "caption_safe_zone"), expect, "position_y {y}");
        if expect == QcStatus::Warn {
            assert!(hint(&r, "caption_safe_zone").contains("clip set-captions"));
        }
    }
    // disabled captions never warn
    let (mut clip, render) = clip_for(4.0, true, 0.99);
    clip.captions.enabled = false;
    let r = run_qc(&t, &clip, &render, &f, &font, &dir.path().join("qc")).await.unwrap();
    assert_eq!(status(&r, "caption_safe_zone"), QcStatus::Pass);
}

#[tokio::test]
async fn qc_wrong_dimensions_and_missing_audio_fail() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.mp4");
    make_sized(&t, &small, 540, 960, 3);
    let (clip, render) = clip_for(3.0, false, 0.70);
    let r = run_qc(&t, &clip, &render, &small, &font, &dir.path().join("qc")).await.unwrap();
    assert!(!r.passed);
    assert_eq!(status(&r, "dimensions"), QcStatus::Fail);

    let silent = dir.path().join("noaudio.mp4");
    make_video_only(&t, &silent, 3);
    let r = run_qc(&t, &clip, &render, &silent, &font, &dir.path().join("qc2")).await.unwrap();
    assert!(!r.passed);
    assert_eq!(status(&r, "streams"), QcStatus::Fail);
    assert!(r.checks.iter().all(|c| c.name != "loudness" && c.name != "long_silence"));
    assert!(r.contact_sheet.is_file());
}

#[tokio::test]
async fn qc_missing_render_file_is_not_found() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (clip, render) = clip_for(3.0, true, 0.70);
    let e = run_qc(&t, &clip, &render, &dir.path().join("nope.mp4"), &font, dir.path()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("rustler render run"));
}

// ---- QC v2 -----------------------------------------------------------------------------

#[tokio::test]
async fn v2_checks_degrade_without_engine_data() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("good.mp4");
    make_good(&t, &f, 6);
    let (clip, render) = clip_for(6.0, true, 0.70); // center-crop + captions on
    let r = run_qc(&t, &clip, &render, &f, &font, &dir.path().join("qc")).await.unwrap();
    assert!(r.passed, "a missing engine must never fail QC: {}", verdict(&r));
    for name in ["caption_over_face", "face_visible_ratio", "face_cut_off"] {
        assert_eq!(status(&r, name), QcStatus::Warn, "{name}");
        let c = r.checks.iter().find(|c| c.name == name).unwrap();
        assert_eq!(c.value, serde_json::json!({"skipped": "engine unavailable"}), "{name}");
        assert!(c.fix_hint.is_some(), "{name} needs a fix_hint");
    }
    // no overlay, no transcript words: the hook has no text
    assert_eq!(status(&r, "hook_text_present"), QcStatus::Warn);
    assert!(hint(&r, "hook_text_present").contains("clp_00000000000000000000000001"));
    assert_eq!(status(&r, "duration_for_platform"), QcStatus::Pass);
}

#[tokio::test]
async fn v2_checks_use_the_extras_passed_in() {
    use rustler_media::facepath::{FaceBox, FaceSample};
    use rustler_media::qc::{QcBrief, QcExtras, run_qc_with};
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("good.mp4");
    make_good(&t, &f, 6);
    let (clip, render) = clip_for(6.0, true, 0.70);
    // a face sitting where the captions are, in every sample
    let face = FaceBox { x: 0.3, y: 0.62, w: 0.4, h: 0.2, score: 0.9 };
    let samples: Vec<FaceSample> = (0..12).map(|i| FaceSample { t: f64::from(i) * 0.5, faces: vec![face] }).collect();
    let extras = QcExtras {
        faces: Some(samples),
        overlay_starts: vec![0.5],
        brief: Some(QcBrief { platform: None, min_s: None, max_s: Some(4.0) }),
        ..QcExtras::default()
    };
    let r = run_qc_with(&t, &clip, &render, &f, &font, &dir.path().join("qc"), &extras).await.unwrap();
    assert!(r.passed, "{}", verdict(&r));
    assert_eq!(status(&r, "caption_over_face"), QcStatus::Warn);
    assert!(hint(&r, "caption_over_face").contains("rustler clip set-captions clp_00000000000000000000000001 --position-y 0.56"));
    assert_eq!(status(&r, "face_visible_ratio"), QcStatus::Pass);
    assert_eq!(status(&r, "face_cut_off"), QcStatus::Pass);
    assert_eq!(status(&r, "hook_text_present"), QcStatus::Pass);
    assert_eq!(status(&r, "duration_for_platform"), QcStatus::Warn);
    assert!(hint(&r, "duration_for_platform").contains("rustler edit trim clp_00000000000000000000000001 --end-delta -2"));
    // contact sheet stays the last check
    assert_eq!(r.checks.last().unwrap().name, "contact_sheet");
}

fn view_with_sheet(sheet: &Path) -> QcView {
    let report = QcReport {
        schema: "rustler.qc@1".into(),
        clip_id: "clp_00000000000000000000000001".parse().unwrap(),
        render_id: "rnd_00000000000000000000000001".parse().unwrap(),
        passed: true,
        checks: vec![rustler_core::QcCheck {
            name: "contact_sheet".into(),
            status: QcStatus::Pass,
            value: serde_json::json!(sheet.display().to_string()),
            expected: "sheet".into(),
            fix_hint: None,
        }],
        contact_sheet: sheet.to_path_buf(),
    };
    view(report)
}

#[test]
fn qc_view_paths_are_stored_relative_and_resolved_on_read() {
    let root = tempfile::tempdir().unwrap();
    let sheet = root.path().join("clips").join("clp_1").join("qc").join("rnd_1.jpg");
    let v = view_with_sheet(&sheet);
    let rel = v.relative_to(root.path());
    let want = Path::new("clips").join("clp_1").join("qc").join("rnd_1.jpg");
    assert_eq!(rel.report.contact_sheet, want);
    assert_eq!(rel.look_at, vec![want.clone()]);
    assert_eq!(rel.report.checks[0].value, serde_json::json!(want.display().to_string()));
    assert!(!rel.verdict.contains(&root.path().display().to_string()), "{}", rel.verdict);
    // resolving gives the absolute view back
    assert_eq!(rel.resolved_in(root.path()), v);
    // a path outside the project stays as it is
    let outside = view_with_sheet(Path::new(if cfg!(windows) { "C:\\elsewhere\\s.jpg" } else { "/elsewhere/s.jpg" }));
    assert_eq!(outside.relative_to(root.path()).report.contact_sheet, outside.report.contact_sheet);
}

#[test]
fn old_absolute_qc_records_are_re_rooted_in_a_copied_project() {
    let original = tempfile::tempdir().unwrap();
    let copy = tempfile::tempdir().unwrap();
    let old_sheet = original.path().join("clips").join("clp_1").join("qc").join("rnd_1.jpg");
    let new_sheet = copy.path().join("clips").join("clp_1").join("qc").join("rnd_1.jpg");
    std::fs::create_dir_all(new_sheet.parent().unwrap()).unwrap();
    std::fs::write(&new_sheet, b"x").unwrap();
    // the old record points into the original folder, which is gone: the copy's file is used
    let old = view_with_sheet(&old_sheet);
    drop(original);
    let r = old.resolved_in(copy.path());
    assert_eq!(r.report.contact_sheet, new_sheet);
    assert_eq!(r.look_at, vec![new_sheet.clone()]);
    assert_eq!(r.report.checks[0].value, serde_json::json!(new_sheet.display().to_string()));
    assert!(r.verdict.contains(&new_sheet.display().to_string()));
    // an absolute path that still exists is used as it is
    let still = view_with_sheet(&new_sheet);
    assert_eq!(still.resolved_in(Path::new("/nowhere")).report.contact_sheet, new_sheet);
}
