//! qc.run and export.clip on a synthetic project with real ffmpeg-made renders.
//! Tool-dependent tests skip (printing why) when ffmpeg/ffprobe or a font are missing.

use chrono::Utc;
use rustler_commands::register_all;
use rustler_core::layout::{self, LayoutParams};
use rustler_core::model::{CaptionScript, WordTiming};
use rustler_core::{
    AudioSettings, Canvas, CaptionTrack, Clip, ClipId, ErrorCode, MediaProbe, ProjectStore, RenderId,
    RenderQuality, RenderRecord, Segment, SegmentId, Source, SourceId, SourceOrigin, Transcript, Word,
};
use rustler_registry::{Ctx, Registry};
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    reg: Registry,
    clip_id: ClipId,
    render_id: RenderId,
}

fn tool_paths() -> Option<(PathBuf, ToolPaths)> {
    let home = ToolPaths::default_home();
    let paths = ToolPaths::from_home(&home);
    if !paths.ffmpeg.exists() || !paths.ffprobe.exists() {
        eprintln!("SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)", paths.bin.display());
        return None;
    }
    if rustler_media::qc::default_font(&paths).is_none() {
        eprintln!("SKIP: no font for the contact sheet");
        return None;
    }
    Some((home, paths))
}

fn ffmpeg(t: &ToolPaths, args: &[&str]) {
    let out = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
}

/// 6 s 1080x1920 render: `good` = testsrc2 + loudnormalised tone, else all black.
fn make_render(t: &ToolPaths, path: &Path, good: bool) {
    let v = if good {
        "testsrc2=size=1080x1920:rate=30:duration=6"
    } else {
        "color=c=black:size=1080x1920:rate=30:duration=6"
    };
    let mut args = vec!["-f", "lavfi", "-i", v, "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=6"];
    if good {
        args.extend(["-af", "loudnorm=I=-14:TP=-2:LRA=11,aresample=48000"]);
    }
    args.extend(["-t", "6", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ar", "48000"]);
    args.push(path.to_str().unwrap());
    ffmpeg(t, &args);
}

fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
    Word { i, text: text.to_string(), start, end, conf: None, speaker: None }
}

/// Project with one source (+ transcript), one clip [10, 16) and one render record whose mp4 is `good` or black.
fn fixture(good: bool) -> Option<Fx> {
    let (home, paths) = tool_paths()?;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (store, _) = ProjectStore::create(&root, "t").unwrap();

    let source_id = SourceId::new();
    store
        .save_source(&Source {
            id: source_id.clone(),
            name: "talk.mp4".into(),
            origin: SourceOrigin::File { path: PathBuf::from("/x/talk.mp4") },
            media_path: PathBuf::from("/x/talk.mp4"),
            probe: MediaProbe { duration: 60.0, has_video: true, has_audio: true, width: 1920, height: 1080, fps: 30.0, ..Default::default() },
            proxy: None,
            audio_wav: None,
            imported_at: Utc::now(),
        })
        .unwrap();
    let words = vec![
        w(0, "So", 9.0, 9.2), // outside the segment
        w(1, "Hello", 10.2, 10.6),
        w(2, "there", 10.65, 11.0),
        w(3, "my", 11.05, 11.2),
        w(4, "friends.", 11.25, 11.9),
        w(5, "Bye", 15.0, 15.4),
    ];
    let transcript = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source_id.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    store
        .write_json(&Path::new("analysis").join(source_id.as_str()).join("transcript.json"), &transcript)
        .unwrap();

    let canvas = Canvas::default();
    let layout = layout::preset("center-crop", 1920, 1080, &canvas, &LayoutParams::default()).unwrap();
    let clip_id = ClipId::new();
    let render_id = RenderId::new();
    let rel = Path::new("clips").join(clip_id.as_str()).join("renders").join(format!("{render_id}.mp4"));
    std::fs::create_dir_all(root.join(&rel).parent().unwrap()).unwrap();
    make_render(&paths, &root.join(&rel), good);
    let mut clip = Clip {
        schema: "rustler.clip@1".into(),
        id: clip_id.clone(),
        name: "My hook (1) / test".into(),
        revision: 0,
        canvas,
        segments: vec![Segment { id: SegmentId::new(), source_id, start: 10.0, end: 16.0, layout }],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![RenderRecord {
            id: render_id.clone(),
            path: rel,
            quality: RenderQuality::Final,
            encoder: "libx264".into(),
            created_at: Utc::now(),
            qc: None,
        }],
        created_at: Utc::now(),
    };
    store.save_clip(&mut clip, 0).unwrap();
    let mut reg = Registry::new();
    register_all(&mut reg);
    Some(Fx { _dir: dir, root, home, reg, clip_id, render_id })
}

impl Fx {
    fn ctx(&self) -> Ctx {
        Ctx::new(Some(self.root.clone()), self.home.clone())
    }
    fn store(&self) -> ProjectStore {
        ProjectStore::open(&self.root).unwrap()
    }
    async fn run(&self, cmd: &str, input: Value) -> rustler_core::Result<Value> {
        self.reg.dispatch(cmd, input, self.ctx()).await
    }
    fn events(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("events.ndjson"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn commands_are_registered_with_schemas() {
    let mut reg = Registry::new();
    register_all(&mut reg);
    let qc = reg.describe("qc.run").unwrap();
    assert!(qc.input_schema["properties"]["clip_id"].is_object());
    assert!(qc.input_schema["properties"]["allow_fail"].is_object());
    assert_eq!(qc.positional, &["clip_id"]);
    let ex = reg.describe("export.clip").unwrap();
    assert!(ex.input_schema["properties"]["name"].is_object());
    assert!(ex.output_schema["properties"]["video"].is_object());
}

#[tokio::test]
async fn qc_run_saves_report_and_links_render() {
    let Some(fx) = fixture(true) else { return };
    let out = fx.run("qc.run", json!({"clip_id": fx.clip_id})).await.unwrap();
    assert_eq!(out["passed"], true);
    assert_eq!(out["schema"], "rustler.qc@1");
    assert_eq!(out["render_id"], fx.render_id.to_string());
    assert!(out["verdict"].as_str().unwrap().starts_with("PASS"), "{}", out["verdict"]);
    let sheet = PathBuf::from(out["look_at"][0].as_str().unwrap());
    assert!(sheet.is_file(), "contact sheet {}", sheet.display());
    assert_eq!(out["dry_run"], false);

    let report_path = PathBuf::from(out["report_path"].as_str().unwrap());
    assert_eq!(report_path, fx.root.join("clips").join(fx.clip_id.as_str()).join("qc").join(format!("{}.json", fx.render_id)));
    let saved: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(saved["passed"], true);
    assert!(saved["look_at"].is_array());

    let clip = fx.store().load_clip(&fx.clip_id).unwrap();
    assert_eq!(clip.revision, 2, "save_clip bumped the revision once");
    let qc_rel = clip.renders[0].qc.clone().expect("RenderRecord.qc set");
    assert_eq!(fx.root.join(&qc_rel), report_path);
    // history snapshot of the pre-QC revision exists (undo-able)
    assert!(fx.store().load_clip_snapshot(&fx.clip_id, 1).is_ok());
    let ev = fx.events();
    assert!(ev.iter().any(|e| e["kind"] == "qc.run" && e["data"]["passed"] == true && e["actor"] == "agent"));
}

#[tokio::test]
async fn qc_run_failure_is_qc_failed_with_report_path_in_hint() {
    let Some(fx) = fixture(false) else { return };
    let e = fx.run("qc.run", json!({"clip_id": fx.clip_id, "render_id": fx.render_id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::QcFailed);
    assert_eq!(e.code.exit_code(), 7);
    assert!(e.message.contains("black_frames"), "{}", e.message);
    let hint = e.hint.clone().unwrap();
    let report = fx.root.join("clips").join(fx.clip_id.as_str()).join("qc").join(format!("{}.json", fx.render_id));
    assert!(hint.contains(&report.display().to_string()), "{hint}");
    assert!(hint.contains("clip set-layout"), "{hint}");
    assert!(report.is_file(), "the failing report is still saved");
    assert!(fx.store().load_clip(&fx.clip_id).unwrap().renders[0].qc.is_some());

    // allow_fail -> data with passed:false
    let out = fx.run("qc.run", json!({"clip_id": fx.clip_id, "allow_fail": true})).await.unwrap();
    assert_eq!(out["passed"], false);
    let black = out["checks"].as_array().unwrap().iter().find(|c| c["name"] == "black_frames").unwrap();
    assert_eq!(black["status"], "fail");
    assert!(out["verdict"].as_str().unwrap().starts_with("FAIL"));
    assert!(out["warnings"][0].as_str().unwrap().contains("allow_fail"));

    // --yes does the same
    let mut ctx = fx.ctx();
    ctx.yes = true;
    let out = fx.reg.dispatch("qc.run", json!({"clip_id": fx.clip_id}), ctx).await.unwrap();
    assert_eq!(out["passed"], false);
}

#[tokio::test]
async fn qc_run_without_renders_is_not_found() {
    let Some(fx) = fixture(true) else { return };
    let store = fx.store();
    let mut clip = store.load_clip(&fx.clip_id).unwrap();
    let base = clip.revision;
    clip.renders.clear();
    store.save_clip(&mut clip, base).unwrap();
    let e = fx.run("qc.run", json!({"clip_id": fx.clip_id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("rustler render run"));
    let e = fx.run("qc.run", json!({"clip_id": fx.clip_id, "render_id": "rnd_nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn qc_run_dry_run_saves_nothing() {
    let Some(fx) = fixture(true) else { return };
    let mut ctx = fx.ctx();
    ctx.dry_run = true;
    let out = fx.reg.dispatch("qc.run", json!({"clip_id": fx.clip_id}), ctx).await.unwrap();
    assert_eq!(out["dry_run"], true);
    assert_eq!(out["passed"], true);
    assert!(!fx.root.join("clips").join(fx.clip_id.as_str()).join("qc").exists());
    let clip = fx.store().load_clip(&fx.clip_id).unwrap();
    assert_eq!(clip.revision, 1);
    assert!(clip.renders[0].qc.is_none());
    assert!(fx.events().is_empty());
}

#[tokio::test]
async fn export_copies_render_report_and_srt() {
    let Some(fx) = fixture(true) else { return };
    let out = fx.run("export.clip", json!({"clip_id": fx.clip_id})).await.unwrap();
    // name sanitised from "My hook (1) / test"
    let video = PathBuf::from(out["video"].as_str().unwrap());
    assert_eq!(video, fx.root.join("exports").join("My hook (1) _ test.mp4"));
    assert!(std::fs::metadata(&video).unwrap().len() > 10_000);
    assert_eq!(out["qc_passed"], true);

    let report: Value = serde_json::from_slice(&std::fs::read(out["report"].as_str().unwrap()).unwrap()).unwrap();
    assert!(out["report"].as_str().unwrap().ends_with("My hook (1) _ test.report.json"));
    for k in ["clip", "render", "qc", "source_refs", "created_at"] {
        assert!(report.get(k).is_some(), "report.{k}");
    }
    assert_eq!(report["qc"]["passed"], true);
    assert_eq!(report["source_refs"][0]["source_name"], "talk.mp4");
    assert_eq!(report["source_refs"][0]["start"], 10.0);

    // QC ran automatically because none existed, and was saved
    assert!(fx.store().load_clip(&fx.clip_id).unwrap().renders[0].qc.is_some());

    let srt = std::fs::read_to_string(out["srt"].as_str().unwrap()).unwrap();
    assert!(srt.starts_with("1\n"), "{srt}");
    assert!(srt.contains("Hello there my"), "{srt}");
    assert!(srt.contains("00:00:00,200 -->"), "first cue starts at 10.2-10.0 s: {srt}");
    assert!(srt.contains("friends."));
    assert!(!srt.contains("So\n") && !srt.contains(" So "), "word outside the segment excluded: {srt}");
    assert!(srt.contains("Bye"));
    assert!(fx.events().iter().any(|e| e["kind"] == "export.clip"));
}

#[tokio::test]
async fn export_with_custom_name_and_captions_off_has_no_srt() {
    let Some(fx) = fixture(true) else { return };
    let store = fx.store();
    let mut clip = store.load_clip(&fx.clip_id).unwrap();
    let base = clip.revision;
    clip.captions.enabled = false;
    store.save_clip(&mut clip, base).unwrap();
    let out = fx.run("export.clip", json!({"clip_id": fx.clip_id, "name": "../evil:name"})).await.unwrap();
    let video = PathBuf::from(out["video"].as_str().unwrap());
    assert_eq!(video.parent().unwrap(), fx.root.join("exports"));
    assert_eq!(video.file_name().unwrap(), "_evil_name.mp4");
    assert!(out.get("srt").is_none());
    assert_eq!(std::fs::read_dir(fx.root.join("exports")).unwrap().count(), 3, "mp4 + report + post.json");
}

#[tokio::test]
async fn export_refuses_failed_qc_unless_yes() {
    let Some(fx) = fixture(false) else { return };
    let e = fx.run("export.clip", json!({"clip_id": fx.clip_id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("black_frames"), "{}", e.message);
    assert!(e.hint.unwrap().contains("--yes"));
    assert_eq!(std::fs::read_dir(fx.root.join("exports")).unwrap().count(), 0);

    let mut ctx = fx.ctx();
    ctx.yes = true;
    let out = fx.reg.dispatch("export.clip", json!({"clip_id": fx.clip_id, "name": "forced"}), ctx).await.unwrap();
    assert_eq!(out["qc_passed"], false);
    assert!(out["warnings"][0].as_str().unwrap().contains("despite failed QC"));
    assert!(Path::new(out["video"].as_str().unwrap()).is_file());
    // the exported report records the failure
    let report: Value = serde_json::from_slice(&std::fs::read(out["report"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(report["qc"]["passed"], false);
}

#[tokio::test]
async fn export_dry_run_writes_nothing() {
    let Some(fx) = fixture(true) else { return };
    let mut ctx = fx.ctx();
    ctx.dry_run = true;
    let out = fx.reg.dispatch("export.clip", json!({"clip_id": fx.clip_id}), ctx).await.unwrap();
    assert_eq!(out["dry_run"], true);
    assert_eq!(std::fs::read_dir(fx.root.join("exports")).unwrap().count(), 0);
    assert!(fx.events().is_empty());
}
