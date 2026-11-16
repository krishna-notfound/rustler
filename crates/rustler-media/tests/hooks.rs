//! Viral hook styles (hook-big, hook-box-yellow, hook-outline, hook-pov, hook-question), their
//! entry animations, and the flash / whip transitions: ASS output (pure) and real renders (skip
//! with a printed reason when ffmpeg or the fonts are missing).
//! Set `RUSTLER_TEST_OUT=<dir>` to keep extracted frames for a visual check.

use rustler_core::{
    AudioSettings, Background, CaptionTrack, Clip, ClipId, Fit, Layout, MediaProbe, Rect, Region, RenderQuality, Segment,
    SegmentId, SourceId,
};
use rustler_media::compile::{
    CompileExtras, CompiledRender, OverlayAnim, OverlayStyle, SourceMedia, TextOverlay, Transition, TransitionKind, compile_with,
    display_text, effective_duration, overlay_layout, overlays_ass,
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

fn canvas() -> rustler_core::Canvas {
    rustler_core::Canvas { width: 1080, height: 1920, fps: 30.0 }
}

fn ov(text: &str, style: OverlayStyle) -> TextOverlay {
    TextOverlay::new("ovl_1", text, 0.0, 3.0, style)
}

fn event(ass: &str) -> String {
    ass.lines().find(|l| l.starts_with("Dialogue:")).unwrap().to_string()
}

// ---------------------------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------------------------

#[test]
fn there_are_five_hook_presets_with_distinct_names_and_animations() {
    assert_eq!(OverlayStyle::HOOKS.len(), 5);
    let names: Vec<&str> = OverlayStyle::HOOKS.iter().map(|s| s.name()).collect();
    assert_eq!(names, ["hook-big", "hook-box-yellow", "hook-outline", "hook-pov", "hook-question"]);
    for s in OverlayStyle::HOOKS {
        assert!(s.is_hook_preset());
        assert_ne!(s.default_anim(), OverlayAnim::None, "{} animates by default", s.name());
        assert!(!s.description().is_empty());
        // the name is what serde and --style use
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.name()));
        assert_eq!(serde_json::from_value::<OverlayStyle>(json!(s.name())).unwrap(), s);
    }
    let anims: std::collections::HashSet<_> = OverlayStyle::HOOKS.iter().map(|s| s.default_anim()).collect();
    assert!(anims.len() >= 4, "pop / zoom-in / slide / typewriter are all used: {anims:?}");
    assert!(!OverlayStyle::HookTop.is_hook_preset() && OverlayStyle::HookTop.default_anim() == OverlayAnim::None);
    assert!((OverlayStyle::HookBig.default_font_size_factor() - 0.10).abs() < 1e-9);
}

#[test]
fn old_overlays_keep_their_exact_ass_event() {
    let o = ov("He quit his job", OverlayStyle::HookTop);
    let ass = overlays_ass(&[o], &canvas(), 10.0).unwrap();
    let e = event(&ass);
    assert!(e.ends_with("{\\an5\\pos(540,230)\\fad(0,150)}He quit his job"), "{e}");
    let json = serde_json::to_value(ov("x", OverlayStyle::HookTop)).unwrap();
    assert!(json.get("anim").is_none() && json.get("sfx_id").is_none(), "old files stay byte-compatible: {json}");
    let back: TextOverlay = serde_json::from_value(json!({"id":"a","text":"t","start":0.0,"end":1.0})).unwrap();
    assert_eq!((back.anim, back.sfx_id.clone()), (None, None));
}

#[test]
fn display_text_shapes_pov_question_and_big() {
    assert_eq!(display_text(&ov("quit my job", OverlayStyle::HookBig)), "QUIT MY JOB");
    assert_eq!(display_text(&ov("you quit your job", OverlayStyle::HookPov)), "POV: you quit your job");
    assert_eq!(display_text(&ov("pov: you quit", OverlayStyle::HookPov)), "POV: you quit");
    assert_eq!(display_text(&ov("POV you quit", OverlayStyle::HookPov)), "POV you quit");
    assert_eq!(display_text(&ov("Would you quit", OverlayStyle::HookQuestion)), "Would you quit?");
    assert_eq!(display_text(&ov("Would you quit?", OverlayStyle::HookQuestion)), "Would you quit?");
    assert_eq!(display_text(&ov("same", OverlayStyle::HookTop)), "same");
}

#[test]
fn hook_big_is_two_uppercase_lines_at_ten_percent_with_a_pop() {
    let o = ov("he quit his job to build this in thirty days", OverlayStyle::HookBig);
    let l = overlay_layout(&o, &canvas());
    assert_eq!(l.lines.len(), 2, "{l:?}");
    assert!(l.lines.iter().all(|s| s.chars().all(|c| !c.is_lowercase())));
    assert!(l.font_size >= 90 && l.font_size <= 108, "about 0.10 of the width: {}", l.font_size);
    assert_eq!(l.padding, 0, "no box");
    let short = overlay_layout(&ov("he quit his job", OverlayStyle::HookBig), &canvas());
    assert_eq!(short.lines, ["HE QUIT", "HIS JOB"], "a short hook still breaks into two lines");
    assert_eq!(overlay_layout(&ov("wow", OverlayStyle::HookBig), &canvas()).lines.len(), 1);
    let ass = overlays_ass(&[o], &canvas(), 10.0).unwrap();
    let e = event(&ass);
    assert!(e.contains("\\fscx70\\fscy70\\t(0,140,\\fscx112\\fscy112)\\t(140,260,\\fscx100\\fscy100)"), "{e}");
    assert!(e.contains("\\N"), "{e}");
    assert!(ass.contains("Style: ov0,Poppins ExtraBold,") && ass.contains(",1,13,3,5,0,0,0,1"), "thick outline, no box: {ass}");
}

#[test]
fn hook_box_yellow_is_black_on_yellow_and_slides_from_above() {
    let ass = overlays_ass(&[ov("nobody tells you this", OverlayStyle::HookBoxYellow)], &canvas(), 10.0).unwrap();
    let style = ass.lines().find(|l| l.starts_with("Style: ov0")).unwrap();
    // text colour dark, box (outline) colour #FFE000 = &H0000E0FF, BorderStyle 3
    assert!(style.contains(",&H00141414,&H00141414,&H0000E0FF,"), "{style}");
    assert!(style.contains(",3,"), "{style}");
    let e = event(&ass);
    assert!(e.contains("\\move(540,"), "{e}");
    assert!(e.contains(",0,260)"), "{e}");
}

#[test]
fn hook_outline_types_out_letter_by_letter() {
    let ass = overlays_ass(&[ov("stop scrolling", OverlayStyle::HookOutline)], &canvas(), 10.0).unwrap();
    let e = event(&ass);
    // 13 letters, one block each, spaces plain; the first appears at once, later ones later
    assert_eq!(e.matches("\\1a&HFF&").count(), 13, "{e}");
    assert!(e.contains("{\\1a&HFF&\\3a&HFF&\\4a&HFF&\\t(0,1,"), "{e}");
    assert!(e.contains("\\t(45,46,"), "45 ms per letter at most: {e}");
    let style = ass.lines().find(|l| l.starts_with("Style: ov0")).unwrap();
    assert!(style.contains(",&H0000E0FF,&H0000E0FF,&H00000000,"), "yellow text, black outline: {style}");
}

#[test]
fn hook_pov_is_left_aligned_with_a_yellow_pov_accent_and_slides_from_the_left() {
    let ass = overlays_ass(&[ov("you quit your job", OverlayStyle::HookPov)], &canvas(), 10.0).unwrap();
    let e = event(&ass);
    assert!(e.contains("\\an4\\move("), "{e}");
    assert!(e.contains("{\\1c&H0000E0FF&}POV:{\\1c&H00FFFFFF&\\fscx100\\fscy100} you"), "{e}");
    // starts off the left edge
    let from: i32 = e.split("\\move(").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
    assert!(from < 0, "{e}");
}

#[test]
fn hook_question_ends_in_a_big_red_question_mark_and_zooms_in() {
    let ass = overlays_ass(&[ov("would you quit", OverlayStyle::HookQuestion)], &canvas(), 10.0).unwrap();
    let e = event(&ass);
    assert!(e.contains("{\\1c&H004D4DFF&}?"), "boxed: red, same size (a bigger glyph would stretch the box): {e}");
    assert!(e.contains("\\fscx140\\fscy140\\t(0,300,\\fscx100\\fscy100)"), "{e}");
    let mut plain = ov("would you quit", OverlayStyle::HookQuestion);
    plain.boxed = false;
    let e = event(&overlays_ass(&[plain], &canvas(), 10.0).unwrap());
    assert!(e.contains("{\\1c&H004D4DFF&\\fscx135\\fscy135}?"), "unboxed: red and bigger: {e}");
}

#[test]
fn anim_none_turns_a_hook_preset_static_and_any_style_can_animate() {
    let mut o = ov("stay put", OverlayStyle::HookBig);
    o.anim = Some(OverlayAnim::None);
    assert!(!event(&overlays_ass(&[o], &canvas(), 10.0).unwrap()).contains("\\t("));
    let mut p = ov("typed plain", OverlayStyle::Plain);
    p.anim = Some(OverlayAnim::Typewriter);
    assert!(event(&overlays_ass(&[p], &canvas(), 10.0).unwrap()).contains("\\1a&HFF&"));
}

#[test]
fn flash_and_whip_map_to_xfade_with_short_defaults() {
    assert_eq!(TransitionKind::Flash.xfade_name(), Some("fadewhite"));
    assert_eq!(TransitionKind::Flashblack.xfade_name(), Some("fadeblack"));
    assert_eq!(TransitionKind::Whip.xfade_name(), Some("hblur"));
    assert_eq!(TransitionKind::Flash.default_duration(), 0.1);
    assert_eq!(TransitionKind::Whip.default_duration(), 0.2);
    assert_eq!(TransitionKind::Dissolve.default_duration(), 0.3);
    assert_eq!(serde_json::from_value::<TransitionKind>(json!("flash")).unwrap(), TransitionKind::Flash);
    assert_eq!(serde_json::from_value::<TransitionKind>(json!("whip")).unwrap(), TransitionKind::Whip);
}

// ---------------------------------------------------------------------------------------------
// Real renders
// ---------------------------------------------------------------------------------------------

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

fn fonts(tmp: &Path) -> Option<PathBuf> {
    let d = tmp.join("fonts flat");
    std::fs::create_dir_all(&d).unwrap();
    let p = real_home().join("fonts").join("Poppins").join("Poppins-ExtraBold.ttf");
    if !p.is_file() {
        eprintln!("SKIP: Poppins ExtraBold missing in ~/.rustler/fonts (run `rustler setup`)");
        return None;
    }
    std::fs::copy(&p, d.join("Poppins-ExtraBold.ttf")).unwrap();
    Some(d)
}

fn ff(t: &ToolPaths, args: &[&str]) {
    let o = Command::new(&t.ffmpeg).args(["-hide_banner", "-v", "error", "-y"]).args(args).output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn media_probe(t: &ToolPaths, p: &Path) -> MediaProbe {
    let o = Command::new(&t.ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"])
        .arg(p)
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
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

/// 8 s landscape source: a flat blue-grey half and a test pattern half, with a sine.
fn make_source(t: &ToolPaths, out: &Path) {
    ff(
        t,
        &[
            "-f", "lavfi", "-i", "color=c=0x4a6a8a:s=960x1080:r=30:d=8",
            "-f", "lavfi", "-i", "testsrc2=s=960x1080:r=30:d=8",
            "-f", "lavfi", "-i", "sine=f=440:r=48000:d=8",
            "-filter_complex", "[0:v][1:v]hstack=inputs=2,format=yuv420p[v]",
            "-map", "[v]", "-map", "2:a", "-c:a", "aac", "-c:v", "libx264", "-preset", "ultrafast", "-g", "30",
            &out.to_string_lossy(),
        ],
    );
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

fn make_clip(src: &SourceId, ranges: &[(f64, f64)]) -> Clip {
    let segs: Vec<Segment> = ranges
        .iter()
        .map(|&(s, e)| Segment { id: SegmentId::new(), source_id: src.clone(), start: s, end: e, layout: center_crop() })
        .collect();
    serde_json::from_value(json!({
        "schema": "rustler.clip@1", "id": ClipId::new(), "name": "hooks", "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": segs, "captions": CaptionTrack::default(), "audio": AudioSettings::default(),
        "renders": [], "created_at": "2026-11-16T00:00:00Z"
    }))
    .unwrap()
}

fn keep(p: &Path, name: &str) {
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(p, dir.join(name));
    }
}

fn frame_n(t: &ToolPaths, video: &Path, n: u64, out: &Path) -> image::RgbImage {
    ff(
        t,
        &[
            "-i", &video.to_string_lossy(), "-vf", &format!("select=eq(n\\,{n})"), "-fps_mode", "passthrough",
            "-frames:v", "1", "-update", "1", &out.to_string_lossy(),
        ],
    );
    image::open(out).unwrap().to_rgb8()
}

async fn render_clip(
    t: &ToolPaths,
    tmp: &Path,
    name: &str,
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    fonts: &Path,
    x: &CompileExtras,
) -> PathBuf {
    let out = tmp.join(format!("{name}.mp4"));
    let c: CompiledRender = compile_with(
        clip,
        sources,
        None,
        fonts,
        &encoders::choice("libx264", RenderQuality::Draft),
        &out,
        &tmp.join(format!("work_{name}")),
        x,
    )
    .unwrap();
    render(t, &c, effective_duration(clip, x), &CancellationToken::new(), &ProgressSink::noop()).await.unwrap();
    out
}

/// Share of pixels in the box that are close to `rgb` (per channel tolerance 40).
fn share_near(im: &image::RgbImage, rect: (u32, u32, u32, u32), rgb: [u8; 3]) -> f64 {
    let (x0, y0, x1, y1) = rect;
    let (mut hit, mut n) = (0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = im.get_pixel(x, y).0;
            if p.iter().zip(rgb).all(|(a, b)| (*a as i32 - b as i32).abs() < 40) {
                hit += 1;
            }
            n += 1;
        }
    }
    hit as f64 / n as f64
}

#[tokio::test]
async fn render_every_hook_style_and_animation_settles() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let Some(fonts) = fonts(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src);
    let id = SourceId::new();
    let mut sources = HashMap::new();
    sources.insert(id.clone(), SourceMedia { id: id.clone(), path: src.clone(), probe: media_probe(&t, &src) });
    let clip = make_clip(&id, &[(0.0, 4.0)]);
    let cases = [
        ("hook-big", OverlayStyle::HookBig, "he quit his job to build this in thirty days"),
        ("hook-box-yellow", OverlayStyle::HookBoxYellow, "Nobody tells you this"),
        ("hook-outline", OverlayStyle::HookOutline, "Stop scrolling right now"),
        ("hook-pov", OverlayStyle::HookPov, "you finally quit your job"),
        ("hook-question", OverlayStyle::HookQuestion, "Would you quit tomorrow"),
    ];
    for (name, style, text) in cases {
        let o = TextOverlay::new("ovl_1", text, 0.0, 3.0, style);
        let l = overlay_layout(&o, &clip.canvas);
        let x = CompileExtras { overlays: vec![o], ..Default::default() };
        let out = render_clip(&t, tmp.path(), name, &clip, &sources, &fonts, &x).await;
        // settled (1.5 s = frame 45) and gone (3.6 s = frame 108)
        let on = frame_n(&t, &out, 45, &tmp.path().join(format!("{name}_on.png")));
        let off = frame_n(&t, &out, 108, &tmp.path().join(format!("{name}_off.png")));
        keep(&tmp.path().join(format!("{name}_on.png")), &format!("{name}_on.png"));
        let rect = (60, l.top as u32, 1020, l.bottom as u32);
        let diff: u64 = (rect.1..rect.3)
            .step_by(4)
            .flat_map(|y| (rect.0..rect.2).step_by(4).map(move |x| (x, y)))
            .map(|(x, y)| {
                on.get_pixel(x, y).0.iter().zip(off.get_pixel(x, y).0).map(|(a, b)| (*a as i64 - b as i64).unsigned_abs()).sum::<u64>()
            })
            .sum();
        assert!(diff > 100_000, "{name}: the hook changes the frame while it is on (diff {diff})");
        match style {
            OverlayStyle::HookBoxYellow => {
                assert!(share_near(&on, rect, [255, 224, 0]) > 0.25, "{name}: yellow box");
            }
            OverlayStyle::HookQuestion => {
                assert!(share_near(&on, rect, [255, 77, 77]) > 0.002, "{name}: red question mark");
            }
            OverlayStyle::HookOutline => {
                assert!(share_near(&on, rect, [255, 224, 0]) > 0.02, "{name}: yellow typed text");
            }
            _ => {}
        }
        assert!(l.fits, "{name} fits in two lines: {l:?}");
    }
}

#[tokio::test]
async fn render_flash_and_whip_transitions_at_a_cut() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("src.mp4");
    make_source(&t, &src);
    let id = SourceId::new();
    let mut sources = HashMap::new();
    sources.insert(id.clone(), SourceMedia { id: id.clone(), path: src.clone(), probe: media_probe(&t, &src) });
    let clip = make_clip(&id, &[(0.0, 2.0), (4.0, 6.0)]);
    for (name, kind, white) in [("flash", TransitionKind::Flash, true), ("flashblack", TransitionKind::Flashblack, false), ("whip", TransitionKind::Whip, false)] {
        let x = CompileExtras {
            transitions: vec![Transition { after_segment_id: clip.segments[0].id.clone(), kind, duration: kind.default_duration() }],
            ..Default::default()
        };
        let out = render_clip(&t, tmp.path(), name, &clip, &sources, tmp.path(), &x).await;
        // 4 s of picture minus the overlap (3 frames for flash, 6 for whip)
        let (frames, secs) = (if matches!(kind, TransitionKind::Whip) { 114 } else { 117 }, 4.0 - kind.default_duration());
        let probe = media_probe(&t, &out);
        assert!((probe.duration - secs).abs() < 0.08, "{name}: duration {} vs {secs}", probe.duration);
        let mid = frames / 2 + 1; // the cut sits at 2 s minus half the overlap, frame ~58/57
        let mut best = (0.0f64, 0u64);
        for n in (mid - 5)..(mid + 5) {
            let im = frame_n(&t, &out, n, &tmp.path().join(format!("{name}_{n}.png")));
            let mean = im.pixels().map(|p| (p.0[0] as f64 + p.0[1] as f64 + p.0[2] as f64) / 3.0).sum::<f64>() / (im.width() * im.height()) as f64;
            let score = if white { mean } else { 255.0 - mean };
            if score > best.0 {
                best = (score, n);
            }
            if n == mid {
                keep(&tmp.path().join(format!("{name}_{n}.png")), &format!("{name}_cut.png"));
            }
        }
        match kind {
            TransitionKind::Flash => assert!(best.0 > 235.0, "flash reaches white at the cut: {best:?}"),
            TransitionKind::Flashblack => assert!(best.0 > 235.0, "flashblack reaches black at the cut: {best:?}"),
            _ => {}
        }
        eprintln!("{name}: peak {best:?}");
    }
}
