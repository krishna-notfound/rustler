//! Layout presets: compute normalized regions for a source on the canvas.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::error::{ErrorCode, Result, RustlerError};
use crate::model::{Background, Canvas, Fit, Layout, Rect, Region};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PRESETS: &[&str] = &["center-crop", "blur-fit", "split-vertical"];

const STAGE: &str = "layout";

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
#[serde(default)]
pub struct LayoutParams {
    /// 0..1 horizontal center for center-crop (default 0.5).
    pub focus_x: Option<f64>,
    /// split-vertical: source rect for the top pane.
    pub top: Option<Rect>,
    /// split-vertical: source rect for the bottom pane.
    pub bottom: Option<Rect>,
    /// split-vertical: top pane height fraction (default 0.5).
    pub split: Option<f64>,
}

fn validation(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg)
}

/// `Validation` for unknown presets (listing `PRESETS`) and missing params
/// ("split-vertical needs --top and --bottom").
pub fn preset(name: &str, src_w: u32, src_h: u32, canvas: &Canvas, params: &LayoutParams) -> Result<Layout> {
    if !PRESETS.contains(&name) {
        return Err(validation(format!("unknown layout preset '{name}'; valid presets: {}", PRESETS.join(", ")))
            .hint("pass one of the listed preset names"));
    }
    if src_w == 0 || src_h == 0 {
        return Err(validation(format!("source dimensions must be non-zero, got {src_w}x{src_h}")));
    }
    if canvas.width == 0 || canvas.height == 0 {
        return Err(validation(format!("canvas dimensions must be non-zero, got {}x{}", canvas.width, canvas.height)));
    }
    let full = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let sa = src_w as f64 / src_h as f64;
    let ca = canvas.width as f64 / canvas.height as f64;

    match name {
        "center-crop" => {
            let focus = params.focus_x.unwrap_or(0.5);
            if !(0.0..=1.0).contains(&focus) {
                return Err(validation(format!("focus_x must be within 0..1, got {focus}")));
            }
            let src = if sa > ca {
                // Source is wider than the canvas: crop horizontally around the focus.
                let w = ca / sa;
                Rect { x: (focus - w / 2.0).clamp(0.0, 1.0 - w), y: 0.0, w, h: 1.0 }
            } else {
                // Source is narrower (or equal): crop vertically, centered.
                let h = sa / ca;
                Rect { x: 0.0, y: (1.0 - h) / 2.0, w: 1.0, h }
            };
            Ok(Layout {
                preset: Some(name.to_string()),
                regions: vec![Region { id: "main".into(), src, dst: full, z: 0, fit: Fit::Cover }],
                background: Background::Black,
            })
        }
        "blur-fit" => {
            // Contain: scale the whole source to the canvas width (or height when it is too tall).
            let dst = if sa >= ca {
                let h = ca / sa;
                Rect { x: 0.0, y: (1.0 - h) / 2.0, w: 1.0, h }
            } else {
                let w = sa / ca;
                Rect { x: (1.0 - w) / 2.0, y: 0.0, w, h: 1.0 }
            };
            Ok(Layout {
                preset: Some(name.to_string()),
                regions: vec![Region { id: "main".into(), src: full, dst, z: 0, fit: Fit::Contain }],
                background: Background::BlurFill,
            })
        }
        _ => {
            let (Some(top), Some(bottom)) = (params.top, params.bottom) else {
                return Err(validation("split-vertical needs --top and --bottom")
                    .hint("pass normalized source rects, e.g. --top 0.75,0,0.25,0.33 --bottom 0,0,0.75,1"));
            };
            let split = params.split.unwrap_or(0.5);
            if !(split > 0.0 && split < 1.0) {
                return Err(validation(format!("split must be within (0, 1), got {split}")));
            }
            for (label, r) in [("top", top), ("bottom", bottom)] {
                let ok = r.w > 0.0
                    && r.h > 0.0
                    && r.x >= 0.0
                    && r.y >= 0.0
                    && r.x + r.w <= 1.0 + 1e-9
                    && r.y + r.h <= 1.0 + 1e-9;
                if !ok {
                    return Err(validation(format!("{label} rect must lie within 0..1 with positive size, got {r:?}")));
                }
            }
            Ok(Layout {
                preset: Some(name.to_string()),
                regions: vec![
                    Region {
                        id: "top".into(),
                        src: top,
                        dst: Rect { x: 0.0, y: 0.0, w: 1.0, h: split },
                        z: 0,
                        fit: Fit::Cover,
                    },
                    Region {
                        id: "bottom".into(),
                        src: bottom,
                        dst: Rect { x: 0.0, y: split, w: 1.0, h: 1.0 - split },
                        z: 0,
                        fit: Fit::Cover,
                    },
                ],
                background: Background::Black,
            })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Region system helpers (additive, A2): validation, pixel rects, warnings, custom presets,
// frame boxes -> layouts and webcam detection maths. Nothing above this line changed.
// ---------------------------------------------------------------------------------------------

/// Smallest allowed normalized width/height of a region rect (exclusive: `w` must be > this).
pub const MIN_REGION_SIZE: f64 = 0.02;
/// `cover` warns when source and destination aspect differ by more than this fraction.
pub const ASPECT_WARN_RATIO: f64 = 0.25;
/// A region upscaled more than this many times looks soft.
pub const SOFT_UPSCALE: f64 = 2.6;
/// A face smaller than this fraction of the frame area counts as a "small" (webcam overlay) face.
pub const SMALL_FACE_AREA: f64 = 0.15;

/// A rectangle in whole pixels.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PxRect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

/// Normalized rect -> pixels of a `w` x `h` frame (edges are rounded, so adjacent rects tile).
pub fn rect_px(r: &Rect, w: u32, h: u32) -> PxRect {
    let (fw, fh) = (w as f64, h as f64);
    let (x0, y0) = ((r.x * fw).round() as i64, (r.y * fh).round() as i64);
    let (x1, y1) = (((r.x + r.w) * fw).round() as i64, ((r.y + r.h) * fh).round() as i64);
    PxRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// `Validation` unless `r` lies inside 0..1 and both sides are larger than [`MIN_REGION_SIZE`].
pub fn validate_rect(label: &str, r: &Rect) -> Result<()> {
    if ![r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) {
        return Err(validation(format!("{label} rect has a non-finite value: {r:?}")));
    }
    if r.w <= MIN_REGION_SIZE || r.h <= MIN_REGION_SIZE {
        return Err(validation(format!(
            "{label} rect is too small (w and h must be > {MIN_REGION_SIZE}): w={:.4} h={:.4}",
            r.w, r.h
        ))
        .hint("rects are normalized 0..1 fractions of the frame, e.g. {\"x\":0.7,\"y\":0.05,\"w\":0.28,\"h\":0.3}"));
    }
    let eps = 1e-9;
    if r.x < -eps || r.y < -eps || r.x + r.w > 1.0 + eps || r.y + r.h > 1.0 + eps {
        return Err(validation(format!(
            "{label} rect must lie inside 0..1 (x+w <= 1, y+h <= 1): x={:.4} y={:.4} w={:.4} h={:.4}",
            r.x, r.y, r.w, r.h
        ))
        .hint("shrink the rect or move it back inside the frame"));
    }
    Ok(())
}

/// Moves/resizes a rect by normalized deltas (no clamping; validate the result).
pub fn nudge_rect(r: &Rect, dx: f64, dy: f64, dw: f64, dh: f64) -> Rect {
    Rect { x: r.x + dx, y: r.y + dy, w: r.w + dw, h: r.h + dh }
}

/// Aspect (w/h) of a normalized rect inside a `fw` x `fh` pixel frame.
fn px_aspect(r: &Rect, fw: u32, fh: u32) -> f64 {
    (r.w * fw as f64) / (r.h * fh as f64).max(1e-9)
}

/// Quality warnings for one region: heavy crops (cover with very different aspects) and soft
/// upscales. `src_w`/`src_h` are the source display dims.
pub fn region_warnings(region: &Region, src_w: u32, src_h: u32, canvas: &Canvas) -> Vec<String> {
    let mut out = Vec::new();
    let sa = px_aspect(&region.src, src_w, src_h);
    let da = px_aspect(&region.dst, canvas.width, canvas.height);
    if region.fit == Fit::Cover && sa > 0.0 && da > 0.0 {
        let (hi, lo) = (sa.max(da), sa.min(da));
        if hi / lo - 1.0 > ASPECT_WARN_RATIO {
            out.push(format!(
                "region '{}': source aspect {sa:.2} vs destination aspect {da:.2} with fit cover: will crop heavily (about {:.0}% of the source rect is cut off)",
                region.id,
                (1.0 - lo / hi) * 100.0
            ));
        }
    }
    let src_px_w = region.src.w * src_w as f64;
    let dst_px_w = region.dst.w * canvas.width as f64;
    let src_px_h = region.src.h * src_h as f64;
    let dst_px_h = region.dst.h * canvas.height as f64;
    let up = if region.fit == Fit::Cover {
        (dst_px_w / src_px_w.max(1.0)).max(dst_px_h / src_px_h.max(1.0))
    } else {
        (dst_px_w / src_px_w.max(1.0)).min(dst_px_h / src_px_h.max(1.0))
    };
    if up > SOFT_UPSCALE {
        out.push(format!(
            "region '{}': upscaled {up:.1}x ({}x{} source pixels): will look soft; pick a larger source rect or a smaller destination",
            region.id,
            src_px_w.round() as i64,
            src_px_h.round() as i64
        ));
    }
    out
}

/// A region id (`r1`, `r2`, ...) unused in every given layout.
pub fn next_region_id(layouts: &[&Layout]) -> String {
    let mut n = 1;
    loop {
        let id = format!("r{n}");
        if layouts.iter().all(|l| l.regions.iter().all(|r| r.id != id)) {
            return id;
        }
        n += 1;
    }
}

/// True for the built-in preset names.
pub fn is_builtin(name: &str) -> bool {
    PRESETS.contains(&name)
}

/// `Validation` unless `name` is a safe custom preset name (`[a-z0-9][a-z0-9_-]{0,39}`) that does
/// not shadow a built-in preset.
pub fn validate_preset_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        return Err(validation(format!("invalid layout preset name '{name}'"))
            .hint("use 1-40 characters: lowercase letters, digits, '-' or '_', starting with a letter or digit"));
    }
    if is_builtin(name) {
        return Err(validation(format!("'{name}' is a built-in preset name")).hint("choose another name for the custom preset"));
    }
    Ok(())
}

/// Project-relative path of a custom preset file: `layouts/<name>.json`.
pub fn preset_rel(name: &str) -> std::path::PathBuf {
    std::path::Path::new("layouts").join(format!("{name}.json"))
}

/// `<project>/layouts/<name>.json`, schema `rustler.layout@1`. Rects are normalized, so the
/// preset applies to any source and canvas size.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct LayoutPresetFile {
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub regions: Vec<Region>,
    pub background: Background,
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Clip the preset was saved from (informational).
    #[serde(default)]
    pub from_clip_id: Option<String>,
}

/// What a framed box on a frame means.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BoxRole {
    Top,
    Bottom,
    Main,
    Inset,
}

/// A rect an agent or the GUI drew on a source frame, with the role it should play.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct FrameBox {
    pub role: BoxRole,
    /// Normalized rect on the source frame.
    pub rect: Rect,
    /// Optional destination on the canvas (only used for `inset`).
    #[serde(default)]
    pub dst: Option<Rect>,
}

/// Default destination of an inset: 34% of the canvas width, source aspect, top-right with a 4%
/// margin.
pub fn default_inset_dst(src: &Rect, src_w: u32, src_h: u32, canvas: &Canvas) -> Rect {
    let w = 0.34;
    let aspect = px_aspect(src, src_w, src_h);
    let h = (w * canvas.width as f64 / aspect / canvas.height as f64).clamp(MIN_REGION_SIZE * 2.0, 0.5);
    Rect { x: 1.0 - w - 0.04, y: 0.04, w, h }
}

/// Top pane height fraction that shows `top` without cropping on the canvas (clamped 0.25..0.6).
pub fn split_for_rect(top: &Rect, src_w: u32, src_h: u32, canvas: &Canvas) -> f64 {
    let a = px_aspect(top, src_w, src_h);
    ((canvas.width as f64 / canvas.height as f64) / a).clamp(0.25, 0.6)
}

/// Builds a layout from boxes: `top`+`bottom` (vertical split, optional `inset`) or `main`
/// (full canvas, optional `inset`). Every rect is validated. `main_fit` applies to `main` only.
pub fn layout_from_boxes(
    boxes: &[FrameBox],
    src_w: u32,
    src_h: u32,
    canvas: &Canvas,
    split: Option<f64>,
    background: Option<Background>,
    main_fit: Fit,
) -> Result<Layout> {
    let find = |role: BoxRole| boxes.iter().filter(|b| b.role == role).collect::<Vec<_>>();
    for role in [BoxRole::Top, BoxRole::Bottom, BoxRole::Main, BoxRole::Inset] {
        if find(role).len() > 1 {
            return Err(validation(format!("role {role:?} given more than once")).hint("use each role at most once"));
        }
    }
    for b in boxes {
        validate_rect(&format!("{:?}", b.role).to_lowercase(), &b.rect)?;
        if let Some(d) = &b.dst {
            validate_rect("inset dst", d)?;
        }
    }
    let first = |role: BoxRole| find(role).first().copied().copied();
    let (top, bottom, main, inset) = (first(BoxRole::Top), first(BoxRole::Bottom), first(BoxRole::Main), first(BoxRole::Inset));
    let full = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let mut regions = Vec::new();
    let mut preset = None;
    let mut bg = background.unwrap_or(Background::Black);
    match (top, bottom, main) {
        (Some(t), Some(b), None) => {
            let s = split.unwrap_or_else(|| split_for_rect(&t.rect, src_w, src_h, canvas));
            if !(s > 0.0 && s < 1.0) {
                return Err(validation(format!("split must be within (0, 1), got {s}")));
            }
            regions.push(Region { id: "top".into(), src: t.rect, dst: Rect { x: 0.0, y: 0.0, w: 1.0, h: s }, z: 0, fit: Fit::Cover });
            regions.push(Region { id: "bottom".into(), src: b.rect, dst: Rect { x: 0.0, y: s, w: 1.0, h: 1.0 - s }, z: 0, fit: Fit::Cover });
            if inset.is_none() {
                preset = Some("split-vertical".to_string());
            }
        }
        (None, None, Some(m)) => {
            if main_fit == Fit::Contain && background.is_none() {
                bg = Background::BlurFill;
            }
            regions.push(Region { id: "main".into(), src: m.rect, dst: full, z: 0, fit: main_fit });
        }
        _ => {
            return Err(validation("boxes must be either top + bottom, or main (each optionally with one inset)")
                .hint("example: [{\"role\":\"top\",\"rect\":{...}},{\"role\":\"bottom\",\"rect\":{...}}]"));
        }
    }
    if let Some(i) = inset {
        let dst = i.dst.unwrap_or_else(|| default_inset_dst(&i.rect, src_w, src_h, canvas));
        regions.push(Region { id: "inset".into(), src: i.rect, dst, z: 1, fit: Fit::Cover });
    }
    Ok(Layout { preset, regions, background: bg })
}

/// Median of a non-empty slice.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// A face found consistently in the same small spot across the samples.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct ConsistentFace {
    /// Median face box (normalized).
    pub face: Rect,
    /// Samples (frames) the face was found in.
    pub hits: usize,
    /// Samples examined.
    pub samples: usize,
}

/// Finds the small face (area < [`SMALL_FACE_AREA`]) that shows up at about the same place in
/// the most samples. `None` when no small face appears in at least half of the samples (and at
/// least 2 when there are 2 or more samples).
pub fn find_webcam_face(samples: &[Vec<Rect>]) -> Option<ConsistentFace> {
    let n = samples.len();
    if n == 0 {
        return None;
    }
    let small: Vec<Vec<Rect>> = samples
        .iter()
        .map(|fs| fs.iter().filter(|f| f.w * f.h < SMALL_FACE_AREA && f.w * f.h >= 0.0004).copied().collect())
        .collect();
    let near = |a: &Rect, b: &Rect| {
        let (ax, ay) = (a.x + a.w / 2.0, a.y + a.h / 2.0);
        let (bx, by) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        let ratio = (a.w * a.h) / (b.w * b.h).max(1e-9);
        (ax - bx).abs() < 0.06 && (ay - by).abs() < 0.06 && (0.5..2.0).contains(&ratio)
    };
    let need = ((n as f64 * 0.5).ceil() as usize).max(2.min(n));
    let mut best: Option<(usize, f64, Vec<Rect>)> = None;
    for cand in small.iter().flatten() {
        let matched: Vec<Rect> = small.iter().filter_map(|fs| fs.iter().find(|f| near(cand, f)).copied()).collect();
        let area = cand.w * cand.h;
        let better = match &best {
            None => true,
            Some((c, a, _)) => matched.len() > *c || (matched.len() == *c && area > *a),
        };
        if better {
            best = Some((matched.len(), area, matched));
        }
    }
    let (hits, _, matched) = best?;
    if hits < need {
        return None;
    }
    let med = |f: fn(&Rect) -> f64| median(&mut matched.iter().map(f).collect::<Vec<_>>());
    let face = Rect { x: med(|r| r.x), y: med(|r| r.y), w: med(|r| r.w), h: med(|r| r.h) };
    Some(ConsistentFace { face, hits, samples: n })
}

/// Webcam overlay rect guessed from its face: the face box expanded 2.6x wide and 2.2x tall around
/// its centre, clamped to the frame, and snapped flush to a frame edge that is less than 4% away.
pub fn webcam_rect_from_face(face: &Rect) -> Rect {
    let (cx, cy) = (face.x + face.w / 2.0, face.y + face.h / 2.0);
    let (w, h) = (face.w * 2.6, face.h * 2.2);
    let (mut x0, mut y0) = ((cx - w / 2.0).max(0.0), (cy - h / 2.0).max(0.0));
    let (mut x1, mut y1) = ((cx + w / 2.0).min(1.0), (cy + h / 2.0).min(1.0));
    const SNAP: f64 = 0.04;
    if x0 < SNAP {
        x0 = 0.0;
    }
    if y0 < SNAP {
        y0 = 0.0;
    }
    if 1.0 - x1 < SNAP {
        x1 = 1.0;
    }
    if 1.0 - y1 < SNAP {
        y1 = 1.0;
    }
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// Where a rect sits in the frame: "top-right", "bottom-left", "top-center", "left", ..., "center".
pub fn frame_position(r: &Rect) -> String {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let col = if cx < 0.35 { "left" } else if cx > 0.65 { "right" } else { "center" };
    let row = if cy < 0.35 { "top" } else if cy > 0.65 { "bottom" } else { "middle" };
    match (row, col) {
        ("middle", "center") => "center".to_string(),
        ("middle", c) => c.to_string(),
        (r, "center") => format!("{r}-center"),
        (r, c) => format!("{r}-{c}"),
    }
}

/// Split (top pane height fraction) that shows the webcam rect without cropping, clamped 0.30..0.50.
pub fn split_for_webcam(webcam: &Rect, src_w: u32, src_h: u32, canvas: &Canvas) -> f64 {
    let a = px_aspect(webcam, src_w, src_h);
    ((canvas.width as f64 / canvas.height as f64) / a).clamp(0.30, 0.50)
}

/// Gameplay rect for the bottom pane: full source height, the width that fills the bottom pane
/// without cropping, centred but shifted sideways to stay clear of the webcam. The bool is false
/// when the webcam could not be avoided.
pub fn gameplay_rect(webcam: &Rect, split: f64, src_w: u32, src_h: u32, canvas: &Canvas) -> (Rect, bool) {
    let bottom_aspect = canvas.width as f64 / (canvas.height as f64 * (1.0 - split));
    let w = (bottom_aspect * src_h as f64 / src_w as f64).min(1.0);
    let mut x = (1.0 - w) / 2.0;
    let overlaps = |x: f64| x < webcam.x + webcam.w - 1e-9 && x + w > webcam.x + 1e-9;
    let mut clear = true;
    if overlaps(x) {
        let webcam_right = webcam.x + webcam.w / 2.0 >= 0.5;
        let shifted = if webcam_right { webcam.x - w } else { webcam.x + webcam.w };
        if shifted >= -1e-9 && shifted + w <= 1.0 + 1e-9 {
            x = shifted.max(0.0);
        } else {
            clear = false;
        }
    }
    (Rect { x, y: 0.0, w, h: 1.0 }, clear)
}

/// Rounds the rect to 3 decimals (command text / JSON payloads).
pub fn round_rect(r: &Rect) -> Rect {
    let q = |v: f64| (v * 1000.0).round() / 1000.0;
    Rect { x: q(r.x), y: q(r.y), w: q(r.w), h: q(r.h) }
}

#[cfg(test)]
mod region_tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn rect_validation_rules() {
        assert!(validate_rect("a", &r(0.0, 0.0, 1.0, 1.0)).is_ok());
        assert!(validate_rect("a", &r(0.5, 0.5, 0.6, 0.2)).is_err());
        assert!(validate_rect("a", &r(0.0, 0.0, 0.02, 0.5)).is_err());
        assert!(validate_rect("a", &r(-0.1, 0.0, 0.5, 0.5)).is_err());
        assert!(validate_rect("a", &r(f64::NAN, 0.0, 0.5, 0.5)).is_err());
    }

    #[test]
    fn pixel_rects_tile() {
        let a = rect_px(&r(0.0, 0.0, 1.0, 0.44), 1080, 1920);
        let b = rect_px(&r(0.0, 0.44, 1.0, 0.56), 1080, 1920);
        assert_eq!(a.y + a.h, b.y);
        assert_eq!(b.y + b.h, 1920);
    }

    #[test]
    fn heavy_crop_warns() {
        let canvas = Canvas::default();
        let reg = Region { id: "top".into(), src: r(0.0, 0.0, 1.0, 1.0), dst: r(0.0, 0.0, 1.0, 0.25), z: 0, fit: Fit::Cover };
        let w = region_warnings(&reg, 1920, 1080, &canvas);
        assert!(w.iter().any(|m| m.contains("crop heavily")), "{w:?}");
        let ok = Region { id: "m".into(), src: r(0.0, 0.0, 0.316, 1.0), dst: r(0.0, 0.0, 1.0, 1.0), z: 0, fit: Fit::Cover };
        assert!(region_warnings(&ok, 1920, 1080, &canvas).is_empty());
    }

    #[test]
    fn boxes_build_split_and_reject_bad_combos() {
        let c = Canvas::default();
        let boxes = [
            FrameBox { role: BoxRole::Top, rect: r(0.75, 0.0, 0.25, 0.4), dst: None },
            FrameBox { role: BoxRole::Bottom, rect: r(0.1, 0.0, 0.6, 1.0), dst: None },
        ];
        let l = layout_from_boxes(&boxes, 1920, 1080, &c, None, None, Fit::Cover).unwrap();
        assert_eq!(l.preset.as_deref(), Some("split-vertical"));
        assert_eq!(l.regions.len(), 2);
        assert!((l.regions[0].dst.h + l.regions[1].dst.h - 1.0).abs() < 1e-9);
        let bad = [FrameBox { role: BoxRole::Top, rect: r(0.0, 0.0, 1.0, 1.0), dst: None }];
        assert!(layout_from_boxes(&bad, 1920, 1080, &c, None, None, Fit::Cover).is_err());
    }

    #[test]
    fn webcam_found_in_corner_and_gameplay_avoids_it() {
        let face = r(0.84, 0.06, 0.07, 0.12);
        let samples = vec![vec![face], vec![r(0.841, 0.061, 0.07, 0.12)], vec![], vec![face], vec![face], vec![face]];
        let found = find_webcam_face(&samples).expect("face");
        assert_eq!(found.hits, 5);
        let cam = webcam_rect_from_face(&found.face);
        assert!(cam.x < found.face.x && cam.x + cam.w <= 1.0 + 1e-9);
        assert!((cam.x + cam.w - 1.0).abs() < 1e-9, "snaps flush to the right edge");
        assert_eq!(frame_position(&cam), "top-right");
        let split = split_for_webcam(&cam, 1920, 1080, &Canvas::default());
        assert!((0.3..=0.5).contains(&split));
        let (g, clear) = gameplay_rect(&cam, split, 1920, 1080, &Canvas::default());
        assert!(clear && g.x + g.w <= cam.x + 1e-9);
    }

    #[test]
    fn big_or_inconsistent_faces_are_not_webcams() {
        let big = vec![vec![r(0.3, 0.1, 0.4, 0.6)]; 4];
        assert!(find_webcam_face(&big).is_none());
        let moving = vec![vec![r(0.05, 0.05, 0.08, 0.1)], vec![r(0.6, 0.6, 0.08, 0.1)], vec![r(0.3, 0.2, 0.08, 0.1)], vec![]];
        assert!(find_webcam_face(&moving).is_none());
    }

    #[test]
    fn preset_names() {
        assert!(validate_preset_name("my-split_2").is_ok());
        assert!(validate_preset_name("Bad Name").is_err());
        assert!(validate_preset_name("blur-fit").is_err());
        assert!(validate_preset_name("../x").is_err());
    }
}
