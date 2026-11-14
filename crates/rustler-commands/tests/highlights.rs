//! highlights.signals / highlights.candidates / transcript.read_window on a synthetic project.
mod support;

use rustler_commands::transcript_cmds::save_transcript;
use rustler_core::{
    AudioEvents, CaptionScript, ErrorCode, LoudnessPoint, ProjectStore, Scene, Scenes, SourceId, Transcript, Word, WordTiming,
};
use serde_json::{Value, json};
use std::path::Path;
use support::*;

/// 49 sentences, one every 4 s from t=2 (words 0.4 s apart). Most are bland; k=10 opens with a weak
/// opener; k=25..=27 are the hot region (question, emphasis words, exclamation).
fn sentence(k: usize) -> &'static str {
    match k {
        10 => "So basically we went there and did things today.",
        20 => "Um yeah the data was fine overall today ok.",
        25 => "Why is this the biggest mistake nobody discusses?",
        26 => "Never trust this secret trick, it is insane!",
        27 => "The truth is actually crazy, bhai, listen closely.",
        _ => "The report said the weather would stay mild today.",
    }
}

fn transcript(id: &SourceId) -> Transcript {
    let mut words = Vec::new();
    for k in 0..49usize {
        let t0 = 2.0 + 4.0 * k as f64;
        for (n, text) in sentence(k).split_whitespace().enumerate() {
            let s = t0 + n as f64 * 0.4;
            words.push(Word { i: words.len() as u32, text: text.into(), start: s, end: s + 0.35, conf: None, speaker: None });
        }
    }
    Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: id.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    ctx: rustler_registry::Ctx,
    id: String,
}

/// `full` adds audio_events.json (hot loudness 100-110 s), scenes.json and source_info.json (heatmap).
fn setup(full: bool) -> Fx {
    let (dir, ctx) = project();
    let store = ProjectStore::open(ctx.project_dir.as_ref().unwrap()).unwrap();
    let id = SourceId::new();
    save_transcript(&store, id.as_str(), &transcript(&id)).unwrap();
    let rel = Path::new("analysis").join(id.as_str());
    if full {
        let curve: Vec<LoudnessPoint> = (0..200).map(|t| LoudnessPoint { t: t as f64, lufs: if (100..110).contains(&t) { -18.0 } else { -30.0 } }).collect();
        store.write_json(&rel.join("audio_events.json"), &AudioEvents { curve, peaks: vec![], silences: vec![] }).unwrap();
        let scenes = Scenes { scenes: [0.0, 95.0, 105.0, 150.0].iter().map(|s| Scene { start: *s, end: s + 1.0 }).collect() };
        store.write_json(&rel.join("scenes.json"), &scenes).unwrap();
        store.write_json(&rel.join("source_info.json"), &json!({"heatmap": [{"start": 90.0, "end": 120.0, "value": 0.9}]})).unwrap();
    }
    Fx { _dir: dir, ctx, id: id.to_string() }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("expected number, got {v}"))
}

#[tokio::test]
async fn signals_rows_are_correct() {
    let reg = registry();
    let fx = setup(true);
    let v = ok(&reg, "highlights.signals", &fx.ctx, json!({"source_id": fx.id})).await;
    assert_eq!(v["total_windows"], 7);
    assert_eq!(v["available"]["audio_events"], true);
    assert_eq!(f(&v["baselines"]["loudness_median_lufs"]), -30.0);
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 7);
    let r0 = &rows[0];
    assert_eq!(r0["start"], 0.0);
    assert_eq!(r0["end"], 30.0);
    assert_eq!(f(&r0["loudness_peak_lu"]), 0.0);
    assert!(r0["heatmap"].is_null(), "no overlap with the heatmap window");
    assert_eq!(r0["questions"], 0);
    assert_eq!(r0["scene_cuts"], 0);
    assert!(r0["unit_start"].is_u64() && r0["unit_end"].is_u64());

    let hot = &rows[3]; // 90-120 s
    assert_eq!(f(&hot["loudness_peak_lufs"]), -18.0);
    assert_eq!(f(&hot["loudness_peak_lu"]), 12.0);
    assert_eq!(hot["questions"], 1);
    assert_eq!(hot["exclamations"], 1);
    assert_eq!(hot["scene_cuts"], 2);
    assert_eq!(f(&hot["heatmap"]), 0.9);
    let emph: Vec<&str> = hot["emphasis_words"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    for w in ["biggest", "mistake", "never", "secret", "insane", "truth", "actually", "crazy", "bhai"] {
        assert!(emph.contains(&w), "missing {w} in {emph:?}");
    }
    assert!(f(&hot["wpm"]) > 100.0);
    assert!(f(&hot["pause_before"]) >= 0.0);
    assert!(v["warnings"].as_array().unwrap().is_empty(), "{}", v["warnings"]);

    // paging
    let p = ok(&reg, "highlights.signals", &fx.ctx, json!({"source_id": fx.id, "window": 10, "limit": 5, "offset": 2})).await;
    assert_eq!(p["returned"], 5);
    assert_eq!(p["rows"][0]["index"], 2);
    assert_eq!(p["next_offset"], 7);
}

#[tokio::test]
async fn missing_files_give_null_and_named_warnings() {
    let reg = registry();
    let fx = setup(false);
    let v = ok(&reg, "highlights.signals", &fx.ctx, json!({"source_id": fx.id})).await;
    let hot = &v["rows"][3];
    assert!(hot["loudness_peak_lu"].is_null());
    assert!(hot["loudness_peak_lufs"].is_null());
    assert!(hot["scene_cuts"].is_null());
    assert!(hot["heatmap"].is_null());
    assert_eq!(hot["questions"], 1, "transcript-based fields still work");
    let w = v["warnings"].to_string();
    assert!(w.contains("rustler analyze audio"), "{w}");
    assert!(w.contains("rustler analyze scenes"), "{w}");
    assert_eq!(v["available"]["audio_events"], false);

    // candidates still work without optional files
    let c = ok(&reg, "highlights.candidates", &fx.ctx, json!({"source_id": fx.id, "min_len": 10, "max_len": 30})).await;
    assert!(!c["candidates"].as_array().unwrap().is_empty());
    assert!(c["warnings"].to_string().contains("analyze audio"));
    let b = c["candidates"][0]["breakdown"].as_array().unwrap();
    let energy = b.iter().find(|p| p["feature"] == "energy").unwrap();
    assert!(energy["value"].is_null() && energy["points"].is_null());
}

#[tokio::test]
async fn transcript_is_required_for_candidates() {
    let reg = registry();
    let (_d, ctx) = project();
    let id = SourceId::new().to_string();
    let e = expect_code(run(&reg, "highlights.candidates", &ctx, json!({"source_id": id})).await, ErrorCode::NotFound);
    assert!(e.message.contains(&id));
}

#[tokio::test]
async fn candidates_are_sentence_aligned_non_overlapping_and_ordered() {
    let reg = registry();
    let fx = setup(true);
    let v = ok(&reg, "highlights.candidates", &fx.ctx, json!({"source_id": fx.id, "count": 4, "min_len": 10, "max_len": 30})).await;
    let cands = v["candidates"].as_array().unwrap();
    assert_eq!(cands.len(), 4, "{v}");
    assert!(v["scoring"]["formula"].as_str().unwrap().contains("hook 25"));
    assert!(v["note"].as_str().unwrap().contains("evidence"));

    // units to check alignment against
    let t = ok(&reg, "transcript.get", &fx.ctx, json!({"source_id": fx.id, "format": "json", "limit": 100})).await;
    let units = t["units"].as_array().unwrap();
    for c in cands {
        let (a, b) = (c["start_unit"].as_u64().unwrap() as usize, c["end_unit"].as_u64().unwrap() as usize);
        assert!((f(&units[a]["start"]) - f(&c["start"])).abs() < 0.01, "start is a unit start");
        assert!((f(&units[b]["end"]) - f(&c["end"])).abs() < 0.01, "end is a unit end");
        assert!(f(&units[b]["boundary_strength"]) >= 0.8);
        let dur = f(&c["duration"]);
        assert!((10.0..=30.0).contains(&dur), "duration {dur}");
        assert_eq!(c["opening_text"], units[a]["text"]);
        let score = f(&c["score"]);
        assert!((0.0..=100.0).contains(&score));
        let sum: f64 = c["breakdown"].as_array().unwrap().iter().filter_map(|p| p["points"].as_f64()).sum();
        assert!((sum - score).abs() < 0.6, "breakdown {sum} vs score {score}");
        let cmd = c["create_cmd"].as_str().unwrap();
        assert!(cmd.starts_with("rustler clip create "), "{cmd}");
        assert!(cmd.contains(&format!("--source-id {}", fx.id)));
        assert!(cmd.contains(&format!("--start-unit {a} --end-unit {b} --expect-text \"")), "{cmd}");
        assert!(!c["reasons"].as_array().unwrap().is_empty());
    }
    // non-overlapping
    for (x, a) in cands.iter().enumerate() {
        for b in cands.iter().skip(x + 1) {
            let overlap = f(&a["end"]).min(f(&b["end"])) - f(&a["start"]).max(f(&b["start"]));
            assert!(overlap <= 0.0, "overlap {overlap} between {a} and {b}");
        }
    }
    // the hot region wins, with its signals visible
    let top = &cands[0];
    let (a, b) = (top["start_unit"].as_u64().unwrap(), top["end_unit"].as_u64().unwrap());
    assert!(a <= 27 && b >= 25, "top candidate {a}..{b} should cover the hot units 25-27");
    assert!(top["signals"]["emphasis_words"].to_string().contains("secret"));
    assert!(f(&top["signals"]["loudness_peak_lu"]) >= 10.0);
    // score ordering: rank-adjusted score never increases down the list
    let adj: Vec<f64> = cands.iter().map(|c| f(&c["score"]) - f(&c["diversity_penalty"])).collect();
    assert!(adj.windows(2).all(|w| w[0] >= w[1] - 1e-9), "{adj:?}");
    assert!(f(&cands[0]["score"]) > f(&cands[3]["score"]));
    // diverse across the timeline: not all four from the hot region
    let starts: Vec<f64> = cands.iter().map(|c| f(&c["start"])).collect();
    let spread = starts.iter().cloned().fold(f64::MIN, f64::max) - starts.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread > 30.0, "starts {starts:?}");
}

#[tokio::test]
async fn weak_openers_are_skipped() {
    let reg = registry();
    let fx = setup(true);
    let v = ok(&reg, "highlights.candidates", &fx.ctx, json!({"source_id": fx.id, "count": 30, "min_len": 8, "max_len": 20})).await;
    assert!(v["skipped_weak_openers"].as_u64().unwrap() >= 2, "units 10 and 20 start with so/um");
    for c in v["candidates"].as_array().unwrap() {
        let open = c["opening_text"].as_str().unwrap();
        assert!(!open.starts_with("So basically") && !open.starts_with("Um yeah"), "weak opener leaked: {open}");
        assert!(![10u64, 20].contains(&c["start_unit"].as_u64().unwrap()));
    }
}

#[tokio::test]
async fn candidate_argument_validation() {
    let reg = registry();
    let fx = setup(false);
    for input in [
        json!({"source_id": fx.id, "platform": "myspace"}),
        json!({"source_id": fx.id, "min_len": 40, "max_len": 20}),
        json!({"source_id": fx.id, "count": 0}),
    ] {
        expect_code(run(&reg, "highlights.candidates", &fx.ctx, input).await, ErrorCode::Validation);
    }
}

#[tokio::test]
async fn read_window_returns_text_with_unit_ids() {
    let reg = registry();
    let fx = setup(false);
    let v = ok(&reg, "transcript.read_window", &fx.ctx, json!({"source_id": fx.id, "start": 102.0, "end": 114.0})).await;
    assert_eq!(v["first_unit"], 25);
    assert_eq!(v["last_unit"], 27);
    let text = v["text"].as_str().unwrap();
    assert!(text.starts_with("[#25] Why is this the biggest mistake"), "{text}");
    assert!(text.contains("[#26] Never trust this secret trick"));
    assert_eq!(v["lines"].as_array().unwrap().len(), 3);
    assert!(v["lines"][0].as_str().unwrap().starts_with("#25 ["));
    assert!(v["next"][0].as_str().unwrap().contains("--start-unit 25 --end-unit 27"));
    expect_code(run(&reg, "transcript.read_window", &fx.ctx, json!({"source_id": fx.id, "start": 10.0, "end": 5.0})).await, ErrorCode::Validation);
}
