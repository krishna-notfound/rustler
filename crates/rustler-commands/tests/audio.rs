//! audio.* command tests (denoise, volume, sfx, list) + a real render with all of them.
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};

fn extras_json(store: &rustler_core::ProjectStore, clip: &str) -> Value {
    serde_json::from_slice(&std::fs::read(store.root.join("clips").join(clip).join("extras.json")).unwrap()).unwrap()
}

async fn two_segment_clip(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> Value {
    let c = call(reg, ctx, "clip.create", json!({"name": "a", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap()
}

#[tokio::test]
async fn audio_commands_store_and_resolve() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let beep = proj.path().join("beep.wav");
    std::fs::write(&beep, b"RIFF").unwrap();
    let reg = registry();
    let clip = two_segment_clip(&reg, &ctx, &src).await;
    let id = clip["id"].as_str().unwrap().to_string();
    let seg1 = clip["segments"][1]["id"].as_str().unwrap().to_string();
    // Another module's key must survive.
    std::fs::write(store.root.join("clips").join(&id).join("extras.json"), r#"{"overlays":[{"id":"ovl_1","text":"hi","start":0.0,"end":1.0}]}"#).unwrap();

    // denoise (default medium) and off.
    let d = call(&reg, &ctx, "audio.denoise", json!({"clip_id": id})).await.unwrap();
    assert_eq!(d["audio_fx"]["denoise"], "medium");
    assert_eq!(extras_json(&store, &id)["audio_fx"]["denoise"], "medium");
    let d = call(&reg, &ctx, "audio.denoise", json!({"clip_id": id, "strength": "strong"})).await.unwrap();
    assert!(d["warnings"][0].as_str().unwrap().contains("watery"));
    assert_eq!(code(&call(&reg, &ctx, "audio.denoise", json!({"clip_id": id, "strength": "max"})).await.unwrap_err()), ErrorCode::Validation);

    // volume: per segment and whole dialogue.
    let v = call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "segment_id": seg1, "db": -6.0})).await.unwrap();
    assert_eq!(v["audio_fx"]["segment_volumes"][&seg1], -6.0);
    let v = call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "db": 2.0})).await.unwrap();
    assert_eq!(v["audio_fx"]["volume_db"], 2.0);
    assert!(v["warnings"][0].as_str().unwrap().contains("loudnorm"));
    assert_eq!(code(&call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "db": 40.0})).await.unwrap_err()), ErrorCode::Validation);
    let nf = call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "segment_id": "seg_nope", "db": 1.0})).await.unwrap_err();
    assert_eq!(code(&nf), ErrorCode::NotFound);
    assert!(nf.hint.unwrap().contains(&seg1));

    // sfx at a time and on a word ("Americans" = word 4, source 3.45 s -> edit 1.4 + 0.45).
    let a = call(&reg, &ctx, "audio.add_sfx", json!({"clip_id": id, "file": beep, "at": 0.5})).await.unwrap();
    assert_eq!(a["sfx_id"], "sfx_1");
    assert_eq!(a["sfx"]["volume_db"], -6.0);
    assert_eq!(a["sfx"]["resolved_at"], 0.5);
    let w = call(
        &reg,
        &ctx,
        "audio.add_sfx",
        json!({"clip_id": id, "file": beep, "at_word": {"source_id": src.id, "word_index": 4}, "volume_db": -10.0}),
    )
    .await
    .unwrap();
    assert_eq!(w["sfx_id"], "sfx_2");
    assert!((w["sfx"]["resolved_at"].as_f64().unwrap() - 1.85).abs() < 1e-9, "{w}");
    for bad in [
        json!({"clip_id": id, "file": beep}),
        json!({"clip_id": id, "file": beep, "at": 1.0, "at_word": {"source_id": src.id, "word_index": 4}}),
        json!({"clip_id": id, "file": beep, "at": 99.0}),
        // "do." (word 11) is not in the clip.
        json!({"clip_id": id, "file": beep, "at_word": {"source_id": src.id, "word_index": 11}}),
        json!({"clip_id": id, "file": media, "at": 1.0, "volume_db": 30.0}),
    ] {
        assert_eq!(code(&call(&reg, &ctx, "audio.add_sfx", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }
    let txt = proj.path().join("notes.txt");
    std::fs::write(&txt, b"x").unwrap();
    assert_eq!(code(&call(&reg, &ctx, "audio.add_sfx", json!({"clip_id": id, "file": txt, "at": 1.0})).await.unwrap_err()), ErrorCode::Validation);
    assert_eq!(
        code(&call(&reg, &ctx, "audio.add_sfx", json!({"clip_id": id, "file": proj.path().join("none.wav"), "at": 1.0})).await.unwrap_err()),
        ErrorCode::NotFound
    );

    // Re-cut: drop the first segment -> the word-anchored effect moves with the word.
    let mut c = store.load_clip(&id.parse::<rustler_core::ClipId>().unwrap()).unwrap();
    let base = c.revision;
    c.segments.remove(0);
    store.save_clip(&mut c, base).unwrap();
    let l = call(&reg, &ctx, "audio.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["denoise"], "strong");
    assert_eq!(l["volume_db"], 2.0);
    assert_eq!(l["segment_volumes"][0]["db"], -6.0);
    let sfx = l["sfx"].as_array().unwrap();
    assert_eq!(sfx.len(), 2);
    assert!((sfx[1]["resolved_at"].as_f64().unwrap() - 0.45).abs() < 1e-9, "{l}");
    assert!(l["music"].is_null());

    // dry-run remove writes nothing; real remove does.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let r = call(&reg, &dry, "audio.remove_sfx", json!({"clip_id": id, "sfx_id": "sfx_1"})).await.unwrap();
    assert_eq!(r["dry_run"], true);
    assert_eq!(extras_json(&store, &id)["sfx"].as_array().unwrap().len(), 2);
    let r = call(&reg, &ctx, "audio.remove_sfx", json!({"clip_id": id, "sfx_id": "sfx_1"})).await.unwrap();
    assert_eq!(r["remaining"], json!(["sfx_2"]));
    assert_eq!(code(&call(&reg, &ctx, "audio.remove_sfx", json!({"clip_id": id, "sfx_id": "sfx_1"})).await.unwrap_err()), ErrorCode::NotFound);

    let x = extras_json(&store, &id);
    assert_eq!(x["overlays"][0]["id"], "ovl_1", "other keys survive: {x}");
    // Turning everything off removes the key.
    call(&reg, &ctx, "audio.denoise", json!({"clip_id": id, "strength": "off"})).await.unwrap();
    call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "db": 0.0})).await.unwrap();
    call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "segment_id": seg1, "db": 0.0})).await.unwrap();
    assert!(extras_json(&store, &id).get("audio_fx").is_none());
}

#[tokio::test]
async fn render_run_with_denoise_volume_and_word_sfx() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let beep = proj.path().join("beep.wav");
    make_music(&ffmpeg_in(&home), &beep, 1);
    let reg = registry();
    let clip = two_segment_clip(&reg, &ctx, &src).await;
    let id = clip["id"].as_str().unwrap().to_string();
    let seg0 = clip["segments"][0]["id"].as_str().unwrap().to_string();

    call(&reg, &ctx, "audio.denoise", json!({"clip_id": id, "strength": "light"})).await.unwrap();
    call(&reg, &ctx, "audio.volume", json!({"clip_id": id, "segment_id": seg0, "db": -3.0})).await.unwrap();
    call(&reg, &ctx, "audio.add_sfx", json!({"clip_id": id, "file": beep, "at_word": {"source_id": src.id, "word_index": 4}}))
        .await
        .unwrap();
    // Probe rejects a file without audio when tools are present.
    let silent = proj.path().join("silent.mp4");
    make_video(&ffmpeg_in(&home), &silent, 1, 320, 240, false);
    assert_eq!(code(&call(&reg, &ctx, "audio.add_sfx", json!({"clip_id": id, "file": silent, "at": 1.0})).await.unwrap_err()), ErrorCode::Validation);

    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    eprintln!("render.run: {}", r["loudness"]);
    let rep = &r["loudness"];
    assert_eq!(rep["loudness_mode"], "two-pass", "{r}");
    let floor = rep["noise_floor_db"].as_f64().expect("noise floor measured");
    assert!(rep["denoise_filter"].as_str().unwrap().starts_with("highpass=f=80,afftdn=nr=12:nf="), "{rep}");
    let rid = r["render_id"].as_str().unwrap();
    let rw = store.root.join("clips").join(&id).join("renders").join(rid);
    let g = std::fs::read_to_string(rw.join("graph.txt")).unwrap();
    // "Americans" at edit 1.85 s = 88800 samples; segment 0 is 3 dB quieter.
    assert!(g.contains("adelay=delays=88800S:all=1"), "{g}");
    assert!(g.contains("volume=-3dB,apad"), "{g}");
    let used = rep["denoise_filter"].as_str().unwrap();
    assert!(g.contains(used), "{g}");
    if (floor + 4.0 - -46.0).abs() > 0.05 {
        assert!(!g.contains("afftdn=nr=12:nf=-46"), "measured floor replaced the default: {g}");
    }
    let lg = std::fs::read_to_string(rw.join("graph_loudness.txt")).unwrap();
    assert!(lg.contains(rep["denoise_filter"].as_str().unwrap()), "loudness pass hears the same denoise");
    assert!((r["duration"].as_f64().unwrap() - 3.4).abs() < 1e-9);
}
