//! Pure compiler tests (no ffmpeg needed).

use rustler_core::RenderQuality;
use rustler_core::{
    AudioSettings, Background, CaptionTrack, Clip, ClipId, ErrorCode, Fit, Layout, MediaProbe,
    Rect, Region, Segment, SegmentId, SourceId,
};
use rustler_media::compile::{
    CompileExtras, CropKey, MusicBed, SourceMedia, build_graph, compile, compile_with,
};
use rustler_media::encoders::{self, EncoderChoice};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn probe(w: u32, h: u32, dur: f64, audio: bool) -> MediaProbe {
    MediaProbe {
        duration: dur,
        has_video: true,
        has_audio: audio,
        width: w,
        height: h,
        rotation: 0,
        fps: 30.0,
        video_codec: Some("h264".into()),
        audio_codec: audio.then(|| "aac".into()),
        sample_rate: audio.then_some(48000),
        channels: audio.then_some(2),
    }
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

/// 9:16 center crop of a 1920x1080 source (what `layout::preset("center-crop")` produces).
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

struct Fx {
    clip: Clip,
    sources: HashMap<SourceId, SourceMedia>,
}

fn seg(src: &SourceId, start: f64, end: f64, layout: Layout) -> Segment {
    Segment {
        id: SegmentId::new(),
        source_id: src.clone(),
        start,
        end,
        layout,
    }
}

fn make_clip(segments: Vec<Segment>, fps: f64) -> Clip {
    serde_json::from_value(json!({
        "schema": "rustler.clip@1",
        "id": ClipId::new(),
        "name": "test",
        "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": fps},
        "segments": segments,
        "captions": CaptionTrack::default(),
        "audio": AudioSettings::default(),
        "renders": [],
        "created_at": "2026-11-15T00:00:00Z"
    }))
    .unwrap()
}

fn fixture(layout: Layout, p: MediaProbe, start: f64, end: f64) -> Fx {
    let id = SourceId::new();
    let clip = make_clip(vec![seg(&id, start, end, layout)], 30.0);
    let mut sources = HashMap::new();
    sources.insert(
        id.clone(),
        SourceMedia {
            id,
            path: PathBuf::from("/media/in.mp4"),
            probe: p,
        },
    );
    Fx { clip, sources }
}

fn graph(fx: &Fx, captions: bool) -> String {
    build_graph(&fx.clip, &fx.sources, captions, &CompileExtras::default())
        .unwrap()
        .graph
}

fn enc() -> EncoderChoice {
    encoders::choice("libx264", RenderQuality::Draft)
}

fn args_str(a: &[std::ffi::OsString]) -> Vec<String> {
    a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
}

#[test]
fn single_segment_center_crop_graph() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 5.0, 15.0);
    let g = graph(&fx, false);
    assert!(g.contains("crop=606:1080:656:0"), "{g}");
    assert!(
        g.contains("scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920"),
        "{g}"
    );
    assert!(g.contains("concat=n=1:v=1:a=1"), "{g}");
    assert!(g.contains("loudnorm=I=-14:TP=-2:LRA=11"), "{g}");
    assert!(g.contains("trim=end_frame=300"), "{g}");
    assert!(g.contains("atrim=end_sample=480000"), "{g}");
    // A full-canvas cover region needs no background/overlay.
    assert!(!g.contains("overlay="), "{g}");
    assert!(!g.contains("color=c=black"), "{g}");
}

#[test]
fn blur_fill_graph_has_gblur() {
    let fx = fixture(blur_fit(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let g = graph(&fx, false);
    assert!(g.contains("scale=270:480:force_original_aspect_ratio=increase,crop=270:480,gblur=sigma=10,lutyuv=y=val*0.6,scale=1080:1920"), "{g}");
    assert!(g.contains("split=2"), "{g}");
    assert!(g.contains("force_original_aspect_ratio=decrease"), "{g}");
    assert!(
        g.contains("overlay=x=0+(1080-overlay_w)/2:y=0+(1920-overlay_h)/2"),
        "{g}"
    );
}

#[test]
fn split_vertical_two_overlays() {
    let fx = fixture(split_vertical(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let g = graph(&fx, false);
    assert_eq!(g.matches("overlay=").count(), 2, "{g}");
    assert!(g.contains("overlay=x=0:y=0"), "{g}");
    assert!(g.contains("overlay=x=0:y=960"), "{g}");
    assert!(g.contains("crop=960:1080:0:0,scale=1080:960"), "{g}");
    assert!(g.contains("crop=960:1080:960:0,scale=1080:960"), "{g}");
    assert!(
        g.contains("color=c=black:s=1080x1920:r=30,trim=end_frame=300"),
        "{g}"
    );
}

#[test]
fn no_audio_source_uses_anullsrc() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, false), 0.0, 10.0);
    let g = graph(&fx, false);
    assert!(g.contains("anullsrc=r=48000:cl=stereo"), "{g}");
    assert!(!g.contains("[0:a"), "{g}");
}

#[test]
fn captions_reference_relative_ass() {
    let tmp = tempfile::tempdir().unwrap();
    let ass = tmp.path().join("elsewhere.ass");
    std::fs::write(&ass, "[Script Info]\n").unwrap();
    let fonts = tmp.path().join("fontsrc");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("A.ttf"), b"x").unwrap();
    let work = tmp.path().join("work");
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let out = tmp.path().join("out.mp4");
    let c = compile(
        &fx.clip,
        &fx.sources,
        Some(&ass),
        &fonts,
        &enc(),
        &out,
        &work,
    )
    .unwrap();
    assert!(
        c.filter_script.contains("ass=captions.ass:fontsdir=fonts"),
        "{}",
        c.filter_script
    );
    assert!(
        !c.filter_script.contains(":\\") && !c.filter_script.contains(":/"),
        "drive letter in graph"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("graph.txt")).unwrap(),
        c.filter_script
    );
    assert!(work.join("captions.ass").is_file());
    assert!(work.join("fonts").join("A.ttf").is_file());
    let a = args_str(&c.args);
    let i = a.iter().position(|s| s == "-/filter_complex").unwrap();
    assert_eq!(a[i + 1], "graph.txt");
}

#[test]
fn args_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 5.0, 15.0);
    let out = tmp.path().join("o.mp4");
    let c = compile(
        &fx.clip,
        &fx.sources,
        None,
        tmp.path(),
        &enc(),
        &out,
        &tmp.path().join("w"),
    )
    .unwrap();
    let a = args_str(&c.args);
    let j = a.join(" ");
    assert!(j.contains("-ss 5.000 -to 15.000 -i "), "{j}");
    assert!(j.contains("-map [vout] -map [aout]"), "{j}");
    assert!(
        j.contains("-c:v libx264 -preset veryfast -crf 28 -pix_fmt yuv420p"),
        "{j}"
    );
    assert!(j.contains("-c:a aac -b:a 160k -ar 48000"), "{j}");
    assert!(j.contains("-r 30"), "{j}");
    assert!(j.contains("-movflags +faststart"), "{j}");
    assert!(Path::new(a.last().unwrap()).is_absolute());
    assert!(!c.filter_script.contains("ass="));
}

fn code(e: rustler_core::RustlerError) -> ErrorCode {
    e.code
}

#[test]
fn rejects_segment_out_of_bounds() {
    let fx = fixture(center_crop(), probe(1920, 1080, 10.0, true), 5.0, 10.2);
    let e = build_graph(&fx.clip, &fx.sources, false, &CompileExtras::default()).unwrap_err();
    assert_eq!(code(e), ErrorCode::Validation);
    // within tolerance is fine
    let fx = fixture(center_crop(), probe(1920, 1080, 10.0, true), 5.0, 10.04);
    assert!(build_graph(&fx.clip, &fx.sources, false, &CompileExtras::default()).is_ok());
}

#[test]
fn rejects_bad_segments_and_videoless_sources() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 5.0, 5.0);
    assert_eq!(
        code(build_graph(&fx.clip, &fx.sources, false, &CompileExtras::default()).unwrap_err()),
        ErrorCode::Validation
    );

    let mut p = probe(0, 0, 60.0, true);
    p.has_video = false;
    let fx = fixture(center_crop(), p, 0.0, 5.0);
    let e = build_graph(&fx.clip, &fx.sources, false, &CompileExtras::default()).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("no video"), "{}", e.message);

    let mut fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 5.0);
    fx.clip.segments.clear();
    assert_eq!(
        code(build_graph(&fx.clip, &fx.sources, false, &CompileExtras::default()).unwrap_err()),
        ErrorCode::Validation
    );
}

#[test]
fn multi_segment_frames_allocated_on_cumulative_grid() {
    let id = SourceId::new();
    let clip = make_clip(
        vec![
            seg(&id, 1.0, 1.25, center_crop()),
            seg(&id, 3.0, 3.25, split_vertical()),
        ],
        30.0,
    );
    let mut sources = HashMap::new();
    sources.insert(
        id.clone(),
        SourceMedia {
            id,
            path: "/a.mp4".into(),
            probe: probe(1920, 1080, 10.0, true),
        },
    );
    let g = build_graph(&clip, &sources, false, &CompileExtras::default())
        .unwrap()
        .graph;
    // 0.25 s = 7.5 frames: cumulative rounding gives 8 + 7 = 15 (= round(0.5 * 30)).
    assert!(g.contains("[0:v:0]fps=30:start_time=0:round=near,tpad=stop_mode=clone:stop_duration=1,trim=end_frame=8"), "{g}");
    assert!(g.contains("[1:v:0]fps=30:start_time=0:round=near,tpad=stop_mode=clone:stop_duration=1,trim=end_frame=7"), "{g}");
    assert!(
        g.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1[vcat][acat]"),
        "{g}"
    );
    assert!(g.contains("atrim=end_sample=12000"), "{g}");
}

#[test]
fn ntsc_rate_is_rational() {
    let id = SourceId::new();
    let clip = make_clip(vec![seg(&id, 0.0, 2.0, center_crop())], 29.97);
    let mut sources = HashMap::new();
    sources.insert(
        id.clone(),
        SourceMedia {
            id,
            path: "/a.mp4".into(),
            probe: probe(1920, 1080, 10.0, true),
        },
    );
    let g = build_graph(&clip, &sources, false, &CompileExtras::default())
        .unwrap()
        .graph;
    assert!(g.contains("fps=30000/1001"), "{g}");
    assert!(g.contains("settb=1001/30000,setpts=N"), "{g}");
}

#[test]
fn music_bed_ducks_under_speech() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let tmp = tempfile::tempdir().unwrap();
    let extras = CompileExtras {
        music: Some(MusicBed::new(tmp.path().join("bed.mp3"))),
        ..Default::default()
    };
    let c = compile_with(
        &fx.clip,
        &fx.sources,
        None,
        tmp.path(),
        &enc(),
        &tmp.path().join("o.mp4"),
        &tmp.path().join("w"),
        &extras,
    )
    .unwrap();
    let g = &c.filter_script;
    assert!(g.contains("[1:a:0]aresample=48000"), "{g}");
    assert!(g.contains("volume=-18dB"), "{g}");
    assert!(g.contains("atrim=end_sample=480000"), "{g}");
    assert!(g.contains("afade=t=in:st=0:d=0.5"), "{g}");
    assert!(g.contains("afade=t=out:st=9:d=1"), "{g}");
    assert!(g.contains("[acat]asplit=2[speech][sc]"), "{g}");
    assert!(
        g.contains(
            "[mus][sc]sidechaincompress=threshold=0.08:ratio=5:attack=100:release=1000[duck]"
        ),
        "{g}"
    );
    let mix = g
        .find("[speech][duck]amix=inputs=2:duration=first:dropout_transition=0:normalize=0")
        .expect(g);
    let ln = g.find("loudnorm=").unwrap();
    assert!(ln > mix, "loudnorm must come after the mix: {g}");
    let a = args_str(&c.args).join(" ");
    assert!(a.contains("-stream_loop -1 -i "), "{a}");
    assert!(a.contains("bed.mp3"), "{a}");
}

#[test]
fn music_bed_without_duck_mixes_directly() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, false), 0.0, 4.0);
    let mut m = MusicBed::new("/m.wav");
    m.duck = false;
    m.volume_db = -12.5;
    let g = build_graph(
        &fx.clip,
        &fx.sources,
        false,
        &CompileExtras {
            music: Some(m),
            ..Default::default()
        },
    )
    .unwrap()
    .graph;
    assert!(!g.contains("sidechaincompress"), "{g}");
    assert!(
        g.contains("[acat][mus]amix=inputs=2:duration=first:dropout_transition=0:normalize=0"),
        "{g}"
    );
    assert!(g.contains("volume=-12.5dB"), "{g}");
    assert!(g.contains("afade=t=out:st=3:d=1"), "{g}");
}

#[test]
fn crop_path_emits_sendcmd_x_only() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let seg_id = fx.clip.segments[0].id.clone();
    let mut extras = CompileExtras::default();
    extras.crop_paths.insert(
        seg_id,
        vec![
            CropKey { t: 0.0, x: 0.0 },
            CropKey { t: 4.0, x: 0.0 },
            CropKey { t: 6.0, x: 1.0 },
        ],
    );
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("w");
    let c = compile_with(
        &fx.clip,
        &fx.sources,
        None,
        tmp.path(),
        &enc(),
        &tmp.path().join("o.mp4"),
        &work,
        &extras,
    )
    .unwrap();
    let g = &c.filter_script;
    assert!(
        g.contains("sendcmd=f=crop_s0r0.cmd,crop@s0r0=606:1080:0:0,scale=1080:1920"),
        "{g}"
    );
    let body = std::fs::read_to_string(work.join("crop_s0r0.cmd")).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert!(lines.len() > 20, "{body}");
    let mut prev_t = 0.0;
    let mut prev_x = 0;
    for l in &lines {
        let p: Vec<&str> = l.split(' ').collect();
        assert_eq!(p.len(), 4, "{l}");
        assert_eq!(p[1], "crop@s0r0");
        assert_eq!(p[2], "x", "only x may change: {l}");
        let t: f64 = p[0].parse().unwrap();
        let x: u32 = p[3].trim_end_matches(';').parse().unwrap();
        assert!(x.is_multiple_of(2) && x >= prev_x && t > prev_t, "{l}");
        assert!((4.0..=6.0).contains(&t), "change outside pan window: {l}");
        prev_t = t;
        prev_x = x;
    }
    assert_eq!(prev_x, 1920 - 606, "pan clamps at the right edge");
}

/// A path that never moves (a single key, as `crop.track_face` returns for a short still
/// segment) must not emit an empty sendcmd file: ffmpeg rejects it ("No commands were
/// specified") and the whole preview/render fails. It becomes a plain crop at that x.
#[test]
fn still_crop_path_uses_plain_crop() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 2.0);
    let seg_id = fx.clip.segments[0].id.clone();
    let mut extras = CompileExtras::default();
    extras.crop_paths.insert(seg_id, vec![CropKey { t: 0.0, x: 0.25 }]);
    let p = build_graph(&fx.clip, &fx.sources, false, &extras).unwrap();
    assert!(p.cmd_files.is_empty(), "{:?}", p.cmd_files);
    assert!(!p.graph.contains("sendcmd"), "{}", p.graph);
    assert!(p.graph.contains("crop=606:1080:480:0"), "{}", p.graph);
}

#[test]
fn crop_path_for_other_segment_is_ignored() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let mut extras = CompileExtras::default();
    extras
        .crop_paths
        .insert(SegmentId::new(), vec![CropKey { t: 0.0, x: 0.2 }]);
    let p = build_graph(&fx.clip, &fx.sources, false, &extras).unwrap();
    assert!(p.cmd_files.is_empty());
    assert!(!p.graph.contains("sendcmd"));
}

#[test]
fn encoder_args_match_plan() {
    let j = |n: &str, q| encoders::encoder_args(n, q).join(" ");
    assert_eq!(
        j("libx264", RenderQuality::Final),
        "-c:v libx264 -preset medium -crf 20 -pix_fmt yuv420p"
    );
    assert_eq!(
        j("libx264", RenderQuality::Draft),
        "-c:v libx264 -preset veryfast -crf 28 -pix_fmt yuv420p"
    );
    assert_eq!(
        j("h264_nvenc", RenderQuality::Final),
        "-c:v h264_nvenc -preset p5 -rc vbr -cq 23 -b:v 0 -pix_fmt yuv420p"
    );
    assert_eq!(
        j("h264_nvenc", RenderQuality::Draft),
        "-c:v h264_nvenc -preset p2 -rc vbr -cq 30 -b:v 0 -pix_fmt yuv420p"
    );
}

#[test]
fn split_vertical_captions_music_graph() {
    let fx = fixture(split_vertical(), probe(1920, 1080, 60.0, true), 12.0, 27.5);
    let extras = CompileExtras {
        music: Some(MusicBed::new("/music/bed.mp3")),
        ..Default::default()
    };
    let g = build_graph(&fx.clip, &fx.sources, true, &extras).unwrap().graph;
    println!("{g}");
    let ass = g.find("ass=captions.ass:fontsdir=fonts").expect(&g);
    assert!(ass > g.find("concat=").unwrap(), "captions burn after concat");
    assert!(g.contains("sidechaincompress"), "{g}");
    assert!(g.contains("afade=t=out:st=14.5:d=1"), "{g}");
}

// ---------------------------------------------------------------------------------------------
// Two-pass loudnorm
// ---------------------------------------------------------------------------------------------

#[test]
fn two_pass_loudness_measurement_pass() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 2.0, 12.0);
    let tmp = tempfile::tempdir().unwrap();
    let extras = CompileExtras {
        music: Some(MusicBed::new(tmp.path().join("bed.mp3"))),
        ..Default::default()
    };
    assert!(extras.two_pass_loudness, "two-pass is the default");
    let work = tmp.path().join("w");
    let c = compile_with(
        &fx.clip,
        &fx.sources,
        None,
        tmp.path(),
        &enc(),
        &tmp.path().join("o.mp4"),
        &work,
        &extras,
    )
    .unwrap();
    // The final graph keeps the single-pass filter text (swapped at render time).
    assert!(
        c.filter_script.contains("loudnorm=I=-14:TP=-2:LRA=11,"),
        "{}",
        c.filter_script
    );
    let lp = c.loudness.as_ref().expect("measurement pass");
    assert_eq!(lp.filter, "loudnorm=I=-14:TP=-2:LRA=11");
    let a = args_str(&lp.args).join(" ");
    assert!(a.contains("-loglevel info"), "{a}");
    assert!(a.contains("-ss 2.000 -to 12.000 -i "), "{a}");
    assert!(a.contains("-stream_loop -1 -i "), "{a}");
    assert!(
        a.ends_with("-/filter_complex graph_loudness.txt -map [aout] -f null -"),
        "{a}"
    );
    // The measurement graph is the same audio graph, audio only, printing JSON.
    let g =
        std::fs::read_to_string(work.join(rustler_media::compile::LOUDNESS_GRAPH_FILE)).unwrap();
    assert!(g.contains("[a0]concat=n=1:v=0:a=1[acat]"), "{g}");
    assert!(!g.contains(":v:0"), "no video in the measurement graph: {g}");
    assert!(g.contains("[1:a:0]aresample=48000"), "{g}");
    assert!(g.contains("sidechaincompress"), "{g}");
    assert!(
        g.contains("loudnorm=I=-14:TP=-2:LRA=11:print_format=json,aresample=48000"),
        "{g}"
    );
    assert!(g.trim_end().ends_with("[aout]"), "{g}");
    // Every audio chain of the final graph before loudnorm is in the measurement graph.
    for line in c
        .filter_script
        .split(";\n")
        .filter(|l| (l.contains(":a:0]") || l.contains("[mus]")) && !l.contains("loudnorm"))
    {
        assert!(g.contains(line), "missing {line} in {g}");
    }

    // Off when disabled or when loudnorm is off.
    let off = CompileExtras {
        two_pass_loudness: false,
        ..Default::default()
    };
    let o = tmp.path().join("o.mp4");
    let c2 = compile_with(&fx.clip, &fx.sources, None, tmp.path(), &enc(), &o, &work, &off).unwrap();
    assert!(c2.loudness.is_none());
    assert!(!work.join(rustler_media::compile::LOUDNESS_GRAPH_FILE).exists());
    let mut clip = fx.clip.clone();
    clip.audio.loudnorm = false;
    let d = CompileExtras::default();
    let c3 = compile_with(&clip, &fx.sources, None, tmp.path(), &enc(), &o, &work, &d).unwrap();
    assert!(c3.loudness.is_none());
    assert!(!c3.filter_script.contains("loudnorm"));
}

#[test]
fn loudnorm_json_parsing_and_second_pass_filter() {
    use rustler_media::compile::{measured_loudnorm, parse_loudnorm_json};
    let log = "[Parsed_loudnorm_12 @ 000001] \n{\n\t\"input_i\" : \"-13.59\",\n\t\"input_tp\" : \"-5.06\",\n\
               \t\"input_lra\" : \"17.40\",\n\t\"input_thresh\" : \"-29.23\",\n\t\"output_i\" : \"-15.89\",\n\
               \t\"output_tp\" : \"-4.16\",\n\t\"output_lra\" : \"11.50\",\n\t\"output_thresh\" : \"-27.50\",\n\
               \t\"normalization_type\" : \"dynamic\",\n\t\"target_offset\" : \"1.89\"\n}\n\
               [out#0/null @ 0000] video:0KiB audio:7500KiB\n";
    let m = parse_loudnorm_json(log).unwrap();
    assert_eq!(
        (m.input_i, m.input_tp, m.input_lra, m.input_thresh, m.target_offset),
        (-13.59, -5.06, 17.4, -29.23, 1.89)
    );
    assert_eq!(
        measured_loudnorm("loudnorm=I=-14:TP=-2:LRA=11", &m),
        "loudnorm=I=-14:TP=-2:LRA=11:measured_I=-13.59:measured_TP=-5.06:measured_LRA=17.4:\
         measured_thresh=-29.23:offset=1.89:linear=true"
    );
    // Digital silence prints -inf: not usable -> single-pass fallback.
    let silent = log.replace("\"-13.59\"", "\"-inf\"");
    assert!(parse_loudnorm_json(&silent).is_none());
    assert!(parse_loudnorm_json("no json here").is_none());
}

// ---------------------------------------------------------------------------------------------
// Text overlays
// ---------------------------------------------------------------------------------------------

use rustler_media::compile::{OverlayStyle, TextOverlay, overlay_layout, overlays_ass};

fn hook(id: &str, text: &str, start: f64, end: f64, style: OverlayStyle) -> TextOverlay {
    TextOverlay::new(id, text, start, end, style)
}

#[test]
fn overlays_burn_after_captions_in_one_graph() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let extras = CompileExtras {
        overlays: vec![hook("ovl_1", "He quit his job for this", 0.0, 3.0, OverlayStyle::HookTop)],
        ..Default::default()
    };
    let plan = build_graph(&fx.clip, &fx.sources, true, &extras).unwrap();
    let g = &plan.graph;
    assert!(
        g.contains(
            "ass=captions.ass:fontsdir=fonts,ass=overlays.ass:fontsdir=fonts,format=yuv420p[vout]"
        ),
        "{g}"
    );
    let ass = plan.overlays_ass.as_ref().unwrap();
    println!("{ass}");
    assert!(ass.contains("PlayResX: 1080\nPlayResY: 1920"), "{ass}");
    assert!(ass.contains("WrapStyle: 2"), "{ass}");
    // 0.085 * 1080 = 92 px, BorderStyle 3 box padded 26 px, centred (Alignment 5).
    assert!(
        ass.contains(
            "Style: ov0,Poppins ExtraBold,92,&H00FFFFFF,&H00FFFFFF,&H00141414,&H80000000,0,0,0,0,\
             100,100,0,0,3,26,0,5,0,0,0,1"
        ),
        "{ass}"
    );
    assert!(
        ass.contains(
            "Dialogue: 0,0:00:00.00,0:00:03.00,ov0,,0,0,0,,{\\an5\\pos(540,230)\\fad(0,150)}He quit his job for this"
        ),
        "{ass}"
    );

    // Without captions the overlay still burns; without overlays there is no second ass.
    let g2 = build_graph(&fx.clip, &fx.sources, false, &extras).unwrap().graph;
    assert!(
        g2.contains("setpts=N,ass=overlays.ass:fontsdir=fonts,format=yuv420p"),
        "{g2}"
    );
    let p3 = build_graph(&fx.clip, &fx.sources, true, &CompileExtras::default()).unwrap();
    assert!(p3.overlays_ass.is_none() && !p3.graph.contains("overlays.ass"));

    // compile_with stages overlays.ass and the fonts next to graph.txt.
    let tmp = tempfile::tempdir().unwrap();
    let fonts = tmp.path().join("fonts src");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Poppins-ExtraBold.ttf"), b"x").unwrap();
    let work = tmp.path().join("w");
    let o = tmp.path().join("o.mp4");
    compile_with(&fx.clip, &fx.sources, None, &fonts, &enc(), &o, &work, &extras).unwrap();
    assert_eq!(
        std::fs::read_to_string(work.join("overlays.ass")).unwrap(),
        *ass
    );
    assert!(work.join("fonts").join("Poppins-ExtraBold.ttf").is_file());
}

#[test]
fn overlay_wraps_to_two_balanced_lines_and_shrinks() {
    let canvas = rustler_core::Canvas {
        width: 1080,
        height: 1920,
        fps: 30.0,
    };
    let short = hook("a", "Wait for it", 0.0, 3.0, OverlayStyle::HookTop);
    let l = overlay_layout(&short, &canvas);
    assert_eq!(l.lines, vec!["Wait for it"]);
    assert_eq!(l.font_size, 92); // round(1080 * 0.085): mobile-readable hook
    assert!(l.fits);

    let text = "Nobody expected the intern to fix the production outage";
    let long = hook("b", text, 0.0, 3.0, OverlayStyle::HookTop);
    let l = overlay_layout(&long, &canvas);
    assert_eq!(l.lines.len(), 2, "{l:?}");
    assert!(l.fits);
    assert!(l.width <= 1080.0 * 0.86 + 0.5, "{l:?}");
    // Balanced: neither line is more than ~2x the other.
    let (a, b) = (l.lines[0].len() as f64, l.lines[1].len() as f64);
    assert!(a / b < 2.0 && b / a < 2.0, "{l:?}");
    // Block stays on the canvas even at y = 0.
    let mut top = long.clone();
    top.position_y = 0.0;
    let lt = overlay_layout(&top, &canvas);
    assert!(lt.top >= 0.0, "{lt:?}");

    // Too long for 2 lines even when shrunk: still 2 lines, fits = false, smallest size.
    let words = "supercalifragilistic expialidocious ".repeat(5);
    let lh = overlay_layout(&hook("c", &words, 0.0, 3.0, OverlayStyle::HookTop), &canvas);
    assert_eq!(lh.lines.len(), 2);
    assert!(!lh.fits);
    assert_eq!(lh.font_size, 55); // round(92 * 0.6)

    // ASS: \N between lines; overlays past the end are dropped, ends clamped, fade-in after t=0.
    let late = hook("d", "late", 12.0, 14.0, OverlayStyle::Plain);
    let mid = hook("e", "mid {\\b1}", 4.0, 30.0, OverlayStyle::LowerThird);
    let ass = overlays_ass(&[long.clone(), late, mid], &canvas, 10.0).unwrap();
    assert!(ass.contains(&l.lines.join("\\N")), "{ass}");
    assert!(!ass.contains("late"), "{ass}");
    assert!(ass.contains("0:00:04.00,0:00:10.00,ov1,"), "{ass}");
    assert!(ass.contains("\\fad(150,0)}mid b1"), "{ass}");
    assert!(ass.contains("\\pos(540,1536)"), "lower-third at y 0.80: {ass}");
    assert!(overlays_ass(&[], &canvas, 10.0).is_none());
}

#[test]
fn invalid_overlay_is_a_validation_error() {
    let fx = fixture(center_crop(), probe(1920, 1080, 60.0, true), 0.0, 10.0);
    let base = || hook("x", "t", 0.0, 3.0, OverlayStyle::HookTop);
    for bad in [
        TextOverlay {
            position_y: 1.5,
            ..base()
        },
        hook("x", "t", 3.0, 2.0, OverlayStyle::HookTop),
        TextOverlay {
            font_size_factor: 0.0,
            ..base()
        },
    ] {
        let extras = CompileExtras {
            overlays: vec![bad],
            ..Default::default()
        };
        let e = build_graph(&fx.clip, &fx.sources, false, &extras).unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation);
    }
    // JSON shape: "box" (not "boxed"), kebab-case style, defaults for missing fields.
    let o: TextOverlay =
        serde_json::from_value(json!({"id": "ovl_1", "text": "hi", "start": 0, "end": 2})).unwrap();
    assert_eq!(
        (o.style, o.position_y, o.font_size_factor, o.boxed),
        (OverlayStyle::HookTop, 0.12, 0.085, true)
    );
    let v = serde_json::to_value(hook("ovl_2", "x", 0.0, 1.0, OverlayStyle::LowerThird)).unwrap();
    assert_eq!(v["style"], "lower-third");
    assert_eq!(v["box"], true);
}
