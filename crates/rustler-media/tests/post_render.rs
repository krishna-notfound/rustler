//! Real renders for audio post, transitions, effects and the progress bar. Skip (with a printed
//! reason) when ffmpeg is missing. `RUSTLER_TEST_OUT=<dir>` keeps the frames for a visual check.

use rustler_core::{
    AudioSettings, Background, CaptionTrack, Clip, ClipId, Fit, Layout, MediaProbe, Rect, Region, RenderQuality,
    Segment, SegmentId, SourceId,
};
use rustler_media::compile::{
    CompileExtras, CompiledRender, Denoise, Effect, EffectKind, ProgressBar, SfxCue, SourceMedia, Transition,
    TransitionKind, TransitionMap, compile_with, effective_duration,
};
use rustler_media::encoders;
use rustler_media::render::render;
use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio_util::sync::CancellationToken;

fn exe(n: &str) -> String {
    if cfg!(windows) { format!("{n}.exe") } else { n.to_string() }
}

fn real_home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME").filter(|h| !h.is_empty()) {
        return h.into();
    }
    let h = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    PathBuf::from(h).join(".rustler")
}

fn tools(tmp_home: &Path) -> Option<ToolPaths> {
    let bin = real_home().join("bin");
    let (ffmpeg, ffprobe) = (bin.join(exe("ffmpeg")), bin.join(exe("ffprobe")));
    if !ffmpeg.is_file() || !ffprobe.is_file() {
        eprintln!("SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)", bin.display());
        return None;
    }
    Some(ToolPaths {
        home: tmp_home.to_path_buf(),
        bin: bin.clone(),
        ffmpeg,
        ffprobe,
        ytdlp: bin.join(exe("yt-dlp")),
        uv: bin.join(exe("uv")),
        engine_dir: tmp_home.join("engine"),
        venv_python: tmp_home.join("venv"),
        models: tmp_home.join("models"),
        fonts: tmp_home.join("fonts"),
    })
}

fn ff(t: &ToolPaths, args: &[&str]) {
    let o = Command::new(&t.ffmpeg).args(["-hide_banner", "-v", "error", "-y"]).args(args).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn probe_json(t: &ToolPaths, p: &Path) -> Value {
    let o = Command::new(&t.ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format", "-count_frames"])
        .arg(p)
        .output()
        .unwrap();
    assert!(o.status.success());
    serde_json::from_slice(&o.stdout).unwrap()
}

fn media_probe(t: &ToolPaths, p: &Path) -> MediaProbe {
    let v = probe_json(t, p);
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let vs = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    MediaProbe {
        duration: v["format"]["duration"].as_str().unwrap().parse().unwrap(),
        has_video: true,
        has_audio,
        width: vs["width"].as_u64().unwrap() as u32,
        height: vs["height"].as_u64().unwrap() as u32,
        rotation: 0,
        fps: 30.0,
        video_codec: Some("h264".into()),
        audio_codec: has_audio.then(|| "aac".into()),
        sample_rate: None,
        channels: None,
    }
}

/// (video frames, video duration, audio duration)
fn stream_info(t: &ToolPaths, p: &Path) -> (u64, f64, f64) {
    let v = probe_json(t, p);
    let s = v["streams"].as_array().unwrap();
    let vs = s.iter().find(|s| s["codec_type"] == "video").unwrap();
    let a = s.iter().find(|s| s["codec_type"] == "audio").unwrap();
    let f = |x: &Value| x.as_str().unwrap().parse::<f64>().unwrap();
    (vs["nb_read_frames"].as_str().unwrap().parse().unwrap(), f(&vs["duration"]), f(&a["duration"]))
}

fn center_crop() -> Layout {
    let w = 1080.0 * 9.0 / 16.0 / 1920.0;
    Layout {
        preset: Some("center-crop".into()),
        regions: vec![Region {
            id: "main".into(),
            src: Rect { x: (1.0 - w) / 2.0, y: 0.0, w, h: 1.0 },
            dst: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            z: 0,
            fit: Fit::Cover,
        }],
        background: Background::Black,
    }
}

fn make_clip(src: &SourceId, ranges: &[(f64, f64)], loudnorm: bool) -> Clip {
    let segs: Vec<Segment> = ranges
        .iter()
        .map(|&(s, e)| Segment { id: SegmentId::new(), source_id: src.clone(), start: s, end: e, layout: center_crop() })
        .collect();
    let audio = AudioSettings { loudnorm, ..AudioSettings::default() };
    serde_json::from_value(json!({
        "schema": "rustler.clip@1", "id": ClipId::new(), "name": "post", "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": segs, "captions": CaptionTrack::default(), "audio": audio,
        "renders": [], "created_at": "2026-11-15T00:00:00Z"
    }))
    .unwrap()
}

fn sources_for(t: &ToolPaths, path: &Path) -> (SourceId, HashMap<SourceId, SourceMedia>) {
    let id = SourceId::new();
    let mut m = HashMap::new();
    m.insert(id.clone(), SourceMedia { id: id.clone(), path: path.to_path_buf(), probe: media_probe(t, path) });
    (id, m)
}

async fn compile_render(
    t: &ToolPaths,
    tmp: &Path,
    name: &str,
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    x: &CompileExtras,
) -> PathBuf {
    let out = tmp.join(format!("{name}.mp4"));
    let c: CompiledRender = compile_with(
        clip,
        sources,
        None,
        tmp,
        &encoders::choice("libx264", RenderQuality::Draft),
        &out,
        &tmp.join(format!("work_{name}")),
        x,
    )
    .unwrap();
    render(t, &c, effective_duration(clip, x), &CancellationToken::new(), &ProgressSink::noop()).await.unwrap();
    out
}

fn keep(p: &Path, name: &str) {
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(p, dir.join(name));
    }
}

/// Exact output frame `n` as png.
fn frame_n(t: &ToolPaths, video: &Path, n: u64, out: &Path) -> image::RgbImage {
    ff(
        t,
        &[
            "-i",
            &video.to_string_lossy(),
            "-vf",
            &format!("select=eq(n\\,{n})"),
            "-fps_mode",
            "passthrough",
            "-frames:v",
            "1",
            "-update",
            "1",
            &out.to_string_lossy(),
        ],
    );
    image::open(out).unwrap().to_rgb8()
}

fn px(im: &image::RgbImage, x: u32, y: u32) -> [u8; 3] {
    im.get_pixel(x, y).0
}

/// Pixels brighter than 200 in all channels.
fn white_count(im: &image::RgbImage) -> u64 {
    im.pixels().filter(|p| p.0.iter().all(|c| *c > 200)).count() as u64
}

/// Mono 48 kHz float samples of the file's audio.
fn pcm(t: &ToolPaths, p: &Path) -> Vec<f32> {
    let o = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-v", "error", "-i"])
        .arg(p)
        .args(["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"])
        .output()
        .unwrap();
    assert!(o.status.success());
    o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn rms_db(s: &[f32], a: f64, b: f64) -> f64 {
    let w = &s[(a * 48000.0) as usize..((b * 48000.0) as usize).min(s.len())];
    let ms = w.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / w.len().max(1) as f64;
    10.0 * ms.max(1e-20).log10()
}

#[tokio::test]
async fn dissolve_blends_red_into_blue_frame_exact() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("rb.mp4");
    ff(
        &t,
        &[
            "-f", "lavfi", "-i", "color=c=red:s=1920x1080:r=30:d=2",
            "-f", "lavfi", "-i", "color=c=blue:s=1920x1080:r=30:d=2",
            "-f", "lavfi", "-i", "sine=f=440:r=48000:d=4",
            "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0,format=yuv420p[v]",
            "-map", "[v]", "-map", "2:a", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac",
            &src.to_string_lossy(),
        ],
    );
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 1.5), (2.5, 4.0)], true);
    let x = CompileExtras {
        transitions: vec![Transition {
            after_segment_id: clip.segments[0].id.clone(),
            kind: TransitionKind::Dissolve,
            duration: 0.4,
        }],
        two_pass_loudness: false,
        ..Default::default()
    };
    assert!((effective_duration(&clip, &x) - 2.6).abs() < 1e-9);
    let out = compile_render(&t, tmp.path(), "dissolve", &clip, &sources, &x).await;

    // 45 + 45 - 12 frames; audio as long as the video.
    let (frames, vd, ad) = stream_info(&t, &out);
    assert_eq!(frames, 78, "frame count");
    assert!((vd - 2.6).abs() < 0.04 && (ad - 2.6).abs() < 0.05, "video {vd} audio {ad}");

    // Transition frames 33..44: frame 39 is the 50% blend, 20 is red, 60 is blue.
    let mid = frame_n(&t, &out, 39, &tmp.path().join("mid.png"));
    keep(&tmp.path().join("mid.png"), "post_dissolve_mid.png");
    let [r, g, b] = px(&mid, 540, 960);
    eprintln!("dissolve midpoint pixel = ({r},{g},{b})");
    assert!((100..=160).contains(&r) && g < 40 && (100..=160).contains(&b), "midpoint is not a blend: {r},{g},{b}");
    let red = frame_n(&t, &out, 20, &tmp.path().join("red.png"));
    let [r, _, b] = px(&red, 540, 960);
    assert!(r > 200 && b < 40, "frame 20 should be red: {r},{b}");
    let blue = frame_n(&t, &out, 60, &tmp.path().join("blue.png"));
    let [r, _, b] = px(&blue, 540, 960);
    assert!(b > 200 && r < 40, "frame 60 should be blue: {r},{b}");
    // A quarter into the transition the red still dominates.
    let q = frame_n(&t, &out, 36, &tmp.path().join("q.png"));
    let [r, _, b] = px(&q, 540, 960);
    assert!(r > b, "frame 36 should still lean red: {r},{b}");
}

/// Black 1920x1080 with a centred white 200x200 box.
fn box_source(t: &ToolPaths, out: &Path, secs: u32) {
    ff(
        t,
        &[
            "-f", "lavfi", "-i",
            &format!("color=c=black:s=1920x1080:r=30:d={secs},drawbox=x=860:y=440:w=200:h=200:color=white:t=fill"),
            "-f", "lavfi", "-i", &format!("sine=f=440:r=48000:d={secs}"),
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac",
            &out.to_string_lossy(),
        ],
    );
}

#[tokio::test]
async fn punch_in_zooms_and_progress_bar_grows() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("box.mp4");
    box_source(&t, &src, 3);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 3.0)], false);
    let x = CompileExtras {
        effects: vec![Effect { id: "fx_1".into(), kind: EffectKind::PunchIn, start: 1.0, end: 2.0, amount: Some(2.0), focus_x: None, focus_y: None, extra: Default::default() }],
        progress_bar: Some(ProgressBar::default()),
        ..Default::default()
    };
    let out = compile_render(&t, tmp.path(), "punch", &clip, &sources, &x).await;
    let before = frame_n(&t, &out, 15, &tmp.path().join("before.png"));
    let during = frame_n(&t, &out, 45, &tmp.path().join("during.png"));
    let after = frame_n(&t, &out, 75, &tmp.path().join("after.png"));
    keep(&tmp.path().join("before.png"), "post_punch_before.png");
    keep(&tmp.path().join("during.png"), "post_punch_during.png");
    let (b, d, a) = (white_count(&before), white_count(&during), white_count(&after));
    eprintln!("white pixels: before {b}, during {d}, after {a}");
    // A 2x zoom shows the box at 4x the area; after the range it is back.
    let ratio = d as f64 / b as f64;
    assert!((3.5..=4.5).contains(&ratio), "punch-in area ratio {ratio}");
    assert!((a as f64 / b as f64 - 1.0).abs() < 0.1, "zoom ends with the range");

    // Progress bar (top, 8 px, #FFE000): frame 15 of 90 -> 16/90 of 1080 = 192 px.
    let yellow = |p: [u8; 3]| p[0] > 200 && p[1] > 190 && p[2] < 90;
    assert!(yellow(px(&before, 100, 3)), "{:?}", px(&before, 100, 3));
    assert!(!yellow(px(&before, 600, 3)), "{:?}", px(&before, 600, 3));
    assert!(!yellow(px(&before, 100, 20)), "bar is only 8 px high");
    // Frame 45 -> 46/90 = 552 px.
    assert!(yellow(px(&during, 500, 3)) && !yellow(px(&during, 700, 3)));
    let last = frame_n(&t, &out, 89, &tmp.path().join("last.png"));
    assert!(yellow(px(&last, 1075, 3)), "full width on the last frame");
}

#[tokio::test]
async fn zoom_slow_shake_and_flash_render() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("box.mp4");
    box_source(&t, &src, 3);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 3.0)], false);
    let x = CompileExtras {
        effects: vec![
            Effect { id: "fx_1".into(), kind: EffectKind::ZoomSlow, start: 0.0, end: 2.0, amount: Some(2.0), focus_x: None, focus_y: None, extra: Default::default() },
            Effect { id: "fx_2".into(), kind: EffectKind::Flash, start: 2.2, end: 2.4, amount: None, focus_x: None, focus_y: None, extra: Default::default() },
            Effect { id: "fx_3".into(), kind: EffectKind::Shake, start: 2.5, end: 2.9, amount: Some(0.03), focus_x: None, focus_y: None, extra: Default::default() },
        ],
        ..Default::default()
    };
    let out = compile_render(&t, tmp.path(), "fx", &clip, &sources, &x).await;
    let w0 = white_count(&frame_n(&t, &out, 0, &tmp.path().join("z0.png"))) as f64;
    let w30 = white_count(&frame_n(&t, &out, 30, &tmp.path().join("z30.png"))) as f64;
    let w59 = white_count(&frame_n(&t, &out, 59, &tmp.path().join("z59.png"))) as f64;
    eprintln!("zoom-slow white pixels: {w0} {w30} {w59}");
    // z = 1 + n/60: 1.5^2 = 2.25 at frame 30, ~1.98^2 = 3.93 at frame 59.
    assert!((w30 / w0 - 2.25).abs() < 0.2, "frame 30 ratio {}", w30 / w0);
    assert!((w59 / w0 - 3.93).abs() < 0.3, "frame 59 ratio {}", w59 / w0);
    let flash = frame_n(&t, &out, 66, &tmp.path().join("flash.png"));
    keep(&tmp.path().join("flash.png"), "post_flash.png");
    let [r, g, b] = px(&flash, 100, 100);
    assert!(r > 240 && g > 240 && b > 240, "flash start is white: {r},{g},{b}");
    let calm = frame_n(&t, &out, 73, &tmp.path().join("calm.png"));
    assert!(px(&calm, 100, 100)[0] < 30, "flash is over");
    // Shake moves the box off centre on some frame.
    let centres: Vec<f64> = (76..86)
        .map(|n| {
            let im = frame_n(&t, &out, n, &tmp.path().join(format!("s{n}.png")));
            let xs: Vec<u32> = im.enumerate_pixels().filter(|(_, _, p)| p.0[0] > 200).map(|(x, _, _)| x).collect();
            xs.iter().map(|x| *x as f64).sum::<f64>() / xs.len().max(1) as f64
        })
        .collect();
    let spread = centres.iter().cloned().fold(f64::MIN, f64::max) - centres.iter().cloned().fold(f64::MAX, f64::min);
    eprintln!("shake box centres: {centres:?}");
    assert!(spread > 8.0, "shake should move the picture (spread {spread})");
}

#[tokio::test]
async fn sfx_lands_on_the_mapped_time_within_one_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    // Silent picture-only source; a 1 kHz beep as the effect.
    let src = tmp.path().join("gray.mp4");
    ff(
        &t,
        &["-f", "lavfi", "-i", "color=c=gray:s=1920x1080:r=30:d=4", "-c:v", "libx264", "-preset", "ultrafast",
          "-pix_fmt", "yuv420p", &src.to_string_lossy()],
    );
    let beep = tmp.path().join("beep.wav");
    ff(&t, &["-f", "lavfi", "-i", "sine=f=1000:r=48000:d=0.2", &beep.to_string_lossy()]);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 1.5), (2.0, 3.5)], false);
    let x = CompileExtras {
        transitions: vec![Transition {
            after_segment_id: clip.segments[0].id.clone(),
            kind: TransitionKind::Dissolve,
            duration: 0.3,
        }],
        sfx: vec![SfxCue { id: "sfx_1".into(), path: beep.clone(), at: 2.0, volume_db: -6.0 }],
        ..Default::default()
    };
    let expected = TransitionMap::new(&clip, &x.transitions).map_time(2.0);
    assert!((expected - 1.7).abs() < 1e-9);
    let out = compile_render(&t, tmp.path(), "sfx", &clip, &sources, &x).await;
    let s = pcm(&t, &out);
    let onset = s.iter().position(|v| v.abs() > 0.05).expect("beep is audible") as f64 / 48000.0;
    eprintln!("sfx onset {onset:.4}s (expected {expected:.4}s)");
    assert!((onset - expected).abs() <= 1.0 / 30.0, "onset {onset} vs {expected}");
    let peak_db = rms_db(&s, expected + 0.05, expected + 0.15);
    assert!(peak_db > -40.0, "beep level {peak_db}");
}

#[tokio::test]
async fn denoise_lowers_the_noise_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    // Tone 2 s on / 2 s off over white noise at about -39 dBFS RMS.
    let src = tmp.path().join("noisy.mp4");
    ff(
        &t,
        &[
            "-f", "lavfi", "-i", "color=c=gray:s=1920x1080:r=30:d=6",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=6,volume='if(lt(mod(t,4),2),0.5,0)':eval=frame",
            "-f", "lavfi", "-i", "anoisesrc=a=0.02:c=white:r=48000:d=6",
            "-filter_complex", "[1:a][2:a]amix=inputs=2:normalize=0[a]",
            "-map", "0:v", "-map", "[a]", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            "-c:a", "pcm_s16le", &src.with_extension("mov").to_string_lossy(),
        ],
    );
    let src = src.with_extension("mov");
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 6.0)], false);
    let mut levels = Vec::new();
    for (name, d) in [("off", Denoise::Off), ("light", Denoise::Light), ("strong", Denoise::Strong)] {
        let x = CompileExtras { denoise: d, ..Default::default() };
        let out = compile_render(&t, tmp.path(), &format!("dn_{name}"), &clip, &sources, &x).await;
        let s = pcm(&t, &out);
        let (gap, tone) = (rms_db(&s, 2.5, 3.7), rms_db(&s, 0.5, 1.7));
        eprintln!("denoise {name}: gap {gap:.1} dB, tone {tone:.1} dB");
        levels.push((gap, tone));
    }
    let (off, light, strong) = (levels[0], levels[1], levels[2]);
    assert!(light.0 < off.0 - 8.0, "light lowers the floor: {off:?} -> {light:?}");
    assert!(strong.0 < off.0 - 20.0, "strong lowers the floor a lot: {off:?} -> {strong:?}");
    assert!((strong.1 - off.1).abs() < 3.0, "the tone survives: {off:?} -> {strong:?}");
}
