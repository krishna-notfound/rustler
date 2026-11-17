//! f40: caption placement away from faces and source subtitles (+ QC check), undo for single-layer
//! commands, template from a learned style, export / clip list ordered by score.
mod common;

use common::*;
use rustler_core::{ClipId, ErrorCode};
use serde_json::{Value, json};
use std::path::Path;

async fn clip_on(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source, name: &str, layout: &str) -> String {
    let c = call(reg, ctx, "clip.create", json!({"name": name, "source_id": src.id, "start": 1.0, "end": 7.0, "layout": layout, "raw": true}))
        .await
        .unwrap();
    c["id"].as_str().unwrap().to_string()
}

fn extras(store: &rustler_core::ProjectStore, id: &str) -> Value {
    let p = store.root.join("clips").join(id).join("extras.json");
    if !p.is_file() {
        return json!({});
    }
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn revision(store: &rustler_core::ProjectStore, id: &str) -> u64 {
    store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap().revision
}

// ---------------------------------------------------------------------------------------------
// Undo for single-layer commands
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_layer_edit_is_its_own_undo_step() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let id = clip_on(&reg, &ctx, &src, "u", "center-crop").await;

    // A caption change (a clip edit), then an overlay: undo removes only the overlay.
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "position_y": 0.66})).await.unwrap();
    let r0 = revision(&store, &id);
    let o = call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "WAIT FOR IT", "start": 0.0, "end": 2.0})).await.unwrap();
    assert_eq!(o["overlay_id"], "ovl_1");
    assert_eq!(revision(&store, &id), r0 + 1, "an overlay add is a new revision");
    assert_eq!(extras(&store, &id)["overlays"].as_array().unwrap().len(), 1);
    let u = call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert!(!u.to_string().contains("no layer snapshot"), "{u}");
    assert!(extras(&store, &id).get("overlays").is_none(), "undo removed the overlay: {}", extras(&store, &id));
    let c = call(&reg, &ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(c["clip"]["captions"]["position_y"], 0.66, "the caption edit before it stays");

    // Two layer edits -> two undo steps, newest first.
    call(&reg, &ctx, "mask.add", json!({"clip_id": id, "rect": {"x": 0.0, "y": 0.8, "w": 1.0, "h": 0.1}, "kind": "blur", "start": 0.0, "end": 3.0}))
        .await
        .unwrap();
    call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 1.0, "end": 2.0})).await.unwrap();
    let x = extras(&store, &id);
    assert!(x.get("masks").is_some() && x.get("effects").is_some(), "{x}");
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    let x = extras(&store, &id);
    assert!(x.get("masks").is_some() && x.get("effects").is_none(), "first undo removes the effect only: {x}");
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    let x = extras(&store, &id);
    assert!(x.get("masks").is_none() && x.get("effects").is_none(), "second undo removes the mask: {x}");
    let c = call(&reg, &ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(c["clip"]["captions"]["position_y"], 0.66);

    // A dry run and a no-op do not add revisions.
    let r = revision(&store, &id);
    let mut dry = ctx.clone();
    dry.dry_run = true;
    call(&reg, &dry, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 1.0, "end": 2.0})).await.unwrap();
    call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": false})).await.unwrap();
    assert_eq!(revision(&store, &id), r);
    // Progress bar on: one revision, undone in one step.
    call(&reg, &ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": true})).await.unwrap();
    assert_eq!(revision(&store, &id), r + 1);
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert!(extras(&store, &id).get("progress_bar").is_none());
}

// ---------------------------------------------------------------------------------------------
// caption.place + QC check (cached scan: no media needed)
// ---------------------------------------------------------------------------------------------

/// Pretends the source scan already ran for `start..end`: a subtitle band at source y 0.86-0.93.
fn seed_subtitles(store: &rustler_core::ProjectStore, src: &rustler_core::SourceId, start: f64, end: f64) {
    let entry = json!({"entries": [{"start": start, "end": end, "every": 0.5, "frames": 12, "bands": [
        {"kind": "subtitles", "y0": 0.86, "y1": 0.93, "x0": 0.2, "x1": 0.8, "presence": 0.9, "change": 0.6, "first_t": start, "last_t": end}
    ]}]});
    store.write_json(&Path::new("analysis").join(src.as_str()).join("text_bands.json"), &entry).unwrap();
}

#[tokio::test]
async fn caption_place_moves_captions_off_source_subtitles_only_with_apply() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    // Blur-fit: the 16:9 source sits in the middle of the 9:16 canvas, so its bottom subtitles
    // land at about y 0.61-0.64 — right under the default caption baseline 0.70.
    let id = clip_on(&reg, &ctx, &src, "p", "blur-fit").await;
    seed_subtitles(&store, &src.id, 1.0, 7.0);
    let r0 = revision(&store, &id);

    let p = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "faces": false})).await.unwrap();
    assert_eq!(p["current_position_y"], 0.7);
    assert_eq!(p["current_ok"], false, "{p}");
    let seg = &p["segments"][0];
    let t = &seg["source_text"][0];
    assert_eq!(t["kind"], "subtitles");
    let (y0, y1) = (t["y0"].as_f64().unwrap(), t["y1"].as_f64().unwrap());
    assert!((0.58..0.66).contains(&y0) && (0.6..0.68).contains(&y1), "{t}");
    assert!(seg["at_current"][0].as_str().unwrap().contains("burned-in subtitles"), "{seg}");
    let y = p["suggested_position_y"].as_f64().unwrap();
    let (lo, hi) = rustler_media::qc::caption_band(y);
    assert!(hi < y0 - 0.005 || lo > y1 + 0.005, "suggestion {y} (band {lo}-{hi}) clears {y0}-{y1}");
    assert!((0.10..=0.85).contains(&y));
    assert!(seg["at_suggested"].as_array().unwrap().is_empty(), "{seg}");
    assert_eq!(p["applied"], false);
    assert_eq!(revision(&store, &id), r0, "measuring changes nothing");
    assert!(p["next"][0].as_str().unwrap().contains("--apply true"), "{p}");

    // QC's check sees the same band (pure check on the measured spans).
    let clip = store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap();
    let spans = vec![rustler_media::qc::SourceTextSpan { segment_id: "s".into(), out_start: 0.0, out_end: 6.0, y0, y1 }];
    let ex = rustler_media::qc::QcExtras { source_text: Some(Ok(spans.clone())), ..Default::default() };
    let c = rustler_media::qc::source_text_check(&clip, &ex).unwrap();
    assert_eq!(c.name, "caption_over_source_text");
    assert_eq!(c.status, rustler_core::QcStatus::Warn, "{c:?}");
    assert!(c.fix_hint.as_deref().unwrap().contains("rustler caption place"), "{c:?}");

    // --apply: one new revision at the suggestion; QC is then clean; undo puts it back.
    let a = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "faces": false, "apply": true})).await.unwrap();
    assert_eq!(a["applied"], true);
    assert_eq!(revision(&store, &id), r0 + 1);
    let clip = store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap();
    assert_eq!(clip.captions.position_y, y);
    let c = rustler_media::qc::source_text_check(&clip, &ex).unwrap();
    assert_eq!(c.status, rustler_core::QcStatus::Pass, "{c:?}");
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert_eq!(store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap().captions.position_y, 0.7);

    // A stale base_revision is refused; bad sampling too.
    let e = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "apply": true, "faces": false, "base_revision": 1})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::RevisionConflict);
    let e = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "every": 0.01})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
}

#[tokio::test]
async fn caption_place_center_crop_ignores_text_outside_the_crop() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let id = clip_on(&reg, &ctx, &src, "c", "center-crop").await;
    // Static text in the far left corner of the 16:9 source: cropped away by a centred 9:16 crop.
    let entry = json!({"entries": [{"start": 1.0, "end": 7.0, "every": 0.5, "frames": 12, "bands": [
        {"kind": "static", "y0": 0.05, "y1": 0.1, "x0": 0.0, "x1": 0.15, "presence": 1.0, "change": 0.0, "first_t": 1.0, "last_t": 7.0}
    ]}]});
    store.write_json(&Path::new("analysis").join(src.id.as_str()).join("text_bands.json"), &entry).unwrap();
    let p = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "faces": false})).await.unwrap();
    assert!(p["segments"][0]["source_text"].as_array().unwrap().is_empty(), "{p}");
    assert_eq!(p["suggested_position_y"], 0.7, "nothing in the way: the position stays");
    assert_eq!(p["current_ok"], true);
}

// ---------------------------------------------------------------------------------------------
// Template from a learned style
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn template_from_reference_profile() {
    let (_proj, store, ctx) = new_project();
    let reg = registry();
    let e = call(&reg, &ctx, "template.create", json!({"name": "learned", "from_reference": "my-style"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    assert!(e.hint.as_deref().unwrap().contains("reference analyze my-style"), "{e:?}");
    let profile = json!({"name": "my-style", "aggregate": {
        "videos": 2,
        "cuts_per_minute": {"count": 2, "median": 9.5},
        "caption": {"present_in": 2, "y_center": {"count": 2, "median": 0.7}, "height": {"count": 2, "median": 0.06},
                    "words_per_line": {"count": 2, "median": 2.0}, "fill_colors": [{"hex": "#ffffff", "share": 0.8}, {"hex": "#ffe81a", "share": 0.15}]},
        "hook": {"present_in": 0},
        "zoom_per_minute": {"count": 2, "median": 3.0},
        "loudness_lufs": {"count": 2, "median": -14.2},
        "music_present_in": {"videos": 0, "of": 2}
    }});
    store.write_json(&Path::new("references").join("my-style").join("profile.json"), &profile).unwrap();
    let t = call(&reg, &ctx, "template.create", json!({"name": "learned", "from_reference": "my-style"})).await.unwrap();
    let doc = &t["template"];
    assert_eq!(doc["captions"]["position_y"], 0.73);
    assert_eq!(doc["captions"]["max_words_per_line"], 2);
    assert_eq!(doc["captions"]["preset"], "bold-karaoke");
    assert_eq!(doc["audio"]["target_lufs"], -14.2);
    assert!(doc.get("effects").is_none() && doc.get("hook").is_none(), "{doc}");
    assert_eq!(doc["based_on"]["reference_set"], "my-style");
    let notes = t["notes"].to_string();
    assert!(notes.contains("never adds automatic zooms") && notes.contains("voice only"), "{notes}");
    // It is a normal template: shown and listed.
    let s = call(&reg, &ctx, "template.show", json!({"name": "learned"})).await.unwrap();
    assert_eq!(s["template"]["captions"]["preset"], "bold-karaoke", "{s}");
    let e = call(&reg, &ctx, "template.create", json!({"name": "other", "from_reference": "my-style", "template": {}})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
}

// ---------------------------------------------------------------------------------------------
// Ordered by score
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn clip_list_and_export_batch_rank_by_agent_then_retention_score() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let a = clip_on(&reg, &ctx, &src, "a", "center-crop").await;
    let b = clip_on(&reg, &ctx, &src, "b", "center-crop").await;
    let c = clip_on(&reg, &ctx, &src, "c", "center-crop").await;
    let d = clip_on(&reg, &ctx, &src, "d", "center-crop").await;
    // b: agent 80; c: agent 80 + retention 70; a: retention only 90; d: nothing.
    call(&reg, &ctx, "clip.score_set", json!({"clip_id": b, "hook": 20, "flow": 20, "value": 20, "trend": 20})).await.unwrap();
    call(&reg, &ctx, "clip.score_set", json!({"clip_id": c, "hook": 20, "flow": 20, "value": 20, "trend": 20})).await.unwrap();
    let put_retention = |id: &str, score: u32| {
        let p = store.root.join("clips").join(id).join("extras.json");
        let mut x: Value = if p.is_file() { serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap() } else { json!({}) };
        x["meta"]["retention"] = json!({"score": score});
        std::fs::write(&p, serde_json::to_vec(&x).unwrap()).unwrap();
    };
    put_retention(&c, 70);
    put_retention(&a, 90);

    let l = call(&reg, &ctx, "clip.list", json!({"sort": "score"})).await.unwrap();
    let ids: Vec<&str> = l["clips"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![c.as_str(), b.as_str(), a.as_str(), d.as_str()], "{l}");
    assert_eq!(l["clips"][0]["rank"], 1);
    assert_eq!(l["clips"][0]["agent_score"], 80.0);
    assert_eq!(l["clips"][0]["retention_score"], 70.0);
    // Default: project order, no rank.
    let l = call(&reg, &ctx, "clip.list", json!({})).await.unwrap();
    assert_eq!(l["clips"][0]["id"], a.as_str());
    assert!(l["clips"][0].get("rank").is_none());

    // export.batch walks the same order (none is rendered, so all are skipped with their rank).
    let e = call(&reg, &ctx, "export.batch", json!({"all": true})).await.unwrap();
    assert_eq!(e["order"], "score");
    let sk: Vec<(&str, u64)> = e["skipped"].as_array().unwrap().iter().map(|s| (s["clip_id"].as_str().unwrap(), s["rank"].as_u64().unwrap())).collect();
    assert_eq!(sk, vec![(c.as_str(), 1), (b.as_str(), 2), (a.as_str(), 3), (d.as_str(), 4)], "{e}");
    assert!(e["warnings"].to_string().contains("no agent or retention score"), "{e}");
    let g = call(&reg, &ctx, "export.batch", json!({"clip_ids": [d, a], "order": "given"})).await.unwrap();
    assert_eq!(g["skipped"][0]["clip_id"], d.as_str());
    assert_eq!(g["order"], "given");
}

// ---------------------------------------------------------------------------------------------
// Per-segment caption positions
// ---------------------------------------------------------------------------------------------

/// `(id, source start, source end)` of each segment.
fn segments(store: &rustler_core::ProjectStore, id: &str) -> Vec<(String, f64, f64)> {
    let c = store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap();
    c.segments.iter().map(|s| (s.id.to_string(), s.start, s.end)).collect()
}

#[tokio::test]
async fn segment_caption_position_set_undo_follows_recuts_and_shows_in_the_timeline() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let id = clip_on(&reg, &ctx, &src, "seg", "center-crop").await;
    call(&reg, &ctx, "edit.split", json!({"clip_id": id, "out_t": 2.0})).await.unwrap();
    let s = segments(&store, &id);
    assert_eq!(s.len(), 2);
    let (a, b) = (s[0].0.clone(), s[1].0.clone());

    // Validation: only position_y with a segment, a real segment, a value in 0..1.
    let e = call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": b, "style": "hormozi-pop", "position_y": 0.3})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": b})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": "seg_nope", "position_y": 0.3})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    let e = call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": b, "position_y": 1.3})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    // Set: one new revision, the clip-wide position untouched.
    let r0 = revision(&store, &id);
    let out = call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": b, "position_y": 0.3})).await.unwrap();
    assert_eq!(revision(&store, &id), r0 + 1);
    assert_eq!(out["captions"]["position_y"], 0.7, "{out}");
    assert_eq!(extras(&store, &id)["caption_positions"], json!({b.as_str(): 0.3}));
    // The same value again is a no-op.
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": b, "position_y": 0.3})).await.unwrap();
    assert_eq!(revision(&store, &id), r0 + 1);
    // A clip-wide change keeps it and says so.
    let w = call(&reg, &ctx, "caption.set", json!({"clip_id": id, "position_y": 0.66})).await.unwrap();
    assert!(w["warnings"].to_string().contains("keep their own caption position"), "{w}");
    assert_eq!(extras(&store, &id)["caption_positions"][b.as_str()], 0.3);

    // The timeline document carries it in tracks.extra; a bad value there is refused.
    let t = call(&reg, &ctx, "timeline.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(t["document"]["tracks"]["extra"]["caption_positions"][b.as_str()], 0.3, "{}", t["document"]["tracks"]);
    let rev = t["revision"].as_u64().unwrap();
    let bad = json!([{"op": "replace", "path": format!("/tracks/extra/caption_positions/{b}"), "value": 2.0}]);
    let e = call(&reg, &ctx, "timeline.apply", json!({"clip_id": id, "base_revision": rev, "patch": bad})).await.unwrap_err();
    assert!(e.message.contains("caption_positions"), "{e:?}");

    // Undo the clip-wide change, then the segment position: exactly one step each.
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert_eq!(extras(&store, &id)["caption_positions"][b.as_str()], 0.3);
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert!(extras(&store, &id).get("caption_positions").is_none(), "{}", extras(&store, &id));
    let c = store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap();
    assert_eq!(c.captions.position_y, 0.7);

    // Clear: back to the clip-wide position, its own revision.
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": b, "position_y": 0.25})).await.unwrap();
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": a, "position_y": 0.4})).await.unwrap();
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": a, "clear_segment_position": true})).await.unwrap();
    assert_eq!(extras(&store, &id)["caption_positions"], json!({b.as_str(): 0.25}));

    // A re-cut segment keeps its picture's position: both halves of b inherit 0.25.
    call(&reg, &ctx, "edit.split", json!({"clip_id": id, "out_t": 4.0})).await.unwrap();
    let s = segments(&store, &id);
    assert_eq!(s.len(), 3);
    let pos = extras(&store, &id)["caption_positions"].clone();
    assert_eq!(pos.as_object().unwrap().len(), 2, "{pos}");
    assert_eq!(pos[s[1].0.as_str()], 0.25);
    assert_eq!(pos[s[2].0.as_str()], 0.25);
    assert!(pos.get(s[0].0.as_str()).is_none());
    // Deleting a segment with its own position drops it with a warning.
    let d = call(&reg, &ctx, "edit.delete_segment", json!({"clip_id": id, "segment_id": s[2].0})).await.unwrap();
    assert!(d["warnings"].to_string().contains("had its own caption position"), "{d}");
    assert_eq!(extras(&store, &id)["caption_positions"], json!({s[1].0.as_str(): 0.25}));
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert_eq!(extras(&store, &id)["caption_positions"][s[2].0.as_str()], 0.25, "undo brings it back");
}

#[tokio::test]
async fn caption_place_apply_sets_per_segment_positions_when_segments_differ() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let id = clip_on(&reg, &ctx, &src, "two", "center-crop").await;
    call(&reg, &ctx, "edit.split", json!({"clip_id": id, "out_t": 2.0})).await.unwrap();
    let s = segments(&store, &id);
    // Segment A: source text over the top 60 % (captions only fit low); segment B (longer): text
    // over the bottom half (captions only fit high). No single position clears both.
    let band = |y0: f64, y1: f64, a: f64, b: f64| json!({"kind": "subtitles", "y0": y0, "y1": y1, "x0": 0.0, "x1": 1.0, "presence": 0.9, "change": 0.6, "first_t": a, "last_t": b});
    let cache = json!({"entries": [
        {"start": s[0].1, "end": s[0].2, "every": 0.5, "frames": 4, "bands": [band(0.10, 0.60, s[0].1, s[0].2)]},
        {"start": s[1].1, "end": s[1].2, "every": 0.5, "frames": 8, "bands": [band(0.45, 0.95, s[1].1, s[1].2)]},
    ]});
    store.write_json(&Path::new("analysis").join(src.id.as_str()).join("text_bands.json"), &cache).unwrap();
    let r0 = revision(&store, &id);

    let p = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "faces": false})).await.unwrap();
    let y = p["suggested_position_y"].as_f64().unwrap();
    assert!(y <= 0.38, "the longer segment B wins the clip-wide spot: {y}");
    let plan = p["suggested_segment_positions"].as_object().unwrap().clone();
    assert_eq!(plan.keys().collect::<Vec<_>>(), [&s[0].0], "{p}");
    let own = plan[&s[0].0].as_f64().unwrap();
    assert!(own >= 0.71, "segment A gets a low spot of its own: {own}");
    assert!(p["remaining_conflicts"].as_array().unwrap().is_empty(), "{p}");
    assert_eq!(p["strategy"], "clip-wide");
    // The --per-segment alternative keeps 0.70 clip-wide: both segments need their own spot.
    assert_eq!(p["per_segment_positions"].as_object().unwrap().len(), 2, "{p}");
    assert!(p["next"].to_string().contains(&format!("--segment-id {}", s[0].0)), "{p}");
    assert_eq!(revision(&store, &id), r0, "measuring changes nothing");
    assert!(extras(&store, &id).get("caption_positions").is_none());

    // --apply: clip-wide + per-segment as ONE revision; measuring again finds nothing to change.
    let a = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "faces": false, "apply": true})).await.unwrap();
    assert_eq!(a["applied"], true);
    assert_eq!(revision(&store, &id), r0 + 1);
    assert_eq!(extras(&store, &id)["caption_positions"], json!({s[0].0.as_str(): own}));
    let c = store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap();
    assert!((c.captions.position_y - y).abs() < 1e-9);
    let again = call(&reg, &ctx, "caption.place", json!({"clip_id": id, "faces": false})).await.unwrap();
    assert_eq!(again["current_ok"], true, "{again}");
    assert_eq!(again["segments"][0]["current_y"], own);

    // QC's check judges each segment at its own position.
    let spans: Vec<rustler_media::qc::SourceTextSpan> = again["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            let t = &g["source_text"][0];
            rustler_media::qc::SourceTextSpan {
                segment_id: g["segment_id"].as_str().unwrap().into(),
                out_start: g["out_start"].as_f64().unwrap(),
                out_end: g["out_end"].as_f64().unwrap(),
                y0: t["y0"].as_f64().unwrap(),
                y1: t["y1"].as_f64().unwrap(),
            }
        })
        .collect();
    let positions = vec![rustler_media::qc::CaptionPosition { segment_id: s[0].0.clone(), out_start: 0.0, out_end: 2.0, position_y: own }];
    let ex = rustler_media::qc::QcExtras { source_text: Some(Ok(spans.clone())), caption_positions: positions, ..Default::default() };
    assert_eq!(rustler_media::qc::source_text_check(&c, &ex).unwrap().status, rustler_core::QcStatus::Pass);
    let ex = rustler_media::qc::QcExtras { source_text: Some(Ok(spans)), ..Default::default() };
    assert_eq!(rustler_media::qc::source_text_check(&c, &ex).unwrap().status, rustler_core::QcStatus::Warn, "one position for both is wrong");

    // One undo puts both back.
    call(&reg, &ctx, "history.undo", json!({"clip_id": id})).await.unwrap();
    assert!(extras(&store, &id).get("caption_positions").is_none(), "{}", extras(&store, &id));
    assert_eq!(store.load_clip(&id.parse::<ClipId>().unwrap()).unwrap().captions.position_y, 0.7);
}

#[tokio::test]
async fn render_moves_captions_of_a_segment_with_its_own_position() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let id = clip_on(&reg, &ctx, &src, "r", "center-crop").await;
    call(&reg, &ctx, "edit.split", json!({"clip_id": id, "out_t": 2.0})).await.unwrap();
    let s = segments(&store, &id);
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "segment_id": s[1].0, "position_y": 0.25})).await.unwrap();
    let r = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();
    let work = Path::new(r["path"].as_str().unwrap()).with_extension("");
    let ass = std::fs::read_to_string(work.join("captions.ass")).unwrap();
    let mv = |l: &str| -> u32 { l.trim_start_matches("Dialogue: ").split(',').nth(7).unwrap().parse().unwrap() };
    let start = |l: &str| -> String { l.split(',').nth(1).unwrap().to_string() };
    let d: Vec<&str> = ass.lines().filter(|l| l.starts_with("Dialogue:")).collect();
    assert!(!d.is_empty(), "{ass}");
    // Words before 2.0 s (segment 1) use the style's margin; later ones sit at 0.25.
    let own = (1920.0_f64 * 0.75).round() as u32;
    for l in &d {
        let want = if start(l).as_str() < "0:00:02.00" { 0 } else { own };
        assert_eq!(mv(l), want, "{l}");
    }
    assert!(d.iter().any(|l| mv(l) == own) && d.iter().any(|l| mv(l) == 0), "{ass}");
    // A preview frame of the moved segment renders too.
    let pf = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 3.0})).await.unwrap();
    assert!(Path::new(pf["image"].as_str().unwrap()).is_file());
}
