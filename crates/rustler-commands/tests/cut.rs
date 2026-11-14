//! edit.refine on a synthetic clip (no ffmpeg needed).

use chrono::Utc;
use rustler_commands::cut_cmds;
use rustler_core::layout::{self, LayoutParams};
use rustler_core::model::{CaptionScript, WordTiming};
use rustler_core::units::build_units;
use rustler_core::{
    AudioEvents, AudioSettings, Canvas, CaptionTrack, Clip, ClipId, ErrorCode, MediaProbe, ProjectStore, Segment, SegmentId, Silence,
    Source, SourceId, SourceOrigin, Transcript, Word,
};
use rustler_registry::{Ctx, Registry};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    reg: Registry,
    clip_id: ClipId,
    source_id: SourceId,
}

fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
    Word { i, text: text.to_string(), start, end, conf: None, speaker: None }
}

/// Words:
///   0 So 10.00-10.20 | 1 yeah 10.25-10.50 | (gap .4) 2 the 10.90-11.00 | 3 plan 11.05-11.30 | 4 works. 11.35-11.90
///   (gap .9) 5 We 12.80-12.95 | 6 ship 13.00-13.30 | 7 tomorrow 13.35-13.90 | 8 morning 13.95-14.40 | 9 again. 14.45-14.90
///   (gap 1.1) 10 Nobody 16.00-16.40 | 11 expected. 16.45-17.00
/// The clip has one segment `[seg_start, seg_end)`.
fn fixture(seg_start: f64, seg_end: f64, with_silence: bool, with_transcript: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (store, _) = ProjectStore::create(&root, "t").unwrap();
    let source_id = SourceId::new();
    let source = Source {
        id: source_id.clone(),
        name: "synthetic".into(),
        origin: SourceOrigin::File { path: PathBuf::from("/nonexistent/synthetic.mp4") },
        media_path: PathBuf::from("/nonexistent/synthetic.mp4"),
        probe: MediaProbe { duration: 60.0, has_video: true, has_audio: true, width: 1920, height: 1080, fps: 30.0, ..Default::default() },
        proxy: None,
        audio_wav: None,
        imported_at: Utc::now(),
    };
    store.save_source(&source).unwrap();
    let rel = Path::new("analysis").join(source_id.as_str());
    if with_transcript {
        let words = vec![
            w(0, "So", 10.00, 10.20),
            w(1, "yeah", 10.25, 10.50),
            w(2, "the", 10.90, 11.00),
            w(3, "plan", 11.05, 11.30),
            w(4, "works.", 11.35, 11.90),
            w(5, "We", 12.80, 12.95),
            w(6, "ship", 13.00, 13.30),
            w(7, "tomorrow", 13.35, 13.90),
            w(8, "morning", 13.95, 14.40),
            w(9, "again.", 14.45, 14.90),
            w(10, "Nobody", 16.00, 16.40),
            w(11, "expected.", 16.45, 17.00),
        ];
        let units = build_units(&words);
        let t = Transcript {
            schema: "rustler.transcript@1".into(),
            source_id: source_id.clone(),
            language: "en".into(),
            engine: "test".into(),
            script: CaptionScript::Latin,
            words,
            word_timing: WordTiming::Exact,
        };
        store.write_json(&rel.join("transcript.json"), &t).unwrap();
        store.write_json(&rel.join("units.json"), &units).unwrap();
    }
    if with_silence {
        let ev = AudioEvents {
            curve: vec![],
            peaks: vec![],
            silences: vec![Silence { start: 10.60, end: 10.85 }, Silence { start: 14.55, end: 15.90 }],
        };
        store.write_json(&rel.join("audio_events.json"), &ev).unwrap();
    }
    let canvas = Canvas::default();
    let layout = layout::preset("center-crop", 1920, 1080, &canvas, &LayoutParams::default()).unwrap();
    let mut clip = Clip {
        schema: "rustler.clip@1".into(),
        id: ClipId::new(),
        name: "c".into(),
        revision: 0,
        canvas,
        segments: vec![Segment { id: SegmentId::new(), source_id: source_id.clone(), start: seg_start, end: seg_end, layout }],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![],
        created_at: Utc::now(),
    };
    store.save_clip(&mut clip, 0).unwrap();
    let mut reg = Registry::new();
    cut_cmds::register(&mut reg);
    Fx { _dir: dir, root, reg, clip_id: clip.id.clone(), source_id }
}

impl Fx {
    fn ctx(&self) -> Ctx {
        Ctx::new(Some(self.root.clone()), self.root.join("home"))
    }
    async fn run(&self, input: Value) -> rustler_core::Result<Value> {
        self.reg.dispatch("edit.refine", input, self.ctx()).await
    }
    async fn run_dry(&self, input: Value) -> rustler_core::Result<Value> {
        let mut ctx = self.ctx();
        ctx.dry_run = true;
        self.reg.dispatch("edit.refine", input, ctx).await
    }
    fn clip(&self) -> Clip {
        ProjectStore::open(&self.root).unwrap().load_clip(&self.clip_id).unwrap()
    }
    fn id(&self) -> String {
        self.clip_id.to_string()
    }
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

fn after(v: &Value) -> (f64, f64) {
    let a = &v["segments"][0]["after"];
    (a["start"].as_f64().unwrap(), a["end"].as_f64().unwrap())
}

#[tokio::test]
async fn skips_opener_and_extends_to_sentence_end_using_word_gaps() {
    let fx = fixture(9.9, 14.0, false, true);
    let v = fx.run(json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(v["changed"], true);
    let (s, e) = after(&v);
    // "So yeah" skipped: start between the end of "yeah" (10.50) and "the" (10.90): word gap, lead 0.15
    approx(s, 10.75);
    // "morning" straddles 14.0 -> extend to the sentence end "again." (14.90) + 0.25 s tail in the 1.1 s gap
    approx(e, 15.15);
    let seg = &v["segments"][0];
    assert_eq!(seg["opening_before"].as_str().unwrap().split(' ').next(), Some("So"));
    assert!(seg["opening_after"].as_str().unwrap().starts_with("the plan works."), "{}", seg["opening_after"]);
    assert!(seg["closing_after"].as_str().unwrap().ends_with("again."));
    let reasons = seg["reasons"].to_string();
    assert!(reasons.contains("skipped weak opener 'So yeah'"), "{reasons}");
    assert!(reasons.contains("extended to the sentence end"), "{reasons}");
    // saved with a new revision
    assert_eq!(v["revision"], 2);
    let c = fx.clip();
    assert_eq!(c.revision, 2);
    approx(c.segments[0].start, 10.75);
    approx(c.segments[0].end, 15.15);
    assert!(v["warnings"].to_string().contains("analyze audio"), "no audio_events warning");
    // idempotent
    let again = fx.run(json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(again["changed"], false, "{again}");
    assert_eq!(fx.clip().revision, 2);
}

#[tokio::test]
async fn aligns_to_silence_troughs() {
    let fx = fixture(9.9, 14.0, true, true);
    let v = fx.run(json!({"clip_id": fx.id()})).await.unwrap();
    let (s, e) = after(&v);
    // start silence 10.60-10.85 (0.25 s): lead = min(0.15, 0.125) -> 10.85 - 0.125
    approx(s, 10.725);
    // end silence 14.55-15.90 clipped to the word gap after "again." (14.90..15.90): tail 0.25
    approx(e, 15.15);
    assert!(v["segments"][0]["reasons"].to_string().contains("silence trough"));
    assert!(v["warnings"].as_array().unwrap().is_empty(), "{}", v["warnings"]);
}

#[tokio::test]
async fn pulls_back_when_the_previous_sentence_end_is_closer() {
    // end 13.2: last word starting before it is "ship" (13.00), not sentence-final.
    // forward "again." is 1.7 s away, back "works." (11.90) is 1.3 s away.
    let fx = fixture(10.9, 13.2, false, true);
    let v = fx.run(json!({"clip_id": fx.id(), "max_shift": 2.0})).await.unwrap();
    let (_, e) = after(&v);
    assert!(v["segments"][0]["reasons"].to_string().contains("pulled back"), "{v}");
    // "works." ends 11.90, next word 12.80: gap 0.9 -> tail 0.25
    approx(e, 12.15);
    // default max_shift 1.5: only the back move (1.3 s) is within reach
    let fx2 = fixture(10.9, 13.2, false, true);
    let (_, e2) = after(&fx2.run(json!({"clip_id": fx2.id()})).await.unwrap());
    approx(e2, 12.15);
}

#[tokio::test]
async fn max_shift_limits_moves() {
    let fx = fixture(9.9, 14.0, false, true);
    let v = fx.run(json!({"clip_id": fx.id(), "max_shift": 0.5})).await.unwrap();
    let (s, e) = after(&v);
    let reasons = v["segments"][0]["reasons"].to_string();
    assert!(reasons.contains("opener 'So yeah' kept"), "{reasons}");
    assert!((s - 9.9).abs() <= 0.5 + 1e-9);
    // only finishing the straddled word "morning" (14.40) is within 0.5 s; "again." is 0.9 s away
    assert!(reasons.contains("does not land on a sentence end"), "{reasons}");
    assert!(e > 14.4 && e < 14.45, "end {e}");
}

#[tokio::test]
async fn dry_run_does_not_save() {
    let fx = fixture(9.9, 14.0, false, true);
    let v = fx.run_dry(json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["changed"], true);
    approx(after(&v).0, 10.75);
    let c = fx.clip();
    assert_eq!(c.revision, 1);
    approx(c.segments[0].start, 9.9);
    approx(c.segments[0].end, 14.0);
}

#[tokio::test]
async fn already_clean_clip_is_unchanged() {
    // starts on "the" 0.13 s ahead (target lead 0.15: a 0.02 s move is ignored), ends 0.25 s after "works."
    let fx = fixture(10.77, 12.15, false, true);
    let v = fx.run(json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(v["changed"], false, "{v}");
    assert_eq!(fx.clip().revision, 1);
}

#[tokio::test]
async fn errors() {
    let fx = fixture(9.9, 14.0, false, true);
    let e = fx.run(json!({"clip_id": fx.id(), "expect_revision": 7})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::RevisionConflict);
    let e = fx.run(json!({"clip_id": fx.id(), "segment_id": "seg_nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = fx.run(json!({"clip_id": fx.id(), "max_shift": -1})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let nt = fixture(9.9, 14.0, false, false);
    let e = nt.run(json!({"clip_id": nt.id()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap_or_default().contains(&format!("analyze transcribe {}", nt.source_id)));
}
