//! Punch-in planner end to end (template.apply, clip.create --template, clip.retention fixes),
//! template tags surviving effect.add, and layers re-timed when an edit shortens the clip.
mod common;

use common::*;
use rustler_core::{ProjectStore, Word};
use rustler_registry::{Ctx, Registry};
use serde_json::{Value, json};

struct Fx {
    _tmp: tempfile::TempDir,
    store: ProjectStore,
    ctx: Ctx,
    reg: Registry,
    src: rustler_core::Source,
    sfx: String,
}

/// Talking-head words every 0.3 s from 0.5 s to 59.5 s; every 5th is a stressed word.
fn talk() -> Vec<Word> {
    let text = ["people", "the", "judged", "my", "NEVER", "and", "accent.", "so", "really", "a"];
    (0..198).map(|i| word(i, text[i as usize % text.len()], 0.5 + i as f64 * 0.3, 0.5 + i as f64 * 0.3 + 0.26)).collect()
}

fn fx() -> Fx {
    let (tmp, store, ctx) = new_project();
    let media = tmp.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(60.0, true, true));
    write_transcript(&store, &src.id, talk());
    let sfx = tmp.path().join("whoosh.wav");
    std::fs::write(&sfx, b"RIFF").unwrap();
    // The starters add no punch-ins (the agent decides); an agent's own template with an explicit
    // punch-in pattern drives the planner here.
    let tdir = store.root.join("templates");
    std::fs::create_dir_all(&tdir).unwrap();
    let punchy = json!({
        "schema": "rustler.template@1", "name": "punchy", "version": 1, "created_by": "agent",
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "captions": {"enabled": true, "preset": "hormozi-pop", "position_y": 0.7, "max_words_per_line": 3},
        "hook": {"position_y": 0.12, "font_size_factor": 0.1, "box": true, "duration": 3.0, "max_words": 7},
        "progress_bar": {"enabled": true, "color": "#FFE000", "height": 8, "position": "top"},
        "audio": {"loudnorm": true, "target_lufs": -14.0, "true_peak": -2.0},
        "effects": [{"kind": "punch-in", "at": "emphasis", "duration": 2.0, "amount": 1.08, "amount_max": 1.2, "per_10s": 5.0}]
    });
    std::fs::write(tdir.join("punchy@1.json"), serde_json::to_vec_pretty(&punchy).unwrap()).unwrap();
    Fx { _tmp: tmp, store, ctx, reg: registry(), src, sfx: sfx.to_string_lossy().into_owned() }
}

impl Fx {
    async fn call(&self, name: &str, v: Value) -> Value {
        call(&self.reg, &self.ctx, name, v).await.unwrap_or_else(|e| panic!("{name}: {} ({:?})", e.message, e.hint))
    }
    async fn doc(&self, clip: &str) -> Value {
        self.call("timeline.show", json!({"clip_id": clip})).await["document"].clone()
    }
    async fn clip(&self, start: f64, end: f64) -> String {
        let c = self.call("clip.create", json!({"name": "c", "source_id": self.src.id, "start": start, "end": end})).await;
        c["id"].as_str().unwrap().to_string()
    }
}

fn effects(doc: &Value) -> Vec<Value> {
    doc["tracks"]["effects"].as_array().cloned().unwrap_or_default()
}

fn span(e: &Value) -> (f64, f64) {
    (e["start"].as_f64().unwrap(), e["end"].as_f64().unwrap())
}

/// Visual changes: every punch-in start and end plus `extra`.
fn changes(fx: &[Value], extra: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = extra.to_vec();
    for e in fx {
        let (s, en) = span(e);
        v.push(s);
        v.push(en);
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

fn max_in_10s(ch: &[f64]) -> usize {
    ch.iter().map(|&w| ch.iter().filter(|&&c| c >= w && c < w + 10.0).count()).max().unwrap_or(0)
}

#[tokio::test]
async fn template_punch_ins_are_paced_snapped_and_varied() {
    let f = fx();
    let id = f.clip(1.0, 52.0).await; // 51 s, like trial-2 clip 02
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id]})).await;
    let doc = f.doc(&id).await;
    let fx = effects(&doc);
    let ch = changes(&fx, &[]);
    let per10 = ch.len() as f64 * 10.0 / 51.0;
    assert!((3.5..=5.0).contains(&per10), "{per10:.2} changes / 10 s: {fx:?}");
    assert!(max_in_10s(&ch) <= 5, "a 10 s window has {} changes", max_in_10s(&ch));
    let amounts: Vec<f64> = fx.iter().map(|e| e["amount"].as_f64().unwrap()).collect();
    assert!(amounts.iter().all(|a| (1.08..=1.2).contains(a)), "{amounts:?}");
    assert!(amounts.windows(2).all(|w| (w[0] - w[1]).abs() >= 0.025), "{amounts:?}");
    // Each zoom starts 0.04 s before a word start (source word start - 1.0 clip offset).
    for e in &fx {
        let s = span(e).0 + 0.04 + 1.0 - 0.5;
        let k = (s / 0.3).round();
        assert!((s - k * 0.3).abs() < 0.011, "{e} does not start on a word");
        assert_eq!(e["by_template"], "punchy@1");
    }
}

#[tokio::test]
async fn no_punch_ins_on_graphics_or_near_cuts() {
    let f = fx();
    let id = f.clip(1.0, 31.0).await;
    // Split at 10 and 20; the middle part shows a graphic in blur-fit.
    f.call("edit.split", json!({"clip_id": id, "out_t": 10.0})).await;
    f.call("edit.split", json!({"clip_id": id, "out_t": 20.0})).await;
    let segs: Vec<String> = f.doc(&id).await["tracks"]["video"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().into()).collect();
    f.call("clip.set_layout", json!({"clip_id": id, "segment_id": segs[1], "preset": "blur-fit"})).await;
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id]})).await;
    let fx = effects(&f.doc(&id).await);
    assert!(!fx.is_empty());
    for e in &fx {
        let (s, en) = span(e);
        assert!(!(s < 20.0 && en > 10.0), "{e} is on the blur-fit graphic");
        for c in [10.0, 20.0] {
            assert!(s >= c + 0.5 || en <= c - 0.5, "{e} within 0.5 s of the cut at {c}");
        }
    }
}

#[tokio::test]
async fn template_tags_survive_effect_add_and_reapply_replaces() {
    let f = fx();
    let c = f.call("clip.create", json!({"name": "t", "source_id": f.src.id, "start": 1.0, "end": 41.0, "template": "punchy"})).await;
    let id = c["id"].as_str().unwrap().to_string();
    let first = effects(&f.doc(&id).await);
    assert!(!first.is_empty() && first.iter().all(|e| e["by_template"] == "punchy@1"), "{first:?}");
    // A hand-added effect (what clips auto's retention fix does) must not strip the tags.
    f.call("effect.add", json!({"clip_id": id, "kind": "flash", "start": 0.2, "end": 0.4})).await;
    let list = f.call("effect.list", json!({"clip_id": id})).await;
    let tagged = list["effects"].as_array().unwrap().iter().filter(|e| e["by_template"] == "punchy@1").count();
    assert_eq!(tagged, first.len(), "{list}");
    // Re-applying replaces the template's punch-ins instead of stacking a second set.
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id]})).await;
    let again = effects(&f.doc(&id).await);
    let punch = again.iter().filter(|e| e["kind"] == "punch-in").count();
    assert_eq!(punch, first.len(), "{again:?}");
    assert_eq!(again.iter().filter(|e| e["kind"] == "flash").count(), 1);
}

#[tokio::test]
async fn shortening_drops_and_clamps_layers_then_template_applies() {
    let f = fx();
    let id = f.clip(1.0, 41.0).await;
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id]})).await;
    f.call("effect.add", json!({"clip_id": id, "kind": "shake", "start": 18.0, "end": 22.0})).await;
    f.call("overlay.add_text", json!({"clip_id": id, "text": "LATE", "style": "plain", "start": 30.0, "end": 32.0})).await;
    f.call("audio.add_sfx", json!({"clip_id": id, "file": f.sfx, "at": 35.0})).await;
    let before = effects(&f.doc(&id).await).len();
    // Cut the clip to 0..20 s: split, delete the tail.
    f.call("edit.split", json!({"clip_id": id, "out_t": 20.0})).await;
    let segs: Vec<String> = f.doc(&id).await["tracks"]["video"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().into()).collect();
    let out = f.call("edit.delete_segment", json!({"clip_id": id, "segment_id": segs[1]})).await;
    let warns: Vec<String> = out["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    assert!(warns.iter().any(|w| w.starts_with("effect ") && w.contains("removed")), "{warns:?}");
    assert!(warns.iter().any(|w| w.starts_with("text ") && w.contains("removed")), "{warns:?}");
    assert!(warns.iter().any(|w| w.starts_with("sfx ") && w.contains("removed")), "{warns:?}");
    let doc = f.doc(&id).await;
    let fx = effects(&doc);
    assert!(fx.len() < before);
    assert!(fx.iter().all(|e| span(e).1 <= 20.0 + 1e-9), "{fx:?}");
    let shake = fx.iter().find(|e| e["kind"] == "shake").expect("the straddling shake is kept, clamped");
    assert_eq!(span(shake), (18.0, 20.0));
    assert!(warns.iter().any(|w| w.contains("shortened") || w.contains("removed")));
    assert!(doc["tracks"]["text"].as_array().is_none_or(|t| t.iter().all(|x| x["text"] != "LATE")));
    assert!(doc["tracks"]["audio"]["sfx"].as_array().is_none_or(|s| s.is_empty()));
    // The template applies again (trial 2: "/tracks/effects/9 ... is outside the clip").
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id], "dry_run": true})).await;
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id]})).await;
}

#[tokio::test]
async fn shortening_works_on_clips_with_v2_hook_styles() {
    let f = fx();
    let id = f.clip(1.0, 41.0).await;
    f.call("overlay.add_text", json!({"clip_id": id, "text": "They judged my brain", "style": "hook-big"})).await;
    f.call("effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 30.0, "end": 31.5})).await;
    f.call("effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 8.0, "end": 9.0})).await;
    let split = f.call("edit.split", json!({"clip_id": id, "out_t": 20.0})).await;
    let segs = split["segments"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let out = f.call("edit.delete_segment", json!({"clip_id": id, "segment_id": segs[1]})).await;
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with("effect fx_1 removed")), "{out}");
    let list = f.call("effect.list", json!({"clip_id": id})).await;
    let fx = list["effects"].as_array().unwrap();
    assert_eq!(fx.len(), 1, "{list}");
    assert_eq!(span(&fx[0]), (8.0, 9.0));
    // The hook (a style the document model does not read) is kept as stored.
    let extras: Value = serde_json::from_slice(&std::fs::read(f.store.root.join("clips").join(&id).join("extras.json")).unwrap()).unwrap();
    assert_eq!(extras["overlays"][0]["style"], "hook-big");
    // Retention still plans punch-ins on such a clip.
    let r = f.call("clip.retention", json!({"clip_id": id, "timeline_only": true, "per_second": false})).await;
    assert!(r["dead_zones"].as_array().unwrap().iter().any(|z| z["fixes"].as_array().unwrap().iter().any(|x| x["action"] == "punch_in")), "{r}");
}

#[tokio::test]
async fn edits_move_effects_with_their_content() {
    let f = fx();
    let id = f.clip(1.0, 31.0).await;
    f.call("effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 22.0, "end": 23.0, "amount": 1.1})).await;
    f.call("edit.split", json!({"clip_id": id, "out_t": 10.0})).await;
    f.call("edit.split", json!({"clip_id": id, "out_t": 15.0})).await;
    let segs: Vec<String> = f.doc(&id).await["tracks"]["video"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().into()).collect();
    f.call("edit.delete_segment", json!({"clip_id": id, "segment_id": segs[1]})).await;
    let fx = effects(&f.doc(&id).await);
    assert_eq!(fx.len(), 1);
    assert_eq!(span(&fx[0]), (17.0, 18.0), "the punch-in stays on its words (5 s earlier)");
}

#[tokio::test]
async fn retention_punch_fix_uses_the_planner_and_does_not_stack() {
    let f = fx();
    let id = f.clip(1.0, 41.0).await;
    // A hand punch-in at 5..6.5; nothing else: long static stretches.
    f.call("effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 5.0, "end": 6.5})).await;
    let r = f.call("clip.retention", json!({"clip_id": id, "timeline_only": true, "per_second": false})).await;
    let mut planned: Vec<(f64, f64, f64)> = Vec::new();
    for z in r["dead_zones"].as_array().unwrap() {
        for fix in z["fixes"].as_array().unwrap().iter().filter(|x| x["action"] == "punch_in") {
            for part in fix["command"].as_str().unwrap().split("&&") {
                let toks: Vec<&str> = part.split_whitespace().collect();
                let v = |k: &str| toks.iter().position(|t| *t == k).map(|i| toks[i + 1].parse::<f64>().unwrap()).unwrap();
                planned.push((v("--start"), v("--end"), v("--amount")));
            }
        }
    }
    assert!(!planned.is_empty(), "{r}");
    for (s, e, a) in &planned {
        assert!(!(s < &7.0 && e > &4.5), "stacks on the existing punch-in: {s}..{e}");
        assert!((1.08..=1.2).contains(a));
    }
    let mut ch: Vec<f64> = vec![5.0, 6.5];
    for (s, e, _) in &planned {
        ch.push(*s);
        ch.push(*e);
    }
    ch.sort_by(|a, b| a.total_cmp(b));
    assert!(max_in_10s(&ch) <= 5, "{ch:?}");
}

#[tokio::test]
async fn punch_ins_centre_on_the_face_when_faces_are_known() {
    let f = fx();
    let id = f.clip(1.0, 21.0).await;
    // center-crop shows source x 0.342..0.658; the face sits at source x 0.36 (left of the crop).
    let faces = json!({
        "width": 1920, "height": 1080, "fps": 1.0,
        "samples": (0..22).map(|t| json!({"t": t as f64, "faces": [{"x": 0.33, "y": 0.2, "w": 0.06, "h": 0.12, "score": 0.9}]})).collect::<Vec<_>>()
    });
    let dir = f.store.root.join("analysis").join(f.src.id.as_str());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("faces.json"), serde_json::to_vec(&faces).unwrap()).unwrap();
    f.call("template.apply", json!({"name": "punchy", "clip_ids": [id]})).await;
    let fx = effects(&f.doc(&id).await);
    assert!(!fx.is_empty());
    for e in &fx {
        let (x, y) = (e["focus_x"].as_f64().unwrap(), e["focus_y"].as_f64().unwrap());
        let half = 0.5 / e["amount"].as_f64().unwrap();
        assert!(x < 0.5 && (x - half).abs() < 0.01, "{e}: the face is at the left edge, the zoom is pulled left (inside the frame)");
        assert!(y <= 0.5 && y >= half - 0.001, "{e}");
    }
}
