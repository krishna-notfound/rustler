//! reference.* tests on synthetic videos with known cut times, a caption band at y~0.75, hook text,
//! a zoom and a subtitle sidecar. Need ffmpeg + uv + the engine (OpenCV); each test skips with a printed
//! reason when a tool is missing.
mod support;

use rustler_core::ErrorCode;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use support::*;

const PHRASES: [&str; 8] = ["ALPHA BETA", "GAMMA DELTA", "EPSILON ZETA", "ETA THETA", "IOTA KAPPA", "LAMBDA MU", "NU XI", "OMICRON PI"];

fn font_dir() -> Option<PathBuf> {
    let d = home().join("fonts").join("Poppins");
    if d.join("Poppins-Bold.ttf").is_file() {
        Some(d)
    } else {
        eprintln!("SKIP: Poppins font missing under {} (run `rustler setup`)", d.display());
        None
    }
}

fn ffmpeg_ok(cwd: Option<&Path>, args: &[&str]) {
    let mut c = Command::new(ffmpeg());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let out = c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]).args(args).output().expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// 12 s, 540x960 @15: colour shots that change at 4 s and 8 s, a yellow caption (changes every 0.75 s) with
/// its top at y = 0.75 h, a white hook for the first 2.5 s, a sine tone, and a subtitle sidecar (12 cues x 2 words).
fn make_captioned(dir: &Path, fonts: &Path) -> PathBuf {
    let out = dir.join("captioned.mp4");
    let mut vf = vec![
        "drawtext=fontfile=Poppins-Bold.ttf:text='WAIT FOR IT':fontsize=54:fontcolor=white:borderw=3:bordercolor=black:x=(w-text_w)/2:y=h*0.12:enable='lt(t,2.5)'".to_string(),
    ];
    for k in 0..16 {
        let (a, b) = (k as f64 * 0.75, (k + 1) as f64 * 0.75 - 0.05);
        vf.push(format!(
            "drawtext=fontfile=Poppins-Bold.ttf:text='{}':fontsize=64:fontcolor=yellow:borderw=4:bordercolor=black:x=(w-text_w)/2:y=h*0.75:enable='between(t,{a:.2},{b:.2})'",
            PHRASES[k % PHRASES.len()]
        ));
    }
    let bg = "color=c=0x305070:s=540x960:r=15:d=4[a];color=c=0x703030:s=540x960:r=15:d=4[b];color=c=0x307040:s=540x960:r=15:d=4[c];[a][b][c]concat=n=3:v=1:a=0[v]";
    let fc = format!("{bg};[v]{}[o]", vf.join(","));
    ffmpeg_ok(
        Some(fonts),
        &["-filter_complex", &fc, "-f", "lavfi", "-i", "sine=frequency=300:duration=12", "-map", "[o]", "-map", "0:a", "-t", "12", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", out.to_str().unwrap()],
    );
    let mut srt = String::new();
    for k in 0..12 {
        srt.push_str(&format!("{}\n00:00:{:02},000 --> 00:00:{:02},000\nword{} other{}\n\n", k + 1, k, k + 1, k, k));
    }
    std::fs::write(dir.join("captioned.srt"), srt).unwrap();
    out
}

/// A 1080x1920 random-blob texture (deterministic) zooming 1.0 -> 1.6 over 5 s into 540x960.
fn make_zoom(dir: &Path) -> PathBuf {
    use image::{Rgb, RgbImage};
    use imageproc::drawing::{draw_filled_circle_mut, draw_filled_rect_mut};
    use imageproc::rect::Rect;
    let mut img = RgbImage::from_pixel(1080, 1920, Rgb([90, 100, 110]));
    let mut s: u64 = 12345;
    let mut next = |m: u64| {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (s >> 33) % m
    };
    for _ in 0..140 {
        let c = Rgb([next(255) as u8, next(255) as u8, next(255) as u8]);
        let (x, y) = (next(1080) as i32, next(1920) as i32);
        if next(2) == 0 {
            draw_filled_rect_mut(&mut img, Rect::at(x, y).of_size(20 + next(90) as u32, 20 + next(90) as u32), c);
        } else {
            draw_filled_circle_mut(&mut img, (x, y), 10 + next(50) as i32, c);
        }
    }
    let png = dir.join("tex.png");
    img.save(&png).unwrap();
    let out = dir.join("zoom.mp4");
    ffmpeg_ok(
        None,
        &["-loop", "1", "-framerate", "15", "-i", png.to_str().unwrap(), "-t", "5", "-vf", "zoompan=z='1+0.006*on':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s=540x960:fps=15", "-c:v", "libx264", "-pix_fmt", "yuv420p", out.to_str().unwrap()],
    );
    std::fs::remove_file(png).unwrap();
    out
}

fn near(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: got {a}, wanted {b} +- {tol}");
}

#[tokio::test]
async fn import_analyze_list_show_on_synthetic_videos() {
    if !have_media_tools() || !have_engine() {
        return;
    }
    let Some(fonts) = font_dir() else { return };
    let (_keep, ctx) = project();
    let reg = registry();
    let refs = tempfile::tempdir().unwrap();
    make_captioned(refs.path(), &fonts);
    make_zoom(refs.path());
    std::fs::write(refs.path().join("notes.txt"), "not a video").unwrap();
    std::fs::write(refs.path().join("broken.mp4"), "this is not a video").unwrap();

    // bad names and missing inputs are validation / not_found errors
    let e = run(&reg, "reference.import", &ctx, json!({"name": "../x", "folder": refs.path()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = run(&reg, "reference.import", &ctx, json!({"name": "a", "folder": refs.path().join("nope")})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = run(&reg, "reference.analyze", &ctx, json!({"name": "ghost"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    let imp = ok(&reg, "reference.import", &ctx, json!({"name": "synth", "folder": refs.path()})).await;
    let videos = imp["videos"].as_array().unwrap();
    assert_eq!(videos.len(), 2, "{imp}");
    assert_eq!(videos[0]["id"], "v01");
    assert_eq!(imp["skipped"].as_array().unwrap().len(), 1, "broken.mp4 is skipped: {imp}");
    let proj = ctx.project_dir.clone().unwrap();
    for v in videos {
        assert!(proj.join(v["proxy"].as_str().unwrap()).is_file());
    }
    assert!(videos[0]["audio"].is_string(), "captioned has audio: {}", videos[0]);
    assert!(videos[1]["audio"].is_null(), "zoom has no audio");
    // originals untouched
    assert!(refs.path().join("captioned.mp4").is_file());

    // a second import under the same name needs --replace
    let e = run(&reg, "reference.import", &ctx, json!({"name": "synth", "folder": refs.path()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    let l = ok(&reg, "reference.list", &ctx, json!({})).await;
    assert_eq!(l["sets"][0]["name"], "synth");
    assert_eq!(l["sets"][0]["videos"], 2);
    assert_eq!(l["sets"][0]["analyzed"], false);

    let a = ok(&reg, "reference.analyze", &ctx, json!({"name": "synth", "no_faces": true})).await;
    let vs = a["videos"].as_array().unwrap();
    assert_eq!(vs.len(), 2);
    let cap = &vs[0];
    assert_eq!(cap["file"], "captioned.mp4");

    // cuts at 4 s and 8 s -> 10 per minute
    let times: Vec<f64> = cap["cuts"]["times"].as_array().unwrap().iter().map(|t| t.as_f64().unwrap()).collect();
    assert_eq!(times.len(), 2, "{times:?}");
    near(times[0], 4.0, 0.25, "first cut");
    near(times[1], 8.0, 0.25, "second cut");
    near(cap["cuts"]["per_minute"].as_f64().unwrap(), 10.0, 0.5, "cuts per minute");
    near(cap["cuts"]["shot_length"]["median"].as_f64().unwrap(), 4.0, 0.4, "median shot");

    // caption band near y = 0.75 + half a line; hook gone by ~2.5 s; yellow fill
    let c = &cap["text"]["caption"];
    assert_eq!(c["present"], true, "{}", cap["text"]);
    let y = c["y_center"].as_f64().unwrap();
    assert!((0.74..=0.83).contains(&y), "caption y_center {y}");
    let wpl = c["words_per_line"]["median"].as_f64().unwrap();
    assert!((1.0..=3.0).contains(&wpl), "words per line {wpl}");
    let hex = c["fill_colors"][0]["hex"].as_str().unwrap();
    let (r, g, b) = (u8::from_str_radix(&hex[1..3], 16).unwrap(), u8::from_str_radix(&hex[3..5], 16).unwrap(), u8::from_str_radix(&hex[5..7], 16).unwrap());
    assert!(r > 200 && g > 200 && b < 100, "fill colour {hex}");
    let hook = &cap["text"]["hook"];
    assert_eq!(hook["present"], true, "{}", cap["text"]);
    assert!(hook["visible_to"].as_f64().unwrap() <= 3.5 && hook["y1"].as_f64().unwrap() < 0.3, "{hook}");

    // loudness of the 300 Hz tone is measured; speech rate from the sidecar: 12 cues x 2 words, 1 s each
    let lufs = cap["loudness"]["integrated_lufs"].as_f64().unwrap();
    assert!((-60.0..0.0).contains(&lufs), "{lufs}");
    near(cap["speech"]["wpm_while_speaking"].as_f64().unwrap(), 120.0, 3.0, "wpm");
    assert!(cap["music"]["available"].as_bool().unwrap());

    // the zoom clip: no cuts, a zoom-in event, no caption, no audio
    let z = &vs[1];
    assert_eq!(z["cuts"]["count"], 0, "{}", z["cuts"]);
    let kinds: Vec<&str> = z["zoom"]["events"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"zoom_in"), "{}", z["zoom"]);
    assert_eq!(z["text"]["caption"]["present"], false);
    assert!(z["loudness"].is_null() && z["speech"].is_null());

    // images: contact sheets first, then filmstrips; all exist and are real jpgs
    let images = a["image"].as_array().unwrap();
    assert_eq!(images.len(), 4);
    for i in images {
        assert!(std::fs::metadata(i.as_str().unwrap()).unwrap().len() > 2000, "{i}");
    }
    assert!(images[0].as_str().unwrap().ends_with("v01_contact.jpg") && images[2].as_str().unwrap().ends_with("v01_first3s.jpg"));
    let strip = image::open(images[2].as_str().unwrap()).unwrap();
    assert!(strip.width() >= 4 * 300, "filmstrip is 4 columns of 300 px: {}", strip.width());

    // aggregate + profile.json
    let agg = &a["aggregate"];
    assert_eq!(agg["videos"], 2);
    assert_eq!(agg["caption"]["present_in"], 1);
    assert!(agg["summary"].as_array().unwrap().iter().any(|s| s.as_str().unwrap().contains("cuts per minute")));
    let saved: Value = serde_json::from_slice(&std::fs::read(a["profile_path"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(saved["name"], "synth");

    let l = ok(&reg, "reference.list", &ctx, json!({})).await;
    assert_eq!(l["sets"][0]["analyzed"], true);
    let s = ok(&reg, "reference.show", &ctx, json!({"name": "synth", "video": "v01"})).await;
    assert_eq!(s["image"].as_array().unwrap().len(), 2);
    assert!(s["profile"]["aggregate"].is_object());
    let e = run(&reg, "reference.show", &ctx, json!({"name": "synth", "video": "v09"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    // max_videos limits the run
    let a1 = ok(&reg, "reference.analyze", &ctx, json!({"name": "synth", "max_videos": 1, "no_faces": true})).await;
    assert_eq!(a1["videos"].as_array().unwrap().len(), 1);

    // replace rebuilds the set
    let again = ok(&reg, "reference.import", &ctx, json!({"name": "synth", "files": [refs.path().join("zoom.mp4")], "replace": true})).await;
    assert_eq!(again["videos"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn import_needs_videos() {
    if !have_media_tools() {
        return;
    }
    let (_keep, ctx) = project();
    let reg = registry();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "x").unwrap();
    let e = run(&reg, "reference.import", &ctx, json!({"name": "empty", "folder": d.path()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    // recursive finds nested files only when asked
    std::fs::create_dir_all(d.path().join("sub")).unwrap();
    std::fs::write(d.path().join("sub").join("b.mp4"), "not a video").unwrap();
    let e = run(&reg, "reference.import", &ctx, json!({"name": "empty", "folder": d.path()})).await.unwrap_err();
    assert!(e.message.contains("no video files"), "{}", e.message);
    let e = run(&reg, "reference.import", &ctx, json!({"name": "empty", "folder": d.path(), "recursive": true})).await.unwrap_err();
    assert!(e.message.contains("none of the files"), "{}", e.message);
}
