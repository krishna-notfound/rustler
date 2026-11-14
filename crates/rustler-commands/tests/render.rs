//! render.* command tests: a real render of a synthetic 1920x1080 10 s source (needs ffmpeg;
//! skipped with a printed reason otherwise).
mod common;

use common::*;
use rustler_core::ErrorCode;
use rustler_tools::ToolPaths;
use serde_json::{Value, json};

struct Fx {
    _tmp: tempfile::TempDir,
    _proj: tempfile::TempDir,
    home: std::path::PathBuf,
    paths: ToolPaths,
    ctx: rustler_registry::Ctx,
    store: rustler_core::ProjectStore,
    src: rustler_core::Source,
    reg: rustler_registry::Registry,
}

async fn fx(video_name: &str, with_audio: bool) -> Option<Fx> {
    let (tmp, home) = tools_home()?;
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = ToolPaths::from_home(&home);
    let video = proj.path().join(video_name);
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, with_audio);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    Some(Fx { _tmp: tmp, _proj: proj, home, paths, ctx, store, src, reg: registry() })
}

async fn probe(f: &Fx, p: &str) -> rustler_core::MediaProbe {
    rustler_media::probe(&f.paths, std::path::Path::new(p)).await.unwrap()
}

#[tokio::test]
async fn preview_frame_and_render_center_crop_with_captions_and_music() {
    let Some(f) = fx("my clip (1) – टेस्ट.mp4", true).await else { return };
    // units 0..2 = the three sentences (about 0.9 s .. 7.15 s)
    let clip: Value = call(
        &f.reg,
        &f.ctx,
        "clip.create",
        json!({"name": "demo", "source_id": f.src.id, "start_unit": 0, "end_unit": 2, "caption_style": "hormozi-pop"}),
    )
    .await
    .unwrap();
    let id = clip["id"].as_str().unwrap().to_string();
    let duration = clip["duration"].as_f64().unwrap();

    // 1. preview frame: 1080x1920 jpg with captions burned in
    let pf = call(&f.reg, &f.ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 4.0})).await.unwrap();
    let img = pf["image"].as_str().unwrap();
    assert!(std::fs::metadata(img).unwrap().len() > 2000);
    let p = probe(&f, img).await;
    assert_eq!((p.width, p.height), (1080, 1920));
    // keep a copy for eyeballing
    let _ = std::fs::copy(img, std::env::temp_dir().join("rustler_10b_preview.jpg"));

    let e = call(&f.reg, &f.ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 999.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    // 2. real render (draft, CPU) with the music bed
    let music = f._proj.path().join("bed.wav");
    make_music(&ffmpeg_in(&f.home), &music, 12);
    call(&f.reg, &f.ctx, "audio.add_music", json!({"clip_id": id, "path": music})).await.unwrap();
    let r = call(&f.reg, &f.ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    assert_eq!(r["encoder"], "libx264");
    assert!(r["render_id"].as_str().unwrap().starts_with("rnd_"));
    assert_eq!(r["next"][0], format!("rustler qc run {id}"));
    let out = r["path"].as_str().unwrap();
    let pr = probe(&f, out).await;
    assert_eq!((pr.width, pr.height), (1080, 1920));
    assert!(pr.has_audio && pr.has_video);
    assert_eq!(pr.video_codec.as_deref(), Some("h264"));
    assert_eq!(pr.audio_codec.as_deref(), Some("aac"));
    assert!((pr.duration - duration).abs() < 0.2, "{} vs {duration}", pr.duration);
    // captions.ass was written into the render workdir with the chosen style's words
    let work = std::path::Path::new(out).with_extension("");
    let ass = std::fs::read_to_string(work.join("captions.ass")).unwrap();
    assert!(ass.contains("Dialogue:") && ass.to_uppercase().contains("COUNTRY"), "{ass}");

    // 3. RenderRecord + event
    let shown = call(&f.reg, &f.ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    let renders = shown["clip"]["renders"].as_array().unwrap();
    assert_eq!(renders.len(), 1);
    assert_eq!(renders[0]["id"], r["render_id"]);
    assert_eq!(renders[0]["quality"], "draft");
    let events = std::fs::read_to_string(f.store.root.join("events.ndjson")).unwrap();
    assert!(events.lines().any(|l| l.contains("\"render.run\"")));
}

#[tokio::test]
async fn render_blur_fit_without_captions_and_without_audio() {
    let Some(f) = fx("silent.mp4", false).await else { return };
    let clip = call(
        &f.reg,
        &f.ctx,
        "clip.create",
        json!({"name": "silent", "source_id": f.src.id, "start": 1.0, "end": 4.0, "layout": "blur-fit", "captions": false}),
    )
    .await
    .unwrap();
    let id = clip["id"].as_str().unwrap();
    let r = call(&f.reg, &f.ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    let pr = probe(&f, r["path"].as_str().unwrap()).await;
    assert_eq!((pr.width, pr.height), (1080, 1920));
    assert!(pr.has_audio, "a silent AAC track is added for sources without audio");
    assert!((pr.duration - 3.0).abs() < 0.2, "{}", pr.duration);
}

#[tokio::test]
async fn render_dry_run_and_missing_clip() {
    let Some(mut f) = fx("dry.mp4", true).await else { return };
    let e = call(&f.reg, &f.ctx, "render.run", json!({"clip_id": "clp_missing"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let clip = call(&f.reg, &f.ctx, "clip.create", json!({"name": "d", "source_id": f.src.id, "start": 1.0, "end": 3.0})).await.unwrap();
    f.ctx.dry_run = true;
    let r = call(&f.reg, &f.ctx, "render.run", json!({"clip_id": clip["id"]})).await.unwrap();
    assert_eq!(r["dry_run"], true);
    assert!(!std::path::Path::new(r["path"].as_str().unwrap()).exists());
    f.ctx.dry_run = false;
    let c = f.store.load_clip(&clip["id"].as_str().unwrap().parse().unwrap()).unwrap();
    assert!(c.renders.is_empty());
}
