//! `template.create --from-reference <set>` (f40): a template document from a learned style
//! (`references/<set>/profile.json`, written by `reference.analyze`).
//!
//! What maps onto template fields: caption position / words per line / a built-in caption style
//! close to the measured colours, hook position and on-screen time, loudness target. What the
//! profile cannot drive is written to `notes` (pacing, music, layouts, fonts). Zooms are recorded
//! as a note only: Rustler never adds automatic zooms (user rule); the agent places punch-ins.

use crate::template_cmds::Template;
use rustler_core::{ErrorCode, RustlerError};
use serde_json::{Map, Value, json};

type Result<T> = rustler_core::Result<T>;

fn med(v: &Value, path: &str) -> Option<f64> {
    v.pointer(path).and_then(|s| s.get("median")).and_then(Value::as_f64).filter(|x| x.is_finite())
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

fn hex_rgb(h: &str) -> Option<[f64; 3]> {
    let h = h.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(f64::from);
    Some([p(0)?, p(2)?, p(4)?])
}

/// Built-in caption styles with a coloured active word, by that colour.
const ACCENTS: &[(&str, [f64; 3])] = &[("bold-karaoke", [255.0, 224.0, 0.0]), ("hormozi-pop", [43.0, 255.0, 94.0]), ("boxed-highlight", [123.0, 44.0, 255.0])];

/// The built-in style closest to the measured caption colours, with the reason.
pub fn preset_for_colors(fills: &[Value]) -> (String, String) {
    let accent = fills.iter().find_map(|f| {
        let rgb = hex_rgb(f.get("hex")?.as_str()?)?;
        let share = f.get("share").and_then(Value::as_f64).unwrap_or(0.0);
        let (mx, mn) = (rgb.iter().copied().fold(0.0, f64::max), rgb.iter().copied().fold(255.0, f64::min));
        (share >= 0.05 && mx - mn >= 80.0 && mx >= 120.0).then(|| (f["hex"].as_str().unwrap_or_default().to_string(), rgb))
    });
    match accent {
        None => ("clean-white".into(), "captions are plain white (no coloured accent measured)".into()),
        Some((hex, rgb)) => {
            let d = |c: &[f64; 3]| (0..3).map(|i| (c[i] - rgb[i]).powi(2)).sum::<f64>();
            let (name, _) = ACCENTS.iter().min_by(|a, b| d(&a.1).total_cmp(&d(&b.1))).expect("accents");
            ((*name).into(), format!("accent colour {hex} measured; {name} is the closest built-in style"))
        }
    }
}

/// The template document plus notes on what was derived and what could not be.
pub fn from_profile(name: &str, set: &str, profile: &Value) -> Result<(Template, Vec<String>)> {
    let agg = profile.get("aggregate").filter(|a| a.is_object()).ok_or_else(|| {
        RustlerError::new(ErrorCode::Validation, "template.create", format!("reference set '{set}' has no aggregate profile"))
            .hint(format!("run `rustler reference analyze {set}` first"))
    })?;
    let n = agg.get("videos").and_then(Value::as_u64).unwrap_or(0);
    let half = n.div_ceil(2).max(1);
    let mut notes: Vec<String> = vec![format!("learned from reference set '{set}' ({n} video(s)); measurements are medians across the videos")];
    let mut t = Map::new();
    t.insert("name".into(), json!(name));
    t.insert("created_by".into(), json!("agent"));
    t.insert("based_on".into(), json!({"reference_set": set}));
    t.insert("description".into(), json!(format!("Look learned from the reference set '{set}'")));

    // Captions
    let cap = agg.get("caption").cloned().unwrap_or(Value::Null);
    let cap_in = cap.get("present_in").and_then(Value::as_u64).unwrap_or(0);
    if cap_in >= half && n > 0 {
        let mut c = Map::new();
        c.insert("enabled".into(), json!(true));
        if let Some(yc) = med(agg, "/caption/y_center") {
            let h = med(agg, "/caption/height").unwrap_or(0.04);
            // Our position_y is the caption baseline: the bottom of the measured band.
            let y = r2((yc + h / 2.0).clamp(rustler_media::qc::SAFE_ZONE_MIN_Y, rustler_media::qc::SAFE_ZONE_MAX_Y));
            c.insert("position_y".into(), json!(y));
            notes.push(format!("captions: band centred at y {yc:.2} ({h:.3} tall) -> position_y {y:.2} (baseline, kept inside 0.10-0.85)"));
        }
        if let Some(w) = med(agg, "/caption/words_per_line") {
            let k = (w.round() as i64).clamp(1, 12);
            c.insert("max_words_per_line".into(), json!(k));
            notes.push(format!("captions: about {w:.1} words per line -> max_words_per_line {k}"));
        }
        let fills: Vec<Value> = cap.get("fill_colors").and_then(Value::as_array).cloned().unwrap_or_default();
        let (preset, why) = preset_for_colors(&fills);
        c.insert("preset".into(), json!(preset));
        let all: Vec<String> = fills.iter().filter_map(|f| f.get("hex").and_then(Value::as_str).map(str::to_string)).collect();
        notes.push(format!(
            "captions: style {preset} ({why}; measured fill colours {}). Fonts, outline width and animation are not measured: compare the style (`rustler caption list-styles`) with the contact sheets, or make a closer style with `rustler caption create-style`",
            if all.is_empty() { "none".to_string() } else { all.join(", ") }
        ));
        t.insert("captions".into(), Value::Object(c));
    } else {
        notes.push(format!("captions: a burned-in caption band was found in {cap_in}/{n} videos; the template leaves caption settings alone"));
    }

    // Hook
    let hook_in = agg.pointer("/hook/present_in").and_then(Value::as_u64).unwrap_or(0);
    if hook_in >= half && n > 0 {
        let mut h = Map::new();
        if let (Some(a), Some(b)) = (med(agg, "/hook/y_top"), med(agg, "/hook/y_bottom")) {
            h.insert("position_y".into(), json!(r2(((a + b) / 2.0).clamp(0.05, 0.95))));
        }
        if let Some(d) = med(agg, "/hook/visible_to_s") {
            h.insert("duration".into(), json!(r2(d.clamp(0.5, 30.0))));
        }
        h.insert("max_words".into(), json!(7));
        notes.push(format!(
            "hook: on-screen hook text at the start in {hook_in}/{n} videos; the template sets where and how long it shows, the words are per clip: `rustler overlay add-text <clip> --text \"...\" --style hook-top`"
        ));
        t.insert("hook".into(), Value::Object(h));
    } else {
        notes.push(format!("hook: on-screen hook text at the start in only {hook_in}/{n} videos; no hook look set"));
    }

    // Loudness
    if let Some(l) = med(agg, "/loudness_lufs") {
        let target = (l * 10.0).round() / 10.0;
        let target = target.clamp(-24.0, -9.0);
        t.insert("audio".into(), json!({"loudnorm": true, "target_lufs": target}));
        notes.push(format!("loudness: references measure {l:.1} LUFS integrated -> loudnorm target {target:.1}"));
    } else {
        notes.push("loudness: not measured in the references; the template leaves the loudness target alone".into());
    }

    // Notes only: pacing, zoom, music, layouts.
    if let Some(c) = med(agg, "/cuts_per_minute") {
        let s = med(agg, "/shot_length_s").map_or(String::new(), |s| format!(", median shot {s:.1} s"));
        notes.push(format!(
            "pacing: {c:.1} cuts per minute{s}. Not a template field: tighten with `rustler edit remove-pauses <clip>` and pick moments with that rhythm"
        ));
    }
    if let Some(z) = med(agg, "/zoom_per_minute") {
        notes.push(format!(
            "zoom: references show {z:.1} zoom/punch events per minute. Recorded only: Rustler never adds automatic zooms; place punch-ins yourself where they help (`rustler effect add <clip> --kind punch-in --start S --end E`)"
        ));
    }
    let music = agg.get("music_present_in");
    let (mv, mof) = (music.and_then(|m| m.get("videos")).and_then(Value::as_u64).unwrap_or(0), music.and_then(|m| m.get("of")).and_then(Value::as_u64).unwrap_or(0));
    if mof > 0 {
        if mv * 2 > mof {
            notes.push(format!(
                "music: a music bed was heard in {mv}/{mof} videos. The template picks no track (no music asset is known from the references): add one with `rustler audio add-music <clip> asset:<id>` (see `rustler asset search --kind music`)"
            ));
        } else {
            notes.push(format!("music: music heard in only {mv}/{mof} videos: these references are mostly voice only"));
        }
    } else {
        notes.push("music: not measured (no audio in the references)".into());
    }
    if let Some(top) = agg.get("layout_share").and_then(Value::as_object).and_then(|m| m.iter().max_by(|a, b| a.1.as_f64().unwrap_or(0.0).total_cmp(&b.1.as_f64().unwrap_or(0.0)))) {
        notes.push(format!(
            "layout: most common shot type {} ({:.0}% of shots). Layouts follow each clip's own shots (smart defaults at clip create); not set by this template",
            top.0,
            top.1.as_f64().unwrap_or(0.0) * 100.0
        ));
    }
    notes.push("not derived: fonts, caption animation, transitions, sound effects, keyword colours, brand images".into());
    t.insert("notes".into(), json!(notes));
    let tpl: Template = serde_json::from_value(Value::Object(t))
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "template.create", format!("built template is invalid: {e}")))?;
    Ok((tpl, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Value {
        json!({"name": "x", "aggregate": {
            "videos": 3,
            "cuts_per_minute": {"count": 3, "median": 12.0},
            "shot_length_s": {"count": 9, "median": 4.2},
            "caption": {"present_in": 3, "y_center": {"count": 3, "median": 0.66}, "height": {"count": 3, "median": 0.04},
                        "words_per_line": {"count": 3, "median": 2.6},
                        "fill_colors": [{"hex": "#ffffff", "share": 0.7}, {"hex": "#30f060", "share": 0.2}]},
            "hook": {"present_in": 2, "y_top": {"count": 2, "median": 0.12}, "y_bottom": {"count": 2, "median": 0.2}, "visible_to_s": {"count": 2, "median": 2.5}},
            "zoom_per_minute": {"count": 3, "median": 4.0},
            "loudness_lufs": {"count": 3, "median": -13.04},
            "music_present_in": {"videos": 2, "of": 3},
            "layout_share": {"face_closeup": 0.8, "wide": 0.2}
        }})
    }

    #[test]
    fn maps_captions_hook_loudness_and_notes_the_rest() {
        let (t, notes) = from_profile("learned", "mine", &profile()).unwrap();
        let c = t.captions.as_ref().unwrap();
        assert_eq!(c.position_y, Some(0.68));
        assert_eq!(c.max_words_per_line, Some(3));
        assert_eq!(c.preset.as_deref(), Some("hormozi-pop"));
        let h = t.hook.as_ref().unwrap();
        assert_eq!(h.position_y, Some(0.16));
        assert_eq!(h.duration, Some(2.5));
        assert_eq!(t.audio.as_ref().unwrap().target_lufs, Some(-13.0));
        assert!(t.effects.is_none(), "never automatic zooms");
        let all = notes.join("\n");
        assert!(all.contains("never adds automatic zooms") && all.contains("4.0 zoom"), "{all}");
        assert!(all.contains("music bed was heard in 2/3"), "{all}");
        assert!(all.contains("12.0 cuts per minute"), "{all}");
        assert_eq!(t.notes, notes);
        assert_eq!(t.based_on, Some(json!({"reference_set": "mine"})));
    }

    #[test]
    fn missing_parts_are_notes_not_fields() {
        let p = json!({"aggregate": {"videos": 2, "caption": {"present_in": 0}, "hook": {"present_in": 0}, "music_present_in": {"videos": 0, "of": 2}}});
        let (t, notes) = from_profile("x", "s", &p).unwrap();
        assert!(t.captions.is_none() && t.hook.is_none() && t.audio.is_none());
        assert!(notes.iter().any(|n| n.contains("leaves caption settings alone")));
        assert!(from_profile("x", "s", &json!({})).is_err());
        assert_eq!(preset_for_colors(&[json!({"hex": "#ffffff", "share": 1.0})]).0, "clean-white");
        assert_eq!(preset_for_colors(&[json!({"hex": "#fff200", "share": 0.3})]).0, "bold-karaoke");
    }
}
