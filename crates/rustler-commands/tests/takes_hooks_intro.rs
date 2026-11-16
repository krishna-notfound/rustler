//! f39 content suggestions: `edit.find_bad_takes` (and its fixes in `clip.retention`), the
//! intro-trailer skip in `highlights.candidates` / `clips.auto`, hook + cold-open suggestions in
//! `clips.auto` (never applied), and caption word fragments. No media tools needed.

mod common;

use common::{add_source, new_project, probe_1080p, word};
use rustler_core::model::{CaptionScript, WordTiming};
use rustler_core::{ProjectStore, SourceId, Transcript, Word};
use serde_json::{Value, json};
use std::path::Path;

fn transcript(store: &ProjectStore, source: &SourceId, words: Vec<Word>, timing: WordTiming) {
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: timing,
    };
    store.write_json(&Path::new("analysis").join(source.as_str()).join("transcript.json"), &t).unwrap();
}

/// Words of `text` from `t0`, 0.3 s apart (0.25 s long), indices from `i0`.
fn say(text: &str, t0: f64, i0: u32) -> Vec<Word> {
    text.split_whitespace().enumerate().map(|(k, w)| word(i0 + k as u32, w, t0 + k as f64 * 0.3, t0 + k as f64 * 0.3 + 0.25)).collect()
}

const TAKES: &str = "So I think I think the real thing is focus. You need to ship ship fast and learn from users every week.";

async fn clip_on(store: &ProjectStore, ctx: &rustler_registry::Ctx, timing: WordTiming) -> (Value, String) {
    let reg = common::registry();
    let src = add_source(store, Path::new("talk.mp4"), probe_1080p(60.0, true, true));
    transcript(store, &src.id, say(TAKES, 10.0, 0), timing);
    let c = common::call(&reg, ctx, "clip.create", json!({"name": "t", "source_id": src.id, "start": 9.9, "end": 17.0, "raw": true})).await.unwrap();
    (c, src.id.to_string())
}

#[tokio::test]
async fn find_bad_takes_lists_cuts_and_never_applies_them() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let (c, _) = clip_on(&store, &ctx, WordTiming::Exact).await;
    let id = c["id"].as_str().unwrap().to_string();
    let rev = store.load_clip(&serde_json::from_value(json!(id)).unwrap()).unwrap().revision;

    let out = common::call(&reg, &ctx, "edit.find_bad_takes", json!({"clip_id": id})).await.unwrap();
    let f = out["findings"].as_array().unwrap();
    let kinds: Vec<&str> = f.iter().map(|x| x["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"restart") && kinds.contains(&"stutter"), "{out:#}");
    let restart = f.iter().find(|x| x["kind"] == "restart").unwrap();
    assert_eq!(restart["cut_text"], "I think");
    assert_eq!(restart["word_indices"], json!([1, 2]));
    assert!(restart["confidence"].as_f64().unwrap() >= 0.5);
    let cmd = restart["command"].as_str().unwrap();
    assert_eq!(cmd, format!("rustler edit remove-words {id} --word-indices '[1,2]'"));
    // Read-only: the clip is unchanged.
    let clip = store.load_clip(&serde_json::from_value(json!(id)).unwrap()).unwrap();
    assert_eq!(clip.revision, rev);

    // The command the finding names cuts the first take as one span (no sliver between the words).
    let cut = common::call(&reg, &ctx, "edit.remove_words", json!({"clip_id": id, "word_indices": [1, 2]})).await.unwrap();
    let segs = cut["segments"].as_array().unwrap();
    assert_eq!(segs.len(), 2, "{cut:#}");
    assert!((segs[0]["end"].as_f64().unwrap() - 10.3).abs() < 1e-6);
    assert!((segs[1]["start"].as_f64().unwrap() - 10.85).abs() < 1e-6, "cut runs from 'I' to the end of 'think'");
    let again = common::call(&reg, &ctx, "edit.find_bad_takes", json!({"clip_id": id})).await.unwrap();
    assert!(again["findings"].as_array().unwrap().iter().all(|x| x["kind"] != "restart"), "{again:#}");

    // clip.retention lists them as take fixes (bad_take).
    let r = common::call(&reg, &ctx, "clip.retention", json!({"clip_id": id, "timeline_only": true, "per_second": false})).await.unwrap();
    let tf = r["take_fixes"].as_array().expect("take_fixes");
    assert!(tf.iter().all(|x| x["action"] == "bad_take" && x["command"].as_str().unwrap().contains("edit remove-words")), "{tf:?}");
}

#[tokio::test]
async fn approximate_timings_are_skipped_and_named() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let (c, src) = clip_on(&store, &ctx, WordTiming::Approximate).await;
    let out = common::call(&reg, &ctx, "edit.find_bad_takes", json!({"clip_id": c["id"]})).await.unwrap();
    assert!(out["findings"].as_array().unwrap().is_empty());
    assert_eq!(out["words_checked"], 0);
    let sk = &out["skipped_sources"][0];
    assert_eq!(sk["source_id"], src);
    assert!(sk["reason"].as_str().unwrap().contains("approximate"));
    let e = common::call(&reg, &ctx, "edit.find_bad_takes", json!({"clip_id": c["id"], "min_confidence": 2.0})).await.unwrap_err();
    assert_eq!(common::code(&e), rustler_core::ErrorCode::Validation);
}

const TRAILER: [&str; 3] = [
    "Was there any point ever in your life where nobody else was convinced but just you?",
    "Launching the product was kind of an example of that and most people thought it was too early.",
    "There is no way that anyone was going to build super intelligence and not have a crazy life.",
];

/// A 20-minute episode: a 3-line trailer in the first ~20 s, a subscribe request, then unique
/// sentences every 5 s with the three trailer lines said again (in full) at 400, 700 and 1000 s.
fn episode() -> Vec<Word> {
    let mut w: Vec<Word> = Vec::new();
    let push = |text: &str, t0: f64, w: &mut Vec<Word>| {
        let n = w.len() as u32;
        w.extend(say(text, t0, n));
    };
    push(TRAILER[0], 1.0, &mut w);
    push(TRAILER[1], 7.0, &mut w);
    push(TRAILER[2], 14.0, &mut w);
    push("Before we start please subscribe to the channel today.", 21.0, &mut w);
    let mut planted = [400.0, 700.0, 1000.0].into_iter().zip(TRAILER).peekable();
    let mut t = 30.0;
    let mut k = 0;
    while t < 1190.0 {
        let line = match planted.peek() {
            Some((at, l)) if t >= *at => {
                let l = l.to_string();
                planted.next();
                l
            }
            _ if k % 3 == 0 => format!("Why does topic{k} alpha{k} beta{k} gamma{k} matter so much?"),
            _ => format!("Topic{k} alpha{k} beta{k} gamma{k} delta{k} epsilon{k} zeta{k} matters."),
        };
        push(&line, t, &mut w);
        t += 6.0;
        k += 1;
    }
    w
}

#[tokio::test]
async fn intro_trailer_is_skipped_and_its_originals_ranked() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("episode.mp4"), probe_1080p(1200.0, true, true));
    transcript(&store, &src.id, episode(), WordTiming::Exact);

    let out = common::call(&reg, &ctx, "highlights.candidates", json!({"source_id": src.id, "count": 30, "min_len": 15, "max_len": 40})).await.unwrap();
    let tr = &out["intro_trailer"];
    assert!(tr.is_object(), "trailer reported: {out:#}");
    assert_eq!(tr["start"], 0.0);
    let end = tr["end"].as_f64().unwrap();
    assert!(end > 15.0 && end < 25.0, "{tr:#}");
    assert_eq!(tr["excluded"], true);
    assert_eq!(tr["repeats"].as_array().unwrap().len(), 3);
    assert!(tr["skipped_windows"].as_u64().unwrap() > 0);
    let cands = out["candidates"].as_array().unwrap();
    assert!(cands.iter().all(|c| c["start"].as_f64().unwrap() >= end - 1e-6), "no candidate starts in the trailer");
    assert!(cands.iter().any(|c| c["reasons"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("intro trailer features this line"))));

    // include_intro keeps the trailer windows in the ranking.
    let all = common::call(&reg, &ctx, "highlights.candidates", json!({"source_id": src.id, "count": 30, "min_len": 15, "max_len": 40, "include_intro": true})).await.unwrap();
    assert_eq!(all["intro_trailer"]["excluded"], false);
    assert!(all["candidates"].as_array().unwrap().iter().any(|c| c["start"].as_f64().unwrap() < end));

    // clips.auto reports it and never picks inside it.
    let plan = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "count": 4, "min_len": 15, "max_len": 40, "dry_run": true})).await.unwrap();
    assert!(plan["ranking"]["intro_trailer"].is_object(), "{plan:#}");
    for c in plan["clips"].as_array().unwrap() {
        assert!(c["start"].as_f64().unwrap() >= end - 1e-6);
    }
}

#[tokio::test]
async fn clips_auto_suggests_hooks_and_a_cold_open_without_applying() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(200.0, true, true));
    // One moment that opens on a question, followed by strong answer lines.
    let mut words = say("Welcome back everyone to the show today.", 0.5, 0);
    let mut t = 5.0;
    for line in [
        "What's the worst advice you've gotten as a founder?",
        "Almost all the advice is terrible, honestly.",
        "Listen to less advice and trust your own instinct always.",
        "The biggest mistake is copying everyone else.",
        "Nobody tells you that the secret is patience over ten years.",
        "Build one thing that users love and then stop.",
    ] {
        let n = words.len() as u32;
        words.extend(say(line, t, n));
        t += line.split_whitespace().count() as f64 * 0.3 + 0.6;
    }
    transcript(&store, &src.id, words, WordTiming::Exact);
    let plan = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "ranges": [{"start": 4.9, "end": t}], "dry_run": true})).await.unwrap();
    let fixes = plan["clips"][0]["fixes_suggested"].as_array().unwrap();
    let hooks: Vec<&Value> = fixes.iter().filter(|f| f["action"] == "hook_title").collect();
    assert!((2..=3).contains(&hooks.len()), "{fixes:#?}");
    for h in &hooks {
        let cmd = h["command"].as_str().unwrap();
        assert!(cmd.starts_with("rustler overlay add-text <clip_id> \"") && cmd.ends_with("--style hook-big"), "{cmd}");
        assert!(cmd.split('"').nth(1).unwrap().split_whitespace().count() <= 7, "{cmd}");
    }
    let cold = fixes.iter().find(|f| f["action"] == "cold_open").expect("cold open suggested");
    assert!(cold["command"].as_str().unwrap().starts_with("rustler edit cold-open <clip_id> --start "));
    assert!(cold["why"].as_str().unwrap().contains("opens on a question"));
    assert_eq!(store.load_project().unwrap().clips.len(), 0, "a plan creates nothing");

    // A created clip gets the same suggestions with its id and source seconds (what a real run adds).
    let c = common::call(&reg, &ctx, "clip.create", json!({"name": "q", "source_id": src.id, "start": 4.9, "end": t, "raw": true})).await.unwrap();
    let cid: rustler_core::ClipId = serde_json::from_value(c["id"].clone()).unwrap();
    let before = store.load_clip(&cid).unwrap();
    let s = rustler_commands::auto_cmds::content_suggestions(&store, &cid);
    assert!(s.iter().filter(|f| f.action == "hook_title").count() >= 2, "{s:?}");
    let cold = s.iter().find(|f| f.action == "cold_open").expect("cold open");
    let cmd = cold.command.clone().unwrap();
    assert!(cmd.starts_with(&format!("rustler edit cold-open {cid} --start ")), "{cmd}");
    let start: f64 = cmd.split_whitespace().nth(5).unwrap().parse().unwrap();
    assert!(start > 5.0 && start < t, "source seconds: {start}");
    assert_eq!(store.load_clip(&cid).unwrap(), before, "suggestions never change the clip");
    // The cold-open command runs as given.
    let ran = common::call(&reg, &ctx, "edit.cold_open", json!({"clip_id": cid, "start": start, "end": cmd.split_whitespace().nth(7).unwrap().parse::<f64>().unwrap(), "flash": false})).await;
    assert!(ran.is_ok(), "{ran:?}");
}

#[tokio::test]
async fn captions_join_hyphen_tails_and_drop_a_leading_partial_word() {
    let (_tmp, store, _ctx) = new_project();
    let src = add_source(&store, Path::new("talk.mp4"), probe_1080p(60.0, true, true));
    // Trial 3: whisper split "chop-up" into "chop" + "-up,".
    let words = vec![
        word(0, "launch", 1.0, 1.6),
        word(1, "a", 1.65, 1.75),
        word(2, "chop", 1.8, 1.95),
        word(3, "-up,", 1.95, 2.05),
        word(4, "but", 2.2, 2.3),
        word(5, "most", 2.35, 2.6),
    ];
    let clip = {
        let mut c = rustler_core::Clip {
            schema: "rustler.clip@1".into(),
            id: rustler_core::ClipId::new(),
            name: "c".into(),
            canvas: Default::default(),
            segments: vec![],
            captions: Default::default(),
            audio: Default::default(),
            renders: vec![],
            created_at: chrono::Utc::now(),
            revision: 1,
        };
        let layout = rustler_core::layout::preset("center-crop", 1920, 1080, &c.canvas, &Default::default()).unwrap();
        // Starts inside "launch" (1.0-1.6: its midpoint is in the clip, but 0.25 s of it is cut off).
        c.segments.push(rustler_core::Segment { id: rustler_core::SegmentId::new(), source_id: src.id.clone(), start: 1.25, end: 2.8, layout });
        c
    };
    let by = std::collections::HashMap::from([(src.id.clone(), words)]);
    let cw = rustler_commands::clip_cmds::build_caption_words(&clip, &by);
    let texts: Vec<&str> = cw.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["a", "chop-up,", "but", "most"], "partial first word dropped, tail joined");
}
