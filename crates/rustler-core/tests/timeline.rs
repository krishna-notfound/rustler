//! Timeline v2 contract (`rustler.timeline@2`): schema, serde round trips, unknown fields kept.

use rustler_core::timeline::*;
use rustler_core::{TIMELINE_SCHEMA, Timeline};
use serde_json::{Value, json};

fn full_doc() -> Value {
    json!({
        "schema": TIMELINE_SCHEMA,
        "id": "clp_x",
        "name": "full",
        "revision": 4,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "template_ref": {"id": "podcast-viral", "version": 2},
        "tracks": {
            "video": [{
                "id": "seg_a", "source_id": "src_a", "src_in": 1.0, "src_out": 4.5,
                "layout": {"preset": "center-crop", "regions": [{"id": "main", "src": {"x": 0.3, "y": 0.0, "w": 0.32, "h": 1.0},
                    "dst": {"x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0}, "z": 0, "fit": "cover"}], "background": "black"},
                "crop_path": [{"t": 0.0, "x": 0.3}, {"t": 3.5, "x": 0.35}],
                "speed": 1.25
            }],
            "freezes": [{"id": "frz_1", "out_t": 1.0, "duration": 0.5}],
            "cutaways": [{"id": "cut_1", "out_start": 0.5, "out_end": 1.5, "media": {"file": "/b.mp4", "in": 2.0}, "mode": "pip",
                "rect": {"x": 0.5, "y": 0.3, "w": 0.4, "h": 0.2}, "fit": "cover", "mute": true}],
            "images": [{"id": "img_1", "start_word": {"source_id": "src_a", "word_i": 3}, "file": "/logo.png",
                "rect": {"x": 0.7, "y": 0.02, "w": 0.25, "h": 0.08}, "scale": 1.0, "opacity": 0.9, "rotation": 0.0, "fit": "contain",
                "anim": {"in": "fade", "out": "pop", "duration": 0.3}, "z": 2, "above_captions": false}],
            "masks": [{"id": "msk_1", "out_start": 0.0, "out_end": 2.0, "rect": {"x": 0.0, "y": 0.0, "w": 0.3, "h": 0.1}, "kind": "blur", "strength": 20.0}],
            "text": [{"id": "ovl_1", "text": "He quit", "start": 0.0, "end": 2.5, "style": "hook-top", "position_y": 0.12, "font_size_factor": 0.085, "box": true}],
            "captions": {"enabled": true, "preset": "bold-karaoke", "script": "latin", "position_y": 0.7, "max_words_per_line": 3,
                "word_overrides": [{"word_anchor": {"source_id": "src_a", "word_i": 4}, "color": "#FF0000", "scale": 1.3}],
                "speaker_colors": {"enabled": true, "colors": {"S1": "#FFE000"}}},
            "audio": {"loudnorm": true, "target_lufs": -14.0, "true_peak": -2.0, "lra": 11.0, "denoise": "light", "volume_db": 2.0,
                "segment_volumes": {"seg_a": -3.0},
                "music": {"path": "/m.mp3", "volume_db": -18.0, "duck": true, "fade_in": 0.5, "fade_out": 1.0},
                "sfx": [{"id": "sfx_1", "file": "/w.wav", "at_word": {"source_id": "src_a", "word_index": 2}, "volume_db": -6.0}]},
            "effects": [{"id": "fx_1", "kind": "punch-in", "start": 1.0, "end": 1.6, "amount": 1.2}],
            "transitions": [{"after_segment_id": "seg_a", "kind": "dissolve", "duration": 0.3}],
            "progress_bar": {"color": "#FFE000", "height": 8, "position": "top"},
            "extra": {"future_key": [1, 2]}
        },
        "meta": {"score": {"total": 80}, "post": {"platforms": {}}, "variant_of": "clp_root", "note": "kept"}
    })
}

#[test]
fn schema_generates() {
    let s = serde_json::to_value(schemars::schema_for!(Timeline)).unwrap();
    let text = s.to_string();
    for field in ["tracks", "video", "src_in", "crop_path", "speed", "freezes", "cutaways", "images", "masks", "text", "word_overrides", "sfx", "effects", "transitions", "progress_bar", "template_ref"] {
        assert!(text.contains(&format!("\"{field}\"")), "schema lacks {field}");
    }
}

#[test]
fn full_document_round_trips() {
    let v = full_doc();
    let t: Timeline = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(t.tracks.video[0].speed, Some(1.25));
    assert_eq!(t.tracks.images[0].timing.start_word.as_ref().unwrap().word_i, 3);
    assert_eq!(t.tracks.audio.sfx[0].at_word.as_ref().unwrap().word_index, 2);
    assert_eq!(t.tracks.audio.denoise, DenoiseLevel::Light);
    assert_eq!(t.meta.other["note"], "kept");
    assert_eq!(t.edit_duration(), 3.5);
    let back = serde_json::to_value(&t).unwrap();
    assert_eq!(back, v, "serialize(deserialize(doc)) == doc");
}

#[test]
fn unknown_item_fields_are_kept() {
    let mut v = full_doc();
    v["tracks"]["text"][0]["glow"] = json!({"radius": 3});
    v["tracks"]["images"][0]["blend"] = json!("screen");
    let t: Timeline = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(t.tracks.text[0].extra["glow"]["radius"], 3);
    let back = serde_json::to_value(&t).unwrap();
    assert_eq!(back["tracks"]["text"][0]["glow"], json!({"radius": 3}));
    assert_eq!(back["tracks"]["images"][0]["blend"], "screen");
}

#[test]
fn minimal_document_gets_defaults() {
    let v = json!({
        "schema": TIMELINE_SCHEMA, "id": "clp_y", "name": "min", "revision": 1,
        "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
        "tracks": {"video": []}
    });
    let t: Timeline = serde_json::from_value(v).unwrap();
    assert!(t.tracks.text.is_empty() && t.tracks.freezes.is_empty() && t.tracks.progress_bar.is_none());
    assert!(t.tracks.captions.settings.enabled, "caption defaults");
    assert_eq!(t.tracks.audio.master.target_lufs, -14.0);
    assert!(t.template_ref.is_none());
    // Word anchors accept `word_index` too.
    let o: WordOverride = serde_json::from_value(json!({"word_anchor": {"source_id": "src_a", "word_index": 9}})).unwrap();
    assert_eq!(o.word_anchor.word_i, 9);
}
