//! fix.suggest / fix.apply / qc.compare and the v2 QC checks, on a real clip with renders made by
//! ffmpeg. Engine-based tests need uv + the engine sources + the Lincoln portrait (public domain,
//! cached in `<home>/test-media/lincoln.jpg`) and skip with a printed reason otherwise.
mod common;

use common::*;
use rustler_core::{ClipId, ErrorCode, ProjectStore, RenderId};
use rustler_registry::{Ctx, Registry};
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const PORTRAIT_URL: &str = "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/Abraham_Lincoln_O-77_matte_collodion_print.jpg/960px-Abraham_Lincoln_O-77_matte_collodion_print.jpg";

struct Fx {
    _tmp: TempDir,
    _tools: Option<TempDir>,
    store: ProjectStore,
    ctx: Ctx,
    reg: Registry,
    clip_id: String,
    duration: f64,
    ffmpeg: PathBuf,
}

/// Points the commands at the repo's engine; `false` (printed) when uv or the sources are missing.
fn engine_available() -> bool {
    let home = real_home();
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../engine");
    let uv = home.join("bin").join(if cfg!(windows) { "uv.exe" } else { "uv" });
    if !uv.is_file() || !engine.join("rustler_engine").is_dir() {
        eprintln!("SKIP: uv or engine sources not available");
        return false;
    }
    // SAFETY: tests only ever set these to the same values.
    unsafe {
        std::env::set_var("RUSTLER_ENGINE_PROJECT", std::fs::canonicalize(&engine).unwrap());
        if std::env::var_os("HF_HOME").is_none() {
            std::env::set_var("HF_HOME", home.join("models").join("hf"));
        }
    }
    true
}

fn portrait() -> Option<PathBuf> {
    let cache = real_home().join("test-media").join("lincoln.jpg");
    let big = |p: &Path| std::fs::metadata(p).map(|m| m.len() > 10_000).unwrap_or(false);
    if big(&cache) {
        return Some(cache);
    }
    let _ = std::fs::create_dir_all(cache.parent().unwrap());
    let ok = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .args(["-sSL", "--max-time", "40", "-A", "rustler-tests/0.1 (support@digitalheroes.co.in)", "-o"])
        .arg(&cache)
        .arg(PORTRAIT_URL)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok && big(&cache) {
        Some(cache)
    } else {
        let _ = std::fs::remove_file(&cache);
        eprintln!("SKIP: portrait download failed (offline?)");
        None
    }
}

/// A project with a 10 s 1920x1080 source, transcript and a center-crop clip over the first three
/// sentences. `real_tools`: use the installed tools home (engine possible); else a throwaway home
/// with only ffmpeg/ffprobe (so the engine is unavailable).
async fn fixture(real_tools: bool) -> Option<Fx> {
    let (tools, home) = if real_tools {
        let home = real_home();
        if !home.join("bin").join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }).is_file() {
            eprintln!("SKIP: ffmpeg not installed");
            return None;
        }
        (None, home)
    } else {
        let (t, h) = tools_home()?;
        (Some(t), h)
    };
    let (tmp, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let ffmpeg = ffmpeg_in(&home);
    let video = tmp.path().join("talk.mp4");
    make_video(&ffmpeg, &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&ToolPaths::from_home(&home), &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let clip = call(
        &reg,
        &ctx,
        "clip.create",
        json!({"name": "fix demo", "source_id": src.id, "start_unit": 0, "end_unit": 2, "caption_style": "bold-karaoke"}),
    )
    .await
    .unwrap();
    Some(Fx {
        _tmp: tmp,
        _tools: tools,
        store,
        ctx,
        reg,
        clip_id: clip["id"].as_str().unwrap().to_string(),
        duration: clip["duration"].as_f64().unwrap(),
        ffmpeg,
    })
}

impl Fx {
    async fn call(&self, name: &str, input: Value) -> rustler_core::Result<Value> {
        call(&self.reg, &self.ctx, name, input).await
    }

    async fn ok(&self, name: &str, input: Value) -> Value {
        match self.call(name, input).await {
            Ok(v) => v,
            Err(e) => panic!("{name} failed: {} hint={:?}", e.message, e.hint),
        }
    }

    /// Builds an mp4 with `build(out_path, duration)` and attaches it to the clip as a new render.
    fn attach_render(&self, build: impl FnOnce(&Path, f64)) -> String {
        let rid = RenderId::new();
        let rel = Path::new("clips").join(&self.clip_id).join("renders").join(format!("{rid}.mp4"));
        let abs = self.store.resolve(&rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        build(&abs, self.duration);
        let cid: ClipId = self.clip_id.parse().unwrap();
        let mut clip = self.store.load_clip(&cid).unwrap();
        let base = clip.revision;
        clip.renders.push(
            serde_json::from_value(json!({
                "id": rid, "path": rel, "quality": "final", "encoder": "libx264",
                "created_at": "2026-11-15T00:00:00Z", "qc": null
            }))
            .unwrap(),
        );
        self.store.save_clip(&mut clip, base).unwrap();
        rid.to_string()
    }

    fn clip(&self) -> rustler_core::Clip {
        self.store.load_clip(&self.clip_id.parse().unwrap()).unwrap()
    }

    async fn qc(&self, render_id: &str) -> Value {
        self.ok("qc.run", json!({"clip_id": self.clip_id, "render_id": render_id, "allow_fail": true})).await
    }
}

fn run_ffmpeg(ffmpeg: &Path, args: &[String]) {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg fixture failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// 1080x1920 plain background + tone (loudnormalised). With `Some((portrait, overlay_x, overlay_y))`
/// the portrait is scaled to 640 px wide and composited there (ffmpeg overlay expressions: `w`/`h`
/// are the portrait's size, `W`/`H` the frame's).
fn make_render(ffmpeg: &Path, out: &Path, secs: f64, portrait: Option<(&Path, &str, &str)>) {
    let s = format!("{secs:.3}");
    let mut a: Vec<String> = vec![
        "-f".into(), "lavfi".into(), "-i".into(), format!("color=c=0x203040:size=1080x1920:rate=30:duration={s}"),
    ];
    if let Some((p, _, _)) = portrait {
        a.extend(["-loop".into(), "1".into(), "-t".into(), s.clone(), "-i".into(), p.to_string_lossy().into_owned()]);
    }
    a.extend(["-f".into(), "lavfi".into(), "-i".into(), format!("sine=frequency=440:sample_rate=48000:duration={s}")]);
    let audio_idx = if portrait.is_some() { 2 } else { 1 };
    match portrait {
        Some((_, x, y)) => {
            a.extend([
                "-filter_complex".into(),
                format!("[1:v]scale=640:-1[p];[0:v][p]overlay={x}:{y}:shortest=1[v]"),
                "-map".into(), "[v]".into(),
            ]);
        }
        None => a.extend(["-map".into(), "0:v".into()]),
    }
    a.extend([
        "-map".into(), format!("{audio_idx}:a"),
        "-af".into(), "loudnorm=I=-14:TP=-2:LRA=11,aresample=48000".into(),
        "-t".into(), s,
        "-c:v".into(), "libx264".into(), "-preset".into(), "ultrafast".into(), "-pix_fmt".into(), "yuv420p".into(),
        "-c:a".into(), "aac".into(), "-ar".into(), "48000".into(),
        out.to_string_lossy().into_owned(),
    ]);
    run_ffmpeg(ffmpeg, &a);
}

fn check<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no check {name} in {}", report["checks"]))
}

fn dump(report: &Value) {
    for c in report["checks"].as_array().unwrap() {
        eprintln!("{:<24} {:<5} {} | {}", c["name"].as_str().unwrap(), c["status"].as_str().unwrap(), c["value"], c["fix_hint"].as_str().unwrap_or(""));
    }
}

/// Captions sit at position_y 0.70 (1344 px); put the face right there.
const OVER_CAPTIONS: (&str, &str) = ("(W-w)/2", "1344-0.35*h");

#[tokio::test]
async fn caption_over_face_warns_and_suggests_a_position() {
    if !engine_available() {
        return;
    }
    let Some(p) = portrait() else { return };
    let Some(f) = fixture(true).await else { return };
    let rid = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, Some((&p, OVER_CAPTIONS.0, OVER_CAPTIONS.1))));
    let r = f.qc(&rid).await;
    dump(&r);
    let c = check(&r, "caption_over_face");
    assert_eq!(c["status"], "warn", "{c}");
    let y = c["value"]["suggested_position_y"].as_f64().expect("a suggested position");
    assert!((y - 0.70).abs() > 0.05, "the suggestion {y} must move the captions");
    let hint = c["fix_hint"].as_str().unwrap();
    assert!(hint.contains(&format!("rustler clip set-captions {} --position-y", f.clip_id)), "{hint}");
    assert!(c["value"]["overlap_ratio"].as_f64().unwrap() > 0.2);
    assert_eq!(check(&r, "face_visible_ratio")["status"], "pass", "a face is on screen throughout");
    assert_eq!(check(&r, "face_cut_off")["status"], "pass");
    // engine problems never fail QC
    assert_eq!(r["passed"], true);
}

#[tokio::test]
async fn face_cut_off_warns_when_the_face_sits_on_the_frame_edge() {
    if !engine_available() {
        return;
    }
    let Some(p) = portrait() else { return };
    let Some(f) = fixture(true).await else { return };
    // portrait pushed left so part of the face is off-screen; vertically well above the captions
    let rid = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, Some((&p, "-0.40*w", "H*0.30-0.35*h"))));
    let r = f.qc(&rid).await;
    dump(&r);
    let c = check(&r, "face_cut_off");
    assert_eq!(c["status"], "warn", "{c}");
    assert!(c["value"]["left_edge_samples"].as_u64().unwrap() > 0);
    assert!(c["fix_hint"].as_str().unwrap().contains(&format!("rustler crop track-face {}", f.clip_id)));
    assert_eq!(check(&r, "caption_over_face")["status"], "pass", "face is nowhere near the captions");
}

#[tokio::test]
async fn engine_missing_degrades_to_a_skipped_warning_never_a_failure() {
    // throwaway home: ffmpeg only, no uv, no engine
    let Some(f) = fixture(false).await else { return };
    let rid = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, None));
    let r = f.qc(&rid).await;
    dump(&r);
    assert_eq!(r["passed"], true, "{}", r["verdict"]);
    for name in ["caption_over_face", "face_visible_ratio", "face_cut_off"] {
        let c = check(&r, name);
        assert_eq!(c["status"], "warn", "{name}");
        assert_eq!(c["value"], json!({"skipped": "engine unavailable"}), "{name}");
    }
    // the non-engine v2 checks still run: captions have words in the first 3 s
    assert_eq!(check(&r, "hook_text_present")["status"], "pass");
    assert_eq!(check(&r, "duration_for_platform")["status"], "pass");
}

#[tokio::test]
async fn hook_text_overlays_and_brief_are_read_leniently() {
    let Some(f) = fixture(false).await else { return };
    // captions off + an overlay starting late -> warn; an early overlay -> pass
    f.ok("clip.set_captions", json!({"clip_id": f.clip_id, "enabled": false})).await;
    let rid = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, None));
    let extras = Path::new("clips").join(&f.clip_id).join("extras.json");
    f.store.write_json(&extras, &json!({"overlays": [{"text": "late", "start": 5.0, "end": 6.0}]})).unwrap();
    let r = f.qc(&rid).await;
    let c = check(&r, "hook_text_present");
    assert_eq!(c["status"], "warn", "{c}");
    assert!(c["fix_hint"].as_str().unwrap().contains(&f.clip_id));
    f.store.write_json(&extras, &json!({"overlays": [{"text": "hook", "start": 0.0, "end": 3.0}], "music": null})).unwrap();
    let r = f.qc(&rid).await;
    assert_eq!(check(&r, "hook_text_present")["status"], "pass");
    // brief.json limits (keys read leniently; absent keys are fine)
    f.store.write_json(Path::new("brief.json"), &json!({"platform": "tiktok", "max_duration_s": 2})).unwrap();
    let r = f.qc(&rid).await;
    let c = check(&r, "duration_for_platform");
    assert_eq!(c["status"], "warn", "{c}");
    assert_eq!(c["value"]["source"], "brief");
    assert!(c["fix_hint"].as_str().unwrap().contains(&format!("rustler edit trim {}", f.clip_id)));
}

#[tokio::test]
async fn fix_suggest_orders_failures_before_warnings_and_runs_qc_when_needed() {
    let Some(f) = fixture(false).await else { return };
    // an all-black render: black_frames fails; engine is down, so face checks are skipped warnings
    let rid = f.attach_render(|out, d| {
        run_ffmpeg(
            &f.ffmpeg,
            &[
                "-f".into(), "lavfi".into(), "-i".into(), format!("color=c=black:size=1080x1920:rate=30:duration={d:.3}"),
                "-f".into(), "lavfi".into(), "-i".into(), format!("sine=frequency=440:sample_rate=48000:duration={d:.3}"),
                "-t".into(), format!("{d:.3}"),
                "-c:v".into(), "libx264".into(), "-preset".into(), "ultrafast".into(), "-pix_fmt".into(), "yuv420p".into(),
                "-c:a".into(), "aac".into(), out.to_string_lossy().into_owned(),
            ],
        )
    });
    let s = f.ok("fix.suggest", json!({"clip_id": f.clip_id, "render_id": rid})).await;
    assert_eq!(s["ran_qc"], true, "no report existed, so QC ran first");
    let list = s["suggestions"].as_array().unwrap();
    eprintln!("{}", serde_json::to_string_pretty(list).unwrap());
    assert!(list.len() >= 2);
    assert_eq!(list[0]["check"], "black_frames");
    assert_eq!(list[0]["severity"], "high");
    assert_eq!(list[0]["auto_safe"], false);
    let rank = |s: &str| match s {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    let ranks: Vec<i32> = list.iter().map(|x| rank(x["severity"].as_str().unwrap())).collect();
    assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "severity must not increase down the list: {ranks:?}");
    for x in list {
        let cmds = x["commands"].as_array().unwrap();
        assert!(!cmds.is_empty(), "{x}");
        assert!(cmds.iter().all(|c| c.as_str().unwrap().starts_with("rustler ")), "{x}");
        assert!(!x["why"].as_str().unwrap().is_empty());
    }
    // engine-skipped checks come last and tell the agent to run doctor
    let last = list.last().unwrap();
    assert_eq!(last["commands"][0], "rustler doctor", "{last}");
    assert_eq!(last["severity"], "low");

    // a second call reuses the saved report
    let again = f.ok("fix.suggest", json!({"clip_id": f.clip_id, "render_id": rid})).await;
    assert_eq!(again["ran_qc"], false);
}

#[tokio::test]
async fn fix_suggest_and_apply_move_the_captions_off_the_face() {
    if !engine_available() {
        return;
    }
    let Some(p) = portrait() else { return };
    let Some(f) = fixture(true).await else { return };
    let rid = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, Some((&p, OVER_CAPTIONS.0, OVER_CAPTIONS.1))));
    let s = f.ok("fix.suggest", json!({"clip_id": f.clip_id, "render_id": rid})).await;
    let list = s["suggestions"].as_array().unwrap();
    eprintln!("{}", serde_json::to_string_pretty(list).unwrap());
    let cap = list.iter().find(|x| x["check"] == "caption_over_face").expect("caption_over_face suggestion");
    assert_eq!(cap["auto_safe"], true);
    let cmd = cap["commands"][0].as_str().unwrap();
    assert!(cmd.starts_with(&format!("rustler clip set-captions {} --position-y ", f.clip_id)), "{cmd}");
    let suggested: f64 = cmd.rsplit(' ').next().unwrap().parse().unwrap();
    // creative fixes are listed but never auto-safe
    for x in list.iter().filter(|x| ["hook_text_present", "face_visible_ratio", "face_cut_off", "duration_for_platform", "frozen_video"].contains(&x["check"].as_str().unwrap())) {
        assert_eq!(x["auto_safe"], false, "{x}");
    }

    // a creative check cannot be applied, and a passing check has nothing to apply
    let e = f
        .call("fix.apply", json!({"clip_id": f.clip_id, "render_id": rid, "check": "frozen_video", "rerender": false}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation, "{}", e.message);
    assert!(e.message.contains("creative"), "{}", e.message);
    let e = f
        .call("fix.apply", json!({"clip_id": f.clip_id, "render_id": rid, "check": "streams", "rerender": false}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    let before = f.clip();
    assert!((before.captions.position_y - 0.70).abs() < 1e-9);
    let a = f.ok("fix.apply", json!({"clip_id": f.clip_id, "render_id": rid, "rerender": false})).await;
    eprintln!("{a}");
    let after = f.clip();
    assert!((after.captions.position_y - suggested).abs() < 1e-9, "{} vs {suggested}", after.captions.position_y);
    assert!(after.revision > before.revision, "revision must bump");
    assert_eq!(a["revision_before"], before.revision);
    assert_eq!(a["revision_after"], after.revision);
    assert!(a["fixed_checks"].as_array().unwrap().iter().any(|c| c == "caption_over_face"));
    assert!(a["applied"][0]["cli"].as_str().unwrap().contains("set-captions"));
    assert!(a["after"].is_null(), "rerender:false makes no new render");
    assert_eq!(f.clip().renders.len(), 1);
    // nothing creative was touched: the not_applied list keeps the hook warning
    assert!(a["not_applied"].as_array().unwrap().iter().all(|x| x["auto_safe"] == false));
}

#[tokio::test]
async fn fix_apply_rerenders_and_rechecks() {
    // real render path; engine absent (throwaway home), so only non-engine fixes are involved
    let Some(f) = fixture(false).await else { return };
    f.ok("clip.set_captions", json!({"clip_id": f.clip_id, "position_y": 0.95})).await;
    f.ok("render.run", json!({"clip_id": f.clip_id, "quality": "draft", "prefer_gpu": false})).await;
    let first = f.clip().renders[0].id.to_string();
    let r = f.qc(&first).await;
    assert_eq!(check(&r, "caption_safe_zone")["status"], "warn", "{}", check(&r, "caption_safe_zone"));

    let a = f.ok("fix.apply", json!({"clip_id": f.clip_id})).await;
    assert!(a["after"]["render_id"].as_str().unwrap().starts_with("rnd_"));
    assert!(a["after"]["remaining"].as_array().unwrap().iter().all(|s| s["check"] != "caption_safe_zone"));
    let clip = f.clip();
    assert!((clip.captions.position_y - 0.70).abs() < 1e-9, "safe-zone fix moves the captions to 0.70");
    assert_eq!(clip.renders.len(), 2, "one new render");
    let a = f.ok("fix.suggest", json!({"clip_id": f.clip_id})).await;
    assert_eq!(a["render_id"], clip.renders[1].id.to_string(), "suggest looks at the newest render");
    assert_eq!(a["ran_qc"], false, "fix.apply already QC'd the new render");
    assert!(a["suggestions"].as_array().unwrap().iter().all(|s| s["check"] != "caption_safe_zone"));
}

#[tokio::test]
async fn fix_apply_dry_run_changes_nothing() {
    let Some(mut f) = fixture(false).await else { return };
    f.ok("clip.set_captions", json!({"clip_id": f.clip_id, "position_y": 0.95})).await;
    f.ok("render.run", json!({"clip_id": f.clip_id, "quality": "draft", "prefer_gpu": false})).await;
    let rev = f.clip().revision;
    f.ctx.dry_run = true;
    let a = f.ok("fix.apply", json!({"clip_id": f.clip_id})).await;
    assert_eq!(a["dry_run"], true);
    assert!(!a["applied"].as_array().unwrap().is_empty(), "the plan is listed");
    assert_eq!(f.clip().revision, rev);
    assert_eq!(f.clip().renders.len(), 1);
}

#[tokio::test]
async fn qc_compare_writes_a_labelled_side_by_side_image() {
    if !engine_available() {
        return;
    }
    let Some(p) = portrait() else { return };
    let Some(f) = fixture(true).await else { return };
    let a = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, Some((&p, OVER_CAPTIONS.0, OVER_CAPTIONS.1))));
    // B: the same face moved well above the captions
    let b = f.attach_render(|out, d| make_render(&f.ffmpeg, out, d, Some((&p, "(W-w)/2", "H*0.25-0.35*h"))));
    let r = f.ok("qc.compare", json!({"clip_id": f.clip_id, "render_a": a, "render_b": b})).await;
    eprintln!("{}", r["summary"]);
    let img = PathBuf::from(r["image"].as_str().unwrap());
    assert!(img.is_file(), "{img:?}");
    let dims = image_dims(&img);
    assert!(dims.0 > 1700 && dims.1 > 500, "two columns of contact sheets: {dims:?}");
    assert_eq!(r["look_at"][0], r["image"]);
    let diff = r["diff"].as_array().unwrap();
    let cap = diff.iter().find(|d| d["name"] == "caption_over_face").unwrap();
    assert_eq!(cap["a"], "warn");
    assert_eq!(cap["b"], "pass");
    assert_eq!(cap["change"], "improved");
    assert!(r["summary"].as_str().unwrap().contains("improved"));
    // keep a copy to look at
    let _ = std::fs::copy(&img, std::env::temp_dir().join("rustler_qc_compare.jpg"));

    let same = f.call("qc.compare", json!({"clip_id": f.clip_id, "render_a": a, "render_b": a})).await.unwrap_err();
    assert_eq!(same.code, ErrorCode::Validation);
}

fn image_dims(p: &Path) -> (u32, u32) {
    // JPEG SOF0/2 marker scan keeps the test free of an image dependency
    let b = std::fs::read(p).unwrap();
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return (w, h);
        }
        i += 2 + len;
    }
    panic!("no JPEG size marker in {}", p.display());
}
