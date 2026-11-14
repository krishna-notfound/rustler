//! see.* command tests (need ffmpeg; skipped with a printed reason otherwise).
mod common;

use common::*;
use rustler_core::ErrorCode;
use rustler_tools::ToolPaths;
use serde_json::json;
use std::path::Path;

struct Fx {
    _tmp: tempfile::TempDir,
    _proj: tempfile::TempDir,
    paths: ToolPaths,
    ctx: rustler_registry::Ctx,
    store: rustler_core::ProjectStore,
    src: rustler_core::Source,
    reg: rustler_registry::Registry,
}

async fn fx() -> Option<Fx> {
    let (tmp, home) = tools_home()?;
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = ToolPaths::from_home(&home);
    let video = proj.path().join("src video (1).mp4");
    make_video(&ffmpeg_in(&home), &video, 12, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    Some(Fx { _tmp: tmp, _proj: proj, paths, ctx, store, src, reg: registry() })
}

async fn dims(f: &Fx, p: &str) -> (u32, u32) {
    let pr = rustler_media::probe(&f.paths, Path::new(p)).await.unwrap();
    (pr.width, pr.height)
}

#[tokio::test]
async fn frame_tiles_and_preview() {
    let Some(f) = fx().await else { return };

    let v = call(&f.reg, &f.ctx, "see.frame", json!({"source_id": f.src.id, "t": 3.0, "width": 640})).await.unwrap();
    let img = v["image"].as_str().unwrap();
    assert!(std::fs::metadata(img).unwrap().len() > 1000);
    assert_eq!(dims(&f, img).await.0, 640);

    // default count is 12; centered on a phrase
    let t = call(&f.reg, &f.ctx, "see.tiles", json!({"source_id": f.src.id, "around": "your country can", "window": 8}))
        .await
        .unwrap();
    assert_eq!(t["tiles"].as_array().unwrap().len(), 12);
    let (s, e) = (t["start"].as_f64().unwrap(), t["end"].as_f64().unwrap());
    assert!((e - s - 8.0).abs() < 1e-6, "{s}..{e}");
    assert!(s < 5.9 && 5.9 < e, "phrase at ~5.9 s must be inside {s}..{e}");
    assert!(t["centered_on"].as_str().unwrap().contains("your country can"));
    let times: Vec<f64> = t["tiles"].as_array().unwrap().iter().map(|x| x["t"].as_f64().unwrap()).collect();
    assert!(times.windows(2).all(|p| p[0] < p[1]) && times[0] >= s && *times.last().unwrap() <= e);
    let labels: Vec<&str> = t["tiles"].as_array().unwrap().iter().map(|x| x["text"].as_str().unwrap()).collect();
    assert!(labels.iter().any(|l| l.contains("country")), "{labels:?}");
    assert_eq!(dims(&f, t["image"].as_str().unwrap()).await.0, 1920);

    // centered on a unit, custom count
    let u = call(&f.reg, &f.ctx, "see.tiles", json!({"source_id": f.src.id, "unit": 1, "count": 8, "window": 6})).await.unwrap();
    assert_eq!(u["tiles"].as_array().unwrap().len(), 8);
    assert!(u["centered_on"].as_str().unwrap().starts_with("unit 1"));

    let e = call(&f.reg, &f.ctx, "see.tiles", json!({"source_id": f.src.id, "around": "zebra crossing"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = call(&f.reg, &f.ctx, "see.tiles", json!({"source_id": f.src.id, "unit": 1, "around": "x"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = call(&f.reg, &f.ctx, "see.tiles", json!({"source_id": f.src.id, "unit": 99})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    let p = call(&f.reg, &f.ctx, "see.preview", json!({"source_id": f.src.id, "start": 2.0, "end": 5.0})).await.unwrap();
    assert!(std::fs::metadata(p["video"].as_str().unwrap()).unwrap().len() > 1000);
    let e = call(&f.reg, &f.ctx, "see.preview", json!({"source_id": f.src.id, "start": 2.0, "end": 50.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn frame_argument_validation() {
    let Some(f) = fx().await else { return };
    let e = call(&f.reg, &f.ctx, "see.frame", json!({})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = call(&f.reg, &f.ctx, "see.frame", json!({"source_id": f.src.id, "t": 99.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = call(&f.reg, &f.ctx, "see.frame", json!({"clip_id": "clp_x"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn frame_of_a_clip_delegates_to_preview() {
    let Some(f) = fx().await else { return };
    let clip = call(&f.reg, &f.ctx, "clip.create", json!({"name": "c", "source_id": f.src.id, "start_unit": 1})).await.unwrap();
    let id = clip["id"].as_str().unwrap();
    let v = call(&f.reg, &f.ctx, "see.frame", json!({"clip_id": id, "out_t": 1.0})).await.unwrap();
    let img = v["image"].as_str().unwrap();
    assert!(std::fs::metadata(img).unwrap().len() > 1000);
    assert_eq!(dims(&f, img).await, (1080, 1920));
}
