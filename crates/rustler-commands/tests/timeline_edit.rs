//! edit.insert_segment, edit.cold_open, clip.merge, clip.duplicate, edit.speed, edit.freeze —
//! plus a real render of speed + freeze + cold open checked by QC (skips without ffmpeg).
mod common;

use common::*;
use rustler_commands::{clip_cmds, fx_cmds};
use rustler_core::{ErrorCode, ProjectStore};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

fn extras_json(store: &ProjectStore, clip: &str) -> Value {
    let p = store.root.join("clips").join(clip).join("extras.json");
    if !p.is_file() {
        return json!({});
    }
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

struct Fx {
    _proj: tempfile::TempDir,
    store: ProjectStore,
    ctx: rustler_registry::Ctx,
    reg: rustler_registry::Registry,
    src: rustler_core::Source,
}

impl Fx {
    async fn run(&self, name: &str, input: Value) -> Value {
        call(&self.reg, &self.ctx, name, input.clone()).await.unwrap_or_else(|e| panic!("{name} {input}: {e:?}"))
    }
    async fn fail(&self, name: &str, input: Value) -> rustler_core::RustlerError {
        call(&self.reg, &self.ctx, name, input).await.unwrap_err()
    }
    /// Clip [1.0, 2.4] + [3.0, 5.0] + [5.05, 7.0] (edit 0..1.4, 1.4..3.4, 3.4..5.35).
    async fn clip(&self, name: &str) -> (String, Vec<String>) {
        let c = self.run("clip.create", json!({"name": name, "source_id": self.src.id, "start": 1.0, "end": 2.4})).await;
        let id = c["id"].as_str().unwrap().to_string();
        self.run("clip.add_segment", json!({"clip_id": id, "source_id": self.src.id, "start": 3.0, "end": 5.0})).await;
        let c = self.run("clip.add_segment", json!({"clip_id": id, "source_id": self.src.id, "start": 5.05, "end": 7.0})).await;
        (id, seg_ids(&c))
    }
    async fn show(&self, id: &str) -> Value {
        self.run("timeline.show", json!({"clip_id": id})).await
    }
}

fn seg_ids(v: &Value) -> Vec<String> {
    let list = v.get("segments").or_else(|| v.get("video")).unwrap();
    list.as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

fn fx() -> Fx {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    Fx { _proj: proj, store, ctx, reg: registry(), src }
}

fn item<'a>(show: &'a Value, track: &str, id: &str) -> &'a Value {
    show["resolved"]["items"].as_array().unwrap().iter().find(|i| i["track"] == track && i["id"] == id).unwrap_or_else(|| panic!("{track} {id}"))
}

#[tokio::test]
async fn insert_segment_at_index_and_at_out_t() {
    let x = fx();
    let (id, segs) = x.clip("ins").await;
    x.run("overlay.add_text", json!({"clip_id": id, "text": "Ask not", "start": 2.0, "end": 3.0})).await;
    x.run("transition.set", json!({"clip_id": id, "after_segment_id": segs[1], "kind": "dissolve"})).await;

    // At index 1: everything after the first segment moves by 1 s.
    let r = x.run("edit.insert_segment", json!({"clip_id": id, "source_id": x.src.id, "start": 8.0, "end": 9.0, "at_index": 1})).await;
    let ids = seg_ids(&r);
    assert_eq!(ids.len(), 4);
    assert_eq!(ids[1], r["new_segment_id"].as_str().unwrap());
    assert_eq!((ids[0].as_str(), ids[2].as_str(), ids[3].as_str()), (segs[0].as_str(), segs[1].as_str(), segs[2].as_str()));
    assert!(close(f(&r["edit_duration"]), 6.35));
    let ov = &extras_json(&x.store, &id)["overlays"][0];
    assert_eq!((f(&ov["start"]), f(&ov["end"])), (3.0, 4.0));
    // The inserted segment takes the neighbour's layout (same source -> same regions).
    let clip = x.store.load_clip(&id.parse().unwrap()).unwrap();
    assert_eq!(clip.segments[1].layout, clip.segments[0].layout);

    // At an out time inside a segment: it is split there; the overlay that starts at the split
    // follows its words to after the inserted piece; the transition moves onto the last piece.
    let r = x.run("edit.insert_segment", json!({"clip_id": id, "source_id": x.src.id, "start": 9.0, "end": 9.5, "at_out_t": 3.0})).await;
    let ids = seg_ids(&r);
    assert_eq!(ids.len(), 6, "{r}");
    let clip = x.store.load_clip(&id.parse().unwrap()).unwrap();
    assert_eq!(clip.segments[2].id.as_str(), segs[1], "first piece keeps the id");
    assert!(close(clip.segments[2].end, 3.6) && close(clip.segments[4].start, 3.6), "{:?}", clip.segments);
    assert!(close(clip.segments[3].start, 9.0));
    let xj = extras_json(&x.store, &id);
    assert_eq!((f(&xj["overlays"][0]["start"]), f(&xj["overlays"][0]["end"])), (3.5, 4.5));
    assert_eq!(xj["transitions"][0]["after_segment_id"].as_str().unwrap(), clip.segments[4].id.as_str());
    // On a boundary: no split.
    let r = x.run("edit.insert_segment", json!({"clip_id": id, "source_id": x.src.id, "start": 8.0, "end": 8.5, "at_out_t": 1.4})).await;
    assert_eq!(seg_ids(&r).len(), 7);
    assert_eq!(seg_ids(&r)[1], r["new_segment_id"].as_str().unwrap());

    // Validation: both / neither position, out of range, bad source range.
    for bad in [
        json!({"clip_id": id, "source_id": x.src.id, "start": 8.0, "end": 9.0}),
        json!({"clip_id": id, "source_id": x.src.id, "start": 8.0, "end": 9.0, "at_index": 0, "at_out_t": 1.0}),
        json!({"clip_id": id, "source_id": x.src.id, "start": 8.0, "end": 9.0, "at_index": 99}),
        json!({"clip_id": id, "source_id": x.src.id, "start": 9.0, "end": 12.0, "at_index": 0}),
    ] {
        let e = x.fail("edit.insert_segment", bad.clone()).await;
        assert_eq!(code(&e), ErrorCode::Validation, "{bad} -> {e:?}");
        assert_eq!(e.stage, "edit.insert_segment");
    }
}

#[tokio::test]
async fn cold_open_moves_range_and_adds_flash() {
    let x = fx();
    let (id, segs) = x.clip("cold").await;
    // A punch-in on "ask" (source 4.05-4.3 -> edit 2.45-2.7), an sfx on word 4 ("Americans").
    x.run("effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 2.45, "end": 2.7})).await;
    x.run("audio.add_sfx", json!({"clip_id": id, "file": x.store.resolve(&x.src.media_path), "at_word": {"source_id": x.src.id, "word_index": 4}})).await;
    x.run("overlay.add_text", json!({"clip_id": id, "text": "Ending", "start": 4.0, "end": 5.0, "style": "lower-third"})).await;
    x.run("overlay.add_text", json!({"clip_id": id, "text": "Hook title", "start": 0.0, "end": 1.2})).await;

    // Hook = "Americans ask" (source 3.45-4.3), inside the middle segment.
    let r = x.run("edit.cold_open", json!({"clip_id": id, "start": 3.45, "end": 4.3})).await;
    let clip = x.store.load_clip(&id.parse().unwrap()).unwrap();
    let spans: Vec<(f64, f64)> = clip.segments.iter().map(|s| (s.start, s.end)).collect();
    assert_eq!(spans.len(), 5, "{spans:?}");
    assert!(close(spans[0].0, 3.45) && close(spans[0].1, 4.3), "hook first: {spans:?}");
    assert_eq!(clip.segments[0].id.as_str(), r["new_segment_id"].as_str().unwrap());
    assert!(close(spans[2].0, 3.0) && close(spans[2].1, 3.45) && close(spans[3].0, 4.3) && close(spans[3].1, 5.0), "{spans:?}");
    assert_eq!(clip.segments[2].id.as_str(), segs[1], "the piece before the hook keeps the id");
    assert!(close(f(&r["edit_duration"]), 5.35), "nothing added, nothing lost");
    let xj = extras_json(&x.store, &id);
    // The punch-in moved with "ask" into the hook (source 4.05 -> edit 0.6).
    let fx1 = xj["effects"].as_array().unwrap().iter().find(|e| e["kind"] == "punch-in").unwrap();
    assert!(close(f(&fx1["start"]), 0.6) && close(f(&fx1["end"]), 0.85), "{fx1}");
    // Flash at the cut out of the hook (0.85 s).
    let flash = xj["effects"].as_array().unwrap().iter().find(|e| e["kind"] == "flash").unwrap();
    assert!(close(f(&flash["start"]), 0.85) && close(f(&flash["end"]), 1.1), "{flash}");
    // The ending overlay keeps its words (it was after the hook range).
    assert!(close(f(&xj["overlays"][0]["start"]), 4.0));
    // The hook title that opened the clip keeps opening it.
    assert!(close(f(&xj["overlays"][1]["start"]), 0.0) && close(f(&xj["overlays"][1]["end"]), 1.2), "{}", xj["overlays"][1]);
    // The word-anchored sfx now plays inside the hook.
    let s = x.show(&id).await;
    assert!(close(f(&item(&s, "sfx", "sfx_1")["start"]), 0.0), "{}", item(&s, "sfx", "sfx_1"));

    // Undo brings the original cut and layers back.
    x.run("history.undo", json!({"clip_id": id})).await;
    let clip = x.store.load_clip(&id.parse().unwrap()).unwrap();
    assert_eq!(clip.segments.len(), 3);
    assert_eq!(extras_json(&x.store, &id)["effects"].as_array().unwrap().len(), 1);

    // A whole segment as the hook keeps its id; a range from outside the clip is just inserted.
    let r = x.run("edit.cold_open", json!({"clip_id": id, "start": 5.05, "end": 7.0, "flash": false})).await;
    assert_eq!(seg_ids(&r)[0], segs[2]);
    assert_eq!(seg_ids(&r).len(), 3);
    let r = x.run("edit.cold_open", json!({"clip_id": id, "start": 8.0, "end": 8.6})).await;
    assert_eq!(seg_ids(&r).len(), 4);
    assert!(close(f(&r["edit_duration"]), 5.95));
    // The whole clip as the hook is refused.
    let e = x.fail("edit.cold_open", json!({"clip_id": id, "start": 0.5, "end": 9.9})).await;
    assert_eq!(code(&e), ErrorCode::Validation);
}

#[tokio::test]
async fn cold_open_by_units() {
    let x = fx();
    let (id, _) = x.clip("units").await;
    // Unit 2 = "What your country can do." (the last segment's sentence).
    let r = x.run("edit.cold_open", json!({"clip_id": id, "start_unit": 2, "end_unit": 2, "expect_text": "What your"})).await;
    let clip = x.store.load_clip(&id.parse().unwrap()).unwrap();
    assert!(clip.segments[0].start <= 5.05 + 1e-9 && clip.segments[0].end >= 7.0 - 0.3, "{:?}", clip.segments[0]);
    assert!(r["changes"].to_string().contains("video"));
    let e = x.fail("edit.cold_open", json!({"clip_id": id, "start_unit": 2, "end_unit": 2, "expect_text": "Something else"})).await;
    assert_eq!(code(&e), ErrorCode::Validation);
}

#[tokio::test]
async fn merge_keeps_layers_with_correct_times() {
    let x = fx();
    // A: one segment [1.0, 2.4] (1.4 s) with a hook overlay + sfx.
    let a = x.run("clip.create", json!({"name": "a", "source_id": x.src.id, "start": 1.0, "end": 2.4})).await;
    let a = a["id"].as_str().unwrap().to_string();
    x.run("overlay.add_text", json!({"clip_id": a, "text": "Hook", "start": 0.0, "end": 1.0})).await;
    x.run("audio.add_sfx", json!({"clip_id": a, "file": x.store.resolve(&x.src.media_path), "at": 0.5})).await;
    // B: two segments (2.0 + 1.95 s) with an overlay (same id ovl_1), an effect, a dissolve and a
    // mask anchored to word 7 ("What", source 5.05 -> B edit 2.0).
    let b = x.run("clip.create", json!({"name": "b", "source_id": x.src.id, "start": 3.0, "end": 5.0})).await;
    let b = b["id"].as_str().unwrap().to_string();
    let bc = x.run("clip.add_segment", json!({"clip_id": b, "source_id": x.src.id, "start": 5.05, "end": 7.0})).await;
    let bsegs = seg_ids(&bc);
    x.run("overlay.add_text", json!({"clip_id": b, "text": "Middle", "start": 0.5, "end": 1.5})).await;
    x.run("effect.add", json!({"clip_id": b, "kind": "punch-in", "start": 1.0, "end": 2.0})).await;
    x.run("transition.set", json!({"clip_id": b, "after_segment_id": bsegs[0], "kind": "dissolve"})).await;
    x.run(
        "mask.add",
        json!({"clip_id": b, "kind": "box", "rect": {"x": 0.1, "y": 0.1, "w": 0.2, "h": 0.2}, "start_word": {"source_id": x.src.id, "word_i": 7}, "end_word": {"source_id": x.src.id, "word_i": 9}}),
    )
    .await;
    let b_before = extras_json(&x.store, &b);

    let m = x.run("clip.merge", json!({"clip_ids": [a, b], "name": "a+b", "transition": "fade", "transition_duration": 0.2})).await;
    let mid = m["clip_id"].as_str().unwrap().to_string();
    assert_ne!(mid, a);
    assert!(close(f(&m["edit_duration"]), 5.35));
    let ids = seg_ids(&m);
    assert_eq!(ids.len(), 3);
    assert!(!ids.contains(&bsegs[0]), "fresh segment ids");
    let xj = extras_json(&x.store, &mid);
    let ov = xj["overlays"].as_array().unwrap();
    assert_eq!(ov.len(), 2);
    assert_eq!((ov[0]["id"].as_str(), f(&ov[0]["start"]), f(&ov[0]["end"])), (Some("ovl_1"), 0.0, 1.0));
    assert_eq!(ov[1]["id"], "ovl_2", "colliding id renumbered");
    assert!(close(f(&ov[1]["start"]), 1.9) && close(f(&ov[1]["end"]), 2.9), "B shifted by A's 1.4 s: {}", ov[1]);
    assert!(close(f(&xj["effects"][0]["start"]), 2.4));
    assert!(close(f(&xj["sfx"][0]["at"]), 0.5));
    let tr = xj["transitions"].as_array().unwrap();
    assert_eq!(tr.len(), 2);
    assert_eq!((tr[0]["after_segment_id"].as_str(), tr[0]["kind"].as_str()), (Some(ids[0].as_str()), Some("fade")));
    assert_eq!((tr[1]["after_segment_id"].as_str(), tr[1]["kind"].as_str()), (Some(ids[1].as_str()), Some("dissolve")));
    assert_eq!(xj["masks"][0]["start_word"]["word_i"], 7, "anchors kept");
    let s = x.show(&mid).await;
    let mask = item(&s, "masks", "msk_1");
    assert!(close(f(&mask["start"]), 3.4), "anchor resolves in the merged clip: {mask}");
    assert_eq!(xj["meta"]["merged_from"], json!([a, b]));
    // The project lists it; the sources are untouched.
    let p = x.store.load_project().unwrap();
    assert!(p.clips.iter().any(|c| c.as_str() == mid));
    assert_eq!(extras_json(&x.store, &b), b_before);
    // The same clip twice works too (hook + full).
    let m2 = x.run("clip.merge", json!({"clip_ids": [b, b], "name": "b twice"})).await;
    assert!(close(f(&m2["edit_duration"]), 7.9));
    let e = x.fail("clip.merge", json!({"clip_ids": [a], "name": "one"})).await;
    assert_eq!(code(&e), ErrorCode::Validation);
}

#[tokio::test]
async fn duplicate_copies_clip_and_layers() {
    let x = fx();
    let (id, _) = x.clip("orig").await;
    x.run("overlay.add_text", json!({"clip_id": id, "text": "Hook", "start": 0.0, "end": 1.0})).await;
    x.run("overlay.progress_bar", json!({"clip_id": id, "enabled": true})).await;
    let d = x.run("clip.duplicate", json!({"clip_id": id, "name": "copy"})).await;
    let did = d["clip_id"].as_str().unwrap().to_string();
    let (a, b) = (extras_json(&x.store, &id), extras_json(&x.store, &did));
    assert_eq!(a["overlays"], b["overlays"]);
    assert_eq!(a["progress_bar"], b["progress_bar"]);
    assert_eq!(b["meta"]["duplicated_from"], id.as_str());
    let (c1, c2) = (x.store.load_clip(&id.parse().unwrap()).unwrap(), x.store.load_clip(&did.parse().unwrap()).unwrap());
    assert_eq!(c1.segments, c2.segments);
    assert_eq!(c2.name, "copy");
    assert!(c2.renders.is_empty());
}

#[tokio::test]
async fn speed_retimes_captions_and_duration() {
    let x = fx();
    let (id, segs) = x.clip("speed").await;
    let r = x.run("edit.speed", json!({"clip_id": id, "segment_id": segs[1], "factor": 2.0})).await;
    assert!(close(f(&r["edit_duration"]), 5.35));
    assert!(close(f(&r["rendered_duration"]), 4.35), "{r}");
    assert_eq!(extras_json(&x.store, &id)["segment_speed"][&segs[1]], 2.0);
    let clip = x.store.load_clip(&id.parse().unwrap()).unwrap();
    // QC / retention / render use this.
    assert!(close(fx_cmds::effective_duration_of(&x.store, &clip).unwrap(), 4.35));
    // Captions: "ask" (source 4.05-4.3 -> edit 2.45-2.7) plays at 1.4 + 1.05/2 .. 1.4 + 1.3/2.
    let words: HashMap<_, _> = [(x.src.id.clone(), sample_words())].into_iter().collect();
    let cw = clip_cmds::build_caption_words(&clip, &words);
    let tmap = fx_cmds::transition_map(&x.store, &clip).unwrap();
    let mapped = fx_cmds::map_caption_words(cw, &tmap);
    let ask = mapped.iter().find(|w| w.text.contains("ask")).unwrap();
    assert!(close(ask.out_start, 1.925) && close(ask.out_end, 2.05), "{} {}", ask.out_start, ask.out_end);
    let do_ = mapped.iter().find(|w| w.text.contains("do.")).unwrap();
    assert!(close(do_.out_start, 3.4 - 2.0 + 1.0 + 1.3), "after the segment: shifted by -1.0 ({})", do_.out_start);
    // Reset with factor 1 removes the key; bad factor / segment.
    x.run("edit.speed", json!({"clip_id": id, "segment_id": segs[1], "factor": 1.0})).await;
    assert!(extras_json(&x.store, &id).get("segment_speed").is_none());
    assert_eq!(code(&x.fail("edit.speed", json!({"clip_id": id, "segment_id": segs[1], "factor": 3.0})).await), ErrorCode::Validation);
    assert_eq!(code(&x.fail("edit.speed", json!({"clip_id": id, "segment_id": "seg_nope", "factor": 1.5})).await), ErrorCode::NotFound);
}

#[tokio::test]
async fn freeze_extends_duration() {
    let x = fx();
    let (id, _) = x.clip("freeze").await;
    x.run("overlay.add_text", json!({"clip_id": id, "text": "Wait for it", "start": 2.0, "end": 3.0})).await;
    x.run("overlay.add_text", json!({"clip_id": id, "text": "Later", "start": 2.5, "end": 3.0, "style": "lower-third"})).await;
    let r = x.run("edit.freeze", json!({"clip_id": id, "out_t": 2.0, "duration": 1.0})).await;
    assert!(close(f(&r["rendered_duration"]), 6.35), "{r}");
    assert_eq!(extras_json(&x.store, &id)["freezes"][0]["id"], "frz_1");
    let s = x.show(&id).await;
    // An item starting on the freeze shows during the hold; a later one moves by the hold.
    let a = item(&s, "text", "ovl_1");
    assert!(close(f(&a["rendered_start"]), 2.0) && close(f(&a["rendered_end"]), 4.0), "{a}");
    let b = item(&s, "text", "ovl_2");
    assert!(close(f(&b["rendered_start"]), 3.5), "{b}");
    let fr = item(&s, "freezes", "frz_1");
    assert!(close(f(&fr["rendered_end"]), 3.0));
    for bad in [json!({"clip_id": id, "out_t": 99.0, "duration": 1.0}), json!({"clip_id": id, "out_t": 1.0, "duration": 0.0}), json!({"clip_id": id, "out_t": 1.0, "duration": 11.0})] {
        let e = x.fail("edit.freeze", bad.clone()).await;
        assert_eq!(code(&e), ErrorCode::Validation, "{bad}");
        assert!(e.message.contains("/tracks/freezes/1"), "{}", e.message);
    }
}

/// Real render: cold open + speed + freeze + dissolve; the file's duration matches the effective
/// duration (QC `duration` passes) and the frames show the hook first and the frozen frame.
#[tokio::test]
async fn render_speed_freeze_cold_open_passes_qc() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let ffmpeg = ffmpeg_in(&home);
    let video = proj.path().join("talk.mp4");
    make_video(&ffmpeg, &video, 10, 1920, 1080, true);
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let x = Fx { _proj: proj, store, ctx, reg: registry(), src };
    let (id, segs) = x.clip("real").await;
    x.run("transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 0.2})).await;
    x.run("edit.speed", json!({"clip_id": id, "segment_id": segs[1], "factor": 1.5})).await;
    x.run("edit.freeze", json!({"clip_id": id, "out_t": 4.0, "duration": 0.6})).await;
    let r = x.run("edit.cold_open", json!({"clip_id": id, "start": 8.0, "end": 9.0})).await;
    let expected = f(&r["rendered_duration"]);
    // 1.0 hook + 1.4 + 2.0/1.5 + 1.95 + 0.6 freeze - 0.2 dissolve.
    assert!(close(expected, 1.0 + 1.4 + 2.0 / 1.5 + 1.95 + 0.6 - 0.2), "{expected}");
    let rr = x.run("render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await;
    assert!(close(f(&rr["duration"]), expected));
    let out = Path::new(rr["path"].as_str().unwrap()).to_path_buf();
    let graph = std::fs::read_to_string(store_render_dir(&x.store, &id, rr["render_id"].as_str().unwrap()).join("graph.txt")).unwrap();
    assert!(graph.contains("setpts=PTS/1.5") && graph.contains("atempo=1.5") && graph.contains(",loop=loop=18:size=1:start="), "{graph}");
    let q = x.run("qc.run", json!({"clip_id": id, "allow_fail": true})).await;
    let dur = q["checks"].as_array().unwrap().iter().find(|c| c["name"] == "duration").unwrap();
    assert_eq!(dur["status"], "pass", "{dur}");
    // testsrc2 has a running timestamp: the frozen frames are identical, the hook differs from
    // the frame after the cut.
    let fr = |t: f64| -> Vec<u8> {
        let o = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{t:.3}"), "-i"])
            .arg(&out)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-s", "108x192", "-"])
            .output()
            .unwrap();
        o.stdout
    };
    let tm = fx_cmds::transition_map(&x.store, &x.store.load_clip(&id.parse().unwrap()).unwrap()).unwrap();
    let hold = tm.map_time(4.0 + 1.0); // the freeze moved by the 1 s hook
    let (a, b) = (fr(hold + 0.1), fr(hold + 0.45));
    let diff = a.iter().zip(&b).filter(|(p, q)| p.abs_diff(**q) > 12).count();
    assert!(diff < 30, "frozen frames differ in {diff} px");
    let (c, d) = (fr(hold - 0.5), fr(hold - 0.2));
    let moving = c.iter().zip(&d).filter(|(p, q)| p.abs_diff(**q) > 12).count();
    assert!(moving > 30, "picture moves before the freeze ({moving} px)");
}

fn store_render_dir(store: &ProjectStore, clip: &str, render: &str) -> std::path::PathBuf {
    store.root.join("clips").join(clip).join("renders").join(render)
}
