//! Tests for the edit.* and history.* commands on a synthetic project (no ffmpeg needed).

use chrono::Utc;
use rustler_commands::edit_cmds;
use rustler_core::layout::{self, LayoutParams};
use rustler_core::model::{CaptionScript, WordTiming};
use rustler_core::units::build_units;
use rustler_core::{
    AudioSettings, CaptionTrack, Canvas, Clip, ClipId, ErrorCode, MediaProbe, ProjectStore, Segment, SegmentId, Source,
    SourceId, SourceOrigin, Transcript, Word,
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

/// Source of 60 s, one clip with a single segment [10, 20) (revision 1), synthetic transcript:
///   0 Hello 10.10-10.50 | 1 there. 10.55-11.00 |  pause 1.6 s
///   2 um 12.60-12.80 | 3 this | 4 is | 5 great. 13.35-14.00 |  pause 1.0 s
///   6 Uh, 15.00-15.20 | 7 second | 8 sentence. 15.75-16.50
fn fixture() -> Fx {
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

    let words = vec![
        w(0, "Hello", 10.10, 10.50),
        w(1, "there.", 10.55, 11.00),
        w(2, "um", 12.60, 12.80),
        w(3, "this", 12.85, 13.10),
        w(4, "is", 13.15, 13.30),
        w(5, "great.", 13.35, 14.00),
        w(6, "Uh,", 15.00, 15.20),
        w(7, "second", 15.25, 15.70),
        w(8, "sentence.", 15.75, 16.50),
    ];
    let units = build_units(&words);
    assert_eq!(units.len(), 3, "fixture units");
    let transcript = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source_id.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    let rel = Path::new("analysis").join(source_id.as_str());
    store.write_json(&rel.join("transcript.json"), &transcript).unwrap();
    store.write_json(&rel.join("units.json"), &units).unwrap();

    let canvas = Canvas::default();
    let layout = layout::preset("center-crop", 1920, 1080, &canvas, &LayoutParams::default()).unwrap();
    let mut clip = Clip {
        schema: "rustler.clip@1".into(),
        id: ClipId::new(),
        name: "c".into(),
        revision: 0,
        canvas,
        segments: vec![Segment { id: SegmentId::new(), source_id: source_id.clone(), start: 10.0, end: 20.0, layout }],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![],
        created_at: Utc::now(),
    };
    store.save_clip(&mut clip, 0).unwrap();
    assert_eq!(clip.revision, 1);

    let mut reg = Registry::new();
    edit_cmds::register(&mut reg);
    Fx { _dir: dir, root, reg, clip_id: clip.id.clone(), source_id }
}

impl Fx {
    fn ctx(&self) -> Ctx {
        Ctx::new(Some(self.root.clone()), self.root.join("home"))
    }
    async fn run(&self, cmd: &str, input: Value) -> rustler_core::Result<Value> {
        self.reg.dispatch(cmd, input, self.ctx()).await
    }
    async fn run_dry(&self, cmd: &str, input: Value) -> rustler_core::Result<Value> {
        let mut ctx = self.ctx();
        ctx.dry_run = true;
        self.reg.dispatch(cmd, input, ctx).await
    }
    fn store(&self) -> ProjectStore {
        ProjectStore::open(&self.root).unwrap()
    }
    fn clip(&self) -> Clip {
        self.store().load_clip(&self.clip_id).unwrap()
    }
    fn id(&self) -> String {
        self.clip_id.to_string()
    }
    fn first_seg(&self) -> String {
        self.clip().segments[0].id.to_string()
    }
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

fn segs(v: &Value) -> Vec<(f64, f64)> {
    v["segments"].as_array().unwrap().iter().map(|s| (s["start"].as_f64().unwrap(), s["end"].as_f64().unwrap())).collect()
}

fn events(fx: &Fx) -> Vec<Value> {
    std::fs::read_to_string(fx.root.join("events.ndjson"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn trim_deltas() {
    let fx = fixture();
    let out = fx.run("edit.trim", json!({"clip_id": fx.id(), "start_delta": 0.5, "end_delta": -1.0})).await.unwrap();
    assert_eq!(out["clip_id"], fx.id());
    assert_eq!(out["revision"], 2);
    let s = segs(&out);
    approx(s[0].0, 10.5);
    approx(s[0].1, 19.0);
    approx(out["duration"].as_f64().unwrap(), 8.5);
    approx(out["segments"][0]["out_start"].as_f64().unwrap(), 0.0);
    approx(out["segments"][0]["out_end"].as_f64().unwrap(), 8.5);
    assert_eq!(out["segments"][0]["layout_preset"], "center-crop");
    let on_disk = fx.clip();
    assert_eq!(on_disk.revision, 2);
    approx(on_disk.segments[0].start, 10.5);
    let ev = events(&fx);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["kind"], "edit.trim");
    assert_eq!(ev[0]["actor"], "agent");
}

#[tokio::test]
async fn trim_validates_range() {
    let fx = fixture();
    // past the 60 s source duration
    let e = fx.run("edit.trim", json!({"clip_id": fx.id(), "end": 70.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    // start >= end
    let e = fx.run("edit.trim", json!({"clip_id": fx.id(), "start": 25.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    // nothing to do
    let e = fx.run("edit.trim", json!({"clip_id": fx.id()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    // absolute and delta together
    let e = fx.run("edit.trim", json!({"clip_id": fx.id(), "start": 11.0, "start_delta": 1.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert_eq!(fx.clip().revision, 1, "failed edits must not save");
}

#[tokio::test]
async fn trim_snaps_to_word_boundaries() {
    let fx = fixture();
    let out = fx.run("edit.trim", json!({"clip_id": fx.id(), "start": 12.62, "end": 14.03, "snap": "word"})).await.unwrap();
    let s = segs(&out);
    approx(s[0].0, 12.60);
    approx(s[0].1, 14.00);
}

#[tokio::test]
async fn trim_snap_only_moves_changed_boundary() {
    let fx = fixture();
    let out = fx.run("edit.trim", json!({"clip_id": fx.id(), "start": 12.62, "snap": "word"})).await.unwrap();
    let s = segs(&out);
    approx(s[0].0, 12.60);
    approx(s[0].1, 20.0);
}

#[tokio::test]
async fn split_at_out_t() {
    let fx = fixture();
    let original = fx.first_seg();
    let out = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 3.0})).await.unwrap();
    assert_eq!(out["revision"], 2);
    let s = segs(&out);
    assert_eq!(s.len(), 2);
    approx(s[0].0, 10.0);
    approx(s[0].1, 13.0);
    approx(s[1].0, 13.0);
    approx(s[1].1, 20.0);
    approx(out["segments"][1]["out_start"].as_f64().unwrap(), 3.0);
    approx(out["duration"].as_f64().unwrap(), 10.0);
    let a = out["segments"][0]["id"].as_str().unwrap();
    let b = out["segments"][1]["id"].as_str().unwrap();
    assert_ne!(a, b);
    assert_ne!(a, original);
    assert_ne!(b, original);
    assert_eq!(out["segments"][1]["layout_preset"], "center-crop");
    let clip = fx.clip();
    assert_eq!(clip.segments[0].layout, clip.segments[1].layout);

    // out of range / on a boundary
    let e = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 100.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 3.0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation, "splitting exactly on an existing boundary");
}

/// A face-tracked crop path (extras.json `crop_paths`, keyed by segment id, segment-relative
/// times) must follow the segments through split / remove-pauses / trim and back through undo.
#[tokio::test]
async fn crop_paths_follow_edits() {
    let fx = fixture();
    let seg = fx.first_seg();
    let extras_rel = Path::new("clips").join(fx.id()).join("extras.json");
    // A path that pans from 0.2 at t=0 to 0.6 at t=10 (the whole [10, 20) segment).
    fx.store()
        .write_json(&extras_rel, &json!({"crop_paths": {seg.clone(): [{"t": 0.0, "x": 0.2}, {"t": 10.0, "x": 0.6}]}}))
        .unwrap();
    let paths = |fx: &Fx| -> Value { fx.store().read_json::<Value>(&extras_rel).unwrap()["crop_paths"].clone() };
    let x_at = |keys: &Value, t: f64| -> f64 {
        let k = keys.as_array().unwrap();
        let p: Vec<(f64, f64)> = k.iter().map(|k| (k["t"].as_f64().unwrap(), k["x"].as_f64().unwrap())).collect();
        if t <= p[0].0 {
            return p[0].1;
        }
        for w in p.windows(2) {
            if t <= w[1].0 {
                return w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0);
            }
        }
        p[p.len() - 1].1
    };

    // split at out 3.0 (source 13.0): piece b starts at source 13 -> x there is 0.32.
    let out = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 3.0})).await.unwrap();
    let a = out["segments"][0]["id"].as_str().unwrap().to_string();
    let b = out["segments"][1]["id"].as_str().unwrap().to_string();
    let p = paths(&fx);
    approx(x_at(&p[&a], 0.0), 0.2);
    approx(x_at(&p[&a], 3.0), 0.32);
    approx(x_at(&p[&b], 0.0), 0.32);
    approx(x_at(&p[&b], 7.0), 0.6);
    assert!(p.get(&seg).is_some(), "the old id keeps its path for undo");

    // trim the head of b by 1 s: its keys shift, x at the new start is the source-13.0+1 value.
    fx.run("edit.trim", json!({"clip_id": fx.id(), "segment_id": b, "start_delta": 1.0})).await.unwrap();
    approx(x_at(&paths(&fx)[&b], 0.0), 0.36);
    // undo restores the old start and shifts the keys back without loss.
    fx.run("history.undo", json!({"clip_id": fx.id()})).await.unwrap();
    approx(x_at(&paths(&fx)[&b], 0.0), 0.32);
    approx(x_at(&paths(&fx)[&b], 7.0), 0.6);

    // remove-pauses re-cuts b into new pieces that each carry the right part of the path.
    let out = fx.run("edit.remove_pauses", json!({"clip_id": fx.id(), "min_gap": 0.8, "keep": 0.2})).await.unwrap();
    let p = paths(&fx);
    let clip = fx.clip();
    for s in &clip.segments {
        let keys = p.get(s.id.as_str()).unwrap_or_else(|| panic!("segment {} lost its crop path: {out}", s.id));
        approx(x_at(keys, 0.0), 0.2 + 0.04 * (s.start - 10.0));
        approx(x_at(keys, s.end - s.start), 0.2 + 0.04 * (s.end - 10.0));
    }
}

#[tokio::test]
async fn delete_last_segment_is_validation() {
    let fx = fixture();
    let e = fx.run("edit.delete_segment", json!({"clip_id": fx.id(), "segment_id": fx.first_seg()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert_eq!(fx.clip().segments.len(), 1);

    // after a split one can be deleted
    let out = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 4.0})).await.unwrap();
    let second = out["segments"][1]["id"].as_str().unwrap().to_string();
    let out = fx.run("edit.delete_segment", json!({"clip_id": fx.id(), "segment_id": second})).await.unwrap();
    assert_eq!(out["segments"].as_array().unwrap().len(), 1);
    approx(out["duration"].as_f64().unwrap(), 4.0);

    let e = fx.run("edit.delete_segment", json!({"clip_id": fx.id(), "segment_id": "seg_missing"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn move_segment_reorders() {
    let fx = fixture();
    fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 2.0})).await.unwrap();
    let out = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 5.0})).await.unwrap();
    let ids: Vec<String> = out["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 3);
    let out = fx.run("edit.move_segment", json!({"clip_id": fx.id(), "segment_id": ids[0], "to_index": 2})).await.unwrap();
    let now: Vec<String> = out["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(now, vec![ids[1].clone(), ids[2].clone(), ids[0].clone()]);
    // output times follow the new order: the moved first piece [10,12) is now last
    approx(out["segments"][2]["out_start"].as_f64().unwrap(), 8.0);
    approx(out["segments"][2]["start"].as_f64().unwrap(), 10.0);

    let e = fx.run("edit.move_segment", json!({"clip_id": fx.id(), "segment_id": ids[0], "to_index": 3})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn set_range_by_units_with_padding_and_expect_text() {
    let fx = fixture();
    // unit 1 = "um this is great." 12.60-14.00; prev word ends 11.00, next starts 15.00
    let out = fx
        .run("edit.set_range", json!({"clip_id": fx.id(), "start_unit": 1, "end_unit": 1, "expect_text": "um this is"}))
        .await
        .unwrap();
    let s = segs(&out);
    approx(s[0].0, 12.52);
    approx(s[0].1, 14.15);
    assert_eq!(out["revision"], 2);

    // end padding is clamped to the next word start: unit 0 ends 11.00, next word 12.60 -> 11.15
    let out = fx.run("edit.set_range", json!({"clip_id": fx.id(), "start_unit": 0, "end_unit": 0})).await.unwrap();
    let s = segs(&out);
    approx(s[0].0, 10.02); // 10.10 - 0.08, no previous word
    approx(s[0].1, 11.15);

    // start clamped to the previous word end: unit 2 starts 15.00, prev ends 14.00 -> 14.92
    let out = fx.run("edit.set_range", json!({"clip_id": fx.id(), "start_unit": 2, "end_unit": 2})).await.unwrap();
    approx(segs(&out)[0].0, 14.92);
}

#[tokio::test]
async fn set_range_expect_text_mismatch_is_validation() {
    let fx = fixture();
    let e = fx
        .run("edit.set_range", json!({"clip_id": fx.id(), "start_unit": 1, "end_unit": 2, "expect_text": "completely different words"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("opening words do not match"), "{}", e.message);
    assert!(e.message.contains("um this is"), "should quote the transcript: {}", e.message);
    assert_eq!(fx.clip().revision, 1);
    let e = fx.run("edit.set_range", json!({"clip_id": fx.id(), "start_unit": 99})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = fx.run("edit.set_range", json!({"clip_id": fx.id()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn remove_pauses_splits_and_reports_removed_seconds() {
    let fx = fixture();
    let original = fx.first_seg();
    let out = fx.run("edit.remove_pauses", json!({"clip_id": fx.id()})).await.unwrap();
    // pauses: 11.00-12.60 (1.6 s) and 14.00-15.00 (1.0 s); keep 0.12 s each (0.06 per side)
    let s = segs(&out);
    assert_eq!(s.len(), 3, "{out}");
    approx(s[0].0, 10.0);
    approx(s[0].1, 11.06);
    approx(s[1].0, 12.54);
    approx(s[1].1, 14.06);
    approx(s[2].0, 14.94);
    approx(s[2].1, 20.0);
    approx(out["removed_seconds"].as_f64().unwrap(), (1.6 - 0.12) + (1.0 - 0.12));
    approx(out["duration"].as_f64().unwrap(), 10.0 - 2.36);
    // output timeline is contiguous
    approx(out["segments"][1]["out_start"].as_f64().unwrap(), 1.06);
    approx(out["segments"][2]["out_start"].as_f64().unwrap(), 1.06 + 1.52);
    for seg in out["segments"].as_array().unwrap() {
        assert_ne!(seg["id"], original.as_str());
        assert_eq!(seg["layout_preset"], "center-crop");
    }
    assert_eq!(out["cuts"].as_array().unwrap().len(), 2);

    // a larger min_gap only takes the 1.6 s pause
    let fx2 = fixture();
    let out = fx2.run("edit.remove_pauses", json!({"clip_id": fx2.id(), "min_gap": 1.2, "keep": 0.2})).await.unwrap();
    assert_eq!(segs(&out).len(), 2);
    approx(out["removed_seconds"].as_f64().unwrap(), 1.4);
}

#[tokio::test]
async fn remove_pauses_without_transcript_is_validation() {
    let fx = fixture();
    std::fs::remove_file(fx.root.join("analysis").join(fx.source_id.as_str()).join("transcript.json")).unwrap();
    let e = fx.run("edit.remove_pauses", json!({"clip_id": fx.id()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap_or_default().contains("transcribe"));
}

#[tokio::test]
async fn remove_words_default_fillers() {
    let fx = fixture();
    let out = fx.run("edit.remove_words", json!({"clip_id": fx.id()})).await.unwrap();
    // "um" 12.60-12.80 and "Uh," 15.00-15.20
    let s = segs(&out);
    assert_eq!(s.len(), 3, "{out}");
    approx(s[0].1, 12.60);
    approx(s[1].0, 12.80);
    approx(s[1].1, 15.00);
    approx(s[2].0, 15.20);
    approx(out["removed_seconds"].as_f64().unwrap(), 0.4);
    let removed: Vec<&str> = out["removed_words"].as_array().unwrap().iter().map(|w| w["text"].as_str().unwrap()).collect();
    assert_eq!(removed, vec!["um", "Uh,"]);
}

#[tokio::test]
async fn remove_words_explicit_list_and_indices() {
    let fx = fixture();
    // indices alone: only word 4 "is" (13.15-13.30)
    let out = fx.run("edit.remove_words", json!({"clip_id": fx.id(), "word_indices": [4]})).await.unwrap();
    let s = segs(&out);
    assert_eq!(s.len(), 2);
    approx(s[0].1, 13.15);
    approx(s[1].0, 13.30);
    approx(out["removed_seconds"].as_f64().unwrap(), 0.15);

    // explicit words, case/punctuation-insensitive, plus an index
    let fx = fixture();
    let out = fx.run("edit.remove_words", json!({"clip_id": fx.id(), "words": ["GREAT", "second"], "word_indices": [0]})).await.unwrap();
    assert_eq!(out["removed_words"].as_array().unwrap().len(), 3);

    // unknown index
    let e = fx.run("edit.remove_words", json!({"clip_id": fx.id(), "word_indices": [999]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn undo_restores_previous_segments_and_bumps_revision() {
    let fx = fixture();
    let before = fx.clip().segments.clone();
    fx.run("edit.trim", json!({"clip_id": fx.id(), "start_delta": 1.0})).await.unwrap();
    assert_eq!(fx.clip().revision, 2);
    let out = fx.run("history.undo", json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(out["revision"], 3, "undo is saved as a new revision");
    assert_eq!(out["restored_from"], 1);
    let after = fx.clip();
    assert_eq!(after.revision, 3);
    assert_eq!(after.segments, before);
    let ev = events(&fx);
    assert_eq!(ev.last().unwrap()["kind"], "history.undo");

    // nothing further to undo
    let e = fx.run("history.undo", json!({"clip_id": fx.id()})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn repeated_undo_steps_back_and_restore_redoes() {
    let fx = fixture();
    let s1 = fx.clip().segments.clone(); // rev 1
    fx.run("edit.trim", json!({"clip_id": fx.id(), "start_delta": 1.0})).await.unwrap(); // rev 2
    let s2 = fx.clip().segments.clone();
    fx.run("edit.trim", json!({"clip_id": fx.id(), "start_delta": 1.0})).await.unwrap(); // rev 3
    let s3 = fx.clip().segments.clone();

    fx.run("history.undo", json!({"clip_id": fx.id()})).await.unwrap(); // rev 4 = s2
    assert_eq!(fx.clip().segments, s2);
    fx.run("history.undo", json!({"clip_id": fx.id()})).await.unwrap(); // rev 5 = s1
    assert_eq!(fx.clip().segments, s1);
    assert_eq!(fx.clip().revision, 5);

    // redo the last trim by restoring revision 3
    let out = fx.run("history.restore", json!({"clip_id": fx.id(), "revision": 3})).await.unwrap();
    assert_eq!(out["revision"], 6);
    assert_eq!(fx.clip().segments, s3);

    let list = fx.run("history.list", json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(list["current_revision"], 6);
    let revs: Vec<u64> = list["revisions"].as_array().unwrap().iter().map(|r| r["revision"].as_u64().unwrap()).collect();
    assert_eq!(revs, vec![1, 2, 3, 4, 5, 6]);
    let last = list["revisions"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["current"], true);
    assert_eq!(last["segments"], 1);
    assert!(!last["timestamp"].as_str().unwrap().is_empty());
    assert_eq!(list["revisions"][0]["current"], false);

    // restoring the current revision is rejected; unknown revision is NotFound
    let e = fx.run("history.restore", json!({"clip_id": fx.id(), "revision": 6})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = fx.run("history.restore", json!({"clip_id": fx.id(), "revision": 42})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn history_list_counts_segments() {
    let fx = fixture();
    fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 3.0})).await.unwrap();
    let list = fx.run("history.list", json!({"clip_id": fx.id()})).await.unwrap();
    let r = list["revisions"].as_array().unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0]["segments"], 1);
    assert_eq!(r[1]["segments"], 2);
    approx(r[0]["duration"].as_f64().unwrap(), 10.0);
}

#[tokio::test]
async fn stale_expect_revision_conflicts() {
    let fx = fixture();
    let e = fx.run("edit.trim", json!({"clip_id": fx.id(), "start_delta": 1.0, "expect_revision": 0})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::RevisionConflict);
    assert_eq!(fx.clip().revision, 1);
    // the right revision works
    let out = fx.run("edit.trim", json!({"clip_id": fx.id(), "start_delta": 1.0, "expect_revision": 1})).await.unwrap();
    assert_eq!(out["revision"], 2);
    // and a second writer with the old revision is rejected
    let e = fx.run("edit.split", json!({"clip_id": fx.id(), "out_t": 2.0, "expect_revision": 1})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::RevisionConflict);
    let e = fx.run("history.undo", json!({"clip_id": fx.id(), "expect_revision": 1})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::RevisionConflict);
}

#[tokio::test]
async fn dry_run_does_not_write() {
    let fx = fixture();
    let out = fx.run_dry("edit.split", json!({"clip_id": fx.id(), "out_t": 3.0})).await.unwrap();
    assert_eq!(out["dry_run"], true);
    assert_eq!(out["would"]["segments"].as_array().unwrap().len(), 2);
    assert_eq!(fx.clip().segments.len(), 1);
    assert_eq!(fx.clip().revision, 1);
    assert!(events(&fx).is_empty());
    assert!(!fx.root.join("clips").join(fx.id()).join("history").join("1.clip.json").exists());

    let out = fx.run_dry("edit.remove_pauses", json!({"clip_id": fx.id()})).await.unwrap();
    assert_eq!(out["dry_run"], true);
    approx(out["removed_seconds"].as_f64().unwrap(), 2.36);
    let out = fx.run_dry("history.undo", json!({"clip_id": fx.id()})).await;
    assert!(out.is_err() || out.unwrap()["dry_run"] == true);
    assert_eq!(fx.clip().revision, 1);
}

#[tokio::test]
async fn actor_comes_from_ctx() {
    let fx = fixture();
    let mut ctx = fx.ctx();
    ctx.actor = rustler_core::Actor::Human;
    fx.reg.dispatch("edit.trim", json!({"clip_id": fx.id(), "start_delta": 0.2}), ctx).await.unwrap();
    assert_eq!(events(&fx)[0]["actor"], "human");
}

#[test]
fn commands_are_described_with_examples() {
    let mut reg = Registry::new();
    edit_cmds::register(&mut reg);
    let names: Vec<&str> = [
        "edit.trim", "edit.split", "edit.delete_segment", "edit.move_segment", "edit.set_range",
        "edit.remove_pauses", "edit.remove_words", "history.list", "history.undo", "history.restore",
    ]
    .to_vec();
    for n in names {
        let spec = reg.describe(n).unwrap_or_else(|| panic!("{n} not registered"));
        assert!(!spec.examples.is_empty(), "{n} needs examples");
        assert!(spec.input_schema["properties"]["clip_id"].is_object(), "{n} input schema");
        assert!(spec.output_schema.is_object());
    }
}
