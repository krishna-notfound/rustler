//! v2 layers in the compiler: images, masks, cutaways. Graph tests (pure) and real renders that
//! read pixels back. Real renders skip (with a printed reason) when ffmpeg is missing;
//! `RUSTLER_TEST_OUT=<dir>` keeps the frames for a visual check.

use rustler_core::{
    AudioSettings, Background, CaptionTrack, Clip, ClipId, Fit, Layout, MediaProbe, Rect, Region, RenderQuality,
    Segment, SegmentId, SourceId, TimeMap, Word,
};
use rustler_media::compile::{
    CompileExtras, CompiledRender, Effect, EffectKind, SourceMedia, TextOverlay, OverlayStyle, Transition,
    TransitionKind, build_graph, compile_with, effective_duration,
};
use rustler_media::encoders;
use rustler_media::layers::{
    Anchor, Anim, AnimKind, Cutaway, CutawayMedia, CutawayMode, ImageLayer, MaskKind, MaskLayer, Timing,
    resolve_timing,
};
use rustler_media::render::render;
use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

fn full_frame() -> Layout {
    Layout {
        preset: Some("full".into()),
        regions: vec![Region {
            id: "main".into(),
            src: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
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
        .map(|&(s, e)| Segment { id: SegmentId::new(), source_id: src.clone(), start: s, end: e, layout: full_frame() })
        .collect();
    let audio = AudioSettings { loudnorm: false, ..AudioSettings::default() };
    serde_json::from_value(json!({
        "schema": "rustler.clip@1", "id": ClipId::new(), "name": "layers", "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": segs, "captions": CaptionTrack::default(), "audio": audio,
        "renders": [], "created_at": "2026-11-16T00:00:00Z"
    }))
    .unwrap()
}

fn fake_probe(dur: f64) -> MediaProbe {
    MediaProbe {
        duration: dur,
        has_video: true,
        has_audio: true,
        width: 1080,
        height: 1920,
        rotation: 0,
        fps: 30.0,
        video_codec: Some("h264".into()),
        audio_codec: Some("aac".into()),
        sample_rate: Some(48000),
        channels: Some(2),
    }
}

fn fake_sources(dur: f64) -> (SourceId, HashMap<SourceId, SourceMedia>) {
    let id = SourceId::new();
    let mut m = HashMap::new();
    m.insert(id.clone(), SourceMedia { id: id.clone(), path: PathBuf::from("/src.mp4"), probe: fake_probe(dur) });
    (id, m)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

fn image(id: &str, file: &str, r: Rect, s: f64, e: f64, z: i32) -> ImageLayer {
    ImageLayer {
        id: id.into(),
        timing: Timing::out(s, e),
        asset: None,
        file: PathBuf::from(file),
        rect: r,
        scale: 1.0,
        opacity: 1.0,
        rotation: 0.0,
        fit: Fit::Contain,
        anim: Anim::default(),
        z,
        above_captions: false,
    }
}

fn mask(id: &str, kind: MaskKind, r: Rect, s: f64, e: f64) -> MaskLayer {
    MaskLayer { id: id.into(), timing: Timing::out(s, e), rect: r, kind, strength: None, color: None }
}

fn cutaway(id: &str, file: &str, mode: CutawayMode, r: Option<Rect>, s: f64, e: f64) -> Cutaway {
    Cutaway {
        id: id.into(),
        timing: Timing::out(s, e),
        media: CutawayMedia { source_id: None, asset: None, file: Some(PathBuf::from(file)), in_: 0.0 },
        mode,
        rect: r,
        fit: Fit::Cover,
        mute: true,
    }
}

fn pos(g: &str, needle: &str) -> usize {
    g.find(needle).unwrap_or_else(|| panic!("`{needle}` not in graph:\n{g}"))
}

// ---------------------------------------------------------------------------------------------
// Graph tests (pure)
// ---------------------------------------------------------------------------------------------

#[test]
fn mask_graph_box_blur_pixelate() {
    let (id, sources) = fake_sources(10.0);
    let clip = make_clip(&id, &[(0.0, 6.0)]);
    let mut b = mask("msk_1", MaskKind::Box, rect(0.8, 0.0, 0.2, 0.05), 1.0, 2.0);
    b.color = Some("#FF0000".into());
    let x = CompileExtras {
        masks: vec![
            b,
            mask("msk_2", MaskKind::Blur, rect(0.75, 0.02, 0.25, 0.1), 0.0, 6.0),
            MaskLayer { strength: Some(30.0), ..mask("msk_3", MaskKind::Pixelate, rect(0.0, 0.8, 0.5, 0.1), 2.0, 4.0) },
        ],
        ..Default::default()
    };
    let g = build_graph(&clip, &sources, false, &x).unwrap().graph;
    // Box: drawbox on the chain, frames 30..59, even pixel box.
    assert!(g.contains("drawbox=x=864:y=0:w=216:h=96:color=0xFF0000@1:t=fill:enable='between(n,30,59)'"), "{g}");
    // Blur: split, crop+boxblur of the copy, laid back at the same place.
    assert!(g.contains("split=2[mk1a][mk1b]"), "{g}");
    assert!(g.contains("[mk1b]crop=270:192:810:38,boxblur=luma_radius=20:luma_power=3:chroma_radius=10:chroma_power=3[mk1c]"), "{g}");
    assert!(g.contains("[mk1a][mk1c]overlay=x=810:y=38:enable='between(n,0,179)'"), "{g}");
    // Pixelate: 30 px blocks (540/30 x 192/30 -> 18x6), nearest-neighbour back up.
    assert!(g.contains("crop=540:192:0:1536,scale=18:6:flags=area,scale=540:192:flags=neighbor[mk2c]"), "{g}");
    assert!(g.contains("enable='between(n,60,119)'"), "{g}");
}

#[test]
fn image_graph_inputs_scale_opacity_fade_and_gif() {
    let (id, sources) = fake_sources(10.0);
    let clip = make_clip(&id, &[(0.0, 6.0)]);
    let mut logo = image("img_1", "/logo.png", rect(0.05, 0.05, 0.2, 0.1), 1.0, 4.0, 0);
    logo.opacity = 0.8;
    logo.anim = Anim { in_: AnimKind::Fade, out: AnimKind::SlideUp, duration: 0.5 };
    let mut gif = image("img_2", "/party.gif", rect(0.6, 0.4, 0.3, 0.2), 0.0, 2.0, 1);
    gif.anim = Anim { in_: AnimKind::Pop, out: AnimKind::None, duration: 0.3 };
    let x = CompileExtras { images: vec![logo, gif], ..Default::default() };
    let plan = build_graph(&clip, &sources, false, &x).unwrap();
    let g = &plan.graph;
    // Inputs: the png loops at the canvas rate for its 3 s (+0.1), the gif loops itself.
    assert_eq!(plan.layer_inputs.len(), 2);
    assert_eq!(plan.layer_inputs[0].opts, ["-loop", "1", "-framerate", "30", "-t", "3.100"]);
    assert_eq!(plan.layer_inputs[0].path, PathBuf::from("/logo.png"));
    assert_eq!(plan.layer_inputs[1].opts, ["-ignore_loop", "0", "-t", "2.100"]);
    // Segment input 0, so the layer inputs are 1 and 2.
    assert!(g.contains("[1:v]fps=30,settb=1/30,setpts=N,format=rgba,scale=w=216:h=192:force_original_aspect_ratio=decrease,colorchannelmixer=aa=0.8,fade=t=in:st=0:d=0.5:alpha=1,fade=t=out:st=2.5:d=0.5:alpha=1,setpts=PTS+30[img0]"), "{g}");
    // Centre of the rect (54+108, 96+96); slide-up out moves it up over the last 0.5 s.
    assert!(g.contains("overlay=x='162-w/2':y='192-h/2-115*max(0,(t-3.5)/0.5)':eof_action=pass:enable='between(n,30,119)'"), "{g}");
    // Pop: per-frame scale is the last filter before the overlay.
    assert!(g.contains(":eval=frame,setpts=PTS+0[img1]"), "{g}");
    assert!(g.contains("[2:v]fps=30"), "{g}");

    // compile_with puts the layer inputs after the segment input.
    let tmp = tempfile::tempdir().unwrap();
    let c = compile_with(
        &clip,
        &sources,
        None,
        tmp.path(),
        &encoders::choice("libx264", RenderQuality::Draft),
        &tmp.path().join("o.mp4"),
        &tmp.path().join("w"),
        &x,
    )
    .unwrap();
    let args: Vec<String> = c.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let i = args.iter().position(|a| a == "-ignore_loop").unwrap();
    assert!(args[i + 5].ends_with("party.gif"), "{args:?}");
    let l = args.iter().position(|a| a == "-loop").unwrap();
    assert!(l < i && args[l + 7].ends_with("logo.png"), "{args:?}");
}

#[test]
fn cutaway_graph_full_pip_and_audio() {
    let (id, sources) = fake_sources(10.0);
    let clip = make_clip(&id, &[(0.0, 6.0)]);
    let mut full = cutaway("cut_1", "/broll.mp4", CutawayMode::Full, None, 1.0, 2.0);
    full.media.in_ = 4.5;
    full.mute = false;
    let mut pip = cutaway("cut_2", "/still.jpg", CutawayMode::Pip, Some(rect(0.5, 0.6, 0.4, 0.25)), 2.0, 5.0);
    pip.fit = Fit::Contain;
    let x = CompileExtras { cutaways: vec![full, pip], ..Default::default() };
    let plan = build_graph(&clip, &sources, false, &x).unwrap();
    let g = &plan.graph;
    assert_eq!(plan.layer_inputs[0].opts, ["-ss", "4.500", "-t", "1.100"]);
    assert_eq!(plan.layer_inputs[1].opts, ["-loop", "1", "-framerate", "30", "-t", "3.100"]);
    assert!(g.contains("[1:v]fps=30,settb=1/30,setpts=N,scale=w=1080:h=1920:force_original_aspect_ratio=increase:force_divisible_by=2,crop=1080:1920,setsar=1,format=yuv420p,tpad=stop_mode=clone:stop_duration=1.1,trim=end_frame=30,setpts=PTS+30[cut0]"), "{g}");
    assert!(g.contains("[cut0]overlay=x=0:y=0:eof_action=pass:enable='between(n,30,59)'"), "{g}");
    // PiP contain: scaled inside the 432x480 box, centred in it.
    assert!(g.contains("scale=w=432:h=480:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1"), "{g}");
    assert!(g.contains("[cut1]overlay=x=540+(432-w)/2:y=1152+(480-h)/2:eof_action=pass:enable='between(n,60,149)'"), "{g}");
    // Unmuted cutaway audio is delayed to 1 s and mixed with the speech (amix of 2).
    assert!(g.contains("[1:a:0]aresample=48000"), "{g}");
    assert!(g.contains("adelay=delays=48000S:all=1"), "{g}");
    assert!(g.contains("amix=inputs=2"), "{g}");
    assert!(!g.contains("[2:a:0]"), "muted pip has no audio chain");
}

#[test]
fn layer_z_order_in_graph() {
    let (id, sources) = fake_sources(10.0);
    let clip = make_clip(&id, &[(0.0, 6.0)]);
    let mut above = image("img_top", "/top.png", rect(0.1, 0.1, 0.3, 0.1), 0.0, 6.0, -5);
    above.above_captions = true;
    let x = CompileExtras {
        // Deliberately out of order in storage.
        images: vec![
            above,
            image("img_hi", "/hi.png", rect(0.1, 0.3, 0.3, 0.1), 0.0, 6.0, 7),
            image("img_lo", "/lo.png", rect(0.1, 0.5, 0.3, 0.1), 0.0, 6.0, -1),
        ],
        masks: vec![mask("msk_1", MaskKind::Box, rect(0.0, 0.0, 0.3, 0.1), 0.0, 6.0)],
        cutaways: vec![
            cutaway("cut_pip", "/p.mp4", CutawayMode::Pip, Some(rect(0.5, 0.5, 0.4, 0.3)), 1.0, 2.0),
            cutaway("cut_full", "/f.mp4", CutawayMode::Full, None, 3.0, 4.0),
        ],
        effects: vec![Effect { id: "fx_1".into(), kind: EffectKind::PunchIn, start: 0.5, end: 1.0, amount: None }],
        overlays: vec![TextOverlay::new("ovl_1", "Hook", 0.0, 2.0, OverlayStyle::HookTop)],
        ..Default::default()
    };
    let g = build_graph(&clip, &sources, true, &x).unwrap().graph;
    let full = pos(&g, "[cut1]overlay");
    let msk = pos(&g, "drawbox=");
    let fx = pos(&g, "perspective=");
    let pip = pos(&g, "[cut0]overlay");
    let lo = pos(&g, "[img2]overlay");
    let hi = pos(&g, "[img1]overlay");
    let caps = pos(&g, "ass=captions.ass");
    let text = pos(&g, "ass=overlays.ass");
    let top = pos(&g, "[img0]overlay");
    let order = [full, msk, fx, pip, lo, hi, caps, text, top];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "z-order {order:?}:\n{g}");
    // Every layer input chain is defined before its overlay uses it, and the chain ends in [vout].
    assert!(pos(&g, "setpts=PTS+0[img2]") < lo);
    assert!(g.contains("format=yuv420p[vout]"));
}

#[test]
fn layers_follow_transitions_and_skip_hidden_items() {
    let (id, sources) = fake_sources(10.0);
    let clip = make_clip(&id, &[(0.0, 2.0), (3.0, 5.0)]);
    let x = CompileExtras {
        transitions: vec![Transition {
            after_segment_id: clip.segments[0].id.clone(),
            kind: TransitionKind::Dissolve,
            duration: 0.4,
        }],
        // After the cut: moves 0.4 s (12 frames) earlier. Inside the hidden half: dropped.
        masks: vec![
            mask("msk_after", MaskKind::Box, rect(0.0, 0.0, 0.5, 0.5), 2.5, 3.5),
            mask("msk_hidden", MaskKind::Box, rect(0.5, 0.5, 0.5, 0.5), 2.0, 2.1),
        ],
        ..Default::default()
    };
    assert!((effective_duration(&clip, &x) - 3.6).abs() < 1e-9);
    let g = build_graph(&clip, &sources, false, &x).unwrap().graph;
    assert!(g.contains("enable='between(n,63,92)'"), "{g}");
    assert_eq!(g.matches("drawbox=").count(), 1, "{g}");
}

#[test]
fn invalid_layers_are_validation_errors() {
    let (id, sources) = fake_sources(10.0);
    let clip = make_clip(&id, &[(0.0, 6.0)]);
    let bad = [
        CompileExtras { masks: vec![mask("m", MaskKind::Blur, rect(0.9, 0.0, 0.2, 0.2), 0.0, 1.0)], ..Default::default() },
        CompileExtras { images: vec![ImageLayer { opacity: 1.5, ..image("i", "/a.png", rect(0.1, 0.1, 0.2, 0.2), 0.0, 1.0, 0) }], ..Default::default() },
        CompileExtras { cutaways: vec![cutaway("c", "/a.mp4", CutawayMode::Pip, None, 0.0, 1.0)], ..Default::default() },
    ];
    for x in bad {
        let e = build_graph(&clip, &sources, false, &x).unwrap_err();
        assert_eq!(e.code, rustler_core::ErrorCode::Validation, "{e:?}");
    }
}

fn w(i: u32, text: &str, s: f64, e: f64) -> Word {
    Word { i, text: text.into(), start: s, end: e, conf: None, speaker: None }
}

/// Review focus 4: a word-anchored item whose word is cut out by a re-cut is dropped with a
/// reason, never moved to a wrong time; its other words keep following the picture.
#[test]
fn anchor_removed_by_recut_is_dropped_with_warning() {
    let src = SourceId::new();
    let mut words = HashMap::new();
    words.insert(src.clone(), vec![w(0, "Hello", 1.0, 1.4), w(1, "there", 1.5, 1.9), w(2, "amazing", 3.0, 3.6), w(3, "stuff", 3.7, 4.2)]);
    let t = Timing {
        start_word: Some(Anchor { source_id: src.clone(), word_i: 2 }),
        end_word: Some(Anchor { source_id: src.clone(), word_i: 3 }),
        ..Default::default()
    };
    let seg = |s: f64, e: f64| Segment { id: SegmentId::new(), source_id: src.clone(), start: s, end: e, layout: full_frame() };
    // Clip [0.5, 5.0]: "amazing" at 2.5 .. "stuff" ends 3.7.
    let tm = TimeMap::from_segments(&[seg(0.5, 5.0)]);
    let (s, e) = resolve_timing(&t, &tm, &words).unwrap();
    assert!((s - 2.5).abs() < 1e-9 && (e - 3.7).abs() < 1e-9, "{s} {e}");
    // Re-cut that moves the words: [2.8, 5.0] -> "amazing" at 0.2.
    let tm = TimeMap::from_segments(&[seg(2.8, 5.0)]);
    let (s, _) = resolve_timing(&t, &tm, &words).unwrap();
    assert!((s - 0.2).abs() < 1e-9, "{s}");
    // Re-cut that removes "amazing": dropped with a reason naming the word.
    let tm = TimeMap::from_segments(&[seg(0.5, 2.0), seg(3.65, 5.0)]);
    let err = resolve_timing(&t, &tm, &words).unwrap_err();
    assert!(err.contains("amazing") && err.contains("not in the clip"), "{err}");
    // Unknown word index.
    let bad = Timing { start_word: Some(Anchor { source_id: src.clone(), word_i: 99 }), ..Default::default() };
    assert!(resolve_timing(&bad, &tm, &words).unwrap_err().contains("does not exist"));
    // A start word alone lasts for the word.
    let one = Timing { start_word: Some(Anchor { source_id: src.clone(), word_i: 0 }), ..Default::default() };
    let tm = TimeMap::from_segments(&[seg(0.0, 5.0)]);
    assert_eq!(resolve_timing(&one, &tm, &words).unwrap(), (1.0, 1.4));
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

/// Portrait 1080x1920 test pattern with a white "logo" box in the top-right corner and a tone.
fn logo_source(t: &ToolPaths, out: &Path, secs: u32) {
    ff(
        t,
        &[
            "-f", "lavfi", "-i",
            &format!("testsrc2=s=1080x1920:r=30:d={secs},drawbox=x=840:y=40:w=200:h=120:color=white:t=fill"),
            "-f", "lavfi", "-i", &format!("sine=f=440:r=48000:d={secs}"),
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac",
            &out.to_string_lossy(),
        ],
    );
}

fn solid_video(t: &ToolPaths, out: &Path, color: &str, w: u32, h: u32, secs: u32) {
    ff(
        t,
        &[
            "-f", "lavfi", "-i", &format!("color=c={color}:s={w}x{h}:r=30:d={secs}"),
            "-f", "lavfi", "-i", &format!("sine=f=880:r=48000:d={secs}"),
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac",
            &out.to_string_lossy(),
        ],
    );
}

fn sources_for(t: &ToolPaths, path: &Path) -> (SourceId, HashMap<SourceId, SourceMedia>) {
    let v = probe_json(t, path);
    let id = SourceId::new();
    let mut probe = fake_probe(v["format"]["duration"].as_str().unwrap().parse().unwrap());
    let vs = v["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
    probe.width = vs["width"].as_u64().unwrap() as u32;
    probe.height = vs["height"].as_u64().unwrap() as u32;
    let mut m = HashMap::new();
    m.insert(id.clone(), SourceMedia { id: id.clone(), path: path.to_path_buf(), probe });
    (id, m)
}

async fn compile_render(t: &ToolPaths, tmp: &Path, name: &str, clip: &Clip, sources: &HashMap<SourceId, SourceMedia>, x: &CompileExtras) -> PathBuf {
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

/// Exact output frame `n` (also kept as jpg under RUSTLER_TEST_OUT for a look).
fn frame_n(t: &ToolPaths, video: &Path, n: u64, name: &str) -> image::RgbImage {
    let out = video.with_file_name(format!("{name}.png"));
    ff(
        t,
        &[
            "-i", &video.to_string_lossy(),
            "-vf", &format!("select=eq(n\\,{n})"),
            "-fps_mode", "passthrough", "-frames:v", "1", "-update", "1",
            &out.to_string_lossy(),
        ],
    );
    let im = image::open(&out).unwrap().to_rgb8();
    if let Some(dir) = std::env::var_os("RUSTLER_TEST_OUT") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let small = image::imageops::resize(&im, 360, 640, image::imageops::FilterType::Triangle);
        let _ = small.save(dir.join(format!("{name}.jpg")));
    }
    im
}

/// Mean colour of a pixel box.
fn mean(im: &image::RgbImage, x: u32, y: u32, w: u32, h: u32) -> [f64; 3] {
    let mut s = [0.0; 3];
    for yy in y..y + h {
        for xx in x..x + w {
            let p = im.get_pixel(xx, yy).0;
            for c in 0..3 {
                s[c] += p[c] as f64;
            }
        }
    }
    let n = (w * h) as f64;
    [s[0] / n, s[1] / n, s[2] / n]
}

/// Luma variance of a pixel box (detail measure).
fn variance(im: &image::RgbImage, x: u32, y: u32, w: u32, h: u32) -> f64 {
    let vals: Vec<f64> = (y..y + h)
        .flat_map(|yy| (x..x + w).map(move |xx| (xx, yy)))
        .map(|(xx, yy)| {
            let p = im.get_pixel(xx, yy).0;
            0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
        })
        .collect();
    let m = vals.iter().sum::<f64>() / vals.len() as f64;
    vals.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / vals.len() as f64
}

fn is_white(c: [f64; 3]) -> bool {
    c.iter().all(|v| *v > 225.0)
}

#[tokio::test]
async fn mask_hides_corner_logo() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("logo.mp4");
    logo_source(&t, &src, 3);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 3.0)]);
    // Box over the logo for 1..3 s; blur over a detailed area of the pattern for the whole clip.
    let logo = rect(820.0 / 1080.0, 20.0 / 1920.0, 240.0 / 1080.0, 160.0 / 1920.0);
    let x = CompileExtras {
        masks: vec![
            mask("msk_logo", MaskKind::Box, logo, 1.0, 3.0),
            MaskLayer { strength: Some(40.0), ..mask("msk_blur", MaskKind::Blur, rect(0.1, 0.4, 0.5, 0.2), 0.0, 3.0) },
        ],
        ..Default::default()
    };
    let out = compile_render(&t, tmp.path(), "mask", &clip, &sources, &x).await;
    let plain = compile_render(&t, tmp.path(), "plain", &clip, &sources, &CompileExtras::default()).await;
    let before = frame_n(&t, &out, 15, "mask_before_box");
    let during = frame_n(&t, &out, 60, "mask_during_box");
    let reference = frame_n(&t, &plain, 60, "mask_reference");
    let (b, d) = (mean(&before, 880, 70, 120, 60), mean(&during, 880, 70, 120, 60));
    eprintln!("logo area mean before {b:?}, during {d:?}");
    assert!(is_white(b), "logo visible before the mask starts: {b:?}");
    assert!(d.iter().all(|v| *v < 25.0), "logo hidden by the black box: {d:?}");
    // Outside the mask the picture is untouched.
    let (o1, o2) = (mean(&during, 800, 1400, 40, 40), mean(&reference, 800, 1400, 40, 40));
    assert!((0..3).all(|c| (o1[c] - o2[c]).abs() < 12.0), "{o1:?} vs {o2:?}");
    // The blurred area lost most of its detail compared with the same frame unmasked.
    let (vb, vr) = (variance(&during, 160, 800, 400, 300), variance(&reference, 160, 800, 400, 300));
    eprintln!("blur area variance {vb:.0} vs unmasked {vr:.0}");
    assert!(vb < vr * 0.5, "blur reduced detail: {vb} vs {vr}");
}

#[tokio::test]
async fn png_logo_in_corner_fades_in_and_out() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("blue.mp4");
    solid_video(&t, &src, "blue", 1080, 1920, 4);
    // 300x150 red logo with a transparent 20 px border.
    let mut png = image::RgbaImage::from_pixel(300, 150, image::Rgba([0, 0, 0, 0]));
    for y in 20..130 {
        for x in 20..280 {
            png.put_pixel(x, y, image::Rgba([230, 20, 20, 255]));
        }
    }
    let logo = tmp.path().join("logo.png");
    png.save(&logo).unwrap();
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 4.0)]);
    let mut im = image("img_1", &logo.to_string_lossy(), rect(0.04, 0.03, 0.3, 0.1), 0.5, 3.5, 0);
    im.anim = Anim { in_: AnimKind::Fade, out: AnimKind::Fade, duration: 0.5 };
    let x = CompileExtras { images: vec![im], ..Default::default() };
    let out = compile_render(&t, tmp.path(), "logo", &clip, &sources, &x).await;
    // Rect = (43, 58) 324x192 (even); the 2:1 logo fits 324x162, centred at y 154.
    let at = |n: u64, name: &str| {
        let f = frame_n(&t, &out, n, name);
        mean(&f, 150, 140, 100, 30)
    };
    let (pre, half, full, gone) = (at(10, "logo_pre"), at(22, "logo_half_fade"), at(60, "logo_full"), at(110, "logo_after"));
    eprintln!("logo centre: pre {pre:?} half {half:?} full {full:?} after {gone:?}");
    assert!(pre[2] > 200.0 && pre[0] < 40.0, "not shown before 0.5 s: {pre:?}");
    assert!(full[0] > 190.0 && full[2] < 60.0, "red logo at 2 s: {full:?}");
    assert!(half[0] > 40.0 && half[0] < 190.0 && half[2] > 40.0, "half-way through the fade-in: {half:?}");
    assert!(gone[2] > 200.0 && gone[0] < 40.0, "gone after 3.5 s: {gone:?}");
    // Transparent border shows the background; outside the rect is untouched.
    let f = frame_n(&t, &out, 60, "logo_full2");
    let border = mean(&f, 48, 150, 8, 8);
    assert!(border[2] > 200.0, "transparent border: {border:?}");
}

#[tokio::test]
async fn pip_and_full_cutaways_render() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("speaker.mp4");
    solid_video(&t, &src, "blue", 1080, 1920, 4);
    let green = tmp.path().join("green.mp4");
    solid_video(&t, &green, "green", 1280, 720, 5);
    let red = tmp.path().join("red.mp4");
    solid_video(&t, &red, "red", 1920, 1080, 5);
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 4.0)]);
    let mut full = cutaway("cut_1", &red.to_string_lossy(), CutawayMode::Full, None, 2.5, 3.5);
    full.media.in_ = 1.0;
    let x = CompileExtras {
        cutaways: vec![
            cutaway("cut_2", &green.to_string_lossy(), CutawayMode::Pip, Some(rect(0.5, 0.6, 0.45, 0.25)), 1.0, 2.0),
            full,
        ],
        ..Default::default()
    };
    let out = compile_render(&t, tmp.path(), "cutaway", &clip, &sources, &x).await;
    let v = probe_json(&t, &out);
    let dur: f64 = v["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - 4.0).abs() < 0.06, "duration unchanged: {dur}");
    let before = frame_n(&t, &out, 15, "cut_before");
    let pip = frame_n(&t, &out, 45, "cut_pip");
    let full = frame_n(&t, &out, 90, "cut_full");
    let after = frame_n(&t, &out, 112, "cut_after");
    let pip_box = |f: &image::RgbImage| mean(f, 600, 1200, 300, 300);
    let centre = |f: &image::RgbImage| mean(f, 200, 300, 100, 100);
    eprintln!("pip box before {:?} during {:?}; centre full {:?} after {:?}", pip_box(&before), pip_box(&pip), centre(&full), centre(&after));
    assert!(pip_box(&before)[2] > 200.0, "no pip before 1 s");
    let g = pip_box(&pip);
    assert!(g[1] > 100.0 && g[2] < 60.0 && g[0] < 60.0, "green pip at 1.5 s: {g:?}");
    assert!(centre(&pip)[2] > 200.0, "pip leaves the rest of the frame: {:?}", centre(&pip));
    let r = centre(&full);
    assert!(r[0] > 200.0 && r[2] < 60.0, "full red cutaway at 3 s: {r:?}");
    assert!(pip_box(&full)[0] > 200.0, "full cutaway covers the whole frame");
    assert!(centre(&after)[2] > 200.0, "speaker back after 3.5 s: {:?}", centre(&after));
}

#[tokio::test]
async fn gif_pop_slide_and_rotation_render() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(t) = tools(tmp.path()) else { return };
    let src = tmp.path().join("blue.mp4");
    solid_video(&t, &src, "blue", 1080, 1920, 3);
    let gif = tmp.path().join("y.gif");
    ff(&t, &["-f", "lavfi", "-i", "color=c=yellow:s=120x120:r=10:d=0.5", &gif.to_string_lossy()]);
    let png = tmp.path().join("w.png");
    image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255])).save(&png).unwrap();
    let (id, sources) = sources_for(&t, &src);
    let clip = make_clip(&id, &[(0.0, 3.0)]);
    let mut g = image("img_1", &gif.to_string_lossy(), rect(0.1, 0.4, 0.3, 0.17), 0.0, 3.0, 0);
    g.anim = Anim { in_: AnimKind::Pop, out: AnimKind::Pop, duration: 0.4 };
    let mut s = image("img_2", &png.to_string_lossy(), rect(0.6, 0.6, 0.3, 0.17), 0.5, 2.5, 1);
    s.anim = Anim { in_: AnimKind::SlideUp, out: AnimKind::SlideUp, duration: 0.4 };
    s.rotation = 20.0;
    let x = CompileExtras { images: vec![g, s], ..Default::default() };
    let out = compile_render(&t, tmp.path(), "gif", &clip, &sources, &x).await;
    // The 0.5 s GIF loops for the whole 3 s layer.
    let late = frame_n(&t, &out, 75, "gif_late");
    let y = mean(&late, 240, 900, 60, 60);
    assert!(y[0] > 200.0 && y[1] > 200.0 && y[2] < 80.0, "gif still looping at 2.5 s: {y:?}");
    // Pop: 4 frames in, the gif is smaller than at full size.
    let early = frame_n(&t, &out, 4, "gif_pop_early");
    let yellow = |f: &image::RgbImage| f.pixels().filter(|p| p.0[0] > 200 && p.0[1] > 200 && p.0[2] < 80).count();
    let (ye, yl) = (yellow(&early), yellow(&late));
    eprintln!("yellow pixels early {ye} late {yl}");
    assert!(ye > 0 && (ye as f64) < 0.8 * yl as f64, "pop grows: {ye} vs {yl}");
    let mid = frame_n(&t, &out, 45, "gif_slide_rot");
    let white = |f: &image::RgbImage| f.pixels().filter(|p| p.0.iter().all(|c| *c > 220)).count();
    assert!(white(&mid) > 5000, "rotated white square visible at 1.5 s");
}
