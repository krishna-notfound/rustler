//! Task 1 contract tests: ids, error codes, defaults, JSON shape, schema generation.

use chrono::Utc;
use rustler_core::*;
use std::path::PathBuf;

#[test]
fn ids_have_prefixes() {
    fn check(s: String, prefix: &str) {
        assert!(s.starts_with(prefix), "{s} should start with {prefix}");
        assert_eq!(s.len(), 30, "{s} should be 30 chars");
        assert_eq!(s, s.to_lowercase(), "{s} should be lowercase");
    }
    check(SourceId::new().to_string(), "src_");
    check(ClipId::new().to_string(), "clp_");
    check(RenderId::new().to_string(), "rnd_");
    check(ProjectId::new().to_string(), "prj_");
    check(SegmentId::new().to_string(), "seg_");
}

#[test]
fn ids_serialize_as_plain_strings_and_parse() {
    let id = SourceId::new();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{id}\""));
    let back: SourceId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
    let parsed: SourceId = id.to_string().parse().unwrap();
    assert_eq!(parsed, id);
    let wrong: std::result::Result<SourceId, _> = "clp_abc".parse();
    assert_eq!(wrong.unwrap_err().code, ErrorCode::Validation);
}

#[test]
fn error_exit_codes() {
    assert_eq!(ErrorCode::Usage.exit_code(), 2);
    assert_eq!(ErrorCode::Validation.exit_code(), 2);
    assert_eq!(ErrorCode::NotFound.exit_code(), 3);
    assert_eq!(ErrorCode::DependencyMissing.exit_code(), 4);
    assert_eq!(ErrorCode::Provider.exit_code(), 5);
    assert_eq!(ErrorCode::Media.exit_code(), 6);
    assert_eq!(ErrorCode::QcFailed.exit_code(), 7);
    assert_eq!(ErrorCode::Cancelled.exit_code(), 8);
    assert_eq!(ErrorCode::RevisionConflict.exit_code(), 9);
    assert_eq!(ErrorCode::Internal.exit_code(), 10);
}

#[test]
fn error_builder_and_json() {
    let e = RustlerError::new(ErrorCode::NotFound, "source.show", "no such source")
        .hint("run `rustler source list`")
        .retryable();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(e.stage, "source.show");
    assert_eq!(e.hint.as_deref(), Some("run `rustler source list`"));
    assert!(e.retryable);
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["code"], "not_found");
    assert_eq!(v["retryable"], true);
    assert_eq!(e.to_string(), "NotFound: no such source");
}

#[test]
fn defaults_match_spec() {
    let c = Canvas::default();
    assert_eq!((c.width, c.height, c.fps), (1080, 1920, 30.0));

    let a = AudioSettings::default();
    assert!(a.loudnorm);
    assert_eq!((a.target_lufs, a.true_peak, a.lra), (-14.0, -2.0, 11.0));

    let t = CaptionTrack::default();
    assert!(t.enabled);
    assert_eq!(t.preset, "bold-karaoke");
    assert_eq!(t.script, CaptionScript::Latin);
    assert_eq!(t.position_y, 0.70);
    assert_eq!(t.max_words_per_line, 3);

    let p = Permissions::default();
    assert!(p.network && p.downloads);
    assert!(p.fs_roots.is_empty());
    assert!(p.cookies_file.is_none());
    assert_eq!(p.max_spend_usd, 0.0);

    let s = ProjectSettings::default();
    assert_eq!(s.canvas, Canvas::default());
    assert_eq!(s.caption_script, CaptionScript::Latin);
    assert_eq!(s.default_whisper_model, "small");
}

fn sample_clip() -> Clip {
    let src = SourceId::new();
    let layout = Layout {
        preset: Some("split-vertical".into()),
        regions: vec![
            Region {
                id: "top".into(),
                src: Rect {
                    x: 0.75,
                    y: 0.0,
                    w: 0.25,
                    h: 0.33,
                },
                dst: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 0.5,
                },
                z: 0,
                fit: Fit::Cover,
            },
            Region {
                id: "bottom".into(),
                src: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.75,
                    h: 1.0,
                },
                dst: Rect {
                    x: 0.0,
                    y: 0.5,
                    w: 1.0,
                    h: 0.5,
                },
                z: 1,
                fit: Fit::Contain,
            },
        ],
        background: Background::BlurFill,
    };
    Clip {
        schema: "rustler.clip@1".into(),
        id: ClipId::new(),
        name: "test clip".into(),
        revision: 3,
        canvas: Canvas::default(),
        segments: vec![Segment {
            id: SegmentId::new(),
            source_id: src,
            start: 10.0,
            end: 20.5,
            layout,
        }],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![RenderRecord {
            id: RenderId::new(),
            path: PathBuf::from("clips/x/renders/y.mp4"),
            quality: RenderQuality::Final,
            encoder: "libx264".into(),
            created_at: Utc::now(),
            qc: None,
        }],
        created_at: Utc::now(),
    }
}

#[test]
fn clip_roundtrips_json() {
    let clip = sample_clip();
    let json = serde_json::to_string(&clip).unwrap();
    let back: Clip = serde_json::from_str(&json).unwrap();
    assert_eq!(back, clip);
    assert!(json.contains(r#""script":"latin""#), "{json}");
    assert!(json.contains(r#""background":"blur_fill""#), "{json}");
    assert!(json.contains(r#""fit":"cover""#), "{json}");
    assert!(json.contains(r#""quality":"final""#), "{json}");

    // SourceOrigin is internally tagged by "kind".
    let source = Source {
        id: SourceId::new(),
        name: "podcast".into(),
        origin: SourceOrigin::Url {
            url: "https://www.youtube.com/watch?v=x".into(),
            section: Some(TimeRange {
                start: 60.0,
                end: 120.0,
            }),
            original_offset: 60.0,
        },
        media_path: PathBuf::from("sources/src_x/media.mp4"),
        probe: MediaProbe {
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
        },
        proxy: None,
        audio_wav: None,
        imported_at: Utc::now(),
    };
    let sjson = serde_json::to_string(&source).unwrap();
    assert!(sjson.contains(r#""kind":"url""#), "{sjson}");
    let sback: Source = serde_json::from_str(&sjson).unwrap();
    assert_eq!(sback, source);

    let file = SourceOrigin::File {
        path: PathBuf::from("a.mp4"),
    };
    assert_eq!(serde_json::to_value(&file).unwrap()["kind"], "file");
}

#[test]
fn transcript_and_qc_shapes() {
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: SourceId::new(),
        language: "en".into(),
        engine: "faster-whisper:small".into(),
        script: CaptionScript::AsTranscribed,
        words: vec![Word {
            i: 0,
            text: "hi".into(),
            start: 0.0,
            end: 0.4,
            conf: Some(0.9),
            speaker: None,
        }],
        word_timing: WordTiming::Approximate,
    };
    let v = serde_json::to_value(&t).unwrap();
    assert_eq!(v["script"], "as_transcribed");
    assert_eq!(v["word_timing"], "approximate");
    let back: Transcript = serde_json::from_value(v).unwrap();
    assert_eq!(back, t);

    let u = Unit {
        id: 0,
        kind: UnitKind::Clause,
        word_start: 0,
        word_end: 0,
        start: 0.0,
        end: 0.4,
        text: "hi".into(),
        boundary_strength: 0.5,
    };
    assert_eq!(serde_json::to_value(&u).unwrap()["kind"], "clause");

    let q = QcReport {
        schema: "rustler.qc@1".into(),
        clip_id: ClipId::new(),
        render_id: RenderId::new(),
        passed: false,
        checks: vec![QcCheck {
            name: "duration".into(),
            status: QcStatus::Fail,
            value: serde_json::json!(12.0),
            expected: "10.0 ± 0.10".into(),
            fix_hint: None,
        }],
        contact_sheet: PathBuf::from("qc/r.jpg"),
    };
    assert_eq!(
        serde_json::to_value(&q).unwrap()["checks"][0]["status"],
        "fail"
    );

    let e = Event {
        ts: Utc::now(),
        actor: Actor::Agent,
        kind: "clip.create".into(),
        project_revision: 1,
        data: serde_json::json!({}),
    };
    assert_eq!(serde_json::to_value(&e).unwrap()["actor"], "agent");

    let ae = AudioEvents {
        curve: vec![LoudnessPoint {
            t: 0.0,
            lufs: -20.0,
        }],
        peaks: vec![],
        silences: vec![Silence {
            start: 1.0,
            end: 2.0,
        }],
    };
    let _ = serde_json::to_string(&ae).unwrap();
    let _ = serde_json::to_string(&Scene {
        start: 0.0,
        end: 1.0,
    })
    .unwrap();
}

#[test]
fn schema_generates() {
    let schema = schemars::schema_for!(Clip);
    let s = serde_json::to_string(&schema).unwrap();
    assert!(s.contains("\"segments\""), "{s}");
    let _ = schemars::schema_for!(Project);
    let _ = schemars::schema_for!(Source);
    let _ = schemars::schema_for!(Transcript);
    let _ = schemars::schema_for!(QcReport);
    let _ = schemars::schema_for!(Event);
}
