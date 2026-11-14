//! Real renders through ffmpeg. Skip (with a printed reason) when tools are missing.
//! Set `RUSTLER_TEST_OUT=<dir>` to keep extracted frames/renders for a visual check.

use rustler_core::{
    AudioSettings, Background, CaptionTrack, Clip, ClipId, ErrorCode, Fit, Layout, MediaProbe,
    Rect, Region, RenderQuality, Segment, SegmentId, SourceId,
};
use rustler_media::compile::{
    CompileExtras, CompiledRender, CropKey, MusicBed, SourceMedia, compile_with,
};
use rustler_media::encoders::{self, EncoderChoice};
use rustler_media::render::{preview_frame, render};
use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn exe(n: &str) -> String {
    if cfg!(windows) {
        format!("{n}.exe")
    } else {
        n.to_string()
    }
}

fn real_home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME").filter(|h| !h.is_empty()) {
        return h.into();
    }
    let h =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    PathBuf::from(h).join(".rustler")
}

/// ToolPaths pointing at the real binaries but a throwaway home (encoder cache goes there).
fn tools(tmp_home: &Path) -> Option<ToolPaths> {
    let bin = real_home().join("bin");
    let ffmpeg = bin.join(exe("ffmpeg"));
    let ffprobe = bin.join(exe("ffprobe"));
    if !ffmpeg.is_file() || !ffprobe.is_file() {
        eprintln!(
            "SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)",
            bin.display()
        );
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
    let o = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

struct Probed {
    duration: f64,
    width: u32,
    height: u32,
    vcodec: Option<String>,
    acodec: Option<String>,
    has_audio: bool,
}

fn ffprobe(t: &ToolPaths, p: &Path) -> Probed {
    let o = Command::new(&t.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(p)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "ffprobe: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let vs = streams.iter().find(|s| s["codec_type"] == "video");
    let a = streams.iter().find(|s| s["codec_type"] == "audio");
    Probed {
        duration: v["format"]["duration"]
            .as_str()
            .unwrap_or("0")
            .parse()
            .unwrap(),
        width: vs.and_then(|s| s["width"].as_u64()).unwrap_or(0) as u32,
        height: vs.and_then(|s| s["height"].as_u64()).unwrap_or(0) as u32,
        vcodec: vs.and_then(|s| s["codec_name"].as_str()).map(String::from),
        acodec: a.and_then(|s| s["codec_name"].as_str()).map(String::from),
        has_audio: a.is_some(),
    }
}

fn media_probe(t: &ToolPaths, p: &Path) -> MediaProbe {
    let q = ffprobe(t, p);
    MediaProbe {
        duration: q.duration,
        has_video: q.vcodec.is_some(),
        has_audio: q.has_audio,
        width: q.width,
        height: q.height,
        rotation: 0,
        fps: 30.0,
        video_codec: q.vcodec,
        audio_codec: q.acodec,
        sample_rate: None,
        channels: None,
    }
}

/// 1920x1080 30 fps: left half testsrc2, right half SMPTE bars; 440 Hz tone (unless `audio`
/// is false).
fn make_source(t: &ToolPaths, out: &Path, secs: u32, audio: bool) {
    let d = secs.to_string();
    let mut a: Vec<String> = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("testsrc2=s=960x1080:r=30:d={d}"),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("smptehdbars=s=960x1080:r=30:d={d}"),
    ];
    if audio {
        a.extend([
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("sine=f=440:r=48000:d={d}"),
        ]);
    }
    a.extend([
        "-filter_complex".into(),
        "[0:v][1:v]hstack=inputs=2,format=yuv420p[v]".into(),
        "-map".into(),
        "[v]".into(),
    ]);
    if audio {
        a.extend(["-map".into(), "2:a".into(), "-c:a".into(), "aac".into()]);
    }
    a.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-g".into(),
        "30".into(),
    ]);
    a.push(out.to_string_lossy().into_owned());
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    ff(t, &refs);
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

fn region(id: &str, src: Rect, dst: Rect, z: i32, fit: Fit) -> Region {
    Region {
        id: id.into(),
        src,
        dst,
        z,
        fit,
    }
}

fn center_crop() -> Layout {
    let w = 1080.0 * 9.0 / 16.0 / 1920.0;
    Layout {
        preset: Some("center-crop".into()),
        regions: vec![region(
            "main",
            rect((1.0 - w) / 2.0, 0.0, w, 1.0),
            rect(0.0, 0.0, 1.0, 1.0),
            0,
            Fit::Cover,
        )],
        background: Background::Black,
    }
}

fn blur_fit() -> Layout {
    Layout {
        preset: Some("blur-fit".into()),
        regions: vec![region(
            "main",
            rect(0.0, 0.0, 1.0, 1.0),
            rect(0.0, 0.0, 1.0, 1.0),
            0,
            Fit::Contain,
        )],
        background: Background::BlurFill,
    }
}

fn split_vertical() -> Layout {
    Layout {
        preset: Some("split-vertical".into()),
        regions: vec![
            region(
                "top",
                rect(0.0, 0.0, 0.5, 1.0),
                rect(0.0, 0.0, 1.0, 0.5),
                0,
                Fit::Cover,
            ),
            region(
                "bottom",
                rect(0.5, 0.0, 0.5, 1.0),
                rect(0.0, 0.5, 1.0, 0.5),
                1,
                Fit::Cover,
            ),
        ],
        background: Background::Black,
    }
}

fn make_clip(src: &SourceId, start: f64, end: f64, layout: Layout) -> Clip {
    let seg = Segment {
        id: SegmentId::new(),
        source_id: src.clone(),
        start,
        end,
        layout,
    };
    serde_json::from_value(json!({
        "schema": "rustler.clip@1",
        "id": ClipId::new(),
        "name": "test",
        "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": [seg],
        "captions": CaptionTrack::default(),
        "audio": AudioSettings::default(),
        "renders": [],
        "created_at": "2026-11-15T00:00:00Z"
    }))
    .unwrap()
}

fn sources_for(t: &ToolPaths, path: &Path) -> (SourceId, HashMap<SourceId, SourceMedia>) {
    let id = SourceId::new();
    let mut m = HashMap::new();
    m.insert(
        id.clone(),
        SourceMedia {
            id: id.clone(),
            path: path.to_path_buf(),
            probe: media_probe(t, path),
        },
    );
    (id, m)
}

fn x264() -> EncoderChoice {
    encoders::choice("libx264", RenderQuality::Draft)
}

/// Copies `p` to `$RUSTLER_TEST_OUT/<name>` when that env var is set.
fn keep(p: &Path, name: &str) {
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(p, dir.join(name));
    }
}

fn frame_at(t: &ToolPaths, video: &Path, at: f64, out: &Path) {
    ff(
        t,
        &[
            "-ss",
            &format!("{at:.3}"),
            "-i",
            &video.to_string_lossy(),
            "-frames:v",
            "1",
            "-q:v",
            "3",
            "-update",
            "1",
            &out.to_string_lossy(),
        ],
    );
}

fn mean_rgb(img: &Path) -> [f64; 3] {
    let im = image::open(img).unwrap().to_rgb8();
    let mut s = [0f64; 3];
    for p in im.pixels() {
        for c in 0..3 {
            s[c] += p[c] as f64;
        }
    }
    let n = (im.width() * im.height()) as f64;
    [s[0] / n, s[1] / n, s[2] / n]
}

async fn do_render(t: &ToolPaths, c: &CompiledRender, dur: f64) -> rustler_core::Result<()> {
    render(t, c, dur, &CancellationToken::new(), &ProgressSink::noop()).await
}

fn assert_vertical(p: &Probed, dur: f64) {
    assert_eq!((p.width, p.height), (1080, 1920));
    assert!(
        (p.duration - dur).abs() <= 0.1,
        "duration {} vs {dur}",
        p.duration
    );
    assert_eq!(p.vcodec.as_deref(), Some("h264"));
    assert_eq!(p.acodec.as_deref(), Some("aac"));
}

#[tokio::test]
async fn render_center_crop_10s() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 12, true);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 1.0, 11.0, center_crop());
    let out = tmp.path().join("out.mp4");
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &x264(),
        &out,
        &tmp.path().join("work"),
        &CompileExtras::default(),
    )
    .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sink = ProgressSink::new(move |e| ev.lock().unwrap().push(e.pct.unwrap_or(-1.0)));
    render(&t, &c, 10.0, &CancellationToken::new(), &sink)
        .await
        .unwrap();
    assert_vertical(&ffprobe(&t, &out), 10.0);
    let ev = events.lock().unwrap();
    assert!(
        ev.len() >= 3 && *ev.last().unwrap() == 1.0,
        "progress events {ev:?}"
    );
    let jpg = tmp.path().join("cc.jpg");
    frame_at(&t, &out, 5.0, &jpg);
    keep(&jpg, "center_crop.jpg");
}

#[tokio::test]
async fn render_blur_fit_and_split_vertical() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 4, true);
    let (id, sources) = sources_for(&t, &src);
    for (name, layout) in [
        ("blur_fit", blur_fit()),
        ("split_vertical", split_vertical()),
    ] {
        let clip = make_clip(&id, 0.0, 3.0, layout);
        let out = tmp.path().join(format!("{name}.mp4"));
        let c = compile_with(
            &clip,
            &sources,
            None,
            tmp.path(),
            &x264(),
            &out,
            &tmp.path().join(name),
            &CompileExtras::default(),
        )
        .unwrap();
        do_render(&t, &c, 3.0).await.unwrap();
        assert_vertical(&ffprobe(&t, &out), 3.0);
        let jpg = tmp.path().join(format!("{name}.jpg"));
        frame_at(&t, &out, 1.5, &jpg);
        keep(&jpg, &format!("{name}.jpg"));
        if name == "split_vertical" {
            // Bottom half shows SMPTE bars (saturated), top half testsrc2: they must differ.
            let im = image::open(&jpg).unwrap().to_rgb8();
            let top = im.get_pixel(540, 480);
            let bottom = im.get_pixel(540, 1440);
            assert_ne!(top, bottom);
        } else {
            // Blur-fit: picture centred, blurred+darkened backdrop above it is not black.
            let im = image::open(&jpg).unwrap().to_rgb8();
            let p = im.get_pixel(540, 200);
            assert!(
                p[0] as u32 + p[1] as u32 + p[2] as u32 > 30,
                "backdrop looks black: {p:?}"
            );
        }
    }
}

#[tokio::test]
async fn render_nvenc_when_available() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let picked = encoders::pick_encoder(&t, RenderQuality::Final, true).await;
    eprintln!("pick_encoder(prefer_gpu) -> {}", picked.name);
    assert!(
        tmp.path().join("encoders.json").is_file(),
        "probe results cached"
    );
    let cpu = encoders::pick_encoder(&t, RenderQuality::Final, false).await;
    assert_eq!(cpu.name, "libx264");
    if !encoders::probe_encoder(&t.ffmpeg, "h264_nvenc").await {
        eprintln!("SKIP: h264_nvenc probe failed on this machine");
        return;
    }
    assert_eq!(picked.name, "h264_nvenc");
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 4, true);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 3.0, center_crop());
    let out = tmp.path().join("nv.mp4");
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &picked,
        &out,
        &tmp.path().join("w"),
        &CompileExtras::default(),
    )
    .unwrap();
    do_render(&t, &c, 3.0).await.unwrap();
    assert_vertical(&ffprobe(&t, &out), 3.0);
}

const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,96,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,6,0,2,40,40,500,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:00:10.00,Default,,0,0,0,,HELLO टेस्ट RUSTLER\n";

fn fonts_dir(tmp: &Path) -> PathBuf {
    let d = tmp.join("fonts src");
    std::fs::create_dir_all(&d).unwrap();
    for f in [
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\Nirmala.ttc",
        "C:\\Windows\\Fonts\\Nirmala.ttf",
    ] {
        let p = Path::new(f);
        if p.is_file() {
            let _ = std::fs::copy(p, d.join(p.file_name().unwrap()));
        }
    }
    d
}

#[tokio::test]
async fn render_handles_awkward_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let dir = tmp.path().join("dir with space [x]");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("my clip (1) – टेस्ट.mp4");
    make_source(&t, &src, 4, true);
    let (id, sources) = sources_for(&t, &src);
    let ass = dir.join("caps 'quoted' – टेस्ट.ass");
    std::fs::write(&ass, ASS).unwrap();
    let clip = make_clip(&id, 0.5, 3.5, center_crop());
    let work = dir.join("work dir (टेस्ट)");
    let out = dir.join("out – टेस्ट.mp4");
    let c = compile_with(
        &clip,
        &sources,
        Some(&ass),
        &fonts_dir(tmp.path()),
        &x264(),
        &out,
        &work,
        &CompileExtras::default(),
    )
    .unwrap();
    do_render(&t, &c, 3.0).await.unwrap();
    assert_vertical(&ffprobe(&t, &out), 3.0);
    let jpg = tmp.path().join("caps.jpg");
    frame_at(&t, &out, 1.0, &jpg);
    keep(&jpg, "captions_awkward.jpg");
}

#[tokio::test]
async fn render_video_without_audio_gets_silent_track() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("silent.mp4");
    make_source(&t, &src, 4, false);
    let (id, sources) = sources_for(&t, &src);
    assert!(!sources[&id].probe.has_audio);
    let clip = make_clip(&id, 0.0, 3.0, blur_fit());
    let out = tmp.path().join("out.mp4");
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &x264(),
        &out,
        &tmp.path().join("w"),
        &CompileExtras::default(),
    )
    .unwrap();
    do_render(&t, &c, 3.0).await.unwrap();
    assert_vertical(&ffprobe(&t, &out), 3.0);
}

#[tokio::test]
async fn preview_frame_matches_size() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 4, true);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 3.0, split_vertical());
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &x264(),
        &tmp.path().join("never.mp4"),
        &tmp.path().join("w"),
        &CompileExtras::default(),
    )
    .unwrap();
    let jpg = tmp.path().join("prev.jpg");
    preview_frame(&t, &c, 1.5, &jpg).await.unwrap();
    assert_eq!(image::image_dimensions(&jpg).unwrap(), (1080, 1920));
    assert!(!tmp.path().join("never.mp4").exists());
    keep(&jpg, "preview_split.jpg");
    let e = preview_frame(&t, &c, 30.0, &tmp.path().join("late.jpg"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Media);
}

#[tokio::test]
async fn failed_render_removes_partial_output() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 2, true);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 1.0, center_crop());
    let out = tmp.path().join("bad.mp4");
    let bad = EncoderChoice {
        name: "libx264".into(),
        args: vec!["-c:v".into(), "no_such_encoder".into()],
    };
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &bad,
        &out,
        &tmp.path().join("w"),
        &CompileExtras::default(),
    )
    .unwrap();
    let e = do_render(&t, &c, 1.0).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Media);
    assert!(e.message.contains("no_such_encoder"), "{}", e.message);
    assert!(!out.exists());
}

/// Volume (dB) of the low band (< ~70 Hz: the music's 55 Hz tone, little speech) in [a, b].
fn low_band_db(t: &ToolPaths, p: &Path, a: f64, b: f64) -> f64 {
    let o = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(p)
        .args([
            "-vn",
            "-af",
            &format!("atrim={a}:{b},lowpass=f=70,lowpass=f=70,volumedetect"),
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stderr);
    let line = s
        .lines()
        .find(|l| l.contains("mean_volume:"))
        .unwrap_or_else(|| panic!("{s}"));
    line.split("mean_volume:")
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(" dB")
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn render_music_bed_ducks() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let jfk = real_home().join("test-media").join("jfk.wav");
    if !jfk.is_file() {
        eprintln!("SKIP: {} missing", jfk.display());
        return;
    }
    // 10 s: speech for 5 s, then silence.
    let src = tmp.path().join("speech.mp4");
    ff(
        &t,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=30:d=10",
            "-i",
            &jfk.to_string_lossy(),
            "-filter_complex",
            "[1:a]atrim=0:5,aresample=48000,apad[a]",
            "-map",
            "0:v",
            "-map",
            "[a]",
            "-t",
            "10",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            &src.to_string_lossy(),
        ],
    );
    // 3 s music loop (forces -stream_loop): 55 Hz + 220 Hz.
    let music = tmp.path().join("bed music – टेस्ट.wav");
    ff(
        &t,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.4*sin(2*PI*55*t)+0.2*sin(2*PI*220*t):s=48000:d=3",
            "-c:a",
            "pcm_s16le",
            &music.to_string_lossy(),
        ],
    );
    let (id, sources) = sources_for(&t, &src);

    let mut outs = Vec::new();
    for (name, duck, loudnorm) in [
        ("duck", true, false),
        ("flat", false, false),
        ("final", true, true),
    ] {
        let mut clip = make_clip(&id, 0.0, 10.0, center_crop());
        clip.audio.loudnorm = loudnorm;
        let mut bed = MusicBed::new(&music);
        bed.duck = duck;
        let extras = CompileExtras {
            music: Some(bed),
            ..Default::default()
        };
        let out = tmp.path().join(format!("{name}.mp4"));
        let c = compile_with(
            &clip,
            &sources,
            None,
            tmp.path(),
            &x264(),
            &out,
            &tmp.path().join(name),
            &extras,
        )
        .unwrap();
        do_render(&t, &c, 10.0).await.unwrap();
        assert_vertical(&ffprobe(&t, &out), 10.0);
        keep(&out, &format!("music_{name}.mp4"));
        outs.push(out);
    }
    let (duck_speech, flat_speech) = (
        low_band_db(&t, &outs[0], 1.0, 4.5),
        low_band_db(&t, &outs[1], 1.0, 4.5),
    );
    let (duck_quiet, flat_quiet) = (
        low_band_db(&t, &outs[0], 7.5, 8.9),
        low_band_db(&t, &outs[1], 7.5, 8.9),
    );
    eprintln!(
        "music low band dB: speech duck {duck_speech} flat {flat_speech}; quiet duck {duck_quiet} flat {flat_quiet}"
    );
    assert!(
        flat_speech - duck_speech > 3.0,
        "music not ducked under speech"
    );
    assert!(
        (flat_quiet - duck_quiet).abs() < 1.5,
        "music should recover after speech"
    );
    // Music present where the source is silent.
    assert!(flat_quiet > -45.0, "music missing: {flat_quiet}");
}

#[tokio::test]
async fn render_crop_path_pans() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    // Left half red, right half blue.
    let src = tmp.path().join("rb.mp4");
    ff(
        &t,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=960x1080:r=30:d=10",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=960x1080:r=30:d=10",
            "-f",
            "lavfi",
            "-i",
            "sine=f=300:r=48000:d=10",
            "-filter_complex",
            "[0:v][1:v]hstack=inputs=2,format=yuv420p[v]",
            "-map",
            "[v]",
            "-map",
            "2:a",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            &src.to_string_lossy(),
        ],
    );
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 10.0, center_crop());
    let mut extras = CompileExtras::default();
    extras.crop_paths.insert(
        clip.segments[0].id.clone(),
        vec![
            CropKey { t: 0.0, x: 0.0 },
            CropKey { t: 4.0, x: 0.0 },
            CropKey { t: 6.0, x: 1.0 },
        ],
    );
    let out = tmp.path().join("pan.mp4");
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &x264(),
        &out,
        &tmp.path().join("w"),
        &extras,
    )
    .unwrap();
    do_render(&t, &c, 10.0).await.unwrap();
    assert_vertical(&ffprobe(&t, &out), 10.0);
    let mut seen = Vec::new();
    for (at, name) in [
        (2.0, "pan_2s.jpg"),
        (4.5, "pan_4.5s.jpg"),
        (8.0, "pan_8s.jpg"),
    ] {
        let jpg = tmp.path().join(name);
        frame_at(&t, &out, at, &jpg);
        keep(&jpg, name);
        seen.push(mean_rgb(&jpg));
    }
    eprintln!("mean rgb at 2s/4.5s/8s: {seen:?}");
    assert!(
        seen[0][0] > 150.0 && seen[0][2] < 60.0,
        "t=2 should be red: {:?}",
        seen[0]
    );
    assert!(
        seen[2][2] > 150.0 && seen[2][0] < 60.0,
        "t=8 should be blue: {:?}",
        seen[2]
    );
    // t=4.5: x = 0.25 * 1920 = 480, so the 606 px window holds 480 px red + 126 px blue.
    assert!(
        seen[1][0] > 150.0 && seen[1][2] > 30.0,
        "t=4.5 should straddle the seam: {:?}",
        seen[1]
    );
}

#[tokio::test]
async fn cancelled_render_removes_output() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 12, true);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 12.0, blur_fit());
    let out = tmp.path().join("cancel.mp4");
    let slow = encoders::choice("libx264", RenderQuality::Final);
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &slow,
        &out,
        &tmp.path().join("w"),
        &CompileExtras::default(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        c2.cancel();
    });
    let e = render(&t, &c, 12.0, &cancel, &ProgressSink::noop())
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Cancelled);
    assert!(!out.exists(), "partial output left behind");
}

// ---------------------------------------------------------------------------------------------
// Two-pass loudnorm
// ---------------------------------------------------------------------------------------------

/// (integrated LUFS, true peak dBTP) of a rendered file, via ebur128.
fn loudness(t: &ToolPaths, p: &Path) -> (f64, f64) {
    let o = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(p)
        .args([
            "-vn",
            "-af",
            "ebur128=peak=true:framelog=quiet",
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stderr);
    let val = |key: &str| -> f64 {
        s.lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix(key))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("{key} missing in {s}"))
    };
    (val("I:"), val("Peak:"))
}

#[tokio::test]
async fn render_two_pass_loudness_hits_target() {
    use rustler_media::render::{LoudnessMode, render_with_report};
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let jfk = real_home().join("test-media").join("jfk.wav");
    if !jfk.is_file() {
        eprintln!("SKIP: {} missing", jfk.display());
        return;
    }
    // 10 s: speech for 5 s, then silence; plus a ducked music bed (the case that landed at
    // about -15.9 LUFS with single-pass loudnorm).
    let src = tmp.path().join("speech.mp4");
    ff(
        &t,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=30:d=10",
            "-i",
            &jfk.to_string_lossy(),
            "-filter_complex",
            "[1:a]atrim=0:5,aresample=48000,apad[a]",
            "-map",
            "0:v",
            "-map",
            "[a]",
            "-t",
            "10",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            &src.to_string_lossy(),
        ],
    );
    let music = tmp.path().join("bed.wav");
    ff(
        &t,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.4*sin(2*PI*55*t)+0.2*sin(2*PI*220*t):s=48000:d=3",
            "-c:a",
            "pcm_s16le",
            &music.to_string_lossy(),
        ],
    );
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 10.0, center_crop());
    let target = clip.audio.target_lufs;

    let mut results = Vec::new();
    for (name, two_pass) in [("single", false), ("two_pass", true)] {
        let extras = CompileExtras {
            music: Some(MusicBed::new(&music)),
            two_pass_loudness: two_pass,
            ..Default::default()
        };
        let out = tmp.path().join(format!("{name}.mp4"));
        let c = compile_with(
            &clip,
            &sources,
            None,
            tmp.path(),
            &x264(),
            &out,
            &tmp.path().join(name),
            &extras,
        )
        .unwrap();
        let rep = render_with_report(
            &t,
            &c,
            10.0,
            &CancellationToken::new(),
            &ProgressSink::noop(),
        )
        .await
        .unwrap();
        assert_vertical(&ffprobe(&t, &out), 10.0);
        let (i, tp) = loudness(&t, &out);
        eprintln!(
            "{name}: integrated {i} LUFS, true peak {tp} dBTP, mode {:?}, measured {:?}",
            rep.loudness_mode, rep.measured
        );
        keep(&out, &format!("loudness_{name}.mp4"));
        results.push((rep, i, tp));
    }
    let (single, two) = (&results[0], &results[1]);
    assert_eq!(single.0.loudness_mode, LoudnessMode::SinglePass);
    assert_eq!(two.0.loudness_mode, LoudnessMode::TwoPass, "{:?}", two.0.warnings);
    let m = two.0.measured.expect("measured values");
    assert!(m.input_i < -5.0 && m.input_i > -40.0, "{m:?}");
    assert!(
        (two.1 - target).abs() <= 1.0,
        "two-pass integrated {} LUFS is not within 1 LU of {target}",
        two.1
    );
    assert!(
        (two.1 - target).abs() < (single.1 - target).abs() + 0.05,
        "two-pass ({}) should be at least as close as single-pass ({})",
        two.1,
        single.1
    );
    // The final graph on disk carries the measured filter.
    let g = std::fs::read_to_string(tmp.path().join("two_pass").join("graph.txt")).unwrap();
    assert!(g.contains(":measured_I=") && g.contains(":linear=true"), "{g}");
}

#[tokio::test]
async fn two_pass_falls_back_on_silent_audio() {
    use rustler_media::render::{LoudnessMode, render_with_report};
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    // No audio stream at all: anullsrc -> digital silence -> loudnorm prints -inf.
    let src = tmp.path().join("silent.mp4");
    make_source(&t, &src, 3, false);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 2.0, center_crop());
    let out = tmp.path().join("out.mp4");
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &x264(),
        &out,
        &tmp.path().join("w"),
        &CompileExtras::default(),
    )
    .unwrap();
    assert!(c.loudness.is_some());
    let rep = render_with_report(&t, &c, 2.0, &CancellationToken::new(), &ProgressSink::noop())
        .await
        .unwrap();
    eprintln!("silent: {rep:?}");
    assert_eq!(rep.loudness_mode, LoudnessMode::SinglePass);
    assert!(!rep.warnings.is_empty());
    assert_vertical(&ffprobe(&t, &out), 2.0);
}

// ---------------------------------------------------------------------------------------------
// Hook title overlay
// ---------------------------------------------------------------------------------------------

/// Flat fonts dir with Poppins ExtraBold + Anton from `~/.rustler/fonts/**`, or None.
fn rustler_fonts(tmp: &Path) -> Option<PathBuf> {
    let d = tmp.join("fonts flat");
    std::fs::create_dir_all(&d).unwrap();
    let root = real_home().join("fonts");
    let mut n = 0;
    for (sub, file) in [
        ("Poppins", "Poppins-ExtraBold.ttf"),
        ("Anton", "Anton-Regular.ttf"),
    ] {
        let p = root.join(sub).join(file);
        if p.is_file() {
            std::fs::copy(&p, d.join(file)).unwrap();
            n += 1;
        }
    }
    (n == 2).then_some(d)
}

/// Bottom captions like the bold-karaoke preset at position_y 0.70 (MarginV 576, Anton 78).
const CAPTIONS_ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Anton,78,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,6,2,2,40,40,576,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:00:06.00,Default,,0,0,0,,AND SO MY FELLOW AMERICANS\n";

/// Share of pixels in rows [y0, y1) x columns [x0, x1) that are near-black / near-white.
fn band_stats(img: &Path, x0: u32, x1: u32, y0: u32, y1: u32) -> (f64, f64) {
    let im = image::open(img).unwrap().to_rgb8();
    let (mut dark, mut white, mut n) = (0u32, 0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = im.get_pixel(x, y);
            let (r, g, b) = (p[0] as u32, p[1] as u32, p[2] as u32);
            if r < 45 && g < 45 && b < 45 {
                dark += 1;
            }
            if r > 215 && g > 215 && b > 215 {
                white += 1;
            }
            n += 1;
        }
    }
    (dark as f64 / n as f64, white as f64 / n as f64)
}

#[tokio::test]
async fn render_hook_title_above_captions() {
    use rustler_media::compile::{OverlayStyle, TextOverlay, overlay_layout};
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let Some(fonts) = rustler_fonts(tmp.path()) else {
        eprintln!("SKIP: Poppins/Anton fonts missing in ~/.rustler/fonts (run `rustler setup`)");
        return;
    };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src, 7, true);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, 0.0, 6.0, center_crop());
    let ass = tmp.path().join("caps.ass");
    std::fs::write(&ass, CAPTIONS_ASS).unwrap();
    let hook = TextOverlay::new(
        "ovl_1",
        "He quit his job to build this in 30 days",
        0.0,
        3.0,
        OverlayStyle::HookTop,
    );
    let layout = overlay_layout(&hook, &clip.canvas);
    eprintln!("hook layout: {layout:?}");
    let extras = CompileExtras {
        overlays: vec![hook],
        ..Default::default()
    };
    let out = tmp.path().join("hook.mp4");
    let c = compile_with(
        &clip,
        &sources,
        Some(&ass),
        &fonts,
        &x264(),
        &out,
        &tmp.path().join("w"),
        &extras,
    )
    .unwrap();
    // Preview frame (same graph) while the hook is on screen.
    let prev = tmp.path().join("hook_preview.jpg");
    preview_frame(&t, &c, 1.0, &prev).await.unwrap();
    keep(&prev, "hook_preview.jpg");
    do_render(&t, &c, 6.0).await.unwrap();
    assert_vertical(&ffprobe(&t, &out), 6.0);
    let on = tmp.path().join("hook_on.jpg");
    let off = tmp.path().join("hook_off.jpg");
    frame_at(&t, &out, 1.0, &on);
    frame_at(&t, &out, 4.5, &off);
    keep(&on, "hook_on.jpg");
    keep(&off, "hook_off.jpg");

    // The box band (dark box + white text) is there while the hook is on, gone after.
    let (y0, y1) = (layout.top as u32 + 4, layout.bottom as u32 - 4);
    let (x0, x1) = (300, 780);
    for img in [&prev, &on] {
        let (dark, white) = band_stats(img, x0, x1, y0, y1);
        eprintln!("{}: hook band dark {dark:.2} white {white:.2}", img.display());
        assert!(dark > 0.35, "no dark box at the top: dark {dark:.2}");
        assert!(white > 0.05, "no white text in the box: white {white:.2}");
    }
    let (dark_off, _) = band_stats(&off, x0, x1, y0, y1);
    assert!(dark_off < 0.2, "hook still visible after its end: {dark_off:.2}");
    // Captions (bottom at 0.70 * 1920 = 1344) still render, well below the hook.
    let (_, cap_white) = band_stats(&on, 100, 980, 1344 - 90, 1344);
    assert!(cap_white > 0.03, "captions missing: {cap_white:.2}");
    assert!(layout.bottom < 1344.0 - 200.0, "{layout:?}");
}
