//! Colour grade in the compiled graph (f38): per segment, on the source picture before layout
//! scaling / composition and before captions; neutral = no filter; LUTs staged by name.

use rustler_core::{AudioSettings, Background, CaptionTrack, Clip, ClipId, Fit, Layout, MediaProbe, Rect, Region, Segment, SegmentId, SourceId};
use rustler_media::compile::{CompileExtras, SourceMedia, Transition, TransitionKind, build_graph};
use rustler_media::grade::{Grade, cube_from_fn};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

fn probe() -> MediaProbe {
    MediaProbe {
        duration: 60.0,
        has_video: true,
        has_audio: true,
        width: 1920,
        height: 1080,
        rotation: 0,
        fps: 30.0,
        video_codec: Some("h264".into()),
        audio_codec: Some("aac".into()),
        sample_rate: Some(48000),
        channels: Some(2),
    }
}

fn region(src: Rect, dst: Rect, z: i32, fit: Fit) -> Region {
    Region { id: format!("r{z}"), src, dst, z, fit }
}

fn full() -> Rect {
    Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
}

fn layouts() -> Vec<Layout> {
    let w = 1080.0 * 9.0 / 16.0 / 1920.0;
    vec![
        Layout { preset: Some("center-crop".into()), regions: vec![region(Rect { x: (1.0 - w) / 2.0, y: 0.0, w, h: 1.0 }, full(), 0, Fit::Cover)], background: Background::Black },
        Layout { preset: Some("blur-fit".into()), regions: vec![region(full(), full(), 0, Fit::Contain)], background: Background::BlurFill },
        Layout {
            preset: Some("split-vertical".into()),
            regions: vec![
                region(Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }, Rect { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }, 0, Fit::Cover),
                region(Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }, Rect { x: 0.0, y: 0.5, w: 1.0, h: 0.5 }, 1, Fit::Cover),
            ],
            background: Background::Black,
        },
    ]
}

fn clip_with(layout: Layout, n: usize) -> (Clip, HashMap<SourceId, SourceMedia>) {
    let src = SourceId::new();
    let segments: Vec<Segment> = (0..n)
        .map(|k| Segment { id: SegmentId::new(), source_id: src.clone(), start: 2.0 * k as f64, end: 2.0 * k as f64 + 2.0, layout: layout.clone() })
        .collect();
    let clip: Clip = serde_json::from_value(json!({
        "schema": "rustler.clip@1", "id": ClipId::new(), "name": "g", "revision": 0,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "segments": segments, "captions": CaptionTrack::default(), "audio": AudioSettings::default(),
        "renders": [], "created_at": "2026-11-15T00:00:00Z"
    }))
    .unwrap();
    let mut sources = HashMap::new();
    sources.insert(src.clone(), SourceMedia { id: src, path: PathBuf::from("/media/in.mp4"), probe: probe() });
    (clip, sources)
}

#[test]
fn neutral_grade_adds_no_filter() {
    let (clip, sources) = clip_with(layouts().remove(0), 1);
    let plain = build_graph(&clip, &sources, false, &CompileExtras::default()).unwrap().graph;
    let mut x = CompileExtras::default();
    x.grades.insert(clip.segments[0].id.clone(), Grade { exposure: Some(0.0), intensity: Some(1.0), ..Default::default() });
    assert_eq!(build_graph(&clip, &sources, false, &x).unwrap().graph, plain);
}

#[test]
fn grade_sits_on_the_source_picture_for_every_layout() {
    for layout in layouts() {
        let name = layout.preset.clone().unwrap();
        let (clip, sources) = clip_with(layout, 1);
        let mut x = CompileExtras::default();
        x.grades.insert(clip.segments[0].id.clone(), Grade { preset: Some("warm".into()), ..Default::default() });
        let g = build_graph(&clip, &sources, true, &x).unwrap().graph;
        // Right after the segment's trim, before any split / crop / scale / blur.
        let at = g.find("setpts=PTS-STARTPTS,colortemperature=temperature=5500:pl=1,vibrance=intensity=0.1").unwrap_or_else(|| panic!("{name}: {g}"));
        for later in ["crop=", "scale=", "gblur", "subtitles", "ass="] {
            if let Some(p) = g.find(later) {
                assert!(p > at, "{name}: {later} before the grade: {g}");
            }
        }
        assert_eq!(g.matches("colortemperature").count(), 1, "{name}: graded once, shared by every region / the blur background");
    }
}

#[test]
fn per_segment_grades_luts_and_transitions() {
    let (clip, sources) = clip_with(layouts().remove(0), 3);
    let tmp = tempfile::tempdir().unwrap();
    let lut = tmp.path().join("look.cube");
    std::fs::write(&lut, cube_from_fn("t", 5, |r, g, b| (r, g, b * 0.9))).unwrap();
    let mut x = CompileExtras::default();
    let lut_s = lut.to_string_lossy().into_owned();
    x.grades.insert(clip.segments[0].id.clone(), Grade { lut: Some(lut_s.clone()), lut_strength: Some(0.5), ..Default::default() });
    x.grades.insert(clip.segments[1].id.clone(), Grade { preset: Some("bw".into()), lut: Some(lut_s), ..Default::default() });
    x.transitions = vec![Transition { after_segment_id: clip.segments[0].id.clone(), kind: TransitionKind::Dissolve, duration: 0.3 }];
    let plan = build_graph(&clip, &sources, false, &x).unwrap();
    let g = &plan.graph;
    // One staged copy of the LUT for both segments.
    assert_eq!(plan.lut_files, vec![("grade_lut0.cube".to_string(), lut.clone())]);
    assert!(g.contains("[s0ga][s0gb];[s0gb]lut3d=file='grade_lut0.cube':interp=tetrahedral[s0gc];[s0ga][s0gc]blend=all_mode=normal:all_opacity=0.5"), "{g}");
    assert!(g.contains("colorchannelmixer=rr=0.2126") && g.contains(",lut3d=file='grade_lut0.cube':interp=tetrahedral[s1in0]"), "{g}");
    // Segment 3 is untouched; the dissolve blends the two graded segment outputs.
    assert!(g.contains("[2:v:0]fps=30:start_time=0:round=near,tpad=stop_mode=clone:stop_duration=1,trim=end_frame=60,setpts=PTS-STARTPTS[s2in0]"), "{g}");
    assert!(g.contains("xfade=transition=fade"), "{g}");
    // A bad grade is a validation error naming the segment.
    x.grades.insert(clip.segments[2].id.clone(), Grade { contrast: Some(9.0), ..Default::default() });
    let e = build_graph(&clip, &sources, false, &x).unwrap_err();
    assert!(e.message.contains(clip.segments[2].id.as_str()) && e.message.contains("contrast"), "{}", e.message);
}
