//! Pure compiler tests for audio post, transitions, effects and the progress bar (no ffmpeg).

use rustler_core::{
    AudioSettings, Background, CaptionTrack, Clip, ClipId, ErrorCode, Fit, Layout, MediaProbe, Rect, Region,
    RenderQuality, Segment, SegmentId, SourceId,
};
use rustler_media::compile::{
    BarPosition, CompileExtras, Denoise, Effect, EffectKind, MusicBed, OverlayStyle, ProgressBar, SfxCue,
    SourceMedia, TextOverlay, Transition, TransitionKind, TransitionMap, build_graph, compile_with,
    effective_duration, ffmpeg_color, noise_floor_from_log,
};
use rustler_media::encoders;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

fn probe(dur: f64, audio: bool) -> MediaProbe {
    MediaProbe {
        duration: dur,
        has_video: true,
        has_audio: audio,
        width: 1920,
        height: 1080,
        rotation: 0,
        fps: 30.0,
        video_codec: Some("h264".into()),
        audio_codec: audio.then(|| "aac".into()),
        sample_rate: audio.then_some(48000),
        channels: audio.then_some(2),
    }
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

struct Fx {
    clip: Clip,
    sources: HashMap<SourceId, SourceMedia>,
}

/// Clip of `ranges` (source seconds) on one 60 s source, 30 fps.
fn fixture(ranges: &[(f64, f64)]) -> Fx {
    let id = SourceId::new();
    let segs: Vec<Segment> = ranges
        .iter()
        .map(|&(s, e)| Segment { id: SegmentId::new(), source_id: id.clone(), start: s, end: e, layout: center_crop() })
        .collect();
    let clip: Clip = serde_json::from_value(json!({
        "schema": "rustler.clip@1",
        "id": ClipId::new(),
        "name": "post",
        "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": segs,
        "captions": CaptionTrack::default(),
        "audio": AudioSettings::default(),
        "renders": [],
        "created_at": "2026-11-15T00:00:00Z"
    }))
    .unwrap();
    let mut sources = HashMap::new();
    sources.insert(id.clone(), SourceMedia { id, path: PathBuf::from("/media/in.mp4"), probe: probe(60.0, true) });
    Fx { clip, sources }
}

fn tr(fx: &Fx, k: usize, kind: TransitionKind, d: f64) -> Transition {
    Transition { after_segment_id: fx.clip.segments[k].id.clone(), kind, duration: d }
}

fn graph(fx: &Fx, x: &CompileExtras) -> String {
    build_graph(&fx.clip, &fx.sources, false, x).unwrap().graph
}

#[test]
fn transition_map_moves_later_times_and_clamps_at_the_midpoint() {
    // Segments of 2 s, 3 s, 2 s; dissolve 0.3 s after seg 0, fade 0.5 s after seg 1.
    let fx = fixture(&[(0.0, 2.0), (10.0, 13.0), (20.0, 22.0)]);
    let ts = vec![tr(&fx, 0, TransitionKind::Dissolve, 0.3), tr(&fx, 1, TransitionKind::Fade, 0.5)];
    let m = TransitionMap::new(&fx.clip, &ts);
    assert_eq!(m.at_boundary(0), Some((TransitionKind::Dissolve, 9)));
    assert_eq!(m.at_boundary(1), Some((TransitionKind::Fade, 15)));
    assert!((m.total_overlap() - 0.8).abs() < 1e-9);
    assert!((m.duration() - 6.2).abs() < 1e-9);
    let x = CompileExtras { transitions: ts.clone(), ..Default::default() };
    assert!((effective_duration(&fx.clip, &x) - 6.2).abs() < 1e-9);

    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(near(m.map_time(1.0), 1.0), "before the first overlap: unchanged");
    // Seg 0 ends at its overlap midpoint (2.0 - 0.15).
    assert!(near(m.map_time(1.95), 1.85));
    assert!(near(m.map_time(2.0), 1.85), "continuous at the boundary");
    // Seg 1 content moves 0.3 s earlier; its first 0.15 s collapse onto the midpoint.
    assert!(near(m.map_time(2.1), 1.85));
    assert!(near(m.map_time(3.0), 2.7));
    // Seg 2 moves by both overlaps.
    assert!(near(m.map_time(6.0), 5.2));
    assert!(near(m.map_time(7.0), 6.2), "end maps to the rendered end");
    // Monotone over a fine sweep.
    let mut prev = -1.0;
    for i in 0..=700 {
        let t = m.map_time(i as f64 * 0.01);
        assert!(t >= prev - 1e-12, "not monotone at {i}");
        prev = t;
    }
    assert!(m.map_interval(2.02, 2.12).is_none(), "word hidden in the transition collapses");
    assert_eq!(m.map_interval(3.0, 3.5), Some((2.7, 3.2)));

    // Cut / unknown segment / after the last segment: no overlap.
    let none = TransitionMap::new(
        &fx.clip,
        &[
            tr(&fx, 0, TransitionKind::Cut, 0.3),
            tr(&fx, 2, TransitionKind::Dissolve, 0.3),
            Transition { after_segment_id: SegmentId::new(), kind: TransitionKind::Fade, duration: 0.3 },
        ],
    );
    assert!(none.is_identity());
    assert_eq!(none.map_time(4.2), 4.2);
}

#[test]
fn transitions_compile_to_frame_exact_xfade_and_acrossfade() {
    let fx = fixture(&[(0.0, 2.0), (10.0, 13.0), (20.0, 21.0), (30.0, 32.0)]);
    // seg0 -> seg1 dissolve 0.3 (9 frames), seg1|seg2 hard cut, seg2 -> seg3 wipeleft 0.5 (15 frames).
    let x = CompileExtras {
        transitions: vec![tr(&fx, 0, TransitionKind::Dissolve, 0.3), tr(&fx, 2, TransitionKind::Wipeleft, 0.5)],
        ..Default::default()
    };
    let plan = build_graph(&fx.clip, &fx.sources, false, &x).unwrap();
    let g = &plan.graph;
    // Group 1 = seg1 + seg2 concatenated.
    assert!(g.contains("[v1][a1][v2][a2]concat=n=2:v=1:a=1[g1vc][g1ac]"), "{g}");
    assert!(g.contains("[v0]settb=1/30,setpts=N[g0v]"), "{g}");
    assert!(g.contains("[a0]asetpts=N/SR/TB[g0a]"), "{g}");
    // Group 0 = 60 frames; offset 51 frames = 1.7 s.
    assert!(g.contains("[g0v][g1v]xfade=transition=fade:duration=0.3:offset=1.7[x1v]"), "{g}");
    assert!(g.contains("[g0a][g1a]acrossfade=ns=14400:c1=tri:c2=tri[x1a]"), "{g}");
    // Accumulated 51 + 120 = 171 frames; offset 171 - 15 = 156 frames = 5.2 s.
    assert!(g.contains("[x1v][g2v]xfade=transition=wipeleft:duration=0.5:offset=5.2[vcat]"), "{g}");
    assert!(g.contains("[x1a][g2a]acrossfade=ns=24000:c1=tri:c2=tri[acat]"), "{g}");
    assert!(!g.contains("concat=n=4"), "{g}");
    assert!((plan.duration - (8.0 - 0.8)).abs() < 1e-9, "{}", plan.duration);
    // The loudness pass runs the same audio join.
    let lg = plan.loudness_graph.unwrap();
    assert!(lg.contains("[a1][a2]concat=n=2:v=0:a=1[g1ac]"), "{lg}");
    assert!(lg.contains("acrossfade=ns=24000:c1=tri:c2=tri[acat]"), "{lg}");
    assert!(!lg.contains("xfade="), "{lg}");

    // Fade = dip to black.
    let x = CompileExtras { transitions: vec![tr(&fx, 1, TransitionKind::Fade, 0.2)], ..Default::default() };
    assert!(graph(&fx, &x).contains("xfade=transition=fadeblack:duration=0.2:"));

    // No transitions: the single concat stays.
    assert!(graph(&fx, &CompileExtras::default()).contains("concat=n=4:v=1:a=1[vcat][acat]"));
}

#[test]
fn transition_longer_than_its_segment_is_rejected() {
    let fx = fixture(&[(0.0, 2.0), (10.0, 10.5), (20.0, 22.0)]);
    // 0.3 + 0.3 s around a 0.5 s segment.
    let x = CompileExtras {
        transitions: vec![tr(&fx, 0, TransitionKind::Dissolve, 0.3), tr(&fx, 1, TransitionKind::Dissolve, 0.3)],
        ..Default::default()
    };
    let e = build_graph(&fx.clip, &fx.sources, false, &x).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("transition"));
}

#[test]
fn overlays_and_effects_follow_the_transition_map() {
    let fx = fixture(&[(0.0, 2.0), (10.0, 14.0)]);
    let x = CompileExtras {
        transitions: vec![tr(&fx, 0, TransitionKind::Dissolve, 0.4)],
        overlays: vec![TextOverlay::new("ovl_1", "Later", 3.0, 4.0, OverlayStyle::HookTop)],
        effects: vec![Effect { id: "fx_1".into(), kind: EffectKind::PunchIn, start: 3.0, end: 4.0, amount: None }],
        ..Default::default()
    };
    let plan = build_graph(&fx.clip, &fx.sources, false, &x).unwrap();
    // 3.0 -> 2.6 and 4.0 -> 3.6 rendered.
    let ass = plan.overlays_ass.unwrap();
    assert!(ass.contains("Dialogue: 0,0:00:02.60,0:00:03.60,"), "{ass}");
    // Frames 78..107 inclusive.
    assert!(plan.graph.contains("enable='between(n,78,107)'"), "{}", plan.graph);
}

#[test]
fn effects_compile_to_perspective_and_fade_before_captions() {
    let fx = fixture(&[(0.0, 6.0)]);
    let fxs = vec![
        Effect { id: "fx_1".into(), kind: EffectKind::PunchIn, start: 0.5, end: 1.5, amount: Some(2.0) },
        Effect { id: "fx_2".into(), kind: EffectKind::ZoomSlow, start: 2.0, end: 4.0, amount: None },
        Effect { id: "fx_3".into(), kind: EffectKind::Shake, start: 4.0, end: 4.5, amount: None },
        Effect { id: "fx_4".into(), kind: EffectKind::Flash, start: 5.0, end: 5.2, amount: None },
    ];
    let x = CompileExtras { effects: fxs, ..Default::default() };
    let g = build_graph(&fx.clip, &fx.sources, true, &x).unwrap().graph;
    // Punch-in 2.0: the centre half of the frame.
    assert!(
        g.contains("perspective=x0='W*((0.5-0.5/(2))+0)':y0='H*((0.5-0.5/(2))+0)':x1='W*((0.5+0.5/(2))+0)'"),
        "{g}"
    );
    assert!(g.contains("interpolation=linear:enable='between(n,15,44)'"), "{g}");
    // Zoom-slow: linear 1 -> 1.15 over frames 60..120 (perspective's `in` is frame index + 1).
    assert!(g.contains("1+0.15*clip((in-1-60)/60,0,1)"), "{g}");
    assert!(g.contains("eval=frame:enable='between(n,60,119)'"), "{g}");
    // Shake: zoom 1.04, sinusoidal offsets.
    assert!(g.contains("0.5/(1.04)") && g.contains("*sin("), "{g}");
    // Flash: white fade-in starting at frame 150 for 6 frames.
    assert!(g.contains("fade=t=in:st=5:d=0.2:color=white:enable='between(n,150,155)'"), "{g}");
    // Effects come before the captions (captions are never zoomed).
    let (p, a) = (g.find("perspective=").unwrap(), g.find("ass=captions.ass").unwrap());
    assert!(p < a);

    // Invalid amount.
    let bad = CompileExtras {
        effects: vec![Effect { id: "fx_9".into(), kind: EffectKind::PunchIn, start: 0.0, end: 1.0, amount: Some(5.0) }],
        ..Default::default()
    };
    assert_eq!(build_graph(&fx.clip, &fx.sources, false, &bad).unwrap_err().code, ErrorCode::Validation);
}

#[test]
fn progress_bar_overlays_a_sliding_color_strip() {
    let fx = fixture(&[(0.0, 4.0)]);
    let x = CompileExtras {
        progress_bar: Some(ProgressBar { color: "#ff0000".into(), height: 9, position: BarPosition::Bottom }),
        ..Default::default()
    };
    let g = graph(&fx, &x);
    assert!(g.contains("color=c=0xFF0000:s=1080x10:r=30,trim=end_frame=120,format=yuv420p[pbar]"), "{g}");
    assert!(g.contains("[vbar][pbar]overlay=x='W*(n+1)/120-W':y=1910:eval=frame,format=yuv420p[vout]"), "{g}");
    let top = CompileExtras { progress_bar: Some(ProgressBar::default()), ..Default::default() };
    assert!(graph(&fx, &top).contains("color=c=0xFFE000:s=1080x8:") && graph(&fx, &top).contains(":y=0:"));
    assert_eq!(ffmpeg_color("#abc"), None);
    assert_eq!(ffmpeg_color("ffe000").as_deref(), Some("0xFFE000"));
    let bad = CompileExtras {
        progress_bar: Some(ProgressBar { color: "yellow".into(), ..Default::default() }),
        ..Default::default()
    };
    assert_eq!(build_graph(&fx.clip, &fx.sources, false, &bad).unwrap_err().code, ErrorCode::Validation);
}

#[test]
fn denoise_and_gains_sit_before_the_mix_and_loudnorm() {
    let fx = fixture(&[(0.0, 2.0), (5.0, 7.0)]);
    let mut gains = HashMap::new();
    gains.insert(fx.clip.segments[1].id.clone(), -6.0);
    let x = CompileExtras {
        denoise: Denoise::Medium,
        segment_gain_db: gains,
        speech_gain_db: 2.5,
        ..Default::default()
    };
    let plan = build_graph(&fx.clip, &fx.sources, false, &x).unwrap();
    let g = &plan.graph;
    assert!(g.contains("channel_layouts=stereo,volume=-6dB,apad,"), "{g}");
    assert_eq!(g.matches("volume=-6dB").count(), 1, "only segment 1 gets the gain");
    assert!(g.contains("[acat]highpass=f=80,afftdn=nr=20:nf=-42,volume=2.5dB[speech0]"), "{g}");
    assert!(g.contains("[speech0]loudnorm=I=-14"), "{g}");
    // The measurement pass hears the same cleaned dialogue.
    assert!(plan.loudness_graph.unwrap().contains("[acat]highpass=f=80,afftdn=nr=20:nf=-42"));
    assert_eq!(plan.denoise_filter.as_deref(), Some("highpass=f=80,afftdn=nr=20:nf=-42"));
    let ng = plan.noise_graph.unwrap();
    assert!(ng.contains("[acat]highpass=f=80,asetnsamples=n=2400:p=0,astats=metadata=1:reset=1"), "{ng}");
    assert!(ng.contains("ametadata=mode=print:key=lavfi.astats.Overall.RMS_level[aout]"), "{ng}");
    assert!(!ng.contains("afftdn"), "the floor is measured before denoise");
    // nf = measured floor + strength margin.
    assert!(Denoise::Off.filter().is_none());
    assert_eq!(Denoise::Light.filter_for(-38.7).unwrap(), "highpass=f=80,afftdn=nr=12:nf=-34.7");
    assert_eq!(Denoise::Strong.filter_for(-38.7).unwrap(), "highpass=f=80,afftdn=nr=30:nf=-26.7");
    assert_eq!(Denoise::Strong.filter_for(-10.0).unwrap(), "highpass=f=80,afftdn=nr=30:nf=-20");
    let log: String = (0..20)
        .map(|i| format!("[Parsed_ametadata_3 @ 0] lavfi.astats.Overall.RMS_level={}\n", -20.0 - i as f64))
        .chain(["lavfi.astats.Overall.RMS_level=-inf\n".to_string()])
        .collect();
    // p10 = -37, but the median is -29: the floor stays 12 dB under it.
    assert_eq!(noise_floor_from_log(&log), Some(-41.0));
    let quiet_gaps: String = (0..20).map(|i| format!("RMS_level={}\n", if i < 6 { -60.0 } else { -25.0 })).collect::<String>().replace("RMS_level", "lavfi.astats.Overall.RMS_level");
    assert_eq!(noise_floor_from_log(&quiet_gaps), Some(-60.0), "pauses give the floor");
    let tone: String = (0..20).map(|_| "lavfi.astats.Overall.RMS_level=-27\n").collect();
    assert_eq!(noise_floor_from_log(&tone), Some(-39.0), "no pauses: never the signal itself");
    assert_eq!(noise_floor_from_log("nothing"), None);

    // The noise pass reads only the segment inputs.
    let tmp = tempfile::tempdir().unwrap();
    let x = CompileExtras { denoise: Denoise::Light, music: Some(MusicBed::new("/m/bed.mp3")), ..Default::default() };
    let c = compile_with(
        &fx.clip,
        &fx.sources,
        None,
        tmp.path(),
        &encoders::choice("libx264", RenderQuality::Draft),
        &tmp.path().join("o.mp4"),
        &tmp.path().join("w"),
        &x,
    )
    .unwrap();
    let np = c.noise.unwrap();
    let a: Vec<String> = np.args.iter().map(|s| s.to_string_lossy().into_owned()).collect();
    assert!(!a.iter().any(|s| s.ends_with("bed.mp3")), "{a:?}");
    assert_eq!(a.iter().filter(|s| *s == "-i").count(), 2);
    assert!(tmp.path().join("w").join("graph_noise.txt").is_file());
    assert_eq!(np.filter, "highpass=f=80,afftdn=nr=12:nf=-46");
}

#[test]
fn sfx_inputs_follow_the_music_and_skip_the_sidechain() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = fixture(&[(0.0, 2.0), (10.0, 13.0)]);
    let x = CompileExtras {
        music: Some(MusicBed::new("/music/bed.mp3")),
        transitions: vec![tr(&fx, 0, TransitionKind::Dissolve, 0.3)],
        sfx: vec![
            SfxCue { id: "sfx_1".into(), path: "/sfx/whoosh.wav".into(), at: 3.0, volume_db: -6.0 },
            // Past the end: dropped (no input).
            SfxCue { id: "sfx_2".into(), path: "/sfx/late.wav".into(), at: 9.0, volume_db: -6.0 },
        ],
        ..Default::default()
    };
    let plan = build_graph(&fx.clip, &fx.sources, false, &x).unwrap();
    assert_eq!(plan.sfx_inputs, vec![PathBuf::from("/sfx/whoosh.wav")]);
    let g = &plan.graph;
    // Input 2 + 1 music = index 3; at 3.0 edit -> 2.7 s rendered = 129600 samples.
    assert!(g.contains("[3:a:0]aresample=48000,"), "{g}");
    assert!(g.contains("volume=-6dB,adelay=delays=129600S:all=1,apad,atrim=end_sample=225600"), "{g}");
    assert!(g.contains("[acat]asplit=2[speech][sc]"), "{g}");
    assert!(g.contains("[speech][duck][sfx0]amix=inputs=3:duration=first"), "{g}");

    let c = compile_with(
        &fx.clip,
        &fx.sources,
        None,
        tmp.path(),
        &encoders::choice("libx264", RenderQuality::Draft),
        &tmp.path().join("o.mp4"),
        &tmp.path().join("w"),
        &x,
    )
    .unwrap();
    let a: Vec<String> = c.args.iter().map(|s| s.to_string_lossy().into_owned()).collect();
    let i_music = a.iter().position(|s| s.ends_with("bed.mp3")).unwrap();
    let i_sfx = a.iter().position(|s| s.ends_with("whoosh.wav")).unwrap();
    assert!(i_sfx > i_music && a[i_sfx - 1] == "-i");
    assert!(!a.iter().any(|s| s.ends_with("late.wav")));
    // The loudness pass reads the same inputs.
    let la: Vec<String> = c.loudness.unwrap().args.iter().map(|s| s.to_string_lossy().into_owned()).collect();
    assert!(la.iter().any(|s| s.ends_with("whoosh.wav")));

    // Without music: speech + sfx mixed directly.
    let x = CompileExtras {
        sfx: vec![SfxCue { id: "sfx_1".into(), path: "/sfx/b.wav".into(), at: 1.0, volume_db: 0.0 }],
        ..Default::default()
    };
    let g = graph(&fx, &x);
    assert!(g.contains("[2:a:0]aresample") && g.contains("[acat][sfx0]amix=inputs=2:"), "{g}");
}
