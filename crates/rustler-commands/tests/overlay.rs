//! overlay.* command tests (+ render.run with a hook title and two-pass loudness).
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};

async fn clip_on(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> (String, f64) {
    let clip = call(reg, ctx, "clip.create", json!({"name": "hook", "source_id": src.id, "start_unit": 0, "end_unit": 2}))
        .await
        .unwrap();
    (clip["id"].as_str().unwrap().to_string(), clip["duration"].as_f64().unwrap())
}

fn extras_json(store: &rustler_core::ProjectStore, clip: &str) -> Value {
    serde_json::from_slice(&std::fs::read(store.root.join("clips").join(clip).join("extras.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn overlay_crud_keeps_other_extras_keys() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let (id, duration) = clip_on(&reg, &ctx, &src).await;
    assert!(duration > 3.0, "{duration}");

    // Other modules' keys must survive overlay writes.
    let dir = store.root.join("clips").join(&id);
    std::fs::write(
        dir.join("extras.json"),
        r#"{"music":{"path":"C:/m.wav","volume_db":-18.0,"duck":true,"fade_in":0.5,"fade_out":1.0},"crop_paths":{"seg_x":[{"t":0.0,"x":0.3}]},"sfx":[{"id":"s1"}]}"#,
    )
    .unwrap();

    let a = call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "  He quit his job for this  "})).await.unwrap();
    assert_eq!(a["overlay_id"], "ovl_1");
    let o = &a["overlay"];
    assert_eq!(o["text"], "He quit his job for this");
    assert_eq!((o["start"].as_f64(), o["end"].as_f64()), (Some(0.0), Some(3.0)));
    assert_eq!(o["style"], "hook-top");
    assert_eq!(o["position_y"], 0.12);
    assert_eq!(o["box"], true);
    assert_eq!(o["layout"]["lines"][0], "He quit his job for this");
    assert!(a["warnings"].is_null(), "{a}");
    assert!(a["next"][0].as_str().unwrap().contains("render preview-frame"));

    let x = extras_json(&store, &id);
    assert_eq!(x["music"]["path"], "C:/m.wav");
    assert_eq!(x["crop_paths"]["seg_x"][0]["x"], 0.3);
    assert_eq!(x["sfx"][0]["id"], "s1");
    assert_eq!(x["overlays"][0]["id"], "ovl_1");

    // Second overlay: long hook -> warning; lower-third default position.
    let b = call(
        &reg,
        &ctx,
        "overlay.add_text",
        json!({"clip_id": id, "text": "This one simple trick changed how I edit every video", "style": "lower-third", "start": 1.0, "end": 2.5}),
    )
    .await
    .unwrap();
    assert_eq!(b["overlay_id"], "ovl_2");
    assert_eq!(b["overlay"]["position_y"], 0.8);
    let h = call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "one two three four five six seven eight"}))
        .await
        .unwrap();
    assert!(h["warnings"][0].as_str().unwrap().contains("<= 7 words"), "{h}");
    let hid = h["overlay_id"].as_str().unwrap().to_string();
    assert_eq!(hid, "ovl_3");
    call(&reg, &ctx, "overlay.remove", json!({"clip_id": id, "overlay_id": hid})).await.unwrap();

    // Validation.
    for bad in [
        json!({"clip_id": id, "text": "   "}),
        json!({"clip_id": id, "text": "x".repeat(121)}),
        json!({"clip_id": id, "text": "late", "start": duration + 1.0}),
        json!({"clip_id": id, "text": "long", "start": 0.0, "end": duration + 1.0}),
        json!({"clip_id": id, "text": "backwards", "start": 2.0, "end": 1.0}),
        json!({"clip_id": id, "text": "off canvas", "position_y": 1.4}),
    ] {
        let e = call(&reg, &ctx, "overlay.add_text", bad.clone()).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "{bad}");
    }

    // Edit: text + end; style change resets style defaults.
    let e = call(&reg, &ctx, "overlay.edit", json!({"clip_id": id, "overlay_id": "ovl_1", "text": "Nobody saw this coming", "end": 2.0}))
        .await
        .unwrap();
    assert_eq!(e["overlay"]["text"], "Nobody saw this coming");
    assert_eq!(e["overlay"]["end"], 2.0);
    let e = call(&reg, &ctx, "overlay.edit", json!({"clip_id": id, "overlay_id": "ovl_2", "style": "plain"})).await.unwrap();
    assert_eq!((e["overlay"]["position_y"].as_f64(), e["overlay"]["box"].as_bool()), (Some(0.5), Some(false)));
    let nf = call(&reg, &ctx, "overlay.edit", json!({"clip_id": id, "overlay_id": "ovl_9", "text": "x"})).await.unwrap_err();
    assert_eq!(nf.code, ErrorCode::NotFound);
    assert!(nf.hint.unwrap().contains("ovl_1"));

    // list
    let l = call(&reg, &ctx, "overlay.list", json!({"clip_id": id})).await.unwrap();
    let ids: Vec<&str> = l["overlays"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ovl_1", "ovl_2"]);
    assert!(l["overlays"][0]["layout"]["font_size"].as_u64().unwrap() > 0);

    // dry-run remove writes nothing; real remove does; removing the last drops the key.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let r = call(&reg, &dry, "overlay.remove", json!({"clip_id": id, "overlay_id": "ovl_1"})).await.unwrap();
    assert_eq!(r["dry_run"], true);
    assert_eq!(extras_json(&store, &id)["overlays"].as_array().unwrap().len(), 2);
    let r = call(&reg, &ctx, "overlay.remove", json!({"clip_id": id, "overlay_id": "ovl_1"})).await.unwrap();
    assert_eq!(r["remaining"], json!(["ovl_2"]));
    call(&reg, &ctx, "overlay.remove", json!({"clip_id": id, "overlay_id": "ovl_2"})).await.unwrap();
    let x = extras_json(&store, &id);
    assert!(x.get("overlays").is_none(), "{x}");
    assert_eq!(x["sfx"][0]["id"], "s1");
    assert_eq!(x["music"]["path"], "C:/m.wav");

    // Next id after removals continues from the remaining max (none -> ovl_1).
    let a = call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "again"})).await.unwrap();
    assert_eq!(a["overlay_id"], "ovl_1");
}

#[tokio::test]
async fn render_run_with_hook_title_and_two_pass_loudness() {
    let Some((_tmp, home)) = tools_home() else { return };
    let poppins = real_home().join("fonts").join("Poppins").join("Poppins-ExtraBold.ttf");
    if !poppins.is_file() {
        eprintln!("SKIP: {} missing (run `rustler setup`)", poppins.display());
        return;
    }
    let pdir = home.join("fonts").join("Poppins");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::copy(&poppins, pdir.join("Poppins-ExtraBold.ttf")).unwrap();

    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let (id, _) = clip_on(&reg, &ctx, &src).await;
    call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "Wait for the ending"})).await.unwrap();

    let pf = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 1.0})).await.unwrap();
    let img = pf["image"].as_str().unwrap();
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(img, std::path::Path::new(&dir).join("cmd_hook_preview.jpg"));
    }
    let work = store.root.join("clips").join(&id).join("preview").join("work");
    assert!(std::fs::read_to_string(work.join("overlays.ass")).unwrap().contains("Wait for the ending"));
    assert!(work.join("fonts").join("Poppins-ExtraBold.ttf").is_file());

    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    eprintln!("render.run loudness: {}", r["loudness"]);
    assert_eq!(r["loudness"]["loudness_mode"], "two-pass", "{r}");
    assert!(r["loudness"]["measured"]["input_i"].as_f64().is_some());
    let rid = r["render_id"].as_str().unwrap();
    let rw = store.root.join("clips").join(&id).join("renders").join(rid);
    let g = std::fs::read_to_string(rw.join("graph.txt")).unwrap();
    assert!(g.contains("ass=overlays.ass:fontsdir=fonts") && g.contains("measured_I="), "{g}");

    // Opt out: single pass.
    let r = call(
        &reg,
        &ctx,
        "render.run",
        json!({"clip_id": id, "quality": "draft", "prefer_gpu": false, "two_pass_loudness": false}),
    )
    .await
    .unwrap();
    assert_eq!(r["loudness"]["loudness_mode"], "single-pass", "{r}");
}
