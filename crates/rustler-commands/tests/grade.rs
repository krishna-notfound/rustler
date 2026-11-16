//! grade.* (f38): set / show / clear / presets, per-segment overrides, history undo, LUT assets,
//! templates, and a real render where the picture moves the right way while overlays keep
//! their exact colours.
mod common;

use common::*;
use rustler_core::ErrorCode;
use rustler_media::grade::cube_from_fn;
use serde_json::{Value, json};

fn seg_ids(c: &Value) -> Vec<String> {
    c["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

async fn two_segment_clip(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> (String, Vec<String>) {
    let c = call(reg, ctx, "clip.create", json!({"name": "g", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    let c = call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap();
    (id, seg_ids(&c))
}

#[tokio::test]
async fn set_show_clear_undo_and_segment_overrides() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let (id, segs) = two_segment_clip(&reg, &ctx, &src).await;

    // Nothing is graded by default.
    let s = call(&reg, &ctx, "grade.show", json!({"clip_id": id})).await.unwrap();
    assert!(s["segments"].as_array().unwrap().iter().all(|x| x["filter"].is_null()), "{s}");
    let rev0 = s["revision"].as_u64().unwrap();

    // Clip grade: preset at 60 %.
    let a = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "preset": "cinematic", "intensity": 0.6})).await.unwrap();
    assert_eq!(a["revision"].as_u64().unwrap(), rev0 + 1, "a grade change is a new revision");
    assert_eq!(a["grade"]["clip"]["preset"], "cinematic");
    let f0 = a["segments"][0]["filter"].as_str().unwrap().to_string();
    assert!(f0.starts_with("colorcorrect=") && f0.contains("curves=master="), "{f0}");
    assert_eq!(a["segments"][1]["filter"], a["segments"][0]["filter"]);
    assert!(a["next"][0].as_str().unwrap().starts_with("rustler grade preview"));

    // Merge: a later set keeps the preset; a segment override merges onto the clip grade.
    let b = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "contrast": 1.05})).await.unwrap();
    assert_eq!(b["grade"]["clip"]["preset"], "cinematic");
    assert_eq!(b["grade"]["clip"]["contrast"], 1.05);
    let c = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": segs[1], "exposure": 0.2, "shadows": "0,0,0.1"})).await.unwrap();
    assert_eq!(c["segments"][1]["has_override"], true);
    assert_eq!(c["segments"][0]["has_override"], false);
    assert_ne!(c["segments"][1]["filter"], c["segments"][0]["filter"]);
    assert_eq!(c["grade"]["segments"][&segs[1]]["shadows"], json!({"r": 0.0, "g": 0.0, "b": 0.1}));

    // Validation: ranges, presets, rgb text, unknown segment; nothing written.
    for bad in [
        json!({"clip_id": id, "exposure": 2.0}),
        json!({"clip_id": id, "contrast": 0.1}),
        json!({"clip_id": id, "temperature": 500}),
        json!({"clip_id": id, "preset": "sepia-dream"}),
        json!({"clip_id": id, "highlights": "1,2"}),
    ] {
        assert_eq!(code(&call(&reg, &ctx, "grade.set", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }
    let e = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": "seg_nope", "exposure": 0.1})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    // Dry run: shows the result, writes nothing.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let d = call(&reg, &dry, "grade.set", json!({"clip_id": id, "preset": "bw"})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    let s = call(&reg, &ctx, "grade.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(s["grade"]["clip"]["preset"], "cinematic");
    assert_eq!(s["segments"][1]["filter"], c["segments"][1]["filter"], "show prints what set built");

    // history undo restores the previous grade (the override goes away).
    let u = call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert!(u.get("warnings").is_none_or(|w| !w.to_string().contains("no layer snapshot")), "{u}");
    let s = call(&reg, &ctx, "grade.show", json!({"clip_id": id})).await.unwrap();
    assert!(s["grade"]["segments"].is_null() || s["grade"]["segments"].as_object().unwrap().is_empty(), "{s}");
    assert_eq!(s["grade"]["clip"]["contrast"], 1.05);

    // Re-cuts carry a segment override onto the new pieces.
    call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": segs[1], "saturation": 0.5})).await.unwrap();
    let sp = call(&reg, &ctx, "edit.split", json!({"clip_id": id, "out_t": 2.4})).await.unwrap();
    assert!(sp.to_string().contains("segments"), "{sp}");
    let s = call(&reg, &ctx, "grade.show", json!({"clip_id": id})).await.unwrap();
    let rows = s["segments"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[1]["has_override"].as_bool().unwrap() && rows[2]["has_override"].as_bool().unwrap(), "{s}");
    assert!(s.get("warnings").is_none(), "no orphaned override: {s}");

    // clear one override, then everything; undo brings it back.
    let one = call(&reg, &ctx, "grade.clear", json!({"clip_id": id, "segment_id": rows[1]["segment_id"]})).await.unwrap();
    assert_eq!(one["segments"][1]["has_override"], false);
    let e = call(&reg, &ctx, "grade.clear", json!({"clip_id": id, "segment_id": rows[1]["segment_id"]})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let all = call(&reg, &ctx, "grade.clear", json!({"clip_id": id})).await.unwrap();
    assert!(all["segments"].as_array().unwrap().iter().all(|x| x["filter"].is_null()));
    let x: Value = serde_json::from_slice(&std::fs::read(store.root.join("clips").join(&id).join("extras.json")).unwrap()).unwrap();
    assert!(x.get("grade").is_none(), "{x}");
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    let s = call(&reg, &ctx, "grade.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(s["grade"]["clip"]["preset"], "cinematic");

    // Presets.
    let p = call(&reg, &ctx, "grade.presets", json!({})).await.unwrap();
    let names: Vec<&str> = p["presets"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    for n in ["natural-pop", "warm", "cool", "cinematic", "moody", "bright-clean", "bw", "vintage"] {
        assert!(names.contains(&n), "{names:?}");
    }
    assert!(p["presets"].as_array().unwrap().iter().all(|x| x["filter"].is_string() && !x["description"].as_str().unwrap().is_empty()));
}

#[tokio::test]
async fn lut_assets_and_templates() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let (id, segs) = two_segment_clip(&reg, &ctx, &src).await;

    // A generated (own) LUT goes into the library as kind lut; a broken one is refused.
    let cube = proj.path().join("warm-look.cube");
    std::fs::write(&cube, cube_from_fn("warm", 9, |r, g, b| ((r * 1.08).min(1.0), g, b * 0.92))).unwrap();
    let bad = proj.path().join("bad.cube");
    std::fs::write(&bad, "LUT_3D_SIZE 4\n0 0 0\n").unwrap();
    let e = call(&reg, &ctx, "asset.add", json!({"path": bad, "kind": "lut", "license": "CC0-1.0"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let e = call(&reg, &ctx, "asset.add", json!({"path": media, "kind": "lut", "license": "CC0-1.0"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation, "a .cube file is required");
    let a = call(&reg, &ctx, "asset.add", json!({"path": cube, "kind": "lut", "license": "CC0-1.0"})).await.unwrap();
    let lut_id = a["asset"]["id"].as_str().unwrap().to_string();
    assert_eq!(a["asset"]["kind"], "lut");
    assert!(a["next"].to_string().contains("grade set"), "{a}");

    let g = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "lut": format!("asset:{lut_id}"), "lut_strength": 0.5})).await.unwrap();
    assert_eq!(g["grade"]["clip"]["lut"], format!("asset:{lut_id}"));
    let f = g["segments"][0]["filter"].as_str().unwrap();
    assert!(f.starts_with("split=2[s0ga][s0gb];[s0gb]lut3d=file='grade_lut0.cube'"), "{f}");
    assert!(g["segments"][1]["filter"].as_str().unwrap().contains("[s1gb]lut3d=file='grade_lut0.cube'"), "same staged file for both");
    // A bare id or a path work too; a missing LUT is NotFound; "none" drops it.
    let g2 = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "lut": lut_id})).await.unwrap();
    assert_eq!(g2["grade"]["clip"]["lut"], format!("asset:{lut_id}"));
    let g3 = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "lut": cube})).await.unwrap();
    assert!(g3["grade"]["clip"]["lut"].as_str().unwrap().ends_with("warm-look.cube"));
    // A segment override inherits the clip LUT unless it says "none".
    let s1 = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": segs[1], "exposure": 0.1})).await.unwrap();
    assert!(s1["segments"][1]["filter"].as_str().unwrap().contains("lut3d"), "{s1}");
    let s2 = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": segs[1], "lut": "none"})).await.unwrap();
    assert!(!s2["segments"][1]["filter"].as_str().unwrap().contains("lut3d") && s2["segments"][0]["filter"].as_str().unwrap().contains("lut3d"), "{s2}");
    // --reset on a segment: standalone, nothing inherited (no grade at all here).
    let s3 = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": segs[1], "reset": true})).await.unwrap();
    assert!(s3["segments"][1]["filter"].is_null(), "{s3}");
    call(&reg, &ctx, "grade.clear", json!({"clip_id": id, "segment_id": segs[1]})).await.unwrap();
    let e = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "lut": "nope-lut"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let g4 = call(&reg, &ctx, "grade.set", json!({"clip_id": id, "lut": "none", "preset": "warm"})).await.unwrap();
    assert!(g4["grade"]["clip"].get("lut").is_none() && g4["grade"]["clip"].get("lut_strength").is_none(), "{g4}");

    // Templates: the clip grade is captured and applied; a bad grade in a template is refused.
    let t = call(&reg, &ctx, "template.create", json!({"name": "warm-look", "from_clip": id})).await.unwrap();
    assert_eq!(t["name"], "warm-look", "{t}");
    let shown = call(&reg, &ctx, "template.show", json!({"name": "warm-look"})).await.unwrap();
    assert_eq!(shown.pointer("/template/grade/preset").or(shown.pointer("/grade/preset")), Some(&json!("warm")), "{shown}");
    let c2 = call(&reg, &ctx, "clip.create", json!({"name": "other", "source_id": src.id, "start": 5.05, "end": 7.0})).await.unwrap();
    let id2 = c2["id"].as_str().unwrap().to_string();
    assert!(call(&reg, &ctx, "grade.show", json!({"clip_id": id2})).await.unwrap()["grade"]["clip"].is_null(), "new clips are never graded");
    call(&reg, &ctx, "template.apply", json!({"name": "warm-look", "clip_ids": [id2]})).await.unwrap();
    let s2 = call(&reg, &ctx, "grade.show", json!({"clip_id": id2})).await.unwrap();
    assert_eq!(s2["grade"]["clip"]["preset"], "warm", "{s2}");
    let e = call(&reg, &ctx, "template.create", json!({"name": "bad-grade", "template": {"grade": {"exposure": 5}}})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    assert!(e.message.contains("/grade"), "{}", e.message);
}

/// Mean (r, g, b) and mean |r-g| + |g-b| of a jpg region.
fn region_stats(path: &str, x0: f64, y0: f64, x1: f64, y1: f64) -> ([f64; 3], f64) {
    let img = image::open(path).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let (xa, xb, ya, yb) = ((x0 * w as f64) as u32, (x1 * w as f64) as u32, (y0 * h as f64) as u32, (y1 * h as f64) as u32);
    let (mut s, mut chroma, mut n) = ([0.0; 3], 0.0, 0.0);
    for y in ya..yb {
        for x in xa..xb {
            let p = img.get_pixel(x, y).0;
            for k in 0..3 {
                s[k] += p[k] as f64;
            }
            chroma += (p[0] as f64 - p[1] as f64).abs() + (p[1] as f64 - p[2] as f64).abs();
            n += 1.0;
        }
    }
    ([s[0] / n, s[1] / n, s[2] / n], chroma / n)
}

#[tokio::test]
async fn real_render_moves_pixels_and_keeps_overlay_colours() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 8, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let (id, segs) = two_segment_clip(&reg, &ctx, &src).await;
    let cube = proj.path().join("redder.cube");
    std::fs::write(&cube, cube_from_fn("redder", 9, |r, g, b| ((r * 1.15 + 0.05).min(1.0), g, b * 0.85))).unwrap();

    // Segment 1: warm white balance + a LUT at half strength; segment 2: black and white.
    call(&reg, &ctx, "grade.set", json!({"clip_id": id, "temperature": 4000, "lut": cube, "lut_strength": 0.5})).await.unwrap();
    // --reset on a segment: only bw there, nothing inherited from the clip grade.
    call(&reg, &ctx, "grade.set", json!({"clip_id": id, "segment_id": segs[1], "reset": true, "preset": "bw"})).await.unwrap();
    call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 0.3})).await.unwrap();
    call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": true, "height": 24})).await.unwrap();

    // Before / after previews (edit time 0.7 = segment 1, 2.9 = segment 2).
    let warm = call(&reg, &ctx, "grade.preview", json!({"clip_id": id, "out_t": 0.7})).await.unwrap();
    let bw = call(&reg, &ctx, "grade.preview", json!({"clip_id": id, "out_t": 2.9})).await.unwrap();
    for p in [&warm, &bw] {
        assert!(std::path::Path::new(p["image"].as_str().unwrap()).is_file(), "{p}");
        let (w, h) = image::image_dimensions(p["image"].as_str().unwrap()).unwrap();
        assert!(h == 960 && w > h, "side by side {w}x{h}");
    }
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(warm["image"].as_str().unwrap(), std::path::Path::new(&dir).join("grade_warm.jpg"));
        let _ = std::fs::copy(bw["image"].as_str().unwrap(), std::path::Path::new(&dir).join("grade_bw.jpg"));
    }
    // Picture region (below the bar, above the captions).
    let (b, _) = region_stats(warm["before"].as_str().unwrap(), 0.1, 0.2, 0.9, 0.6);
    let (a, _) = region_stats(warm["after"].as_str().unwrap(), 0.1, 0.2, 0.9, 0.6);
    // testsrc2 is saturated colour bars, so the mean shift is smaller than on a grey card.
    assert!((a[0] - a[2]) > (b[0] - b[2]) + 8.0, "warmer = more red than blue: before {b:?} after {a:?}");
    let (_, chroma_before) = region_stats(bw["before"].as_str().unwrap(), 0.1, 0.2, 0.9, 0.6);
    let (_, chroma_after) = region_stats(bw["after"].as_str().unwrap(), 0.1, 0.2, 0.9, 0.6);
    assert!(chroma_before > 30.0 && chroma_after < 6.0, "bw: R=G=B ({chroma_before} -> {chroma_after})");
    // The progress bar (drawn after the grade) keeps its exact yellow on the bw segment.
    let (bar, _) = region_stats(bw["after"].as_str().unwrap(), 0.02, 0.002, 0.3, 0.008);
    assert!(bar[0] > 220.0 && bar[1] > 200.0 && bar[2] < 70.0, "bar stays #FFE000: {bar:?}");

    // Full render: the graph grades each segment before its layout, the LUT is staged.
    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    let rid = r["render_id"].as_str().unwrap();
    let rw = store.root.join("clips").join(&id).join("renders").join(rid);
    assert!(rw.join("grade_lut0.cube").is_file());
    let g = std::fs::read_to_string(rw.join("graph.txt")).unwrap();
    assert!(g.contains("setpts=PTS-STARTPTS,colortemperature=temperature=4000:pl=1,split=2[s0ga][s0gb]"), "{g}");
    assert!(g.contains("colorchannelmixer=rr=0.2126"), "{g}");
    let out = rustler_media::probe(&paths, std::path::Path::new(r["path"].as_str().unwrap())).await.unwrap();
    assert!((out.duration - 3.1).abs() < 0.05, "{}", out.duration);

    // Measuring reads frames and suggests; it changes nothing.
    let before = std::fs::read(store.root.join("clips").join(&id).join("extras.json")).unwrap();
    let m = call(&reg, &ctx, "grade.measure", json!({"id": id})).await.unwrap();
    assert_eq!(m["frames"].as_array().unwrap().len(), 2, "{m}");
    assert!(!m["read"].as_array().unwrap().is_empty() && m["note"].as_str().unwrap().contains("nothing was changed"));
    assert_eq!(std::fs::read(store.root.join("clips").join(&id).join("extras.json")).unwrap(), before);
    let ms = call(&reg, &ctx, "grade.measure", json!({"id": src.id, "t": [1.0, 4.0]})).await.unwrap();
    assert_eq!(ms["frames"].as_array().unwrap().len(), 2);
    assert!(ms["overall"]["saturation"].as_f64().unwrap() > 0.2, "testsrc2 is colourful: {ms}");
}
