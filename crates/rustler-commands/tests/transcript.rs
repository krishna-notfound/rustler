//! transcript.get / transcript.search (pure: synthetic transcript written to the analysis dir).
mod support;

use rustler_commands::transcript_cmds::save_transcript;
use rustler_core::{CaptionScript, ErrorCode, ProjectStore, SourceId, Transcript, Word, WordTiming};
use serde_json::json;
use support::*;

/// 60 sentences of 4 words each, 2 s apart ("sentence number N ."), plus a known quote.
fn synthetic(store: &ProjectStore, id: &SourceId) -> usize {
    let mut words = Vec::new();
    let mut push = |text: &str, t: f64, words: &mut Vec<Word>| {
        let i = words.len() as u32;
        words.push(Word { i, text: text.to_string(), start: t, end: t + 0.4, conf: None, speaker: None });
    };
    for n in 0..60u32 {
        let t = n as f64 * 2.0;
        if n == 30 {
            for (k, w) in ["And", "so", "my", "fellow", "Américans."].iter().enumerate() {
                push(w, t + k as f64 * 0.3, &mut words);
            }
        } else {
            for (k, w) in ["sentence", "number", &n.to_string(), "here."].iter().enumerate() {
                push(w, t + k as f64 * 0.3, &mut words);
            }
        }
    }
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: id.clone(),
        language: "en".into(),
        engine: "faster-whisper:tiny".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    save_transcript(store, id.as_str(), &t).unwrap().len()
}

fn setup() -> (tempfile::TempDir, rustler_registry::Ctx, String, usize) {
    let (dir, ctx) = project();
    let store = ProjectStore::open(ctx.project_dir.as_ref().unwrap()).unwrap();
    let id = SourceId::new();
    let n = synthetic(&store, &id);
    (dir, ctx, id.to_string(), n)
}

#[tokio::test]
async fn get_pages_and_formats() {
    let reg = registry();
    let (_d, ctx, id, n) = setup();
    assert_eq!(n, 60);

    let v = ok(&reg, "transcript.get", &ctx, json!({"source_id": id, "limit": 25})).await;
    assert_eq!(v["total_units"], 60);
    assert_eq!(v["returned"], 25);
    assert_eq!(v["next_offset"], 25);
    let lines = v["lines"].as_array().unwrap();
    assert!(lines[0].as_str().unwrap().starts_with("#0 [00:00:00.0–"), "{}", lines[0]);
    assert!(v.get("units").is_none());

    let v2 = ok(&reg, "transcript.get", &ctx, json!({"source_id": id, "limit": 25, "offset": 50})).await;
    assert_eq!(v2["returned"], 10);
    assert!(v2["next_offset"].is_null());
    assert!(v2["lines"][0].as_str().unwrap().starts_with("#50 "));

    let j = ok(&reg, "transcript.get", &ctx, json!({"source_id": id, "format": "json", "limit": 3})).await;
    assert_eq!(j["units"].as_array().unwrap().len(), 3);
    assert_eq!(j["units"][1]["id"], 1);
    assert!(j.get("lines").is_none());

    // time window: units overlapping [20, 30) are 10..=14
    let w = ok(&reg, "transcript.get", &ctx, json!({"source_id": id, "start": 20.0, "end": 30.0})).await;
    assert_eq!(w["total_units"], 5);
    assert!(w["lines"][0].as_str().unwrap().starts_with("#10 "));

    // default limit is 200 (all 60 fit)
    let all = ok(&reg, "transcript.get", &ctx, json!({"source_id": id})).await;
    assert_eq!(all["returned"], 60);
}

#[tokio::test]
async fn get_validation_errors_name_next_command() {
    let reg = registry();
    let (_d, ctx, id, _) = setup();
    expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": id, "format": "xml"})).await, ErrorCode::Validation);
    expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": id, "limit": 0})).await, ErrorCode::Validation);
    expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": id, "start": 10.0, "end": 5.0})).await, ErrorCode::Validation);
    expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": "../etc"})).await, ErrorCode::Validation);
    let e = expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": "src_doesnotexist"})).await, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("source list"));
    let e = expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": "url_deadbeef"})).await, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("fetch-transcript"));
}

#[tokio::test]
async fn transcribe_hint_when_source_has_no_transcript() {
    // A source folder exists but nothing was transcribed.
    let reg = registry();
    let (_d, ctx) = project();
    let store = ProjectStore::open(ctx.project_dir.as_ref().unwrap()).unwrap();
    let id = SourceId::new();
    std::fs::create_dir_all(store.source_dir(&id)).unwrap();
    std::fs::write(store.source_dir(&id).join("source.json"), "{}").unwrap();
    let e = expect_code(run(&reg, "transcript.get", &ctx, json!({"source_id": id})).await, ErrorCode::Validation);
    assert!(e.hint.unwrap().contains("analyze transcribe"));
}

#[tokio::test]
async fn search_finds_quote_with_context() {
    let reg = registry();
    let (_d, ctx, id, _) = setup();
    // accent- and case-insensitive; the transcript says "Américans."
    let v = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "FELLOW americans"})).await;
    let m = &v["matches"][0];
    assert_eq!(m["unit_id"], 30);
    assert_eq!(m["kind"], "exact");
    assert_eq!(m["start"], 60.0);
    assert_eq!(m["mmss"], "01:00");
    assert!(m["before"]["text"].as_str().unwrap().contains("sentence number 29"));
    assert!(m["after"]["text"].as_str().unwrap().contains("sentence number 31"));
    assert!(v["next"][0].as_str().unwrap().contains("see tiles"));
}

#[tokio::test]
async fn search_limit_edges_and_fuzzy() {
    let reg = registry();
    let (_d, ctx, id, _) = setup();
    // 59 units contain "sentence number"
    let v = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "sentence number", "limit": 5})).await;
    assert_eq!(v["total_matches"], 59);
    assert_eq!(v["matches"].as_array().unwrap().len(), 5);
    assert!(v["next"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("raise --limit")));
    // first and last unit edges have only one neighbour
    let v = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "number 0 here"})).await;
    assert_eq!(v["matches"][0]["unit_id"], 0);
    assert!(v["matches"][0]["before"].is_null());
    let v = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "number 59 here"})).await;
    assert!(v["matches"][0]["after"].is_null());
    // fuzzy: a misspelling still lands on the quote
    let v = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "fellow amerikans"})).await;
    assert_eq!(v["matches"][0]["unit_id"], 30);
    assert_eq!(v["matches"][0]["kind"], "fuzzy");
    // no match gives a hint, not an error
    let v = ok(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "zebra crossing"})).await;
    assert_eq!(v["total_matches"], 0);
    assert!(!v["next"][0].as_str().unwrap().is_empty());
    expect_code(run(&reg, "transcript.search", &ctx, json!({"source_id": id, "query": "  !! "})).await, ErrorCode::Validation);
}
