//! transition.* / effect.* / overlay.progress_bar tests + a real render where captions,
//! overlays and the rendered duration must follow a transition.
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};

fn extras_json(store: &rustler_core::ProjectStore, clip: &str) -> Value {
    serde_json::from_slice(&std::fs::read(store.root.join("clips").join(clip).join("extras.json")).unwrap()).unwrap()
}

/// Clip of source [1.0, 2.4] ("Hello there friends.") + [3.0, 5.0] ("Fellow Americans ask not.")
/// + [5.05, 7.0] ("What your country can do.").
async fn three_segments(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> Value {
    let c = call(reg, ctx, "clip.create", json!({"name": "fx", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 5.05, "end": 7.0})).await.unwrap()
}

fn seg_ids(c: &Value) -> Vec<String> {
    c["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn transitions_effects_and_progress_bar_crud() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();
    let segs = seg_ids(&c);
    let edit = c["duration"].as_f64().unwrap();
    assert!((edit - 5.35).abs() < 1e-9, "{edit}");
    std::fs::write(store.root.join("clips").join(&id).join("extras.json"), r#"{"sfx":[{"id":"s1","file":"x.wav","at":1.0,"volume_db":-6.0}]}"#).unwrap();

    // dissolve (default 0.3 s) after segment 0.
    let t = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve"})).await.unwrap();
    assert_eq!(t["transitions"][0]["kind"], "dissolve");
    assert_eq!(t["transitions"][0]["frames"], 9);
    assert!((t["rendered_duration"].as_f64().unwrap() - (edit - 0.3)).abs() < 1e-9, "{t}");
    assert!((t["removed"].as_f64().unwrap() - 0.3).abs() < 1e-9);
    // The cut is at 1.4 s; rendered window centred on 1.25 s.
    assert!((t["transitions"][0]["at"].as_f64().unwrap() - 1.4).abs() < 1e-9);
    assert!((t["transitions"][0]["rendered_start"].as_f64().unwrap() - 1.1).abs() < 1e-9, "{t}");
    assert!(t["note"].as_str().unwrap().contains("edit timeline"));
    // Fade after segment 1; replacing keeps one entry per segment.
    call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[1], "kind": "wipeleft", "duration": 0.5})).await.unwrap();
    let t = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[1], "kind": "fade", "duration": 0.5})).await.unwrap();
    assert_eq!(t["transitions"].as_array().unwrap().len(), 2);
    assert_eq!(t["transitions"][1]["kind"], "fade");
    assert!((t["rendered_duration"].as_f64().unwrap() - (edit - 0.8)).abs() < 1e-9);

    // Validation: last segment, too long for its neighbours, bad duration, unknown segment.
    let e = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[2], "kind": "fade"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let e = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 1.6}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation, "1.6 + 0.5 s around the 2 s segment");
    assert_eq!(e.stage, "transition.set");
    let e = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 3.0}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let e = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": "seg_nope", "kind": "fade"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    assert_eq!(extras_json(&store, &id)["transitions"][0]["duration"], 0.3, "failed set wrote nothing");

    // `cut` removes the transition; list; clear one; clear all.
    let t = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "cut"})).await.unwrap();
    assert_eq!(t["transitions"].as_array().unwrap().len(), 1);
    let l = call(&reg, &ctx, "transition.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["transitions"][0]["after_segment_id"], segs[1]);
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let d = call(&reg, &dry, "transition.clear", json!({"clip_id": id})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    assert_eq!(extras_json(&store, &id)["transitions"].as_array().unwrap().len(), 1);
    let e = call(&reg, &ctx, "transition.clear", json!({"clip_id": id, "after_segment_id": segs[0]})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    call(&reg, &ctx, "transition.clear", json!({"clip_id": id})).await.unwrap();
    let x = extras_json(&store, &id);
    assert!(x.get("transitions").is_none(), "{x}");
    assert_eq!(x["sfx"][0]["id"], "s1", "other keys survive");

    // Effects.
    let a = call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 1.0, "end": 2.0})).await.unwrap();
    assert_eq!(a["effect_id"], "fx_1");
    assert_eq!(a["effect"]["amount_used"], 1.15);
    assert_eq!(a["effect"]["rendered_start"], 1.0);
    let f = call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "flash", "start": 3.0, "end": 4.5, "amount": 0.5})).await.unwrap();
    assert_eq!(f["warnings"].as_array().unwrap().len(), 2, "{f}");
    for bad in [
        json!({"clip_id": id, "kind": "punch-in", "start": 2.0, "end": 1.0}),
        json!({"clip_id": id, "kind": "punch-in", "start": 9.0, "end": 10.0}),
        json!({"clip_id": id, "kind": "zoom-slow", "start": 0.0, "end": 1.0, "amount": 4.0}),
        json!({"clip_id": id, "kind": "shake", "start": 0.0, "end": 1.0, "amount": 0.5}),
    ] {
        assert_eq!(code(&call(&reg, &ctx, "effect.add", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }
    assert_eq!(code(&call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "spin", "start": 0.0, "end": 1.0})).await.unwrap_err()), ErrorCode::Validation);
    // With a transition, list reports rendered windows.
    call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 0.4})).await.unwrap();
    let l = call(&reg, &ctx, "effect.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["effects"].as_array().unwrap().len(), 2);
    assert!((l["effects"][1]["rendered_start"].as_f64().unwrap() - 2.6).abs() < 1e-9, "{l}");
    let r = call(&reg, &ctx, "effect.remove", json!({"clip_id": id, "effect_id": "fx_1"})).await.unwrap();
    assert_eq!(r["remaining"], json!(["fx_2"]));
    assert_eq!(code(&call(&reg, &ctx, "effect.remove", json!({"clip_id": id, "effect_id": "fx_1"})).await.unwrap_err()), ErrorCode::NotFound);

    // Progress bar.
    let p = call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": true})).await.unwrap();
    assert_eq!(p["progress_bar"], json!({"color": "#FFE000", "height": 8, "position": "top"}));
    let p = call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": true, "position": "bottom", "height": 12})).await.unwrap();
    assert_eq!(p["progress_bar"]["position"], "bottom");
    assert_eq!(p["progress_bar"]["color"], "#FFE000", "unchanged fields are kept");
    for bad in [json!({"clip_id": id, "enabled": true, "color": "yellow"}), json!({"clip_id": id, "enabled": true, "height": 500})] {
        assert_eq!(code(&call(&reg, &ctx, "overlay.progress_bar", bad).await.unwrap_err()), ErrorCode::Validation);
    }
    assert_eq!(extras_json(&store, &id)["progress_bar"]["height"], 12);
    let l = call(&reg, &ctx, "effect.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["progress_bar"]["height"], 12);
    call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": false})).await.unwrap();
    assert!(extras_json(&store, &id).get("progress_bar").is_none());

    // A deleted segment makes its transition stale (ignored at render, flagged in the list).
    let mut clip = store.load_clip(&id.parse::<rustler_core::ClipId>().unwrap()).unwrap();
    let base = clip.revision;
    clip.segments.remove(0);
    store.save_clip(&mut clip, base).unwrap();
    let l = call(&reg, &ctx, "transition.list", json!({"clip_id": id})).await.unwrap();
    assert!(l["transitions"][0]["problem"].as_str().unwrap().contains("no longer exists"), "{l}");
    assert_eq!(l["removed"], 0.0);
}

/// Start and end (seconds) of every `Dialogue:` line whose text contains `needle`.
fn dialogue_times(ass: &str, needle: &str) -> Vec<(f64, f64)> {
    let t = |s: &str| -> f64 {
        let p: Vec<f64> = s.split(':').map(|x| x.parse().unwrap()).collect();
        p[0] * 3600.0 + p[1] * 60.0 + p[2]
    };
    ass.lines()
        .filter(|l| l.starts_with("Dialogue:") && l.contains(needle))
        .map(|l| {
            let f: Vec<&str> = l.splitn(10, ',').collect();
            (t(f[1]), t(f[2]))
        })
        .collect()
}

#[tokio::test]
async fn render_with_transition_keeps_captions_overlays_and_duration_in_sync() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = call(&reg, &ctx, "clip.create", json!({"name": "fx", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    let c = call(&reg, &ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap();
    let segs = seg_ids(&c);
    call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 0.3}))
        .await
        .unwrap();
    // Overlay at edit 2.0 .. 3.0 -> rendered 1.7 .. 2.7.
    call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "Ask not", "start": 2.0, "end": 3.0})).await.unwrap();
    call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 2.0, "end": 3.0})).await.unwrap();
    call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": true})).await.unwrap();

    // Preview at edit time 2.0 -> rendered 1.7 (inside the punch-in).
    let pf = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 2.0})).await.unwrap();
    let img = pf["image"].as_str().unwrap();
    assert!(img.ends_with("frame_1700ms.jpg"), "{img}");
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(img, std::path::Path::new(&dir).join("cmd_fx_preview.jpg"));
    }

    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    // 1.4 + 2.0 - 0.3.
    assert!((r["duration"].as_f64().unwrap() - 3.1).abs() < 1e-9, "{r}");
    let rid = r["render_id"].as_str().unwrap();
    let rw = store.root.join("clips").join(&id).join("renders").join(rid);
    let ass = std::fs::read_to_string(rw.join("captions.ass")).unwrap();
    // "Fellow" (source 3.0; the preset upper-cases) is edit 1.4 -> moved 0.3 earlier and clamped to the overlap midpoint 1.25.
    let fellow = dialogue_times(&ass, "FELLOW");
    assert!(!fellow.is_empty(), "{ass}");
    assert!((fellow[0].0 - 1.25).abs() < 0.011, "Fellow starts at {:?}", fellow[0]);
    // "Hello there friends." ends before the midpoint; "not." ends at the rendered end (+ tail).
    assert!(dialogue_times(&ass, "HELLO").iter().all(|(_, e)| *e <= 1.25 + 0.16), "{ass}");
    let last_end = ass.lines().filter(|l| l.starts_with("Dialogue:")).map(|l| {
        let f: Vec<&str> = l.splitn(10, ',').collect();
        let p: Vec<f64> = f[2].split(':').map(|x| x.parse().unwrap()).collect();
        p[0] * 3600.0 + p[1] * 60.0 + p[2]
    }).fold(0.0, f64::max);
    assert!(last_end > 3.0 && last_end <= 3.1 + 0.16, "last caption ends at {last_end} (unmapped would be >= 3.4)");
    let ov = std::fs::read_to_string(rw.join("overlays.ass")).unwrap();
    assert!(ov.contains("0:00:01.70,0:00:02.70"), "{ov}");
    let g = std::fs::read_to_string(rw.join("graph.txt")).unwrap();
    assert!(g.contains("xfade=transition=fade:duration=0.3:offset=1.1[vcat]"), "{g}");
    assert!(g.contains("enable='between(n,51,80)'"), "punch-in on rendered frames 51..80: {g}");

    // The file is as long as the rendered timeline (what QC must compare against).
    let out = rustler_media::probe(&paths, std::path::Path::new(r["path"].as_str().unwrap())).await.unwrap();
    assert!((out.duration - 3.1).abs() < 0.05, "file duration {}", out.duration);
    let eff = rustler_commands::fx_cmds::effective_duration_of(&store, &store.load_clip(&id.parse().unwrap()).unwrap()).unwrap();
    assert!((eff - 3.1).abs() < 1e-9);
}
