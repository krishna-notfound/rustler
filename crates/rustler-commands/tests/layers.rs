//! image.* / mask.* / cutaway.* / caption.word* commands, anchors across re-cuts, and a real
//! render with every layer (frames kept under RUSTLER_TEST_OUT for a look).
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

fn extras_json(store: &rustler_core::ProjectStore, clip: &str) -> Value {
    let p = store.root.join("clips").join(clip).join("extras.json");
    if !p.is_file() {
        return json!({});
    }
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

/// Clip of source [1.0, 2.4] ("Hello there friends.") + [3.0, 5.0] ("Fellow Americans ask not.")
/// + [5.05, 7.0] ("What your country can do."); edit duration 5.35 s.
async fn three_segments(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> Value {
    let c = call(reg, ctx, "clip.create", json!({"name": "layers", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 5.05, "end": 7.0})).await.unwrap()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({"x": x, "y": y, "w": w, "h": h})
}

#[tokio::test]
async fn image_crud_validation_dry_run_and_revision() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();
    let rev = c["revision"].as_u64().unwrap();
    let logo = proj.path().join("logo.png");
    std::fs::write(&logo, b"png").unwrap();
    let f = logo.to_string_lossy().to_string();
    std::fs::write(store.root.join("clips").join(&id).join("extras.json"), r#"{"future":[1]}"#).unwrap();

    let a = call(&reg, &ctx, "image.add", json!({"clip_id": id, "file": f, "rect": rect(0.74, 0.03, 0.22, 0.08), "anim_in": "fade", "opacity": 0.9}))
        .await
        .unwrap();
    assert_eq!(a["id"], "img_1");
    assert_eq!(a["item"]["anim"]["in"], "fade");
    assert_eq!(a["item"]["fit"], "contain");
    assert_eq!(a["item"]["when"]["start"], 0.0);
    assert!((a["item"]["when"]["end"].as_f64().unwrap() - 5.35).abs() < 1e-9, "default: whole clip {a}");
    assert!(Path::new(a["item"]["file"].as_str().unwrap()).is_absolute());
    let x = extras_json(&store, &id);
    assert_eq!(x["images"][0]["id"], "img_1");
    assert_eq!(x["future"], json!([1]), "other keys survive");

    // Validation: rect outside 0..1, too small, end past the clip, two media, bad type, bad opacity.
    for bad in [
        json!({"clip_id": id, "file": f, "rect": rect(0.9, 0.0, 0.2, 0.1)}),
        json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.1, 0.02, 0.1)}),
        json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.1, 0.2, 0.1), "start": 1.0, "end": 9.0}),
        json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.1, 0.2, 0.1), "start": 3.0, "end": 2.0}),
        json!({"clip_id": id, "file": f, "asset": "ast_x", "rect": rect(0.1, 0.1, 0.2, 0.1)}),
        json!({"clip_id": id, "file": media.to_string_lossy(), "rect": rect(0.1, 0.1, 0.2, 0.1)}),
        json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.1, 0.2, 0.1), "opacity": 2.0}),
    ] {
        let e = call(&reg, &ctx, "image.add", bad.clone()).await.unwrap_err();
        assert_eq!(code(&e), ErrorCode::Validation, "{bad} -> {e:?}");
        assert_eq!(e.stage, "image.add");
    }
    let e = call(&reg, &ctx, "image.add", json!({"clip_id": id, "file": proj.path().join("nope.png"), "rect": rect(0.1, 0.1, 0.2, 0.1)}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);

    // Dry run validates and returns the item, saves nothing.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let d = call(&reg, &dry, "image.add", json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.5, 0.3, 0.2), "start": 1.0, "end": 2.0}))
        .await
        .unwrap();
    assert_eq!((d["dry_run"].as_bool(), d["id"].as_str()), (Some(true), Some("img_2")));
    assert_eq!(extras_json(&store, &id)["images"].as_array().unwrap().len(), 1);

    // Revision-checked: a stale base_revision is a conflict and writes nothing.
    let e = call(&reg, &ctx, "image.add", json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.5, 0.3, 0.2), "base_revision": rev + 7}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::RevisionConflict);
    let b = call(
        &reg,
        &ctx,
        "image.add",
        json!({"clip_id": id, "file": f, "rect": rect(0.1, 0.5, 0.3, 0.2), "start": 1.0, "end": 2.0, "z": 3, "base_revision": rev}),
    )
    .await
    .unwrap();
    assert_eq!(b["id"], "img_2");

    // Edit, list, remove.
    let e = call(&reg, &ctx, "image.edit", json!({"clip_id": id, "image_id": "img_2", "opacity": 0.5, "anim_out": "slide_up", "above_captions": true}))
        .await
        .unwrap();
    assert_eq!(e["item"]["opacity"], 0.5);
    assert_eq!(e["item"]["anim"]["out"], "slide_up");
    assert_eq!(e["item"]["above_captions"], true);
    let bad = call(&reg, &ctx, "image.edit", json!({"clip_id": id, "image_id": "img_2", "rect": rect(0.5, 0.5, 0.6, 0.2)})).await.unwrap_err();
    assert_eq!(code(&bad), ErrorCode::Validation);
    assert_eq!(extras_json(&store, &id)["images"][1]["rect"]["x"], 0.1, "failed edit wrote nothing");
    let e = call(&reg, &ctx, "image.edit", json!({"clip_id": id, "image_id": "img_9", "opacity": 0.5})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let l = call(&reg, &ctx, "image.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["items"].as_array().unwrap().len(), 2);
    assert_eq!(l["items"][1]["z"], 3);
    let r = call(&reg, &ctx, "image.remove", json!({"clip_id": id, "image_id": "img_1"})).await.unwrap();
    assert_eq!(r["remaining"], json!(["img_2"]));
    call(&reg, &ctx, "image.remove", json!({"clip_id": id, "image_id": "img_2"})).await.unwrap();
    let x = extras_json(&store, &id);
    assert!(x.get("images").is_none(), "empty list removes the key: {x}");
    assert_eq!(x["future"], json!([1]));
}

#[tokio::test]
async fn mask_and_cutaway_crud() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    let broll = add_source(&store, &media, probe_1080p(30.0, true, true));
    let audio_only = add_source(&store, &media, probe_1080p(30.0, false, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();

    // Masks.
    let m = call(&reg, &ctx, "mask.add", json!({"clip_id": id, "kind": "blur", "rect": rect(0.75, 0.02, 0.23, 0.07)})).await.unwrap();
    assert_eq!(m["id"], "msk_1");
    let m = call(&reg, &ctx, "mask.add", json!({"clip_id": id, "kind": "box", "rect": rect(0.05, 0.78, 0.9, 0.08), "start": 2.0, "end": 4.0, "color": "#101010", "strength": 5}))
        .await
        .unwrap();
    assert_eq!(m["warnings"].as_array().unwrap().len(), 1, "strength unused by box: {m}");
    for bad in [
        json!({"clip_id": id, "kind": "blur", "rect": rect(0.0, 0.0, 1.2, 0.1)}),
        json!({"clip_id": id, "kind": "box", "rect": rect(0.0, 0.0, 0.5, 0.1), "color": "black"}),
        json!({"clip_id": id, "kind": "pixelate", "rect": rect(0.0, 0.0, 0.5, 0.1), "strength": 500}),
        json!({"clip_id": id, "kind": "smudge", "rect": rect(0.0, 0.0, 0.5, 0.1)}),
    ] {
        assert_eq!(code(&call(&reg, &ctx, "mask.add", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }
    let m = call(&reg, &ctx, "mask.edit", json!({"clip_id": id, "mask_id": "msk_1", "kind": "pixelate", "strength": 40})).await.unwrap();
    assert_eq!((m["item"]["kind"].as_str(), m["item"]["strength"].as_f64()), (Some("pixelate"), Some(40.0)));
    let l = call(&reg, &ctx, "mask.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["items"].as_array().unwrap().len(), 2);
    assert_eq!(l["items"][1]["when"]["start"], 2.0);
    call(&reg, &ctx, "mask.remove", json!({"clip_id": id, "mask_id": "msk_2"})).await.unwrap();

    // Cutaways.
    let k = call(&reg, &ctx, "cutaway.add", json!({"clip_id": id, "source_id": broll.id, "src_start": 12.0, "start": 1.0, "end": 2.0, "mode": "full"}))
        .await
        .unwrap();
    assert_eq!(k["id"], "cut_1");
    assert_eq!(k["item"]["media"]["source_id"], broll.id.as_str());
    assert_eq!(k["item"]["media"]["in"], 12.0);
    assert_eq!(k["item"]["mute"], true);
    let still = proj.path().join("chart.png");
    std::fs::write(&still, b"png").unwrap();
    let p = call(
        &reg,
        &ctx,
        "cutaway.add",
        json!({"clip_id": id, "file": still, "mode": "pip", "rect": rect(0.52, 0.58, 0.44, 0.25), "start": 3.0, "end": 4.0, "mute": false}),
    )
    .await
    .unwrap();
    assert!(p["warnings"][0].as_str().unwrap().contains("no audio"), "{p}");
    assert_eq!(p["item"]["mute"], true);
    for bad in [
        json!({"clip_id": id, "source_id": broll.id, "start": 1.0, "end": 2.0, "mode": "pip"}),
        json!({"clip_id": id, "source_id": broll.id, "start": 1.0, "mode": "full"}),
        json!({"clip_id": id, "source_id": broll.id, "src_start": 40.0, "start": 1.0, "end": 2.0, "mode": "full"}),
        json!({"clip_id": id, "source_id": audio_only.id, "start": 1.0, "end": 2.0, "mode": "full"}),
        json!({"clip_id": id, "source_id": broll.id, "file": still, "start": 1.0, "end": 2.0, "mode": "full"}),
    ] {
        assert_eq!(code(&call(&reg, &ctx, "cutaway.add", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }
    let e = call(&reg, &ctx, "cutaway.edit", json!({"clip_id": id, "cutaway_id": "cut_1", "mode": "pip", "rect": rect(0.5, 0.6, 0.45, 0.25)}))
        .await
        .unwrap();
    assert_eq!(e["item"]["mode"], "pip");
    let e = call(&reg, &ctx, "cutaway.edit", json!({"clip_id": id, "cutaway_id": "cut_2", "mute": false})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    let e = call(&reg, &ctx, "cutaway.edit", json!({"clip_id": id, "cutaway_id": "cut_1", "mode": "full"})).await.unwrap();
    assert!(e["item"].get("rect").is_none(), "full drops the rect: {e}");
    let l = call(&reg, &ctx, "cutaway.list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["items"].as_array().unwrap().len(), 2);
    let r = call(&reg, &ctx, "cutaway.remove", json!({"clip_id": id, "cutaway_id": "cut_1"})).await.unwrap();
    assert_eq!(r["remaining"], json!(["cut_2"]));
    assert_eq!(code(&call(&reg, &ctx, "cutaway.remove", json!({"clip_id": id, "cutaway_id": "cut_1"})).await.unwrap_err()), ErrorCode::NotFound);
}

#[tokio::test]
async fn caption_word_overrides_crud() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    let mut words = sample_words();
    // A second "Americans" later in the source (outside the clip) must not match.
    words.push(word(12, "americans", 8.0, 8.5));
    write_transcript(&store, &src.id, words);
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();

    let w = call(&reg, &ctx, "caption.word", json!({"clip_id": id, "match": "americans!", "color": "#FF2020", "scale": 1.3})).await.unwrap();
    assert_eq!(w["changed"].as_array().unwrap().len(), 1, "{w}");
    assert_eq!(w["changed"][0]["word_anchor"]["word_i"], 4);
    assert_eq!(w["changed"][0]["text"], "Americans");
    assert!((w["changed"][0]["at"][0].as_f64().unwrap() - 1.85).abs() < 1e-9, "{w}");
    // Same word again merges (emoji added, colour kept).
    let w = call(&reg, &ctx, "caption.word", json!({"clip_id": id, "source_id": src.id, "word_i": 4, "emoji": "🔥"})).await.unwrap();
    assert_eq!((w["total"].as_u64(), w["changed"][0]["color"].as_str(), w["changed"][0]["emoji"].as_str()), (Some(1), Some("#FF2020"), Some("🔥")));
    call(&reg, &ctx, "caption.word", json!({"clip_id": id, "source_id": src.id, "word_i": 0, "hide": true})).await.unwrap();
    for bad in [
        json!({"clip_id": id, "source_id": src.id, "word_i": 4}),
        json!({"clip_id": id, "source_id": src.id, "word_i": 4, "color": "red"}),
        json!({"clip_id": id, "source_id": src.id, "word_i": 4, "scale": 9.0}),
        json!({"clip_id": id, "word_i": 4, "color": "#FFFFFF"}),
        json!({"clip_id": id, "source_id": src.id, "word_i": 12, "color": "#FFFFFF"}),
        json!({"clip_id": id, "source_id": src.id, "word_i": 4, "match": "x", "color": "#FFFFFF"}),
    ] {
        assert_eq!(code(&call(&reg, &ctx, "caption.word", bad.clone()).await.unwrap_err()), ErrorCode::Validation, "{bad}");
    }
    let e = call(&reg, &ctx, "caption.word", json!({"clip_id": id, "match": "banana", "color": "#FFFFFF"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let l = call(&reg, &ctx, "caption.words_list", json!({"clip_id": id})).await.unwrap();
    assert_eq!(l["overrides"].as_array().unwrap().len(), 2);
    assert_eq!(l["overrides"][1]["hide"], true);
    // hide:false on a word with nothing else removes its override.
    let w = call(&reg, &ctx, "caption.word", json!({"clip_id": id, "source_id": src.id, "word_i": 0, "hide": false})).await.unwrap();
    assert_eq!((w["total"].as_u64(), w["changed"].as_array().unwrap().len()), (Some(1), 0));
    let e = call(&reg, &ctx, "caption.words_clear", json!({"clip_id": id, "source_id": src.id, "word_i": 7})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    let r = call(&reg, &ctx, "caption.words_clear", json!({"clip_id": id})).await.unwrap();
    assert_eq!((r["removed"].as_u64(), r["remaining"].as_u64()), (Some(1), Some(0)));
    assert!(extras_json(&store, &id).get("caption_word_overrides").is_none());
}

/// Review focus 4: word-anchored items whose word a re-cut removes are dropped with a warning
/// (list commands and the render), never shown at a wrong time.
#[tokio::test]
async fn anchor_removed_by_recut_is_dropped_with_warning() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let c = three_segments(&reg, &ctx, &src).await;
    let id = c["id"].as_str().unwrap().to_string();
    let segs: Vec<String> = c["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    let logo = proj.path().join("logo.png");
    std::fs::write(&logo, b"png").unwrap();
    // "ask" (word 5, source 4.05) .. "not." (word 6): edit 2.45 .. 3.4.
    let a = call(
        &reg,
        &ctx,
        "image.add",
        json!({"clip_id": id, "file": logo, "rect": rect(0.3, 0.3, 0.3, 0.1),
               "start_word": {"source_id": src.id, "word_i": 5}, "end_word": {"source_id": src.id, "word_i": 6}}),
    )
    .await
    .unwrap();
    assert!((a["item"]["when"]["start"].as_f64().unwrap() - 2.45).abs() < 1e-9, "{a}");
    assert!((a["item"]["when"]["end"].as_f64().unwrap() - 3.4).abs() < 1e-9, "{a}");
    // A mask on "What" (word 7, third segment) survives the re-cut and moves earlier.
    call(&reg, &ctx, "mask.add", json!({"clip_id": id, "kind": "box", "rect": rect(0.0, 0.0, 0.5, 0.1), "start_word": {"source_id": src.id, "word_i": 7}}))
        .await
        .unwrap();
    call(&reg, &ctx, "caption.word", json!({"clip_id": id, "source_id": src.id, "word_i": 5, "color": "#FF0000"})).await.unwrap();
    // A word anchor outside the clip is rejected up front.
    let e = call(&reg, &ctx, "mask.add", json!({"clip_id": id, "kind": "box", "rect": rect(0.0, 0.0, 0.5, 0.1), "start_word": {"source_id": src.id, "word_i": 99}}))
        .await
        .unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);

    // Re-cut: delete the middle segment ("Fellow Americans ask not.").
    call(&reg, &ctx, "edit.delete_segment", json!({"clip_id": id, "segment_id": segs[1]})).await.unwrap();
    let l = call(&reg, &ctx, "image.list", json!({"clip_id": id})).await.unwrap();
    assert!(l["items"][0].get("when").is_none(), "{l}");
    let p = l["items"][0]["problem"].as_str().unwrap();
    assert!(p.contains("'ask'") && p.contains("not in the clip"), "{p}");
    assert!(l["warnings"][0].as_str().unwrap().contains("img_1 will not render"), "{l}");
    let m = call(&reg, &ctx, "mask.list", json!({"clip_id": id})).await.unwrap();
    // "What" was at edit 3.4; now 1.4 (the 2 s segment is gone).
    assert!((m["items"][0]["when"]["start"].as_f64().unwrap() - 1.4).abs() < 1e-9, "{m}");
    let w = call(&reg, &ctx, "caption.words_list", json!({"clip_id": id})).await.unwrap();
    assert!(w["overrides"][0]["problem"].as_str().unwrap().contains("not in the clip"), "{w}");
    assert_eq!(w["overrides"][0]["at"], json!([]));
    // Editing an image to a time window brings it back.
    let e = call(&reg, &ctx, "image.edit", json!({"clip_id": id, "image_id": "img_1", "start": 0.5, "end": 1.0})).await.unwrap();
    assert!(e["item"]["when"]["start"].as_f64().is_some(), "{e}");
    assert!(e["item"].get("start_word").is_none() && e["item"].get("end_word").is_none());
}

// ---------------------------------------------------------------------------------------------
// Real render
// ---------------------------------------------------------------------------------------------

fn ff(ffmpeg: &Path, args: &[&str]) {
    let o = Command::new(ffmpeg).args(["-hide_banner", "-v", "error", "-y"]).args(args).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// Raw RGB of output frame `n` (+ a jpg under RUSTLER_TEST_OUT).
fn frame_rgb(ffmpeg: &Path, video: &Path, n: u64, name: &str) -> Vec<u8> {
    let o = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-i"])
        .arg(video)
        .args(["-vf", &format!("select=eq(n\\,{n})"), "-fps_mode", "passthrough", "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(o.status.success() && o.stdout.len() == 1080 * 1920 * 3, "frame {n}");
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let out = Path::new(&dir).join(format!("{name}.jpg"));
        ff(ffmpeg, &["-i", &video.to_string_lossy(), "-vf", &format!("select=eq(n\\,{n}),scale=360:640"), "-fps_mode", "passthrough", "-frames:v", "1", "-update", "1", &out.to_string_lossy()]);
    }
    o.stdout
}

fn count(rgb: &[u8], x0: usize, y0: usize, x1: usize, y1: usize, f: impl Fn(u8, u8, u8) -> bool) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let k = (y * 1080 + x) * 3;
            if f(rgb[k], rgb[k + 1], rgb[k + 2]) {
                n += 1;
            }
        }
    }
    n
}

#[tokio::test]
async fn render_with_logo_mask_cutaways_and_keyword_colour() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let ffmpeg = ffmpeg_in(&home);
    // Dark grey speaker video with a white "channel logo" in the top-right corner.
    let video = proj.path().join("speaker.mp4");
    ff(
        &ffmpeg,
        &[
            "-f", "lavfi", "-i", "color=c=0x303030:s=1920x1080:r=30:d=10,drawbox=x=1500:y=40:w=300:h=120:color=white:t=fill",
            "-f", "lavfi", "-i", "sine=f=440:r=48000:d=10",
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", &video.to_string_lossy(),
        ],
    );
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let green = proj.path().join("green.mp4");
    ff(&ffmpeg, &["-f", "lavfi", "-i", "color=c=green:s=1280x720:r=30:d=6", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", &green.to_string_lossy()]);
    let blue = proj.path().join("blue.mp4");
    ff(&ffmpeg, &["-f", "lavfi", "-i", "color=c=blue:s=1280x720:r=30:d=6", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", &blue.to_string_lossy()]);
    let logo = proj.path().join("logo.png");
    ff(&ffmpeg, &["-f", "lavfi", "-i", "color=c=0xE01010:s=300x150", "-frames:v", "1", &logo.to_string_lossy()]);

    let reg = registry();
    // Clip [3.0, 5.0] ("Fellow Americans ask not.") + [5.05, 7.0]; edit duration 3.95 s.
    let c = call(&reg, &ctx, "clip.create", json!({"name": "layers", "source_id": src.id, "start": 3.0, "end": 5.0, "layout": "center-crop"}))
        .await
        .unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(&reg, &ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 5.05, "end": 7.0, "layout": "center-crop"})).await.unwrap();
    // A magenta box mask over the canvas top-right corner for the first 2 s (the logo-hiding
    // render with pixel checks is in rustler-media's tests/layers.rs).
    call(&reg, &ctx, "mask.add", json!({"clip_id": id, "kind": "box", "color": "#FF00FF", "rect": rect(0.7, 0.0, 0.3, 0.06), "start": 0.0, "end": 2.0}))
        .await
        .unwrap();
    call(
        &reg,
        &ctx,
        "image.add",
        json!({"clip_id": id, "file": logo, "rect": rect(0.04, 0.03, 0.3, 0.08), "anim_in": "fade", "anim_out": "fade", "start": 0.0, "end": 3.9}),
    )
    .await
    .unwrap();
    call(&reg, &ctx, "cutaway.add", json!({"clip_id": id, "file": green, "mode": "pip", "rect": rect(0.5, 0.35, 0.45, 0.2), "start": 0.5, "end": 1.5}))
        .await
        .unwrap();
    call(&reg, &ctx, "cutaway.add", json!({"clip_id": id, "file": blue, "mode": "full", "start": 2.5, "end": 3.0, "src_start": 1.0}))
        .await
        .unwrap();
    // Keyword: "Americans" in red, bigger, with an emoji; "ask" hidden.
    call(&reg, &ctx, "caption.word", json!({"clip_id": id, "match": "americans", "color": "#FF0000", "scale": 1.25, "emoji": "!"})).await.unwrap();
    call(&reg, &ctx, "caption.word", json!({"clip_id": id, "source_id": src.id, "word_i": 5, "hide": true})).await.unwrap();

    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    assert!(r.get("warnings").is_none(), "{r}");
    let out = Path::new(r["path"].as_str().unwrap()).to_path_buf();
    let rid = r["render_id"].as_str().unwrap();
    let rw = store.root.join("clips").join(&id).join("renders").join(rid);
    let ass = std::fs::read_to_string(rw.join("captions.ass")).unwrap();
    assert!(ass.contains(r"\c&H000000FF&\fscx125\fscy125}AMERICANS !"), "keyword painted: {ass}");
    assert!(!ass.contains("ASK"), "hidden word: {ass}");
    let graph = std::fs::read_to_string(rw.join("graph.txt")).unwrap();
    assert!(graph.contains("drawbox=") && graph.contains("[img0]overlay") && graph.contains("[cut0]overlay") && graph.contains("[cut1]overlay"), "{graph}");

    // 0.2 s (frame 6): "FELLOW" active, "AMERICANS !" red on the caption line; mask + logo shown.
    let f = frame_rgb(&ffmpeg, &out, 6, "cmd_layers_keyword");
    let red_caption = count(&f, 0, 1000, 1080, 1500, |r, g, b| r > 200 && g < 60 && b < 60);
    eprintln!("red caption pixels at 0.2 s: {red_caption}");
    assert!(red_caption > 800, "keyword drawn in red: {red_caption}");
    let magenta = count(&f, 760, 0, 1080, 110, |r, g, b| r > 200 && g < 60 && b > 200);
    assert!(magenta > 20000, "mask box over the top-right corner: {magenta}");
    let logo_px = count(&f, 60, 70, 300, 130, |r, g, b| r > 120 && g < 70 && b < 70); // mid fade-in (0.2 of 0.3 s)
    assert!(logo_px > 8000, "logo top-left: {logo_px}");
    // 1.0 s (frame 30): green PiP.
    let f = frame_rgb(&ffmpeg, &out, 30, "cmd_layers_pip");
    let g = count(&f, 560, 700, 1000, 1050, |r, g, b| g > 100 && r < 60 && b < 60);
    assert!(g > 100_000, "pip: {g}");
    // 2.7 s (frame 81): full blue cutaway, mask gone (ended at 2 s), logo still on top of it.
    let f = frame_rgb(&ffmpeg, &out, 81, "cmd_layers_full");
    let b = count(&f, 300, 400, 800, 900, |r, g, b| b > 200 && r < 60 && g < 60);
    assert!(b > 200_000, "full cutaway: {b}");
    let magenta = count(&f, 760, 0, 1080, 110, |r, g, b| r > 200 && g < 60 && b > 200);
    assert_eq!(magenta, 0, "mask ended");
}
