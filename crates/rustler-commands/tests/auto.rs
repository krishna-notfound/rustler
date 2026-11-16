//! clips.auto (f37): dry-run plan, count + non-overlap, per-clip failure isolation, and a real
//! end-to-end run on `~/.rustler/test-media` (skipped with a printed reason when ffmpeg, uv or the
//! engine are missing). No network.

mod common;
mod support;

use common::{add_source, new_project, probe_1080p, word, write_transcript};
use rustler_core::{ProjectStore, Word};
use rustler_registry::{Ctx, ProgressEvent, ProgressSink};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// `n` sentences of 10 words each, 5 s apart (4.5 s of speech + 0.5 s gap), starting at 0.5 s.
/// Some open with a question or an emphasis word so the hook score has something to see.
fn sentences(n: usize) -> Vec<Word> {
    let mut out = Vec::new();
    let mut i = 0u32;
    for k in 0..n {
        let text: Vec<String> = match k % 4 {
            0 => format!("Why do you think point {k} works for people here?"),
            1 => format!("Nobody tells beginners the secret behind step {k} early enough."),
            2 => format!("Money grows when habits like rule {k} stay simple daily."),
            _ => format!("Teams copy idea {k} because results show up within weeks."),
        }
        .split_whitespace()
        .map(String::from)
        .collect();
        let t0 = 0.5 + k as f64 * 5.0;
        for (j, w) in text.iter().enumerate() {
            let s = t0 + j as f64 * 0.45;
            out.push(word(i, w, s, s + 0.4));
            i += 1;
        }
    }
    out
}

fn capture(ctx: &mut Ctx) -> Arc<Mutex<Vec<ProgressEvent>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    ctx.progress = ProgressSink::new(move |e| sink.lock().unwrap().push(e));
    events
}

fn ranges_of(out: &Value) -> Vec<(f64, f64)> {
    out["clips"].as_array().unwrap().iter().map(|c| (c["start"].as_f64().unwrap(), c["end"].as_f64().unwrap())).collect()
}

fn assert_no_overlap(r: &[(f64, f64)]) {
    for (a, x) in r.iter().enumerate() {
        for y in &r[a + 1..] {
            assert!(x.1 <= y.0 + 1e-6 || y.1 <= x.0 + 1e-6, "picks overlap: {x:?} and {y:?}");
        }
    }
}

fn clip_count(store: &ProjectStore) -> usize {
    store.load_project().unwrap().clips.len()
}

#[tokio::test]
async fn dry_run_plans_count_non_overlapping_clips_without_writing() {
    let reg = common::registry();
    let (_tmp, store, mut ctx) = new_project();
    let src = add_source(&store, std::path::Path::new("talk.mp4"), probe_1080p(152.0, true, true));
    write_transcript(&store, &src.id, sentences(30));
    let events = capture(&mut ctx);

    let out = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "count": 3, "dry_run": true})).await.unwrap();
    assert_eq!(out["dry_run"], json!(true));
    let clips = out["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 3, "count respected: {out:#}");
    let r = ranges_of(&out);
    assert_no_overlap(&r);
    for (c, (s, e)) in clips.iter().zip(&r) {
        assert_eq!(c["status"], "planned");
        assert!(e - s >= 25.0 - 0.5 && e - s <= 60.0 + 0.5, "length within min/max: {s}-{e}");
        assert!(c["why"]["rank_score"].as_f64().unwrap() > 0.0);
        assert!(c["why"]["hook_2s"].as_f64().is_some());
        let plan: Vec<&str> = c["plan"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        assert!(plan[0].starts_with("rustler clip create --name auto-"), "{plan:?}");
        assert!(plan.iter().any(|p| p.starts_with("rustler render batch")));
        assert!(!c["change"].as_array().unwrap().is_empty());
    }
    // Picks come back in time order.
    assert!(r.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(clip_count(&store), 0, "a dry run creates nothing");
    assert_eq!(out["summary"]["picked"], json!(3));
    let stages: Vec<String> = events.lock().unwrap().iter().map(|e| e.stage.clone()).collect();
    for s in ["clips.auto.resolve", "clips.auto.rank", "clips.auto.done"] {
        assert!(stages.iter().any(|x| x == s), "missing progress stage {s}: {stages:?}");
    }

    // Fewer windows than asked: the count is an upper bound and a warning says why.
    let out = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "count": 20, "min_len": 50, "max_len": 60, "dry_run": true})).await.unwrap();
    let n = out["clips"].as_array().unwrap().len();
    assert!((1..20).contains(&n), "{n}");
    assert_no_overlap(&ranges_of(&out));
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("only")));
}

#[tokio::test]
async fn picks_avoid_moments_that_already_have_a_clip() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, std::path::Path::new("talk.mp4"), probe_1080p(152.0, true, true));
    write_transcript(&store, &src.id, sentences(30));
    let first = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "count": 2, "dry_run": true})).await.unwrap();
    let taken = ranges_of(&first)[0];
    // An existing clip on the best moment (raw: no media needed).
    common::call(&reg, &ctx, "clip.create", json!({"name": "mine", "source_id": src.id, "start": taken.0, "end": taken.1, "raw": true})).await.unwrap();
    let again = common::call(&reg, &ctx, "clips.auto", json!({"source_id": src.id, "count": 2, "dry_run": true})).await.unwrap();
    let r = ranges_of(&again);
    assert_eq!(r.len(), 2);
    for x in &r {
        assert!(x.1 <= taken.0 + 1e-6 || taken.1 <= x.0 + 1e-6, "{x:?} overlaps the existing clip {taken:?}");
    }
    assert!(again["ranking"]["excluded"].as_array().unwrap().iter().any(|e| e["reason"].as_str().unwrap().contains("existing clip")));
}

#[tokio::test]
async fn bad_input_is_rejected_before_anything_runs() {
    let reg = common::registry();
    let (_tmp, store, ctx) = new_project();
    let src = add_source(&store, std::path::Path::new("talk.mp4"), probe_1080p(60.0, true, true));
    for bad in [
        json!({"count": 2}),
        json!({"source_id": src.id, "url": "https://example.com/v", "count": 2}),
        json!({"source_id": src.id, "count": 0}),
        json!({"source_id": src.id, "min_len": 40, "max_len": 30}),
        json!({"source_id": src.id, "ranges": [{"start": 5, "end": 15}, {"start": 10, "end": 20}]}),
        json!({"source_id": src.id, "template": "no-such-template"}),
    ] {
        assert!(common::call(&reg, &ctx, "clips.auto", bad.clone()).await.is_err(), "accepted {bad}");
    }
    assert_eq!(clip_count(&store), 0);
}

/// One bad range (past the end of the source) fails at `create`; the other clips go on. With
/// ffmpeg they render, pass QC and export (post.json skeleton); without it they stop at render.
#[tokio::test]
async fn one_failing_clip_does_not_stop_the_others() {
    let reg = common::registry();
    let tools = common::tools_home();
    let (_tmp, store, mut ctx) = new_project();
    let src_id = match &tools {
        Some((_t, home)) => {
            ctx.home = home.clone();
            let video = store.root.join("talk.mp4");
            common::make_video(&common::ffmpeg_in(home), &video, 60, 640, 360, true);
            let v = common::call(&reg, &ctx, "source.import", json!({"input": video})).await.unwrap();
            v["sources"][0]["id"].as_str().unwrap().parse().unwrap()
        }
        None => {
            eprintln!("SKIP (partly): no ffmpeg, the good clips are expected to fail at render");
            add_source(&store, std::path::Path::new("talk.mp4"), probe_1080p(60.0, true, true)).id
        }
    };
    write_transcript(&store, &src_id, sentences(12));
    let events = capture(&mut ctx);
    let out = common::call(
        &reg,
        &ctx,
        "clips.auto",
        json!({
            "source_id": src_id, "raw": true, "quality": "draft",
            "ranges": [{"start": 5.4, "end": 14.6, "note": "first"}, {"start": 20.4, "end": 29.6}, {"start": 50.0, "end": 80.0}]
        }),
    )
    .await
    .unwrap();
    let clips = out["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 3, "{out:#}");
    assert_eq!(clips[2]["status"], "failed");
    assert_eq!(clips[2]["failed_stage"], "create", "{:#}", clips[2]);
    assert!(clips[2]["clip_id"].is_null());
    assert_eq!(clips[0]["why"]["chosen_by"], "agent");
    assert_eq!(clips[0]["why"]["reasons"][0], "first");
    for c in &clips[..2] {
        assert!(c["clip_id"].as_str().is_some_and(|s| s.starts_with("clp_")), "{c:#}");
        assert!(c["retention"]["before_fixes"].as_u64().is_some(), "{c:#}");
        assert!(!c["change"].as_array().unwrap().is_empty());
        if tools.is_some() {
            assert_eq!(c["status"], "done", "{c:#}");
            let video = c["export"]["video"].as_str().unwrap();
            assert!(std::path::Path::new(video).is_file(), "{video}");
            let post: Value = serde_json::from_str(&std::fs::read_to_string(c["export"]["post"].as_str().unwrap()).unwrap()).unwrap();
            assert_eq!(post["meta_present"], json!(false), "copy is left for the agent: {post:#}");
            assert!(post["title"].is_null());
            assert_eq!(c["retention"]["measured_on"], "render");
        } else {
            assert_eq!(c["failed_stage"], "render", "{c:#}");
        }
    }
    assert_eq!(out["summary"]["created"], json!(2));
    assert_eq!(out["summary"]["failed"], json!(if tools.is_some() { 1 } else { 3 }));
    assert_eq!(clip_count(&store), 2);
    if tools.is_some() {
        let ev = events.lock().unwrap();
        let render: Vec<&ProgressEvent> = ev.iter().filter(|e| e.stage == "clips.auto.render").collect();
        assert!(render.iter().any(|e| e.eta_s.is_some()), "render progress carries eta_s");
    }
}

/// Real end-to-end: a 105 s two-person interview, whisper transcript, ranking, smart defaults,
/// draft render, retention, QC and export.
#[tokio::test]
async fn end_to_end_on_test_media() {
    if !support::have_media_tools() || !support::have_engine() {
        return;
    }
    let media = support::home().join("test-media").join("boyle_twoshot_105s.mp4");
    if !media.is_file() {
        eprintln!("SKIP: {} missing", media.display());
        return;
    }
    let reg = support::registry();
    let (_dir, ctx) = support::project();
    let v = support::ok(&reg, "source.import", &ctx, json!({"input": media})).await;
    let src = v["sources"][0]["id"].as_str().unwrap().to_string();
    let t = std::time::Instant::now();
    let out = support::ok(&reg, "clips.auto", &ctx, json!({"source_id": src, "count": 2, "min_len": 15, "max_len": 35, "quality": "draft"})).await;
    eprintln!("clips.auto took {:.1} s: {}", t.elapsed().as_secs_f64(), serde_json::to_string_pretty(&out["summary"]).unwrap());
    let clips = out["clips"].as_array().unwrap();
    assert!(!clips.is_empty() && clips.len() <= 2, "{out:#}");
    assert_no_overlap(&ranges_of(&out));
    for c in clips {
        eprintln!(
            "{} {} {} retention {}->{} fixes {} qc {} {}",
            c["name"], c["mmss"], c["status"], c["retention"]["before_fixes"], c["retention"]["score"],
            c["fixes_applied"].as_array().map_or(0, Vec::len), c["qc"]["verdict"], c["error"]["message"]
        );
    }
    let done: Vec<&Value> = clips.iter().filter(|c| c["status"] == "done").collect();
    assert!(!done.is_empty(), "at least one clip exported: {out:#}");
    for c in done {
        assert!(std::path::Path::new(c["export"]["video"].as_str().unwrap()).is_file());
        assert!(std::path::Path::new(c["export"]["post"].as_str().unwrap()).is_file());
        assert!(c["retention"]["score"].as_u64().is_some());
        assert_eq!(c["retention"]["measured_on"], "render");
    }
}
