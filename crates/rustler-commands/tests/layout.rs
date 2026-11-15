//! layout.* command tests. Most use synthetic clips (no media needed); the render and webcam
//! detection tests need ffmpeg (and, for detection, uv + the engine + network for a public-domain
//! portrait) and skip with a printed reason otherwise.
mod common;

use common::*;
use rustler_core::ErrorCode;
use rustler_registry::Ctx;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Fx {
    _tmp: tempfile::TempDir,
    store: rustler_core::ProjectStore,
    ctx: Ctx,
    src: rustler_core::Source,
    reg: rustler_registry::Registry,
}

fn fx() -> Fx {
    let (tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("C:/nonexistent/video.mp4"), probe_1080p(60.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    Fx { _tmp: tmp, store, ctx, src, reg: registry() }
}

fn rects() -> (Value, Value) {
    (
        json!({"x": 0.75, "y": 0.0, "w": 0.25, "h": 0.33}),
        json!({"x": 0.1, "y": 0.0, "w": 0.6, "h": 1.0}),
    )
}

async fn split_clip(f: &Fx) -> Value {
    let (top, bottom) = rects();
    call(
        &f.reg,
        &f.ctx,
        "clip.create",
        json!({"name": "s", "source_id": f.src.id, "start": 10.0, "end": 20.0, "layout": "split-vertical",
               "layout_params": {"top": top, "bottom": bottom, "split": 0.44}}),
    )
    .await
    .unwrap()
}

async fn show(f: &Fx, clip_id: &str) -> Value {
    call(&f.reg, &f.ctx, "layout.show", json!({"clip_id": clip_id})).await.unwrap()
}

fn approx(v: &Value, want: f64) {
    let got = v.as_f64().unwrap();
    assert!((got - want).abs() < 1e-6, "{got} != {want}");
}

#[tokio::test]
async fn show_reports_normalized_and_pixel_rects() {
    let f = fx();
    let clip = split_clip(&f).await;
    let id = clip["id"].as_str().unwrap();
    let v = show(&f, id).await;
    let seg = &v["segments"][0];
    assert_eq!(seg["preset"], "split-vertical");
    assert_eq!((seg["source_width"].as_u64(), seg["source_height"].as_u64()), (Some(1920), Some(1080)));
    let top = &seg["regions"][0];
    assert_eq!(top["id"], "top");
    assert_eq!(top["fit"], "cover");
    assert_eq!(top["src_px"], json!({"x": 1440, "y": 0, "w": 480, "h": 356}));
    assert_eq!(top["dst_px"], json!({"x": 0, "y": 0, "w": 1080, "h": 845}));
    let bottom = &seg["regions"][1];
    assert_eq!(bottom["dst_px"], json!({"x": 0, "y": 845, "w": 1080, "h": 1075}));
    approx(&bottom["dst"]["y"], 0.44);
    assert_eq!(v["revision"], clip["revision"]);
    assert!(v["next"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().starts_with("rustler see frame")));
}

#[tokio::test]
async fn region_lifecycle_add_edit_nudge_reorder_remove() {
    let f = fx();
    let clip = split_clip(&f).await;
    let id = clip["id"].as_str().unwrap().to_string();
    let rev0 = clip["revision"].as_u64().unwrap();

    // add: picture-in-picture, defaults to the front, id r1, preset becomes custom (null)
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.add_region",
        json!({"clip_id": id, "src": {"x": 0.75, "y": 0.02, "w": 0.23, "h": 0.3},
               "dst": {"x": 0.62, "y": 0.04, "w": 0.34, "h": 0.15}}),
    )
    .await
    .unwrap();
    assert_eq!(v["region_id"], "r1");
    assert_eq!(v["revision"].as_u64().unwrap(), rev0 + 1);
    let seg = &v["segments"][0];
    assert!(seg["preset"].is_null());
    assert_eq!(seg["regions"].as_array().unwrap().len(), 3);
    assert_eq!(seg["regions"][2]["z"], 1);

    // edit: partial update leaves the other fields alone
    let v = call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "r1", "fit": "contain", "z": 5}))
        .await
        .unwrap();
    let r1 = &v["segments"][0]["regions"][2];
    assert_eq!((r1["fit"].as_str(), r1["z"].as_i64()), (Some("contain"), Some(5)));
    approx(&r1["src"]["x"], 0.75);

    // nudge moves the source selection by default and the destination on request
    let v = call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "top", "nudge": {"dx": -0.05, "dy": 0.01}}))
        .await
        .unwrap();
    let top = &v["segments"][0]["regions"][0];
    approx(&top["src"]["x"], 0.70);
    approx(&top["src"]["y"], 0.01);
    approx(&top["dst"]["y"], 0.0);
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.edit_region",
        json!({"clip_id": id, "region_id": "bottom", "nudge": {"dh": -0.1, "dy": 0.1}, "nudge_target": "dst"}),
    )
    .await
    .unwrap();
    let bottom = &v["segments"][0]["regions"][1];
    approx(&bottom["dst"]["y"], 0.54);
    approx(&bottom["dst"]["h"], 0.46);
    approx(&bottom["src"]["w"], 0.6);

    // reorder: back to front
    let v = call(&f.reg, &f.ctx, "layout.reorder", json!({"clip_id": id, "region_ids": ["r1", "bottom", "top"]})).await.unwrap();
    let z: Vec<i64> = v["segments"][0]["regions"].as_array().unwrap().iter().map(|r| r["z"].as_i64().unwrap()).collect();
    assert_eq!(z, [2, 1, 0]); // regions are listed top, bottom, r1; back to front = r1, bottom, top
    let e = call(&f.reg, &f.ctx, "layout.reorder", json!({"clip_id": id, "region_ids": ["top", "bottom"]})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);

    // remove
    let v = call(&f.reg, &f.ctx, "layout.remove_region", json!({"clip_id": id, "region_id": "r1"})).await.unwrap();
    assert_eq!(v["segments"][0]["regions"].as_array().unwrap().len(), 2);
    let e = call(&f.reg, &f.ctx, "layout.remove_region", json!({"clip_id": id, "region_id": "nope"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    assert!(e.hint.as_deref().unwrap_or("").contains("top, bottom"));
    call(&f.reg, &f.ctx, "layout.remove_region", json!({"clip_id": id, "region_id": "bottom"})).await.unwrap();
    let e = call(&f.reg, &f.ctx, "layout.remove_region", json!({"clip_id": id, "region_id": "top"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation, "the last region cannot be removed");

    // every mutation made a revision and an event
    let on_disk = f.store.load_clip(&id.parse().unwrap()).unwrap();
    assert!(on_disk.revision >= rev0 + 6);
    let events = std::fs::read_to_string(f.store.root.join("events.ndjson")).unwrap();
    for kind in ["layout.add_region", "layout.edit_region", "layout.reorder", "layout.remove_region"] {
        assert!(events.contains(kind), "missing event {kind}");
    }
}

#[tokio::test]
async fn validation_and_warnings() {
    let f = fx();
    let clip = split_clip(&f).await;
    let id = clip["id"].as_str().unwrap().to_string();
    let add = |src: Value, dst: Value| {
        let (reg, ctx, id) = (&f.reg, &f.ctx, id.clone());
        async move { call(reg, ctx, "layout.add_region", json!({"clip_id": id, "src": src, "dst": dst})).await }
    };
    let ok = json!({"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3});
    for bad in [
        json!({"x": 0.8, "y": 0.1, "w": 0.3, "h": 0.3}),   // runs past the right edge
        json!({"x": -0.1, "y": 0.1, "w": 0.3, "h": 0.3}),  // negative
        json!({"x": 0.1, "y": 0.1, "w": 0.02, "h": 0.3}),  // too thin
        json!({"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.0}),   // empty
    ] {
        assert_eq!(code(&add(bad.clone(), ok.clone()).await.unwrap_err()), ErrorCode::Validation, "src {bad}");
        assert_eq!(code(&add(ok.clone(), bad).await.unwrap_err()), ErrorCode::Validation);
    }
    // nothing was saved by the failures
    assert_eq!(f.store.load_clip(&id.parse().unwrap()).unwrap().revision, clip["revision"].as_u64().unwrap());

    // editing out of range through a nudge is rejected too
    let e = call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "top", "nudge": {"dx": 0.5}})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let e = call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "top"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation, "an edit without changes is an error");

    // a full 16:9 frame squeezed into a short wide strip with fit cover warns about heavy cropping
    let v = add(json!({"x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0}), json!({"x": 0.0, "y": 0.0, "w": 1.0, "h": 0.2})).await.unwrap();
    let w = v["warnings"].as_array().unwrap();
    assert!(w.iter().any(|m| m.as_str().unwrap().contains("will crop heavily")), "{w:?}");
    // the same region with contain does not
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.edit_region",
        json!({"clip_id": id, "region_id": "r1", "fit": "contain"}),
    )
    .await
    .unwrap();
    assert!(v["warnings"].as_array().is_none_or(|w| w.iter().all(|m| !m.as_str().unwrap().contains("crop heavily"))));
    // a small source rect blown up a lot is flagged as soft
    let v = add(json!({"x": 0.0, "y": 0.0, "w": 0.1, "h": 0.1}), json!({"x": 0.0, "y": 0.0, "w": 1.0, "h": 0.2})).await.unwrap();
    assert!(v["warnings"].as_array().unwrap().iter().any(|m| m.as_str().unwrap().contains("will look soft")));
}

#[tokio::test]
async fn dry_run_and_revision_checks() {
    let f = fx();
    let clip = split_clip(&f).await;
    let id = clip["id"].as_str().unwrap().to_string();
    let rev = clip["revision"].as_u64().unwrap();

    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    let v = call(
        &f.reg,
        &dry,
        "layout.add_region",
        json!({"clip_id": id, "src": {"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3}, "dst": {"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3}}),
    )
    .await
    .unwrap();
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["segments"][0]["regions"].as_array().unwrap().len(), 3);
    assert_eq!(f.store.load_clip(&id.parse().unwrap()).unwrap().revision, rev, "dry-run must not save");

    // stale base_revision -> revision_conflict, nothing saved
    call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "top", "z": 3})).await.unwrap();
    let e = call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "top", "z": 4, "base_revision": rev}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::RevisionConflict);
    assert_eq!(show(&f, &id).await["segments"][0]["regions"][0]["z"], 3);
}

#[tokio::test]
async fn segment_scoping() {
    let f = fx();
    let clip = split_clip(&f).await;
    let id = clip["id"].as_str().unwrap().to_string();
    // a second segment of the same source, same preset
    call(&f.reg, &f.ctx, "clip.add_segment", json!({"clip_id": id, "source_id": f.src.id, "start": 30.0, "end": 36.0,
        "layout": "split-vertical", "layout_params": {"top": rects().0, "bottom": rects().1}}))
    .await
    .unwrap();
    let v = show(&f, &id).await;
    assert_eq!(v["segments"].as_array().unwrap().len(), 2);
    let (s1, s2) = (v["segments"][0]["segment_id"].as_str().unwrap().to_string(), v["segments"][1]["segment_id"].as_str().unwrap().to_string());

    // only the first segment
    let v = call(&f.reg, &f.ctx, "layout.edit_region", json!({"clip_id": id, "segment_id": s1, "region_id": "top", "z": 7})).await.unwrap();
    assert_eq!(v["segments"].as_array().unwrap().len(), 1);
    let all = show(&f, &id).await;
    assert_eq!(all["segments"][0]["regions"][0]["z"], 7);
    assert_eq!(all["segments"][1]["regions"][0]["z"], 0);

    // omitted segment_id = every segment, one shared new id
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.add_region",
        json!({"clip_id": id, "src": {"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3}, "dst": {"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.2}}),
    )
    .await
    .unwrap();
    assert_eq!(v["segments"].as_array().unwrap().len(), 2);
    for s in v["segments"].as_array().unwrap() {
        assert_eq!(s["regions"][2]["id"], "r1");
    }
    // unknown segment
    let e = call(&f.reg, &f.ctx, "layout.show", json!({"clip_id": id, "segment_id": "seg_nope"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let _ = s2;
}

#[tokio::test]
async fn from_frame_boxes_builds_split_inset_and_main() {
    let f = fx();
    let clip = call(&f.reg, &f.ctx, "clip.create", json!({"name": "b", "source_id": f.src.id, "start": 10.0, "end": 20.0}))
        .await
        .unwrap();
    let id = clip["id"].as_str().unwrap().to_string();
    let (top, bottom) = rects();

    let v = call(
        &f.reg,
        &f.ctx,
        "layout.from_frame_boxes",
        json!({"clip_id": id, "boxes": [{"role": "top", "rect": top}, {"role": "bottom", "rect": bottom}]}),
    )
    .await
    .unwrap();
    let seg = &v["segments"][0];
    assert_eq!(seg["preset"], "split-vertical");
    let (t, b) = (&seg["regions"][0], &seg["regions"][1]);
    let split = t["dst"]["h"].as_f64().unwrap();
    assert!((0.25..=0.6).contains(&split));
    approx(&b["dst"]["y"], split);
    // `clip set-layout` style consumers still understand it: the saved preset name is a built-in
    assert!(layout_is_builtin(seg["preset"].as_str().unwrap()));

    // split-vertical + inset -> three regions, custom preset, inset in front
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.from_frame_boxes",
        json!({"clip_id": id, "split": 0.4, "boxes": [{"role": "top", "rect": top}, {"role": "bottom", "rect": bottom},
               {"role": "inset", "rect": {"x": 0.0, "y": 0.0, "w": 0.2, "h": 0.2}}]}),
    )
    .await
    .unwrap();
    let regs = v["segments"][0]["regions"].as_array().unwrap();
    assert_eq!(regs.len(), 3);
    assert_eq!(regs[2]["id"], "inset");
    assert_eq!(regs[2]["z"], 1);
    approx(&regs[0]["dst"]["h"], 0.4);

    // main + inset (contained main over a blurred background)
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.from_frame_boxes",
        json!({"clip_id": id, "main_fit": "contain", "boxes": [{"role": "main", "rect": {"x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0}},
               {"role": "inset", "rect": top, "dst": {"x": 0.55, "y": 0.05, "w": 0.4, "h": 0.2}}]}),
    )
    .await
    .unwrap();
    let seg = &v["segments"][0];
    assert_eq!(seg["background"], "blur_fill");
    assert_eq!(seg["regions"][0]["fit"], "contain");
    approx(&seg["regions"][1]["dst"]["x"], 0.55);

    // bad combinations / rects
    for boxes in [
        json!([{"role": "top", "rect": top}]),
        json!([{"role": "main", "rect": top}, {"role": "top", "rect": top}]),
        json!([{"role": "top", "rect": top}, {"role": "top", "rect": top}, {"role": "bottom", "rect": bottom}]),
        json!([{"role": "main", "rect": {"x": 0.5, "y": 0.0, "w": 0.9, "h": 1.0}}]),
        json!([]),
    ] {
        let e = call(&f.reg, &f.ctx, "layout.from_frame_boxes", json!({"clip_id": id, "boxes": boxes})).await.unwrap_err();
        assert_eq!(code(&e), ErrorCode::Validation, "{boxes}");
    }
}

fn layout_is_builtin(name: &str) -> bool {
    rustler_core::layout::is_builtin(name)
}

#[tokio::test]
async fn custom_presets_save_list_apply_delete() {
    let f = fx();
    let clip = split_clip(&f).await;
    let id = clip["id"].as_str().unwrap().to_string();

    // built-ins are listed with descriptions and required params; nothing custom yet
    let p = call(&f.reg, &f.ctx, "layout.presets", json!({})).await.unwrap();
    let built: Vec<&str> = p["built_in"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
    assert_eq!(built, ["center-crop", "blur-fit", "split-vertical"]);
    let sv = &p["built_in"][2];
    assert_eq!(sv["required_params"], json!(["top", "bottom"]));
    assert!(!sv["description"].as_str().unwrap().is_empty());
    assert!(p["custom"].as_array().unwrap().is_empty());

    // the older command is untouched
    let old = call(&f.reg, &f.ctx, "layout.list_presets", json!({})).await.unwrap();
    assert_eq!(old["presets"].as_array().unwrap().len(), 3);

    // save a customised layout (split + inset)
    call(&f.reg, &f.ctx, "layout.add_region", json!({"clip_id": id, "id": "cam",
        "src": {"x": 0.0, "y": 0.0, "w": 0.2, "h": 0.2}, "dst": {"x": 0.6, "y": 0.02, "w": 0.35, "h": 0.12}})).await.unwrap();
    let s = call(&f.reg, &f.ctx, "layout.save_preset", json!({"name": "streamer-split", "from_clip_id": id, "description": "webcam top, game bottom"}))
        .await
        .unwrap();
    assert_eq!(s["regions"], 3);
    let path = std::path::PathBuf::from(s["path"].as_str().unwrap());
    assert!(path.ends_with("layouts/streamer-split.json") || path.ends_with("layouts\\streamer-split.json"));
    let file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(file["schema"], "rustler.layout@1");
    assert_eq!(file["regions"].as_array().unwrap().len(), 3);

    // names are validated and cannot shadow built-ins or overwrite silently
    for bad in ["blur-fit", "Bad Name", "../x", ""] {
        let e = call(&f.reg, &f.ctx, "layout.save_preset", json!({"name": bad, "from_clip_id": id})).await.unwrap_err();
        assert_eq!(code(&e), ErrorCode::Validation, "{bad:?}");
    }
    let e = call(&f.reg, &f.ctx, "layout.save_preset", json!({"name": "streamer-split", "from_clip_id": id})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    call(&f.reg, &f.ctx, "layout.save_preset", json!({"name": "streamer-split", "from_clip_id": id, "overwrite": true, "description": "webcam top, game bottom"})).await.unwrap();

    let p = call(&f.reg, &f.ctx, "layout.presets", json!({})).await.unwrap();
    assert_eq!(p["custom"][0]["name"], "streamer-split");
    assert_eq!(p["custom"][0]["regions"], 3);
    assert_eq!(p["custom"][0]["description"], "webcam top, game bottom");

    // apply it to a fresh clip (all segments), then to a built-in preset
    let c2 = call(&f.reg, &f.ctx, "clip.create", json!({"name": "c2", "source_id": f.src.id, "start": 30.0, "end": 36.0})).await.unwrap();
    let id2 = c2["id"].as_str().unwrap().to_string();
    let v = call(&f.reg, &f.ctx, "layout.apply_preset", json!({"clip_id": id2, "name": "streamer-split"})).await.unwrap();
    assert_eq!(v["segments"][0]["regions"].as_array().unwrap().len(), 3);
    assert!(v["segments"][0]["preset"].is_null());
    assert_eq!(v["segments"][0]["regions"][2]["id"], "cam");
    // a clip whose layout is custom can still get another segment (preset falls back to center-crop)
    call(&f.reg, &f.ctx, "clip.add_segment", json!({"clip_id": id2, "source_id": f.src.id, "start": 40.0, "end": 44.0})).await.unwrap();

    let v = call(&f.reg, &f.ctx, "layout.apply_preset", json!({"clip_id": id2, "name": "blur-fit"})).await.unwrap();
    assert_eq!(v["segments"][0]["preset"], "blur-fit");
    assert_eq!(v["segments"][0]["regions"].as_array().unwrap().len(), 1);
    // built-in with required params missing -> validation with the preset's hint
    let e = call(&f.reg, &f.ctx, "layout.apply_preset", json!({"clip_id": id2, "name": "split-vertical"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let (top, bottom) = rects();
    let v = call(
        &f.reg,
        &f.ctx,
        "layout.apply_preset",
        json!({"clip_id": id2, "name": "split-vertical", "params": {"top": top, "bottom": bottom}}),
    )
    .await
    .unwrap();
    assert_eq!(v["segments"][0]["preset"], "split-vertical");
    // unknown names list what exists
    let e = call(&f.reg, &f.ctx, "layout.apply_preset", json!({"clip_id": id2, "name": "nope"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    assert!(e.message.contains("streamer-split"));

    // delete: custom only
    let e = call(&f.reg, &f.ctx, "layout.delete_preset", json!({"name": "blur-fit"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    call(&f.reg, &dry, "layout.delete_preset", json!({"name": "streamer-split"})).await.unwrap();
    assert!(path.is_file(), "dry-run keeps the file");
    let d = call(&f.reg, &f.ctx, "layout.delete_preset", json!({"name": "streamer-split"})).await.unwrap();
    assert_eq!(d["deleted"], true);
    assert!(!path.exists());
    let e = call(&f.reg, &f.ctx, "layout.delete_preset", json!({"name": "streamer-split"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
}

// ---------------------------------------------------------------------------------------------
// Real media: render a split clip and detect a webcam
// ---------------------------------------------------------------------------------------------

const PORTRAIT_URL: &str = "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/Abraham_Lincoln_O-77_matte_collodion_print.jpg/960px-Abraham_Lincoln_O-77_matte_collodion_print.jpg";

/// Public-domain portrait (Abraham Lincoln, Alexander Gardner, 1863; Wikimedia Commons), cached in
/// `<home>/test-media/lincoln.jpg`. `None` (with a printed reason) when it cannot be downloaded.
fn portrait() -> Option<PathBuf> {
    let cache = real_home().join("test-media").join("lincoln.jpg");
    if cache.is_file() && std::fs::metadata(&cache).map(|m| m.len() > 10_000).unwrap_or(false) {
        return Some(cache);
    }
    let _ = std::fs::create_dir_all(cache.parent().unwrap());
    let ok = std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .args(["-sSL", "--max-time", "40", "-A", "rustler-tests/0.1 (support@digitalheroes.co.in)", "-o"])
        .arg(&cache)
        .arg(PORTRAIT_URL)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok && std::fs::metadata(&cache).map(|m| m.len() > 10_000).unwrap_or(false) {
        Some(cache)
    } else {
        let _ = std::fs::remove_file(&cache);
        eprintln!("SKIP/fallback: portrait download failed (offline?)");
        None
    }
}

/// 1280x720 testsrc2 "gameplay" with the portrait as a 320 px wide webcam box in the top-right
/// corner (a plain yellow box when no portrait is available).
fn make_stream(ffmpeg: &Path, out: &Path, portrait: Option<&Path>, with_cam: bool) {
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=6"]);
    match (with_cam, portrait) {
        (true, Some(p)) => {
            cmd.arg("-i").arg(p).args(["-filter_complex", "[1:v]scale=320:-1[p];[0:v][p]overlay=W-w-24:24"]);
        }
        (true, None) => {
            cmd.args(["-vf", "drawbox=x=936:y=24:w=320:h=240:color=yellow:t=fill"]);
        }
        _ => {}
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "veryfast"]).arg(out);
    assert!(cmd.status().expect("run ffmpeg").success(), "fixture video failed");
}

#[tokio::test]
async fn render_a_split_clip_from_boxes_and_look_at_it() {
    let Some((tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let video = proj.path().join("stream.mp4");
    let have_portrait = portrait();
    make_stream(&ffmpeg_in(&home), &video, have_portrait.as_deref(), true);
    let probe = rustler_media::probe(&rustler_tools::ToolPaths::from_home(&home), &video).await.unwrap();
    assert_eq!((probe.width, probe.height), (1280, 720));
    let src = add_source(&store, &video, probe);
    let reg = registry();

    let clip = call(&reg, &ctx, "clip.create", json!({"name": "split", "source_id": src.id, "start": 0.0, "end": 4.0})).await.unwrap();
    let id = clip["id"].as_str().unwrap().to_string();
    call(&reg, &ctx, "clip.set_captions", json!({"clip_id": id, "enabled": false})).await.unwrap();

    // webcam top-right (x 0.73..0.98), gameplay left of it
    let v = call(
        &reg,
        &ctx,
        "layout.from_frame_boxes",
        json!({"clip_id": id, "boxes": [
            {"role": "top", "rect": {"x": 0.725, "y": 0.0, "w": 0.275, "h": 0.5}},
            {"role": "bottom", "rect": {"x": 0.0, "y": 0.0, "w": 0.7, "h": 1.0}}]}),
    )
    .await
    .unwrap();
    assert_eq!(v["segments"][0]["preset"], "split-vertical");
    eprintln!("split = {}", v["segments"][0]["regions"][0]["dst"]["h"]);

    let pf = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 1.0})).await.unwrap();
    let img = pf["image"].as_str().unwrap();
    let p = rustler_media::probe(&rustler_tools::ToolPaths::from_home(&home), Path::new(img)).await.unwrap();
    assert_eq!((p.width, p.height), (1080, 1920));
    let keep = std::env::temp_dir().join("rustler_a2_split_preview.jpg");
    std::fs::copy(img, &keep).unwrap();
    eprintln!("PREVIEW: {}", keep.display());

    // an edited region still renders (shrink the webcam pane via edit_region and look again)
    call(&reg, &ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "top", "dst": {"x": 0.0, "y": 0.0, "w": 1.0, "h": 0.4}}))
        .await
        .unwrap();
    call(&reg, &ctx, "layout.edit_region", json!({"clip_id": id, "region_id": "bottom", "dst": {"x": 0.0, "y": 0.4, "w": 1.0, "h": 0.6}}))
        .await
        .unwrap();
    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    let out = r["path"].as_str().unwrap();
    let pr = rustler_media::probe(&rustler_tools::ToolPaths::from_home(&home), Path::new(out)).await.unwrap();
    assert_eq!((pr.width, pr.height), (1080, 1920));
    drop(tmp);
}

/// Points the commands at the repo's engine and the real tools home. `None` (printed) if uv or
/// the engine sources are missing.
fn engine_home() -> Option<PathBuf> {
    let home = real_home();
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../engine");
    let uv = home.join("bin").join(if cfg!(windows) { "uv.exe" } else { "uv" });
    if !uv.is_file() || !engine.join("rustler_engine").is_dir() {
        eprintln!("SKIP: uv or engine sources not available");
        return None;
    }
    // SAFETY: tests only ever set these to the same values.
    unsafe {
        std::env::set_var("RUSTLER_ENGINE_PROJECT", std::fs::canonicalize(&engine).unwrap());
        if std::env::var_os("HF_HOME").is_none() {
            std::env::set_var("HF_HOME", home.join("models").join("hf"));
        }
    }
    Some(home)
}

#[tokio::test]
async fn detect_webcam_finds_a_corner_face_and_rejects_plain_gameplay() {
    let Some(home) = engine_home() else { return };
    if !home.join("bin").join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }).is_file() {
        eprintln!("SKIP: ffmpeg missing");
        return;
    }
    let Some(face_img) = portrait() else {
        eprintln!("SKIP: portrait unavailable (offline)");
        return;
    };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let ffmpeg = ffmpeg_in(&home);
    let with_cam = proj.path().join("with_cam.mp4");
    let plain = proj.path().join("plain.mp4");
    make_stream(&ffmpeg, &with_cam, Some(&face_img), true);
    make_stream(&ffmpeg, &plain, None, false);
    let cam_src = add_source(&store, &with_cam, rustler_media::probe(&paths, &with_cam).await.unwrap());
    let plain_src = add_source(&store, &plain, rustler_media::probe(&paths, &plain).await.unwrap());
    let reg = registry();

    let r = call(&reg, &ctx, "layout.detect_webcam", json!({"source_id": cam_src.id, "samples": 4})).await;
    let v = match r {
        Ok(v) => v,
        Err(e) if e.code != ErrorCode::Validation => {
            eprintln!("SKIP: engine/model unavailable: {e}");
            return;
        }
        Err(e) => panic!("detect_webcam should find the portrait: {e} hint={:?}", e.hint),
    };
    eprintln!("DETECT: {}", serde_json::to_string_pretty(&v).unwrap());
    assert_eq!(v["position"], "top-right");
    let cam = &v["webcam"];
    // the portrait box is x 936..1256, y 24..~424 of 1280x720: the suggestion must cover most of it
    assert!(cam["x"].as_f64().unwrap() > 0.6 && cam["x"].as_f64().unwrap() < 0.78, "{cam}");
    assert!(cam["y"].as_f64().unwrap() < 0.2, "{cam}");
    assert!(cam["x"].as_f64().unwrap() + cam["w"].as_f64().unwrap() > 0.95, "{cam}");
    assert!(v["face_hits"].as_u64().unwrap() >= 3);
    assert_eq!(v["gameplay_clear_of_webcam"], true);
    let split = v["split"].as_f64().unwrap();
    assert!((0.3..=0.5).contains(&split));
    let cmd = v["commands"][0].as_str().unwrap();
    assert!(cmd.starts_with("rustler clip set-layout <clip_id> --preset split-vertical --params '{"), "{cmd}");

    // the suggestion applies as-is through clip.set_layout and through from_frame_boxes
    let clip = call(&reg, &ctx, "clip.create", json!({"name": "d", "source_id": cam_src.id, "start": 0.0, "end": 4.0})).await.unwrap();
    let id = clip["id"].as_str().unwrap();
    let mut input = v["set_layout_input"].clone();
    input["clip_id"] = json!(id);
    let applied = call(&reg, &ctx, "clip.set_layout", input).await.unwrap();
    assert_eq!(applied["segments"][0]["layout"]["preset"], "split-vertical");
    call(&reg, &ctx, "clip.set_captions", json!({"clip_id": id, "enabled": false})).await.unwrap();
    let pf = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 1.0})).await.unwrap();
    let keep = std::env::temp_dir().join("rustler_a2_detected_preview.jpg");
    std::fs::copy(pf["image"].as_str().unwrap(), &keep).unwrap();
    eprintln!("DETECTED PREVIEW: {}", keep.display());
    let mut boxes = v["from_frame_boxes_input"].clone();
    boxes["clip_id"] = json!(id);
    call(&reg, &ctx, "layout.from_frame_boxes", boxes).await.unwrap();

    // no face anywhere -> validation with a manual hint
    let e = call(&reg, &ctx, "layout.detect_webcam", json!({"source_id": plain_src.id, "samples": 3})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    assert!(e.hint.unwrap_or_default().contains("see frame"));

    // bad sample times
    let e = call(&reg, &ctx, "layout.detect_webcam", json!({"source_id": cam_src.id, "at": [999.0]})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
}

/// Pixel size of an image via ffprobe.
fn image_size(home: &Path, img: &Path) -> (u32, u32) {
    let ffprobe = home.join("bin").join(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
    let out = std::process::Command::new(ffprobe)
        .args(["-v", "error", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(img)
        .output()
        .expect("run ffprobe");
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let mut it = s.trim().split(',').map(|v| v.parse::<u32>().unwrap());
    (it.next().unwrap(), it.next().unwrap())
}

/// testsrc2 1280x720 with the portrait scaled to 320 px wide (plus an optional white `border` px frame)
/// overlaid with its right edge 24 px from the frame edge and its top 24 px down. Returns the true
/// outer overlay rectangle in pixels (x0, y0, x1, y1).
fn make_stream_with_border(home: &Path, out: &Path, portrait: &Path, border: u32) -> (u32, u32, u32, u32) {
    let (pw, ph0) = image_size(home, portrait);
    let w = 320u32;
    let h = ((ph0 as f64 * w as f64 / pw as f64) / 2.0).round() as u32 * 2; // scale=320:-2 rounds to even
    let (bw, bh) = (w + 2 * border, h + 2 * border);
    let chain = if border > 0 {
        format!("[1:v]scale={w}:-2,pad={bw}:{bh}:{border}:{border}:white[p];[0:v][p]overlay=W-w-24:24")
    } else {
        format!("[1:v]scale={w}:-2[p];[0:v][p]overlay=W-w-24:24")
    };
    let ok = std::process::Command::new(ffmpeg_in(home))
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=6", "-i"])
        .arg(portrait)
        .args(["-filter_complex", &chain, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "veryfast"])
        .arg(out)
        .status()
        .expect("run ffmpeg")
        .success();
    assert!(ok, "fixture video failed");
    (1280 - 24 - bw, 24, 1280 - 24, 24 + bh)
}

#[tokio::test]
async fn detect_webcam_snaps_to_the_overlay_edges_within_1_5_percent() {
    let Some(home) = engine_home() else { return };
    if !ffmpeg_in(&home).is_file() || !home.join("bin").join(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" }).is_file() {
        eprintln!("SKIP: ffmpeg/ffprobe missing");
        return;
    }
    let Some(face_img) = portrait() else {
        eprintln!("SKIP: portrait unavailable (offline)");
        return;
    };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let reg = registry();
    for border in [0u32, 4] {
        let video = proj.path().join(format!("snap_b{border}.mp4"));
        let (x0, y0, x1, y1) = make_stream_with_border(&home, &video, &face_img, border);
        let src = add_source(&store, &video, rustler_media::probe(&paths, &video).await.unwrap());
        let r = call(&reg, &ctx, "layout.detect_webcam", json!({"source_id": src.id, "samples": 6})).await;
        let v = match r {
            Ok(v) => v,
            Err(e) if e.code != ErrorCode::Validation => {
                eprintln!("SKIP: engine/model unavailable: {e}");
                return;
            }
            Err(e) => panic!("detect_webcam failed (border {border}): {e} hint={:?}", e.hint),
        };
        eprintln!("SNAP border={border}: webcam={} expansion={} source={} conf={}", v["webcam"], v["expansion_webcam"], v["rect_source"], v["rect_confidence"]);
        assert_eq!(v["rect_source"], "edges", "border {border}: {v}");
        assert!(v["rect_confidence"].as_f64().unwrap() >= 0.6);
        let cam = &v["webcam"];
        let (cx, cy) = (cam["x"].as_f64().unwrap(), cam["y"].as_f64().unwrap());
        let (cx1, cy1) = (cx + cam["w"].as_f64().unwrap(), cy + cam["h"].as_f64().unwrap());
        let truth = [x0 as f64 / 1280.0, y0 as f64 / 720.0, x1 as f64 / 1280.0, y1 as f64 / 720.0];
        for (name, got, want) in [("left", cx, truth[0]), ("top", cy, truth[1]), ("right", cx1, truth[2]), ("bottom", cy1, truth[3])] {
            assert!((got - want).abs() <= 0.015, "border {border}: {name} edge {got:.4} vs true {want:.4}");
        }
        // the old expansion is still reported for comparison and the legacy fields are unchanged
        assert!(v["expansion_webcam"]["w"].as_f64().unwrap() > 0.0);
        assert!(v["webcam_px"]["w"].as_i64().unwrap() > 0 && v["gameplay_px"]["w"].as_i64().unwrap() > 0);
        assert!(v["set_layout_input"]["params"]["top"].is_object());
    }
}
