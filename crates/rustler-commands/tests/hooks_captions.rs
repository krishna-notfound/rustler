//! overlay.hook_styles + hook styles on overlay.add_text (with a whoosh), caption.keywords with the
//! keyword-highlight preset, transitions flash/whip, and clip.cover + its export hook.
//! Set `RUSTLER_TEST_OUT=<dir>` to keep the preview frames and the cover for a look.
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};
use std::path::Path;

fn extras_json(store: &rustler_core::ProjectStore, clip: &str) -> Value {
    let p = store.root.join("clips").join(clip).join("extras.json");
    if !p.is_file() {
        return json!({});
    }
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn keep(from: &str, name: &str) {
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(from, Path::new(&dir).join(name));
    }
}

/// Clip of source [1.0, 2.4] + [3.0, 5.0] + [5.05, 7.0]; edit duration 5.35 s.
async fn three_segments(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> Value {
    let c = call(reg, ctx, "clip.create", json!({"name": "hooks", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 5.05, "end": 7.0})).await.unwrap()
}

#[tokio::test]
async fn hook_styles_list_and_add_text_with_style_animation_and_whoosh() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let whoosh = proj.path().join("whoosh.wav");
    std::fs::write(&whoosh, b"RIFF").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();

    let l = call(&reg, &ctx, "overlay.hook_styles", json!({})).await.unwrap();
    let names: Vec<&str> = l["styles"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["hook-big", "hook-box-yellow", "hook-outline", "hook-pov", "hook-question"]);
    let anims: Vec<&str> = l["styles"].as_array().unwrap().iter().map(|s| s["anim"].as_str().unwrap()).collect();
    assert_eq!(anims, ["pop", "slide", "typewriter", "slide", "zoom-in"]);
    assert!(l["styles"][0]["font_size_factor"].as_f64().unwrap() >= 0.10 - 1e-9);

    // POV hook with a whoosh: the sound effect is saved at the hook's start.
    let a = call(
        &reg,
        &ctx,
        "overlay.add_text",
        json!({"clip_id": id, "text": "you quit your job", "style": "hook-pov", "start": 0.5, "end": 3.0, "sfx": whoosh, "sfx_volume_db": -9.0}),
    )
    .await
    .unwrap();
    assert_eq!(a["overlay"]["style"], "hook-pov");
    assert_eq!(a["overlay"]["layout"]["lines"][0], "POV: you quit your job");
    assert!(a["warnings"][0].as_str().unwrap().contains("accent"), "{a}");
    let x = extras_json(&store, &id);
    assert_eq!(x["overlays"][0]["sfx_id"], "sfx_1");
    assert_eq!(x["sfx"][0]["id"], "sfx_1");
    assert_eq!(x["sfx"][0]["at"], 0.5);
    assert_eq!(x["sfx"][0]["volume_db"], -9.0);

    // Moving the hook moves its whoosh; a new style resets the animation; --anim overrides it.
    call(&reg, &ctx, "overlay.edit", json!({"clip_id": id, "overlay_id": "ovl_1", "start": 1.0, "end": 3.5})).await.unwrap();
    assert_eq!(extras_json(&store, &id)["sfx"][0]["at"], 1.0);
    call(&reg, &ctx, "overlay.edit", json!({"clip_id": id, "overlay_id": "ovl_1", "anim": "typewriter"})).await.unwrap();
    assert_eq!(extras_json(&store, &id)["overlays"][0]["anim"], "typewriter");
    call(&reg, &ctx, "overlay.edit", json!({"clip_id": id, "overlay_id": "ovl_1", "style": "hook-big"})).await.unwrap();
    assert!(extras_json(&store, &id)["overlays"][0].get("anim").is_none(), "new style = its default animation");

    // Removing the hook removes its whoosh.
    call(&reg, &ctx, "overlay.remove", json!({"clip_id": id, "overlay_id": "ovl_1"})).await.unwrap();
    let x = extras_json(&store, &id);
    assert!(x.get("overlays").is_none() && x.get("sfx").is_none(), "{x}");

    // A whoosh that is not there: error, nothing saved. Dry run saves nothing either.
    let e = call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "x", "style": "hook-big", "sfx": proj.path().join("none.wav")}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let txt = proj.path().join("notes.txt");
    std::fs::write(&txt, b"x").unwrap();
    let e = call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "x", "sfx": txt})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let x = extras_json(&store, &id);
    assert!(x.get("overlays").is_none() && x.get("sfx").is_none(), "failed adds save nothing: {x}");
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let d = call(&reg, &dry, "overlay.add_text", json!({"clip_id": id, "text": "x", "style": "hook-big", "sfx": whoosh})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    let x = extras_json(&store, &id);
    assert!(x.get("overlays").is_none() && x.get("sfx").is_none(), "dry run saves nothing: {x}");
}

#[tokio::test]
async fn keyword_commands_mark_review_and_keep_agent_colours() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    let mut words = sample_words();
    words[9].text = "country 30".into(); // a number inside the clip
    write_transcript(&store, &src.id, words);
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();

    // Nothing to do / bad input.
    for bad in [json!({"clip_id": id}), json!({"clip_id": id, "words": ["x"], "color": "red"})] {
        assert_eq!(code(&call(&reg, &ctx, "caption.keywords", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }

    // auto + dry run: proposals only.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let d = call(&reg, &dry, "caption.keywords", json!({"clip_id": id, "auto": true})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    let marked = d["marked"].as_array().unwrap();
    let texts: Vec<&str> = marked.iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["Americans", "country 30"], "{d}");
    assert_eq!(marked[0]["reason"], "name");
    assert_eq!(marked[1]["reason"], "number");
    assert!(extras_json(&store, &id).get("caption_word_overrides").is_none(), "dry run saves nothing");
    assert!(d["next"][0].as_str().unwrap().contains("--style keyword-highlight"), "{d}");

    // The agent adds its own word and a colour of its own on another word first.
    call(&reg, &ctx, "caption.word", json!({"clip_id": id, "match": "ask", "color": "#00AAFF"})).await.unwrap();
    let k = call(&reg, &ctx, "caption.keywords", json!({"clip_id": id, "words": ["ask, not"], "auto": true, "color": "#FFE000"})).await.unwrap();
    let by_text = |t: &str| k["marked"].as_array().unwrap().iter().find(|m| m["text"] == t).cloned().unwrap();
    assert_eq!(by_text("ask")["kept_existing_color"], true, "{k}");
    assert_eq!(by_text("ask")["reason"], "agent");
    assert_eq!(by_text("Americans")["color"], "#FFE000");
    let l = call(&reg, &ctx, "caption.words_list", json!({"clip_id": id})).await.unwrap();
    let ov = l["overrides"].as_array().unwrap();
    assert_eq!(ov.iter().find(|o| o["text"] == "ask").unwrap()["color"], "#00AAFF", "the agent's colour wins");
    assert_eq!(ov.iter().find(|o| o["text"] == "Americans").unwrap()["color"], "#FFE000");
    assert_eq!(k["total_overrides"].as_u64().unwrap() as usize, ov.len());

    // A word that is not in the clip is reported, not an error.
    let u = call(&reg, &ctx, "caption.keywords", json!({"clip_id": id, "words": ["banana", "friends"]})).await.unwrap();
    assert!(u["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("'banana'")), "{u}");
    assert_eq!(u["marked"].as_array().unwrap().len(), 1);

    // The preset is available to captions.
    let s = call(&reg, &ctx, "caption.list_styles", json!({})).await.unwrap();
    let names: Vec<&str> = s["styles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"keyword-highlight") && names.contains(&"emoji-karaoke"), "{names:?}");
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "style": "keyword-highlight"})).await.unwrap();
}

#[tokio::test]
async fn keyword_caption_frame_shows_the_keyword_colour() {
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
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "style": "keyword-highlight"})).await.unwrap();
    // "Americans" is spoken 1.85-2.4 s of the edit timeline.
    let green = |p: &str| -> usize {
        let im = image::open(p).unwrap().to_rgb8();
        let (w, h) = im.dimensions();
        let mut n = 0;
        for y in (h as f64 * 0.55) as u32..(h as f64 * 0.85) as u32 {
            for x in 0..w {
                let p = im.get_pixel(x, y).0;
                if p[1] > 200 && p[0] < 90 && p[2] < 90 {
                    n += 1;
                }
            }
        }
        n
    };
    let plain = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 2.0})).await.unwrap();
    let plain_n = green(plain["image"].as_str().unwrap());
    keep(plain["image"].as_str().unwrap(), "keyword_off.jpg");
    call(&reg, &ctx, "caption.keywords", json!({"clip_id": id, "words": ["americans"], "color": "#00FF00"})).await.unwrap();
    let marked = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 2.0})).await.unwrap();
    let marked_n = green(marked["image"].as_str().unwrap());
    keep(marked["image"].as_str().unwrap(), "keyword_on.jpg");
    eprintln!("green pixels in the caption band: plain {plain_n}, with keyword {marked_n}");
    assert!(marked_n > plain_n + 300, "the keyword is drawn green: {plain_n} -> {marked_n}");
}

#[tokio::test]
async fn transition_kinds_flash_whip_have_short_defaults() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();
    let seg = |k: usize| c["segments"][k]["id"].as_str().unwrap().to_string();
    let f = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": seg(0), "kind": "flash"})).await.unwrap();
    assert_eq!(f["transitions"][0]["kind"], "flash");
    assert_eq!(f["transitions"][0]["duration"], 0.1);
    let w = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": seg(1), "kind": "whip"})).await.unwrap();
    assert_eq!(w["transitions"][1]["duration"], 0.2);
    let b = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": seg(0), "kind": "flashblack", "duration": 0.15})).await.unwrap();
    assert_eq!((b["transitions"][0]["kind"].as_str(), b["transitions"][0]["duration"].as_f64()), (Some("flashblack"), Some(0.15)));
    let d = call(&reg, &ctx, "transition.set", json!({"clip_id": id, "after_segment_id": seg(0), "kind": "dissolve"})).await.unwrap();
    assert_eq!(d["transitions"][0]["duration"], 0.3, "other kinds keep 0.3 s");
}

#[tokio::test]
async fn cover_needs_a_render_picks_a_frame_and_export_copies_it() {
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
    let c = call(&reg, &ctx, "clip.create", json!({"name": "cover test", "source_id": src.id, "start": 1.0, "end": 7.0})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();

    // No render yet.
    let e = call(&reg, &ctx, "clip.cover", json!({"clip_id": id})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();

    // Bad input.
    let e = call(&reg, &ctx, "clip.cover", json!({"clip_id": id, "out_t": 99.0})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let e = call(&reg, &ctx, "clip.cover", json!({"clip_id": id, "out_t": 2.0, "auto": true})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);

    // Auto: no engine in the throwaway home, so sharpness decides and a warning says so.
    let a = call(&reg, &ctx, "clip.cover", json!({"clip_id": id, "auto": true})).await.unwrap();
    assert_eq!(a["method"], "auto");
    let img = a["image"].as_str().unwrap();
    assert!(Path::new(img).is_file() && img.ends_with("cover.jpg"), "{a}");
    assert!(!a["candidates"].as_array().unwrap().is_empty());
    let scores: Vec<f64> = a["candidates"].as_array().unwrap().iter().map(|c| c["score"].as_f64().unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "best first: {scores:?}");
    let (w, h) = image::image_dimensions(img).unwrap();
    assert_eq!((w, h), (1080, 1920));
    keep(img, "cover_auto.jpg");
    let rec = extras_json(&store, &id)["cover"].clone();
    assert_eq!(rec["method"], "auto");
    assert_eq!(rec["file"], format!("clips/{id}/cover.jpg"));

    // Manual frame replaces it.
    let m = call(&reg, &ctx, "clip.cover", json!({"clip_id": id, "out_t": 3.0})).await.unwrap();
    assert_eq!((m["method"].as_str(), m["out_t"].as_f64()), (Some("manual"), Some(3.0)));
    assert_eq!(extras_json(&store, &id)["cover"]["method"], "manual");

    // Dry run leaves the project alone.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let before = std::fs::read(store.root.join("clips").join(&id).join("cover.jpg")).unwrap();
    let d = call(&reg, &dry, "clip.cover", json!({"clip_id": id, "out_t": 5.0})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    assert_eq!(std::fs::read(store.root.join("clips").join(&id).join("cover.jpg")).unwrap(), before);
    assert_eq!(extras_json(&store, &id)["cover"]["out_t"], 3.0);

    // Export copies the cover next to the video and the post JSON points at the copy.
    let ex = call(&reg, &ctx, "export.clip", json!({"clip_id": id, "name": "cover_test", "allow_fail": true})).await.unwrap();
    let dir = Path::new(ex["video"].as_str().unwrap()).parent().unwrap().to_path_buf();
    assert!(dir.join("cover_test.cover.jpg").is_file());
    let post: Value = serde_json::from_slice(&std::fs::read(ex["post"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(post["cover"]["file"], "cover_test.cover.jpg", "{post}");
    assert_eq!(post["cover"]["method"], "manual");
}

#[tokio::test]
async fn export_without_a_cover_has_a_null_cover() {
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
    let c = call(&reg, &ctx, "clip.create", json!({"name": "no cover", "source_id": src.id, "start": 1.0, "end": 7.0})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    let ex = call(&reg, &ctx, "export.clip", json!({"clip_id": id, "allow_fail": true})).await.unwrap();
    let post: Value = serde_json::from_slice(&std::fs::read(ex["post"].as_str().unwrap()).unwrap()).unwrap();
    assert!(post["cover"].is_null(), "{post}");
}
