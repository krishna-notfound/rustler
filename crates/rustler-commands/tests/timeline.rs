//! Timeline v2 document (Task 1.1) and timeline.show / schema / apply (Task 1.2).
mod common;

use common::*;
use rustler_commands::timeline_doc;
use rustler_core::{ErrorCode, ProjectStore};
use serde_json::{Value, json};

fn extras_json(store: &ProjectStore, clip: &str) -> Value {
    let p = store.root.join("clips").join(clip).join("extras.json");
    if !p.is_file() {
        return json!({});
    }
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn write_extras(store: &ProjectStore, clip: &str, v: &Value) {
    let p = store.root.join("clips").join(clip).join("extras.json");
    std::fs::write(p, serde_json::to_vec_pretty(v).unwrap()).unwrap();
}

/// Clip [1.0, 2.4] + [3.0, 5.0] + [5.05, 7.0] of a fake source with the sample transcript;
/// edit duration 5.35 s. Returns (clip id, segment ids).
async fn three_segments(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source) -> (String, Vec<String>) {
    let c = call(reg, ctx, "clip.create", json!({"name": "tl", "source_id": src.id, "start": 1.0, "end": 2.4})).await.unwrap();
    let id = c["id"].as_str().unwrap().to_string();
    call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 3.0, "end": 5.0})).await.unwrap();
    let c = call(reg, ctx, "clip.add_segment", json!({"clip_id": id, "source_id": src.id, "start": 5.05, "end": 7.0})).await.unwrap();
    let segs = c["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    (id, segs)
}

struct Fixture {
    _proj: tempfile::TempDir,
    store: ProjectStore,
    ctx: rustler_registry::Ctx,
    reg: rustler_registry::Registry,
    src: rustler_core::Source,
    id: String,
    segs: Vec<String>,
    files: std::path::PathBuf,
}

async fn fixture() -> Fixture {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let (id, segs) = three_segments(&reg, &ctx, &src).await;
    let files = proj.path().to_path_buf();
    for f in ["logo.png", "boom.wav", "music.mp3", "broll.mp4"] {
        std::fs::write(files.join(f), b"x").unwrap();
    }
    Fixture { _proj: proj, store, ctx, reg, src, id, segs, files }
}

/// Every v1 layer through its own command, plus keys written by other modules.
async fn every_feature(f: &Fixture) {
    let (reg, ctx, id) = (&f.reg, &f.ctx, &f.id);
    let file = |n: &str| f.files.join(n).to_string_lossy().to_string();
    call(reg, ctx, "overlay.add_text", json!({"clip_id": id, "text": "He quit his job", "start": 0.0, "end": 2.0})).await.unwrap();
    call(reg, ctx, "audio.add_sfx", json!({"clip_id": id, "file": file("boom.wav"), "at": 0.4})).await.unwrap();
    call(reg, ctx, "audio.add_sfx", json!({"clip_id": id, "file": file("boom.wav"), "at_word": {"source_id": f.src.id, "word_index": 4}})).await.unwrap();
    call(reg, ctx, "audio.denoise", json!({"clip_id": id, "strength": "light"})).await.unwrap();
    call(reg, ctx, "audio.volume", json!({"clip_id": id, "segment_id": f.segs[0], "db": -3.0})).await.unwrap();
    call(reg, ctx, "audio.add_music", json!({"clip_id": id, "path": file("music.mp3")})).await.unwrap();
    call(reg, ctx, "transition.set", json!({"clip_id": id, "after_segment_id": f.segs[0], "kind": "dissolve"})).await.unwrap();
    call(reg, ctx, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 2.0, "end": 3.0})).await.unwrap();
    call(reg, ctx, "overlay.progress_bar", json!({"clip_id": id, "enabled": true})).await.unwrap();
    call(reg, ctx, "image.add", json!({"clip_id": id, "file": file("logo.png"), "rect": {"x": 0.7, "y": 0.03, "w": 0.25, "h": 0.08}})).await.unwrap();
    call(reg, ctx, "mask.add", json!({"clip_id": id, "kind": "blur", "rect": {"x": 0.0, "y": 0.0, "w": 0.3, "h": 0.1}, "start": 0.0, "end": 1.0})).await.unwrap();
    call(reg, ctx, "cutaway.add", json!({"clip_id": id, "file": file("broll.mp4"), "mode": "pip", "rect": {"x": 0.5, "y": 0.3, "w": 0.4, "h": 0.2}, "start": 1.0, "end": 2.0}))
        .await
        .unwrap();
    call(reg, ctx, "caption.word", json!({"clip_id": id, "source_id": f.src.id, "word_i": 4, "color": "#FF0000"})).await.unwrap();
    call(reg, ctx, "clip.score_set", json!({"clip_id": id, "hook": 22, "flow": 18, "value": 20, "trend": 10, "notes": "x"})).await.unwrap();
    let mut x = extras_json(&f.store, id);
    x["crop_paths"] = json!({ f.segs[1].clone(): [{"t": 0.0, "x": 0.3}, {"t": 2.0, "x": 0.35}], "seg_gone": [{"t": 0.0, "x": 0.1}] });
    x["speaker_colors"] = json!({"enabled": true, "colors": {"S1": "#FFE000"}});
    x["post"] = json!({"platforms": {"shorts": {"title": "t"}}});
    x["variant_of"] = json!("clp_root");
    x["future_key"] = json!({"keep": [1, 2]});
    write_extras(&f.store, id, &x);
}

#[tokio::test]
async fn assemble_v1_clip_has_empty_new_tracks() {
    let f = fixture().await;
    let (t, warnings) = timeline_doc::assemble(&f.store, &f.id.parse().unwrap()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(t.schema, "rustler.timeline@2");
    assert_eq!(t.tracks.video.len(), 3);
    assert_eq!((t.tracks.video[0].src_in, t.tracks.video[0].src_out), (1.0, 2.4));
    let tr = &t.tracks;
    assert!(tr.freezes.is_empty() && tr.cutaways.is_empty() && tr.images.is_empty() && tr.masks.is_empty() && tr.text.is_empty());
    assert!(tr.effects.is_empty() && tr.transitions.is_empty() && tr.progress_bar.is_none() && tr.extra.is_empty());
    assert!(tr.audio.sfx.is_empty() && tr.audio.music.is_none() && tr.captions.word_overrides.is_empty());
    assert!(t.template_ref.is_none() && t.meta.score.is_none());
    assert!((t.edit_duration() - 5.35).abs() < 1e-9);
    // And show works on it (no extras.json at all).
    let s = call(&f.reg, &f.ctx, "timeline.show", json!({"clip_id": f.id})).await.unwrap();
    assert_eq!(s["revision"], t.revision);
    assert!((s["resolved"]["rendered_duration"].as_f64().unwrap() - 5.35).abs() < 1e-9);
}

#[tokio::test]
async fn roundtrip_assemble_disassemble_is_identity() {
    let f = fixture().await;
    every_feature(&f).await;
    let cid = f.id.parse().unwrap();
    let before_clip = std::fs::read_to_string(f.store.root.join("clips").join(&f.id).join("clip.json")).unwrap();
    let before = extras_json(&f.store, &f.id);
    let (t, _) = timeline_doc::assemble(&f.store, &cid).unwrap();
    // Every key landed in a typed place (only the unknown key in tracks.extra).
    assert_eq!(t.tracks.extra.keys().collect::<Vec<_>>(), vec!["future_key"]);
    assert_eq!(t.tracks.text.len(), 1);
    assert_eq!(t.tracks.audio.sfx.len(), 2);
    assert!(t.tracks.audio.music.is_some() && t.tracks.progress_bar.is_some() && t.tracks.captions.speaker_colors.is_some());
    assert_eq!(t.tracks.video[1].crop_path.as_ref().unwrap().len(), 2);
    assert!(t.meta.score.is_some() && t.meta.post.is_some() && t.meta.variant_of.is_some());

    let rev = timeline_doc::disassemble(&f.store, &t, t.revision).unwrap();
    assert_eq!(rev, t.revision + 1);
    let after = extras_json(&f.store, &f.id);
    assert_eq!(after, before, "extras.json is unchanged by a round trip");
    let mut c0: Value = serde_json::from_str(&before_clip).unwrap();
    let mut c1: Value = serde_json::from_slice(&std::fs::read(f.store.root.join("clips").join(&f.id).join("clip.json")).unwrap()).unwrap();
    c0["revision"] = json!(0);
    c1["revision"] = json!(0);
    assert_eq!(c0, c1, "clip.json is unchanged apart from the revision");
    let (t2, _) = timeline_doc::assemble(&f.store, &cid).unwrap();
    let mut t1 = t.clone();
    t1.revision = t2.revision;
    assert_eq!(t1, t2, "assemble(disassemble(doc)) == doc");
    // The crop path of a segment that no longer exists stays for undo.
    assert!(after["crop_paths"]["seg_gone"].is_array());
}

#[tokio::test]
async fn owner_types_roundtrip_through_timeline_types() {
    // Guards the 1:1 mapping: what the command modules store reads back unchanged.
    let f = fixture().await;
    every_feature(&f).await;
    let before = extras_json(&f.store, &f.id);
    let (t, _) = timeline_doc::assemble(&f.store, &f.id.parse().unwrap()).unwrap();
    let clip = f.store.load_clip(&f.id.parse().unwrap()).unwrap();
    let (_, map) = timeline_doc::to_storage(&t, Some(&clip), before.as_object().unwrap()).unwrap();
    for (k, v) in before.as_object().unwrap() {
        assert_eq!(map.get(k), Some(v), "key {k}");
    }
}

#[test]
fn schema_generates() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (_p, _s, ctx) = new_project();
        let s = call(&registry(), &ctx, "timeline.schema", json!({})).await.unwrap();
        assert_eq!(s["schema_id"], "rustler.timeline@2");
        let text = s["json_schema"].to_string();
        assert!(text.contains("\"tracks\"") && text.contains("\"word_overrides\"") && text.contains("\"freezes\""));
        assert!(s["notes"].as_array().unwrap().len() >= 4);
    });
}

#[tokio::test]
async fn apply_patch_moves_overlay() {
    let f = fixture().await;
    every_feature(&f).await;
    let s = call(&f.reg, &f.ctx, "timeline.show", json!({"clip_id": f.id})).await.unwrap();
    let rev = s["revision"].as_u64().unwrap();
    assert_eq!(s["document"]["tracks"]["text"][0]["start"], 0.0);
    // Resolved times: the anchored sfx sits on word 4 ("Americans" at 3.45 s source = 1.4 + 0.45).
    let sfx = s["resolved"]["items"].as_array().unwrap().iter().find(|i| i["track"] == "sfx" && i["anchored"] == true).unwrap();
    assert!((sfx["start"].as_f64().unwrap() - 1.85).abs() < 1e-6, "{sfx}");

    let patch = json!([
        {"op": "replace", "path": "/tracks/text/0/start", "value": 1.0},
        {"op": "replace", "path": "/tracks/text/0/end", "value": 3.0},
        {"op": "add", "path": "/tracks/effects/-", "value": {"id": "fx_9", "kind": "shake", "start": 4.0, "end": 4.5}}
    ]);
    // Dry run validates and changes nothing.
    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    let d = call(&f.reg, &dry, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": patch})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["revision"], rev);
    assert_eq!(extras_json(&f.store, &f.id)["overlays"][0]["start"], 0.0);

    let a = call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": patch})).await.unwrap();
    assert_eq!(a["revision"], rev + 1);
    let changes: Vec<&str> = a["changes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert!(changes.contains(&"text ovl_1: changed end, start"), "{changes:?}");
    assert!(changes.contains(&"effects fx_9: added"), "{changes:?}");
    let x = extras_json(&f.store, &f.id);
    assert_eq!((x["overlays"][0]["start"].as_f64(), x["overlays"][0]["end"].as_f64()), (Some(1.0), Some(3.0)));
    assert_eq!(x["effects"][1]["kind"], "shake");
    assert_eq!(x["future_key"], json!({"keep": [1, 2]}), "unknown keys survive");
    // Rendered times follow the dissolve after segment 0 (0.3 s overlap): 3.0 edit -> 2.7 rendered.
    let ov = a["resolved"]["items"].as_array().unwrap().iter().find(|i| i["id"] == "ovl_1").unwrap();
    assert!((ov["rendered_end"].as_f64().unwrap() - 2.7).abs() < 1e-6, "{ov}");
}

#[tokio::test]
async fn apply_stale_revision_conflicts() {
    let f = fixture().await;
    every_feature(&f).await;
    let rev = call(&f.reg, &f.ctx, "timeline.show", json!({"clip_id": f.id})).await.unwrap()["revision"].as_u64().unwrap();
    // Two writers (GUI + agent) based on the same revision.
    let p1 = json!([{"op": "replace", "path": "/tracks/text/0/text", "value": "First writer"}]);
    let p2 = json!([{"op": "replace", "path": "/tracks/text/0/text", "value": "Second writer"}]);
    call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": p1})).await.unwrap();
    let e = call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": p2})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::RevisionConflict, "{e:?}");
    assert_eq!(extras_json(&f.store, &f.id)["overlays"][0]["text"], "First writer", "nothing lost, nothing overwritten");
    // The disassemble path itself checks too.
    let (t, _) = timeline_doc::assemble(&f.store, &f.id.parse().unwrap()).unwrap();
    let e = timeline_doc::disassemble(&f.store, &t, rev).unwrap_err();
    assert_eq!(e.code, ErrorCode::RevisionConflict);
}

#[tokio::test]
async fn apply_invalid_rect_validation_lists_path() {
    let f = fixture().await;
    every_feature(&f).await;
    let rev = call(&f.reg, &f.ctx, "timeline.show", json!({"clip_id": f.id})).await.unwrap()["revision"].as_u64().unwrap();
    let patch = json!([
        {"op": "replace", "path": "/tracks/images/0/rect/x", "value": 0.9},
        {"op": "replace", "path": "/tracks/masks/0/rect/w", "value": 0.01},
        {"op": "replace", "path": "/tracks/text/0/end", "value": 99.0}
    ]);
    let e = call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": patch})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
    assert_eq!(e.stage, "timeline.apply");
    for p in ["/tracks/images/0/rect", "/tracks/masks/0/rect", "/tracks/text/0"] {
        assert!(e.message.contains(p), "error lists {p}: {}", e.message);
    }
    assert_eq!(extras_json(&f.store, &f.id)["images"][0]["rect"]["x"], 0.7, "nothing written");

    // Missing file, unknown source, unresolvable new anchor, bad JSON-Patch path, typo'd type.
    let bad_file = json!([{"op": "replace", "path": "/tracks/images/0/file", "value": f.files.join("nope.png")}]);
    let bad_src = json!([{"op": "replace", "path": "/tracks/video/0/source_id", "value": "src_nope"}]);
    let bad_anchor = json!([{"op": "add", "path": "/tracks/masks/-", "value": {"id": "msk_9", "kind": "box", "rect": {"x": 0.1, "y": 0.1, "w": 0.2, "h": 0.2}, "start_word": {"source_id": f.src.id, "word_i": 999}}}]);
    let bad_ptr = json!([{"op": "replace", "path": "/tracks/text/7/start", "value": 1.0}]);
    let bad_type = json!([{"op": "replace", "path": "/tracks/effects/0/kind", "value": "spin"}]);
    for (patch, needle) in [
        (bad_file, "/tracks/images/0/file"),
        (bad_src, "/tracks/video/0/source_id"),
        (bad_anchor, "/tracks/masks/1"),
        (bad_ptr, "/tracks/text/7/start"),
        (bad_type, "/tracks/effects/0"),
    ] {
        let e = call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": patch})).await.unwrap_err();
        assert_eq!(code(&e), ErrorCode::Validation, "{e:?}");
        assert!(e.message.contains(needle), "{needle}: {}", e.message);
    }
}

#[tokio::test]
async fn apply_document_replace() {
    let f = fixture().await;
    every_feature(&f).await;
    let s = call(&f.reg, &f.ctx, "timeline.show", json!({"clip_id": f.id})).await.unwrap();
    let rev = s["revision"].as_u64().unwrap();
    let mut doc = s["document"].clone();
    // Reorder the segments, drop the mask, retitle, half-speed the middle segment, add a freeze.
    let v = doc["tracks"]["video"].as_array_mut().unwrap();
    v.swap(0, 2);
    doc["tracks"]["video"][1]["speed"] = json!(0.5);
    doc["tracks"]["masks"] = json!([]);
    doc["name"] = json!("reordered");
    doc["tracks"]["freezes"] = json!([{"id": "frz_1", "out_t": 1.0, "duration": 0.5}]);
    let a = call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "document": doc})).await.unwrap();
    let changes = a["changes"].to_string();
    assert!(changes.contains("video: order changed") && changes.contains("masks msk_1: removed") && changes.contains("name:"), "{changes}");
    let clip = f.store.load_clip(&f.id.parse().unwrap()).unwrap();
    assert_eq!(clip.name, "reordered");
    assert_eq!(clip.segments[0].id.as_str(), f.segs[2]);
    let x = extras_json(&f.store, &f.id);
    assert!(x.get("masks").is_none());
    assert_eq!(x["segment_speed"][&f.segs[1]], 0.5);
    assert_eq!(x["freezes"][0]["duration"], 0.5);
    // Rendered duration: 5.35 + 2.0 (half speed on the 2 s segment) + 0.5 freeze; the dissolve
    // after the old first segment is ignored now that it is last (a warning says so).
    let rd = a["resolved"]["rendered_duration"].as_f64().unwrap();
    assert!((rd - 7.85).abs() < 1e-6, "{rd}");
    assert!(a["warnings"].to_string().contains("after the last segment"), "{}", a["warnings"]);
    // A document for another clip is refused.
    let mut other = a["document"].clone();
    other["id"] = json!("clp_other");
    let e = call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": a["revision"], "document": other})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Validation);
}

#[tokio::test]
async fn undo_restores_layers() {
    let f = fixture().await;
    every_feature(&f).await;
    let before = extras_json(&f.store, &f.id);
    let rev = call(&f.reg, &f.ctx, "timeline.show", json!({"clip_id": f.id})).await.unwrap()["revision"].as_u64().unwrap();
    let patch = json!([
        {"op": "remove", "path": "/tracks/images/0"},
        {"op": "replace", "path": "/tracks/text/0/text", "value": "Changed"},
        {"op": "remove", "path": "/tracks/audio/music"}
    ]);
    call(&f.reg, &f.ctx, "timeline.apply", json!({"clip_id": f.id, "base_revision": rev, "patch": patch})).await.unwrap();
    let mid = extras_json(&f.store, &f.id);
    assert!(mid.get("images").is_none() && mid.get("music").is_none());
    let u = call(&f.reg, &f.ctx, "history.undo", json!({"clip_id": f.id})).await.unwrap();
    assert!(u.get("warnings").is_none(), "{u}");
    assert_eq!(extras_json(&f.store, &f.id), before, "every layer is back");
    // Redo through restore brings the change back.
    call(&f.reg, &f.ctx, "history.restore", json!({"clip_id": f.id, "revision": rev + 1})).await.unwrap();
    assert_eq!(extras_json(&f.store, &f.id), mid);
    // A v1 edit (edit.split) also snapshots the layers.
    let r = call(&f.reg, &f.ctx, "edit.split", json!({"clip_id": f.id, "out_t": 2.4})).await.unwrap();
    assert_eq!(r["segments"].as_array().unwrap().len(), 4);
    call(&f.reg, &f.ctx, "overlay.edit", json!({"clip_id": f.id, "overlay_id": "ovl_1", "text": "After split"})).await.unwrap();
    call(&f.reg, &f.ctx, "history.undo", json!({"clip_id": f.id})).await.unwrap();
    assert_eq!(extras_json(&f.store, &f.id)["overlays"][0]["text"], "Changed", "undo of the split restores its layers too");
}
