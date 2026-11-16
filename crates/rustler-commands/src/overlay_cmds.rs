//! Commands: overlay.add_text, overlay.list, overlay.edit, overlay.remove.
//!
//! Text overlays (hook titles, lower-thirds) live in `clips/<id>/extras.json` under `overlays`
//! (read-modify-write through [`crate::extras`], other keys untouched). `render.run` burns them
//! on top of the captions in the same single encode (`overlays.ass`).

use crate::clip_cmds::log_event;
use crate::extras;
use rustler_core::{Canvas, Clip, ClipId, ErrorCode, ProjectStore, RustlerError, TimeMap};
use rustler_media::compile::{
    OVERLAY_MAX_CHARS, OverlayLayout, OverlayStyle, TextOverlay, clean_overlay_text, overlay_layout,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

type Result<T> = rustler_core::Result<T>;

/// extras.json key.
pub const KEY: &str = "overlays";
/// Default on-screen time of a new overlay (seconds).
const DEFAULT_LEN: f64 = 3.0;
/// End may exceed the clip duration by this much (rounding).
const END_TOLERANCE: f64 = 0.05;
/// Hooks read best at this many words or fewer.
const HOOK_WORDS: usize = 7;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested below)
// ---------------------------------------------------------------------------------------------

/// Cleaned text; Validation when empty or longer than 120 characters.
pub fn check_text(stage: &str, text: &str) -> Result<String> {
    let t = clean_overlay_text(text);
    if t.is_empty() {
        return Err(validation(stage, "overlay text is empty").hint("pass the hook text, e.g. \"He quit his job for this\""));
    }
    let n = t.chars().count();
    if n > OVERLAY_MAX_CHARS {
        return Err(validation(stage, format!("overlay text is {n} characters; the limit is {OVERLAY_MAX_CHARS}"))
            .hint("keep hooks <= 7 words"));
    }
    Ok(t)
}

/// Validates `start`/`end` (output seconds) against the clip duration.
pub fn check_times(stage: &str, start: f64, end: f64, duration: f64) -> Result<()> {
    if !(start.is_finite() && end.is_finite()) || start < 0.0 {
        return Err(validation(stage, "start and end must be finite and start >= 0"));
    }
    if start >= duration {
        return Err(validation(stage, format!("start {start:.2}s is past the end of the clip ({duration:.2}s)"))
            .hint("`rustler clip show <clip_id>` shows the output duration"));
    }
    if end <= start {
        return Err(validation(stage, format!("end ({end:.2}) must be after start ({start:.2})")));
    }
    if end > duration + END_TOLERANCE {
        return Err(validation(stage, format!("end {end:.2}s is past the end of the clip ({duration:.2}s)"))
            .hint("pick an end inside the clip, or leave it out (default: start + 3 s, clamped)"));
    }
    Ok(())
}

fn check_style_values(stage: &str, o: &TextOverlay) -> Result<()> {
    if !(0.0..=1.0).contains(&o.position_y) {
        return Err(validation(stage, "position_y must be between 0 (top) and 1 (bottom)"));
    }
    if !(o.font_size_factor.is_finite() && (0.01..=0.2).contains(&o.font_size_factor)) {
        return Err(validation(stage, "font_size_factor must be between 0.01 and 0.2 (hook-top default 0.085)"));
    }
    Ok(())
}

/// Advisory warnings: long hooks, text that does not fit 2 lines, overlap with the captions.
pub fn overlay_warnings(o: &TextOverlay, layout: &OverlayLayout, clip: &Clip) -> Vec<String> {
    let mut w = Vec::new();
    let words = o.text.split_whitespace().count();
    if o.style == OverlayStyle::HookTop && words > HOOK_WORDS {
        w.push(format!("hook has {words} words; keep hooks <= {HOOK_WORDS} words"));
    }
    if !layout.fits {
        w.push("text needs more than 2 lines even at the smallest size; shorten it".into());
    }
    if clip.captions.enabled {
        let c = clip.canvas;
        let size = rustler_captions::presets::find_preset(&clip.captions.preset)
            .map_or(0.072, |p| p.size_factor)
            * c.width as f64;
        // Captions are bottom-anchored at position_y; allow two lines above that.
        let cap_bottom = clip.captions.position_y * c.height as f64 + size * 0.2;
        let cap_top = clip.captions.position_y * c.height as f64 - 2.2 * size;
        if layout.top < cap_bottom && layout.bottom > cap_top {
            w.push(format!(
                "overlay (y {:.0}..{:.0} px) may overlap the captions (about y {:.0}..{:.0} px); move it with --position-y",
                layout.top, layout.bottom, cap_top, cap_bottom
            ));
        }
    }
    w
}

/// Next free id `ovl_<n>`.
pub fn next_id(existing: &[TextOverlay]) -> String {
    let n = existing
        .iter()
        .filter_map(|o| o.id.strip_prefix("ovl_").and_then(|s| s.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    format!("ovl_{}", n + 1)
}

// ---------------------------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------------------------

/// Overlays of a clip (empty when none).
pub fn load_overlays(store: &ProjectStore, clip: &ClipId) -> Result<Vec<TextOverlay>> {
    Ok(extras::get(store, clip, KEY)?.unwrap_or_default())
}

/// Read-modify-write of the overlay list; `f` returns a value passed back to the caller.
fn modify<T>(store: &ProjectStore, clip: &ClipId, f: impl FnOnce(&mut Vec<TextOverlay>) -> Result<T>) -> Result<T> {
    let mut out = None;
    extras::update(store, clip, KEY, |cur| {
        let mut list: Vec<TextOverlay> = match cur {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(v) => serde_json::from_value(v).map_err(|e| {
                RustlerError::new(ErrorCode::Internal, "overlay", format!("extras.json overlays are invalid: {e}"))
            })?,
        };
        out = Some(f(&mut list)?);
        if list.is_empty() {
            return Ok(None);
        }
        serde_json::to_value(&list)
            .map(Some)
            .map_err(|e| RustlerError::new(ErrorCode::Internal, "overlay", e.to_string()))
    })?;
    Ok(out.expect("closure ran"))
}

fn duration_of(clip: &Clip) -> f64 {
    TimeMap::from_segments(&clip.segments).duration()
}

// ---------------------------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct OverlayView {
    #[serde(flatten)]
    pub overlay: TextOverlay,
    /// Wrapped lines, font size and pixel box on the canvas.
    pub layout: OverlayLayout,
}

fn view(o: TextOverlay, canvas: &Canvas) -> OverlayView {
    let layout = overlay_layout(&o, canvas);
    OverlayView { overlay: o, layout }
}

#[derive(Serialize, JsonSchema)]
pub struct OverlayOut {
    pub clip_id: ClipId,
    pub overlay_id: String,
    pub overlay: OverlayView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn next_steps(clip: &Clip, o: &TextOverlay) -> Vec<String> {
    let t = ((o.start + o.end) / 2.0).min(o.start + 1.0);
    vec![
        format!("rustler render preview-frame {} --out-t {t:.2}", clip.id),
        format!("rustler render run {} --quality draft", clip.id),
    ]
}

// ---------------------------------------------------------------------------------------------
// overlay.add_text
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AddTextIn {
    pub clip_id: ClipId,
    /// The text (<= 120 characters; hooks read best at <= 7 words). Wrapped to max 2 lines.
    pub text: String,
    /// Output time it appears (default 0).
    pub start: Option<f64>,
    /// Output time it disappears (default start + 3 s, clamped to the clip).
    pub end: Option<f64>,
    /// hook-top (default) | lower-third | plain
    pub style: Option<OverlayStyle>,
    /// Vertical centre 0 (top) .. 1 (bottom); default by style: hook-top 0.12, lower-third 0.80, plain 0.50.
    pub position_y: Option<f64>,
    /// Font size as a fraction of the canvas width (hook-top 0.085, lower-third 0.05, plain 0.065).
    pub font_size_factor: Option<f64>,
    /// Dark box behind the text (default true, plain: false).
    #[serde(rename = "box")]
    pub boxed: Option<bool>,
}

async fn add_text(ctx: Ctx, i: AddTextIn) -> Result<OverlayOut> {
    const STAGE: &str = "overlay.add_text";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let duration = duration_of(&clip);
    let text = check_text(STAGE, &i.text)?;
    let start = i.start.unwrap_or(0.0);
    let end = i.end.unwrap_or_else(|| (start + DEFAULT_LEN).min(duration));
    check_times(STAGE, start, end, duration)?;
    let style = i.style.unwrap_or_default();
    let mut o = TextOverlay::new(String::new(), text, start, end.min(duration), style);
    if let Some(y) = i.position_y {
        o.position_y = y;
    } else if matches!(style, rustler_media::compile::OverlayStyle::HookTop) {
        // Stacked layouts (webcam over gameplay): put the hook on the seam between the panels
        // instead of over the top panel's face.
        if let Some(seam) = stacked_seam(&clip) {
            o.position_y = seam;
        }
    }
    if let Some(f) = i.font_size_factor {
        o.font_size_factor = f;
    }
    if let Some(b) = i.boxed {
        o.boxed = b;
    }
    check_style_values(STAGE, &o)?;

    if ctx.dry_run {
        o.id = next_id(&load_overlays(&store, &clip.id)?);
    } else {
        o = modify(&store, &clip.id, |list| {
            o.id = next_id(list);
            list.push(o.clone());
            Ok(o.clone())
        })?;
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "overlay_id": o.id, "text": o.text}))?;
    }
    let v = view(o, &clip.canvas);
    let warnings = overlay_warnings(&v.overlay, &v.layout, &clip);
    Ok(OverlayOut {
        clip_id: clip.id.clone(),
        overlay_id: v.overlay.id.clone(),
        next: next_steps(&clip, &v.overlay),
        overlay: v,
        dry_run: ctx.dry_run.then_some(true),
        warnings,
    })
}

// ---------------------------------------------------------------------------------------------
// overlay.list
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ListIn {
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOut {
    pub clip_id: ClipId,
    /// Clip output duration (seconds).
    pub duration: f64,
    pub overlays: Vec<OverlayView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

async fn list(ctx: Ctx, i: ListIn) -> Result<ListOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let duration = duration_of(&clip);
    let mut warnings = Vec::new();
    let overlays: Vec<OverlayView> = load_overlays(&store, &clip.id)?
        .into_iter()
        .map(|o| {
            if o.start >= duration {
                warnings.push(format!("{} starts after the clip ends ({duration:.2}s); it will not render", o.id));
            } else if o.end > duration + END_TOLERANCE {
                warnings.push(format!("{} ends after the clip ({duration:.2}s); it is cut at the end", o.id));
            }
            view(o, &clip.canvas)
        })
        .collect();
    Ok(ListOut { clip_id: clip.id, duration, overlays, warnings })
}

// ---------------------------------------------------------------------------------------------
// overlay.edit
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct EditIn {
    pub clip_id: ClipId,
    pub overlay_id: String,
    pub text: Option<String>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// Changing the style resets position_y / font_size_factor / box to the new style's
    /// defaults unless they are given too.
    pub style: Option<OverlayStyle>,
    pub position_y: Option<f64>,
    pub font_size_factor: Option<f64>,
    #[serde(rename = "box")]
    pub boxed: Option<bool>,
}

/// Applies the edit to `o` (pure; validated by the caller).
pub fn apply_edit(stage: &str, o: &mut TextOverlay, i: &EditIn, duration: f64) -> Result<()> {
    if let Some(t) = &i.text {
        o.text = check_text(stage, t)?;
    }
    if let Some(s) = i.style.filter(|s| *s != o.style) {
        o.style = s;
        o.position_y = s.default_position_y();
        o.font_size_factor = s.default_font_size_factor();
        o.boxed = s.default_box();
    }
    if let Some(y) = i.position_y {
        o.position_y = y;
    }
    if let Some(f) = i.font_size_factor {
        o.font_size_factor = f;
    }
    if let Some(b) = i.boxed {
        o.boxed = b;
    }
    let start = i.start.unwrap_or(o.start);
    let mut end = i.end.unwrap_or(o.end);
    if i.end.is_none() && i.start.is_some() && end <= start {
        // Moved the start past the old end: keep the old length.
        end = (start + (o.end - o.start).max(0.5)).min(duration);
    }
    check_times(stage, start, end, duration)?;
    o.start = start;
    o.end = end.min(duration);
    check_style_values(stage, o)
}

fn not_found(stage: &str, clip: &ClipId, id: &str, list: &[TextOverlay]) -> RustlerError {
    let ids: Vec<&str> = list.iter().map(|o| o.id.as_str()).collect();
    RustlerError::new(ErrorCode::NotFound, stage, format!("clip {clip} has no overlay '{id}'"))
        .hint(if ids.is_empty() { "the clip has no overlays; add one with `rustler overlay add-text`".to_string() } else { format!("overlays: {}", ids.join(", ")) })
}

async fn edit(ctx: Ctx, i: EditIn) -> Result<OverlayOut> {
    const STAGE: &str = "overlay.edit";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let duration = duration_of(&clip);
    let apply = |list: &mut Vec<TextOverlay>| -> Result<TextOverlay> {
        let Some(o) = list.iter_mut().find(|o| o.id == i.overlay_id) else {
            return Err(not_found(STAGE, &clip.id, &i.overlay_id, list));
        };
        apply_edit(STAGE, o, &i, duration)?;
        Ok(o.clone())
    };
    let o = if ctx.dry_run {
        apply(&mut load_overlays(&store, &clip.id)?)?
    } else {
        let o = modify(&store, &clip.id, apply)?;
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "overlay_id": o.id}))?;
        o
    };
    let v = view(o, &clip.canvas);
    let warnings = overlay_warnings(&v.overlay, &v.layout, &clip);
    Ok(OverlayOut {
        clip_id: clip.id.clone(),
        overlay_id: v.overlay.id.clone(),
        next: next_steps(&clip, &v.overlay),
        overlay: v,
        dry_run: ctx.dry_run.then_some(true),
        warnings,
    })
}

// ---------------------------------------------------------------------------------------------
// overlay.remove
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct RemoveIn {
    pub clip_id: ClipId,
    pub overlay_id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct RemoveOut {
    pub clip_id: ClipId,
    pub removed: String,
    /// Ids of the overlays left.
    pub remaining: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn remove(ctx: Ctx, i: RemoveIn) -> Result<RemoveOut> {
    const STAGE: &str = "overlay.remove";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let apply = |list: &mut Vec<TextOverlay>| -> Result<Vec<String>> {
        let Some(pos) = list.iter().position(|o| o.id == i.overlay_id) else {
            return Err(not_found(STAGE, &clip.id, &i.overlay_id, list));
        };
        list.remove(pos);
        Ok(list.iter().map(|o| o.id.clone()).collect())
    };
    let remaining = if ctx.dry_run {
        apply(&mut load_overlays(&store, &clip.id)?)?
    } else {
        let r = modify(&store, &clip.id, apply)?;
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "overlay_id": i.overlay_id}))?;
        r
    };
    Ok(RemoveOut { clip_id: clip.id, removed: i.overlay_id, remaining, dry_run: ctx.dry_run.then_some(true) })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    reg.register(typed(
        "overlay.add_text",
        "Add a text overlay (hook title) on the output timeline; burned above the captions at render. Keep hooks <= 7 words",
        &["clip_id", "text"],
        write,
        vec![
            Example { description: "hook title for the first 3 s", cli: "rustler overlay add-text clp_x \"He quit his job for this\"" },
            Example {
                description: "lower-third label from 2 s to 6 s",
                cli: "rustler overlay add-text clp_x \"Dr. Jane Doe, MIT\" --style lower-third --start 2 --end 6",
            },
        ],
        add_text,
    ));
    reg.register(typed(
        "overlay.list",
        "List a clip's text overlays with their wrapped lines and pixel position",
        &["clip_id"],
        Annotations { read_only: true, ..write },
        vec![Example { description: "overlays of a clip", cli: "rustler overlay list clp_x" }],
        list,
    ));
    reg.register(typed(
        "overlay.edit",
        "Change a text overlay's text, times, style, position, size or box",
        &["clip_id", "overlay_id"],
        write,
        vec![Example { description: "shorter hook, shown until 2.5 s", cli: "rustler overlay edit clp_x ovl_1 --text \"Nobody saw this coming\" --end 2.5" }],
        edit,
    ));
    reg.register(typed(
        "overlay.remove",
        "Remove a text overlay from a clip",
        &["clip_id", "overlay_id"],
        Annotations { destructive: true, ..write },
        vec![Example { description: "remove the hook", cli: "rustler overlay remove clp_x ovl_1" }],
        remove,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_checks() {
        assert_eq!(check_text("t", "  Hello   {\\b1}world ").unwrap(), "Hello b1world");
        assert_eq!(check_text("t", "  ").unwrap_err().code, ErrorCode::Validation);
        assert_eq!(check_text("t", &"x".repeat(121)).unwrap_err().code, ErrorCode::Validation);
        assert!(check_text("t", &"x".repeat(120)).is_ok());
    }

    #[test]
    fn time_checks() {
        assert!(check_times("t", 0.0, 3.0, 10.0).is_ok());
        assert!(check_times("t", 0.0, 10.04, 10.0).is_ok());
        for (s, e) in [(-1.0, 2.0), (10.0, 11.0), (3.0, 3.0), (4.0, 2.0), (0.0, 10.2), (f64::NAN, 1.0)] {
            assert_eq!(check_times("t", s, e, 10.0).unwrap_err().code, ErrorCode::Validation, "{s} {e}");
        }
    }

    #[test]
    fn ids_and_edits() {
        let a = TextOverlay::new("ovl_1", "a", 0.0, 3.0, OverlayStyle::HookTop);
        let b = TextOverlay::new("ovl_7", "b", 0.0, 3.0, OverlayStyle::HookTop);
        assert_eq!(next_id(&[]), "ovl_1");
        assert_eq!(next_id(&[a.clone(), b]), "ovl_8");

        let mut o = a;
        let edit = |style, start, end| EditIn {
            clip_id: ClipId::new(),
            overlay_id: "ovl_1".into(),
            text: None,
            start,
            end,
            style,
            position_y: None,
            font_size_factor: None,
            boxed: None,
        };
        apply_edit("t", &mut o, &edit(Some(OverlayStyle::Plain), None, None), 10.0).unwrap();
        assert_eq!((o.position_y, o.boxed), (0.5, false));
        // Moving the start past the old end keeps the length.
        apply_edit("t", &mut o, &edit(None, Some(5.0), None), 10.0).unwrap();
        assert_eq!((o.start, o.end), (5.0, 8.0));
        assert!(apply_edit("t", &mut o, &edit(None, None, Some(12.0)), 10.0).is_err());
        assert_eq!(o.end, 8.0, "failed edit leaves the overlay unchanged in storage (caller discards)");
    }
}

/// The y of the boundary between two vertically stacked panels in the clip's first segment
/// (e.g. split-vertical: webcam top, gameplay bottom), when there is one.
fn stacked_seam(clip: &Clip) -> Option<f64> {
    let regions = &clip.segments.first()?.layout.regions;
    if regions.len() < 2 {
        return None;
    }
    // A panel that starts well below the top and spans most of the width marks the seam.
    regions
        .iter()
        .filter(|r| r.dst.y > 0.2 && r.dst.y < 0.8 && r.dst.w > 0.8)
        .map(|r| r.dst.y)
        .min_by(|a, b| a.total_cmp(b))
}
