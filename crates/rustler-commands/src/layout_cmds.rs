//! Layout region system: `layout.show`, `layout.add_region`, `layout.edit_region`,
//! `layout.remove_region`, `layout.reorder`, `layout.from_frame_boxes`, custom presets
//! (`layout.save_preset`, `layout.apply_preset`, `layout.delete_preset`, `layout.presets`) and
//! `layout.detect_webcam`.
//!
//! A segment's layout is a list of regions: a normalized source rect (`src`, on the source's
//! display frame) drawn into a normalized destination rect (`dst`, on the canvas) with a fit and a
//! z order. Every mutating command loads the clip, edits a copy, saves with a revision check
//! (`base_revision` defaults to the revision just loaded), appends an event and honors `--dry-run`.
//! `segment_id` is optional everywhere: omitted = every segment.
//!
//! `layout.list_presets` and `clip.set_layout` stay in `clip_cmds`.

use crate::analyze_cmds::spawn_engine;
use crate::clip_cmds::{EmptyIn, layout_preset_info, log_event};
use rustler_core::layout::{self, FrameBox, LayoutParams, LayoutPresetFile, PxRect};
use rustler_core::{
    Background, Canvas, Clip, ClipId, ErrorCode, Fit, Layout, ProjectStore, Rect, Region, Result, RustlerError, Segment,
    SegmentId, SourceId, TimeMap,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct RegionView {
    pub id: String,
    /// Normalized source rect (fractions of the source display frame).
    pub src: Rect,
    /// The same rect in source pixels.
    pub src_px: PxRect,
    /// Normalized destination rect (fractions of the canvas).
    pub dst: Rect,
    /// The same rect in canvas pixels.
    pub dst_px: PxRect,
    pub fit: Fit,
    /// Draw order: higher z is drawn on top.
    pub z: i32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SegmentLayoutView {
    pub segment_id: SegmentId,
    pub source_id: SourceId,
    /// Source display size in pixels (after rotation); `src_px` is measured on it.
    pub source_width: u32,
    pub source_height: u32,
    /// Preset name, or null for a custom (hand-edited or saved-preset) layout.
    pub preset: Option<String>,
    pub background: Background,
    pub regions: Vec<RegionView>,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct LayoutView {
    pub clip_id: ClipId,
    pub revision: u64,
    pub canvas: Canvas,
    pub segments: Vec<SegmentLayoutView>,
    /// The region the command added or edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Source display dimensions of every source a clip uses.
fn source_dims(store: &ProjectStore, clip: &Clip) -> Result<HashMap<SourceId, (u32, u32)>> {
    let mut out = HashMap::new();
    for seg in &clip.segments {
        if !out.contains_key(&seg.source_id) {
            let s = store.load_source(&seg.source_id)?;
            out.insert(seg.source_id.clone(), (s.probe.width, s.probe.height));
        }
    }
    Ok(out)
}

fn target_idx(clip: &Clip, segment_id: &Option<SegmentId>, stage: &str) -> Result<Vec<usize>> {
    let idx: Vec<usize> = clip
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| segment_id.as_ref().is_none_or(|id| id == &s.id))
        .map(|(i, _)| i)
        .collect();
    if idx.is_empty() {
        return Err(RustlerError::new(ErrorCode::NotFound, stage, format!("clip {} has no such segment", clip.id))
            .hint("list segment ids with `rustler clip show <clip_id>`"));
    }
    Ok(idx)
}

fn segment_view(seg: &Segment, dims: (u32, u32), canvas: &Canvas, touched: Option<&[String]>) -> SegmentLayoutView {
    let regions = seg
        .layout
        .regions
        .iter()
        .map(|r| {
            let is_touched = touched.is_none_or(|t| t.contains(&r.id));
            RegionView {
                id: r.id.clone(),
                src: r.src,
                src_px: layout::rect_px(&r.src, dims.0, dims.1),
                dst: r.dst,
                dst_px: layout::rect_px(&r.dst, canvas.width, canvas.height),
                fit: r.fit,
                z: r.z,
                warnings: if is_touched { layout::region_warnings(r, dims.0, dims.1, canvas) } else { Vec::new() },
            }
        })
        .collect();
    SegmentLayoutView {
        segment_id: seg.id.clone(),
        source_id: seg.source_id.clone(),
        source_width: dims.0,
        source_height: dims.1,
        preset: seg.layout.preset.clone(),
        background: seg.layout.background,
        regions,
    }
}

fn next_cmds(clip: &Clip) -> Vec<String> {
    let mid = TimeMap::from_segments(&clip.segments).duration() / 2.0;
    vec![
        format!("rustler render preview-frame {} --out-t {mid:.1}", clip.id),
        format!("rustler layout show {}", clip.id),
    ]
}

/// View of the targeted segments; `touched` limits quality warnings to those region ids
/// (`None` = every region).
fn build_view(store: &ProjectStore, clip: &Clip, idxs: &[usize], touched: Option<&[String]>) -> Result<LayoutView> {
    let dims = source_dims(store, clip)?;
    let mut segments = Vec::new();
    let mut warnings = Vec::new();
    for &ix in idxs {
        let seg = &clip.segments[ix];
        let v = segment_view(seg, dims[&seg.source_id], &clip.canvas, touched);
        for r in &v.regions {
            for w in &r.warnings {
                warnings.push(if idxs.len() > 1 { format!("segment {}: {w}", seg.id) } else { w.clone() });
            }
        }
        segments.push(v);
    }
    Ok(LayoutView {
        clip_id: clip.id.clone(),
        revision: clip.revision,
        canvas: clip.canvas,
        segments,
        region_id: None,
        dry_run: None,
        warnings,
        next: next_cmds(clip),
    })
}

// ---------------------------------------------------------------------------------------------
// Generic revision-checked modification
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Outcome {
    region_id: Option<String>,
    /// Regions to quality-check (`None` = all of them).
    touched: Option<Vec<String>>,
    warnings: Vec<String>,
}

/// Loads the clip, applies `f` to the targeted segments, saves (unless dry-run) with
/// `base_revision` (default: the revision just loaded) and logs `kind`.
fn modify<F>(
    ctx: &Ctx,
    kind: &str,
    clip_id: &ClipId,
    segment_id: &Option<SegmentId>,
    base_revision: Option<u64>,
    f: F,
) -> Result<LayoutView>
where
    F: FnOnce(&ProjectStore, &mut Clip, &[usize]) -> Result<Outcome>,
{
    let store = ctx.store()?;
    let mut clip = store.load_clip(clip_id)?;
    let base = base_revision.unwrap_or(clip.revision);
    let idxs = target_idx(&clip, segment_id, kind)?;
    let out = f(&store, &mut clip, &idxs)?;
    if ctx.dry_run {
        let mut v = build_view(&store, &clip, &idxs, out.touched.as_deref())?;
        v.region_id = out.region_id;
        v.dry_run = Some(true);
        v.warnings.extend(out.warnings);
        return Ok(v);
    }
    store.save_clip(&mut clip, base)?;
    log_event(
        ctx,
        &store,
        kind,
        json!({"clip_id": clip.id, "revision": clip.revision, "region_id": out.region_id, "segments": idxs.len()}),
    )?;
    let mut v = build_view(&store, &clip, &idxs, out.touched.as_deref())?;
    v.region_id = out.region_id;
    v.warnings.extend(out.warnings);
    Ok(v)
}

fn region_ids(layout: &Layout) -> String {
    layout.regions.iter().map(|r| r.id.as_str()).collect::<Vec<_>>().join(", ")
}

fn no_such_region(stage: &str, clip: &Clip, idxs: &[usize], region_id: &str) -> RustlerError {
    let have = idxs.first().map(|&i| region_ids(&clip.segments[i].layout)).unwrap_or_default();
    RustlerError::new(ErrorCode::NotFound, stage, format!("no region '{region_id}' in the targeted segment(s) of clip {}", clip.id))
        .hint(format!("regions in the first targeted segment: {have}. See `rustler layout show {}`", clip.id))
}

fn warn_mixed_sources(dims: &HashMap<SourceId, (u32, u32)>, clip: &Clip, idxs: &[usize]) -> Vec<String> {
    let mut seen: Vec<(u32, u32)> = idxs.iter().map(|&i| dims[&clip.segments[i].source_id]).collect();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() > 1 {
        vec!["the targeted segments use sources with different frame sizes; normalized rects mean different pixels in each".to_string()]
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------------------------
// layout.show
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ShowIn {
    pub clip_id: ClipId,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
}

async fn layout_show(ctx: Ctx, i: ShowIn) -> Result<LayoutView> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let idxs = target_idx(&clip, &i.segment_id, "layout.show")?;
    let mut v = build_view(&store, &clip, &idxs, None)?;
    if let Some(seg) = idxs.first().map(|&ix| &clip.segments[ix]) {
        v.next.push(format!("rustler see frame {} --t {:.1}", seg.source_id, seg.start));
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// layout.add_region / edit_region / remove_region / reorder
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AddRegionIn {
    pub clip_id: ClipId,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// Source rect, normalized 0..1 of the source frame.
    pub src: Rect,
    /// Destination rect, normalized 0..1 of the canvas.
    pub dst: Rect,
    /// cover (default) fills the destination and crops; contain fits the whole source rect inside.
    pub fit: Option<Fit>,
    /// Draw order (default: in front of every existing region).
    pub z: Option<i32>,
    /// Region id (default: r1, r2, ...).
    pub id: Option<String>,
    pub base_revision: Option<u64>,
}

fn valid_region_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn layout_add_region(ctx: Ctx, i: AddRegionIn) -> Result<LayoutView> {
    let stage = "layout.add_region";
    layout::validate_rect("src", &i.src)?;
    layout::validate_rect("dst", &i.dst)?;
    if let Some(id) = &i.id
        && !valid_region_id(id)
    {
        return Err(validation(stage, format!("invalid region id '{id}'")).hint("use letters, digits, '-' or '_' (max 40 characters)"));
    }
    modify(&ctx, stage, &i.clip_id, &i.segment_id, i.base_revision, |_store, clip, idxs| {
        let id = match &i.id {
            Some(id) => {
                if idxs.iter().any(|&ix| clip.segments[ix].layout.regions.iter().any(|r| &r.id == id)) {
                    return Err(validation(stage, format!("a region '{id}' already exists")).hint("pick another id or use layout.edit_region"));
                }
                id.clone()
            }
            None => {
                let layouts: Vec<&Layout> = idxs.iter().map(|&ix| &clip.segments[ix].layout).collect();
                layout::next_region_id(&layouts)
            }
        };
        let z = i.z.unwrap_or_else(|| {
            idxs.iter().flat_map(|&ix| clip.segments[ix].layout.regions.iter().map(|r| r.z)).max().map_or(0, |m| m + 1)
        });
        for &ix in idxs {
            let l = &mut clip.segments[ix].layout;
            l.regions.push(Region { id: id.clone(), src: i.src, dst: i.dst, z, fit: i.fit.unwrap_or(Fit::Cover) });
            l.preset = None;
        }
        Ok(Outcome { region_id: Some(id.clone()), touched: Some(vec![id]), warnings: Vec::new() })
    })
}

/// Normalized deltas; missing fields are 0.
#[derive(Deserialize, JsonSchema, Clone, Copy, Default, Debug)]
#[serde(default)]
pub struct Nudge {
    pub dx: f64,
    pub dy: f64,
    pub dw: f64,
    pub dh: f64,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum NudgeTarget {
    /// Move/resize the selection inside the source frame (default).
    #[default]
    Src,
    /// Move/resize the placement on the canvas.
    Dst,
}

#[derive(Deserialize, JsonSchema)]
pub struct EditRegionIn {
    pub clip_id: ClipId,
    pub region_id: String,
    /// All segments that have the region when omitted.
    pub segment_id: Option<SegmentId>,
    pub src: Option<Rect>,
    pub dst: Option<Rect>,
    pub fit: Option<Fit>,
    pub z: Option<i32>,
    /// Relative adjustment applied after `src`/`dst` (normalized units).
    pub nudge: Option<Nudge>,
    /// Which rect `nudge` moves: src (default) or dst.
    pub nudge_target: Option<NudgeTarget>,
    pub base_revision: Option<u64>,
}

async fn layout_edit_region(ctx: Ctx, i: EditRegionIn) -> Result<LayoutView> {
    let stage = "layout.edit_region";
    if i.src.is_none() && i.dst.is_none() && i.fit.is_none() && i.z.is_none() && i.nudge.is_none() {
        return Err(validation(stage, "nothing to change").hint("pass at least one of src, dst, fit, z, nudge"));
    }
    modify(&ctx, stage, &i.clip_id, &i.segment_id, i.base_revision, |_store, clip, idxs| {
        let target = i.nudge_target.unwrap_or_default();
        let mut found = 0;
        for &ix in idxs {
            let Some(reg) = clip.segments[ix].layout.regions.iter_mut().find(|r| r.id == i.region_id) else { continue };
            let (mut src, mut dst) = (i.src.unwrap_or(reg.src), i.dst.unwrap_or(reg.dst));
            if let Some(n) = &i.nudge {
                match target {
                    NudgeTarget::Src => src = layout::nudge_rect(&src, n.dx, n.dy, n.dw, n.dh),
                    NudgeTarget::Dst => dst = layout::nudge_rect(&dst, n.dx, n.dy, n.dw, n.dh),
                }
            }
            if i.src.is_some() || (i.nudge.is_some() && target == NudgeTarget::Src) {
                layout::validate_rect("src", &src)?;
            }
            if i.dst.is_some() || (i.nudge.is_some() && target == NudgeTarget::Dst) {
                layout::validate_rect("dst", &dst)?;
            }
            reg.src = src;
            reg.dst = dst;
            if let Some(f) = i.fit {
                reg.fit = f;
            }
            if let Some(z) = i.z {
                reg.z = z;
            }
            found += 1;
        }
        if found == 0 {
            return Err(no_such_region(stage, clip, idxs, &i.region_id));
        }
        Ok(Outcome { region_id: Some(i.region_id.clone()), touched: Some(vec![i.region_id.clone()]), warnings: Vec::new() })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct RemoveRegionIn {
    pub clip_id: ClipId,
    pub region_id: String,
    /// All segments that have the region when omitted.
    pub segment_id: Option<SegmentId>,
    pub base_revision: Option<u64>,
}

async fn layout_remove_region(ctx: Ctx, i: RemoveRegionIn) -> Result<LayoutView> {
    let stage = "layout.remove_region";
    modify(&ctx, stage, &i.clip_id, &i.segment_id, i.base_revision, |_store, clip, idxs| {
        let mut found = 0;
        for &ix in idxs {
            let l = &mut clip.segments[ix].layout;
            let Some(pos) = l.regions.iter().position(|r| r.id == i.region_id) else { continue };
            if l.regions.len() == 1 {
                return Err(validation(stage, format!("'{}' is the only region of segment {}", i.region_id, clip.segments[ix].id))
                    .hint("a segment needs at least one region; replace it with layout.edit_region or clip.set_layout"));
            }
            let l = &mut clip.segments[ix].layout;
            l.regions.remove(pos);
            l.preset = None;
            found += 1;
        }
        if found == 0 {
            return Err(no_such_region(stage, clip, idxs, &i.region_id));
        }
        Ok(Outcome { region_id: Some(i.region_id.clone()), touched: Some(Vec::new()), warnings: Vec::new() })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct ReorderIn {
    pub clip_id: ClipId,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// Every region id of the segment, back to front (the last one is drawn on top).
    pub region_ids: Vec<String>,
    pub base_revision: Option<u64>,
}

async fn layout_reorder(ctx: Ctx, i: ReorderIn) -> Result<LayoutView> {
    let stage = "layout.reorder";
    modify(&ctx, stage, &i.clip_id, &i.segment_id, i.base_revision, |_store, clip, idxs| {
        let mut want = i.region_ids.clone();
        want.sort();
        for &ix in idxs {
            let l = &mut clip.segments[ix].layout;
            let mut have: Vec<String> = l.regions.iter().map(|r| r.id.clone()).collect();
            have.sort();
            if have != want {
                return Err(validation(
                    stage,
                    format!("region_ids must list exactly the regions of segment {} (have: {})", clip.segments[ix].id, have.join(", ")),
                )
                .hint("pass every region id once, back to front"));
            }
            let l = &mut clip.segments[ix].layout;
            for r in l.regions.iter_mut() {
                r.z = i.region_ids.iter().position(|id| id == &r.id).unwrap_or(0) as i32;
            }
        }
        Ok(Outcome { touched: Some(Vec::new()), ..Default::default() })
    })
}

// ---------------------------------------------------------------------------------------------
// layout.from_frame_boxes
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct FromBoxesIn {
    pub clip_id: ClipId,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// Either `top` + `bottom`, or `main`; plus an optional `inset` (picture in picture).
    pub boxes: Vec<FrameBox>,
    /// top + bottom: top pane height fraction (default: the top box's aspect, clamped 0.25..0.6).
    pub split: Option<f64>,
    /// black (default) or blur_fill (default for a contained main).
    pub background: Option<Background>,
    /// main only: cover (default) or contain.
    pub main_fit: Option<Fit>,
    pub base_revision: Option<u64>,
}

async fn layout_from_frame_boxes(ctx: Ctx, i: FromBoxesIn) -> Result<LayoutView> {
    let stage = "layout.from_frame_boxes";
    if i.boxes.is_empty() {
        return Err(validation(stage, "boxes is empty").hint("pass e.g. [{\"role\":\"top\",\"rect\":{...}},{\"role\":\"bottom\",\"rect\":{...}}]"));
    }
    modify(&ctx, stage, &i.clip_id, &i.segment_id, i.base_revision, |store, clip, idxs| {
        let dims = source_dims(store, clip)?;
        let canvas = clip.canvas;
        for &ix in idxs {
            let (w, h) = dims[&clip.segments[ix].source_id];
            clip.segments[ix].layout =
                layout::layout_from_boxes(&i.boxes, w, h, &canvas, i.split, i.background, i.main_fit.unwrap_or(Fit::Cover))?;
        }
        Ok(Outcome { warnings: warn_mixed_sources(&dims, clip, idxs), ..Default::default() })
    })
}

// ---------------------------------------------------------------------------------------------
// Custom presets: layout.save_preset / apply_preset / delete_preset / presets
// ---------------------------------------------------------------------------------------------

fn load_custom(store: &ProjectStore, name: &str, stage: &str) -> Result<LayoutPresetFile> {
    layout::validate_preset_name(name)?;
    let rel = layout::preset_rel(name);
    if !store.resolve(&rel).is_file() {
        let custom = list_custom(store).0.into_iter().map(|c| c.name).collect::<Vec<_>>();
        return Err(RustlerError::new(
            ErrorCode::NotFound,
            stage,
            format!("no layout preset '{name}' (built-in: {}; custom: {})", layout::PRESETS.join(", "), if custom.is_empty() { "none".to_string() } else { custom.join(", ") }),
        )
        .hint("list them with `rustler layout presets`; save one with `rustler layout save-preset`"));
    }
    store.read_json(&rel)
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct CustomPresetInfo {
    pub name: String,
    pub description: String,
    pub regions: usize,
    pub background: Background,
    pub path: String,
}

/// Custom presets in `<project>/layouts/`, plus a warning per unreadable file.
fn list_custom(store: &ProjectStore) -> (Vec<CustomPresetInfo>, Vec<String>) {
    let dir = store.root.join("layouts");
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return (out, warnings) };
    let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    for p in files {
        match store.read_json::<LayoutPresetFile>(&p) {
            Ok(f) => out.push(CustomPresetInfo {
                name: f.name,
                description: f.description,
                regions: f.regions.len(),
                background: f.background,
                path: p.display().to_string(),
            }),
            Err(e) => warnings.push(format!("{}: {}", p.display(), e.message)),
        }
    }
    (out, warnings)
}

#[derive(Deserialize, JsonSchema)]
pub struct SavePresetIn {
    /// 1-40 chars: lowercase letters, digits, '-' or '_'. Must not be a built-in preset name.
    pub name: String,
    pub from_clip_id: ClipId,
    /// Segment whose layout is saved (default: the first segment).
    pub segment_id: Option<SegmentId>,
    pub description: Option<String>,
    /// Replace an existing preset of the same name.
    pub overwrite: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct SavePresetOut {
    pub name: String,
    pub path: String,
    pub regions: usize,
    pub background: Background,
    pub overwritten: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

async fn layout_save_preset(ctx: Ctx, i: SavePresetIn) -> Result<SavePresetOut> {
    let stage = "layout.save_preset";
    layout::validate_preset_name(&i.name)?;
    let store = ctx.store()?;
    let clip = store.load_clip(&i.from_clip_id)?;
    let mut warnings = Vec::new();
    let seg = match &i.segment_id {
        Some(id) => clip.segments.iter().find(|s| &s.id == id).ok_or_else(|| {
            RustlerError::new(ErrorCode::NotFound, stage, format!("clip {} has no such segment", clip.id))
                .hint("list segment ids with `rustler clip show <clip_id>`")
        })?,
        None => {
            let first = clip.segments.first().ok_or_else(|| validation(stage, "the clip has no segments"))?;
            if clip.segments.iter().any(|s| s.layout.regions != first.layout.regions || s.layout.background != first.layout.background) {
                warnings.push(format!("the clip's segments have different layouts; saved the first one ({})", first.id));
            }
            first
        }
    };
    if seg.layout.regions.is_empty() {
        return Err(validation(stage, "the segment's layout has no regions"));
    }
    let rel = layout::preset_rel(&i.name);
    let existed = store.resolve(&rel).is_file();
    if existed && !i.overwrite.unwrap_or(false) {
        return Err(validation(stage, format!("a layout preset '{}' already exists", i.name))
            .hint("pass overwrite=true, or `rustler layout delete-preset` it first"));
    }
    let file = LayoutPresetFile {
        schema: "rustler.layout@1".to_string(),
        name: i.name.clone(),
        description: i.description.clone().unwrap_or_default(),
        regions: seg.layout.regions.clone(),
        background: seg.layout.background,
        created_at: Some(chrono::Utc::now()),
        from_clip_id: Some(clip.id.to_string()),
    };
    let path = store.resolve(&rel).display().to_string();
    let next = vec![format!("rustler layout apply-preset <clip_id> {}", i.name), "rustler layout presets".to_string()];
    let out = SavePresetOut {
        name: i.name.clone(),
        path,
        regions: file.regions.len(),
        background: file.background,
        overwritten: existed,
        dry_run: ctx.dry_run.then_some(true),
        warnings,
        next,
    };
    if ctx.dry_run {
        return Ok(out);
    }
    store.write_json(&rel, &file)?;
    log_event(&ctx, &store, stage, json!({"name": i.name, "from_clip_id": clip.id, "regions": file.regions.len()}))?;
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
pub struct ApplyPresetIn {
    pub clip_id: ClipId,
    /// A built-in preset (center-crop, blur-fit, split-vertical) or a custom one from `layout.presets`.
    pub name: String,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// Built-in presets only (e.g. top/bottom/split for split-vertical).
    pub params: Option<LayoutParams>,
    pub base_revision: Option<u64>,
}

async fn layout_apply_preset(ctx: Ctx, i: ApplyPresetIn) -> Result<LayoutView> {
    let stage = "layout.apply_preset";
    modify(&ctx, stage, &i.clip_id, &i.segment_id, i.base_revision, |store, clip, idxs| {
        let dims = source_dims(store, clip)?;
        let canvas = clip.canvas;
        if layout::is_builtin(&i.name) {
            let params = i.params.unwrap_or_default();
            for &ix in idxs {
                let (w, h) = dims[&clip.segments[ix].source_id];
                clip.segments[ix].layout = layout::preset(&i.name, w, h, &canvas, &params)?;
            }
        } else {
            let file = load_custom(store, &i.name, stage)?;
            if file.regions.is_empty() {
                return Err(validation(stage, format!("preset '{}' has no regions", i.name)));
            }
            for &ix in idxs {
                clip.segments[ix].layout = Layout { preset: None, regions: file.regions.clone(), background: file.background };
            }
        }
        Ok(Outcome { warnings: warn_mixed_sources(&dims, clip, idxs), ..Default::default() })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct DeletePresetIn {
    pub name: String,
}

#[derive(Serialize, JsonSchema)]
pub struct DeletePresetOut {
    pub name: String,
    pub deleted: bool,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn layout_delete_preset(ctx: Ctx, i: DeletePresetIn) -> Result<DeletePresetOut> {
    let stage = "layout.delete_preset";
    if layout::is_builtin(&i.name) {
        return Err(validation(stage, format!("'{}' is a built-in preset and cannot be deleted", i.name)));
    }
    let store = ctx.store()?;
    load_custom(&store, &i.name, stage)?;
    let path = store.resolve(&layout::preset_rel(&i.name));
    if ctx.dry_run {
        return Ok(DeletePresetOut { name: i.name, deleted: false, path: path.display().to_string(), dry_run: Some(true) });
    }
    std::fs::remove_file(&path)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, stage, format!("cannot delete {}: {e}", path.display())))?;
    log_event(&ctx, &store, stage, json!({"name": i.name}))?;
    Ok(DeletePresetOut { name: i.name, deleted: true, path: path.display().to_string(), dry_run: None })
}

#[derive(Serialize, JsonSchema)]
pub struct BuiltinPresetInfo {
    pub name: String,
    pub description: String,
    /// Fields of `params` the preset reads.
    pub params: Vec<String>,
    /// Params that must be given.
    pub required_params: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct PresetsOut {
    pub built_in: Vec<BuiltinPresetInfo>,
    pub custom: Vec<CustomPresetInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

async fn layout_presets(ctx: Ctx, _i: EmptyIn) -> Result<PresetsOut> {
    let built_in = layout::PRESETS
        .iter()
        .map(|n| {
            let info = layout_preset_info(n);
            BuiltinPresetInfo {
                name: info.name,
                description: info.description,
                params: info.params,
                required_params: if *n == "split-vertical" { vec!["top".into(), "bottom".into()] } else { Vec::new() },
            }
        })
        .collect();
    let (custom, warnings) = match ctx.store() {
        Ok(store) => list_custom(&store),
        Err(_) => (Vec::new(), Vec::new()),
    };
    Ok(PresetsOut {
        built_in,
        custom,
        warnings,
        next: vec![
            "rustler layout apply-preset <clip_id> <name>".to_string(),
            "rustler layout save-preset <name> <from_clip_id>".to_string(),
        ],
    })
}

// ---------------------------------------------------------------------------------------------
// layout.detect_webcam
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct DetectWebcamIn {
    pub source_id: SourceId,
    /// Source times (seconds) to sample; default: `samples` evenly spaced times.
    pub at: Option<Vec<f64>>,
    /// Number of evenly spaced sample times when `at` is omitted (default 6, max 20).
    pub samples: Option<usize>,
    /// Only used to fill the ready-to-run command text.
    pub clip_id: Option<ClipId>,
    pub segment_id: Option<SegmentId>,
}

#[derive(Serialize, JsonSchema)]
pub struct DetectWebcamOut {
    pub source_id: SourceId,
    pub source_width: u32,
    pub source_height: u32,
    /// Times that were sampled.
    pub times: Vec<f64>,
    /// Samples in which the webcam face was found.
    pub face_hits: usize,
    /// face_hits / samples sampled (0..1).
    pub confidence: f64,
    /// Median face box.
    pub face: Rect,
    /// Suggested webcam rect: snapped to the overlay's edges (`rect_source: "edges"`), or the face
    /// expanded 2.6x wide, 2.2x tall (`rect_source: "expansion"`).
    pub webcam: Rect,
    /// `edges` (borders found by `overlay.find_box`) or `expansion` (face-size guess).
    pub rect_source: String,
    /// Confidence of the edge snap (0..1; 0 when it was not found or the engine call failed).
    pub rect_confidence: f64,
    /// The face-size expansion for comparison (what `webcam` was before edge snapping).
    pub expansion_webcam: Rect,
    pub webcam_px: PxRect,
    /// e.g. top-right.
    pub position: String,
    /// Suggested gameplay rect: full height, bottom-pane width, clear of the webcam when possible.
    pub gameplay: Rect,
    pub gameplay_px: PxRect,
    pub gameplay_clear_of_webcam: bool,
    /// The whole frame (alternative for blur-fit style gameplay).
    pub gameplay_full_frame: Rect,
    /// Suggested top pane height fraction.
    pub split: f64,
    /// `params` for split-vertical.
    pub params: Value,
    /// Ready-to-run commands.
    pub commands: Vec<String>,
    /// Input for `clip.set_layout` (add `clip_id`).
    pub set_layout_input: Value,
    /// Input for `layout.from_frame_boxes` (add `clip_id`); same result with explicit boxes.
    pub from_frame_boxes_input: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

#[derive(Deserialize)]
struct FindBoxResult {
    rect: Option<Rect>,
    confidence: f64,
}

#[derive(Deserialize)]
struct FacesTrackResult {
    samples: Vec<rustler_media::facepath::FaceSample>,
}

fn sample_times(i: &DetectWebcamIn, duration: f64) -> Result<Vec<f64>> {
    let stage = "layout.detect_webcam";
    if let Some(at) = &i.at {
        if at.is_empty() || at.len() > 40 {
            return Err(validation(stage, "`at` must list 1..40 times"));
        }
        if let Some(t) = at.iter().find(|t| !t.is_finite() || **t < 0.0 || (duration > 0.0 && **t > duration)) {
            return Err(validation(stage, format!("sample time {t} is outside the source (0..{duration:.1}s)")));
        }
        return Ok(at.clone());
    }
    let n = i.samples.unwrap_or(6).clamp(1, 20);
    Ok((0..n).map(|k| duration * (k as f64 + 0.5) / n as f64).collect())
}

async fn layout_detect_webcam(ctx: Ctx, i: DetectWebcamIn) -> Result<DetectWebcamOut> {
    let stage = "layout.detect_webcam";
    let store = ctx.store()?;
    let source = store.load_source(&i.source_id)?;
    if !source.probe.has_video || source.probe.width == 0 {
        return Err(validation(stage, "the source has no video").hint("webcam detection needs a video source"));
    }
    let (sw, sh) = (source.probe.width, source.probe.height);
    let times = sample_times(&i, source.probe.duration)?;
    let video = source
        .proxy
        .as_ref()
        .map(|p| store.resolve(p))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| store.resolve(&source.media_path));
    let paths = rustler_tools::ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let mut per_sample: Vec<Vec<Rect>> = Vec::new();
    let mut failure = None;
    for &t in &times {
        // faces.track samples at start + k/fps; fps 1 and end = start + 1 gives exactly the frame at t.
        let start = if source.probe.duration > 1.1 { t.min(source.probe.duration - 1.05) } else { t };
        let end = (start + 1.0).min(source.probe.duration.max(start + 1.0));
        let params = json!({"video_path": video, "start": start, "end": end, "fps": 1.0});
        let res: Result<FacesTrackResult> = engine.call("faces.track", params, &ctx.progress, &ctx.cancel).await;
        match res {
            Ok(r) => per_sample.push(
                r.samples
                    .first()
                    .map(|s| s.faces.iter().map(|f| Rect { x: f.x, y: f.y, w: f.w, h: f.h }).collect())
                    .unwrap_or_default(),
            ),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failure {
        engine.shutdown().await;
        return Err(e);
    }

    let frames_with_faces = per_sample.iter().filter(|f| !f.is_empty()).count();
    let Some(found) = layout::find_webcam_face(&per_sample) else {
        engine.shutdown().await;
        let biggest = per_sample.iter().flatten().map(|f| f.w * f.h).fold(0.0, f64::max);
        return Err(validation(
            stage,
            format!(
                "no small, consistent face found: {frames_with_faces} of {} sampled frames had a face{}",
                per_sample.len(),
                if biggest >= layout::SMALL_FACE_AREA {
                    format!(" (the largest covers {:.0}% of the frame: a full-frame face cam, not an overlay)", biggest * 100.0)
                } else {
                    String::new()
                }
            ),
        )
        .hint(format!(
            "pick the webcam by hand: `rustler see frame {} --t {:.0}`, read the box, then use layout.from_frame_boxes (or clip set-layout --preset split-vertical --params)",
            i.source_id,
            times.first().copied().unwrap_or(0.0)
        )));
    };

    // Snap the webcam rect to the overlay's borders (edges across the sampled frames); keep the
    // face-size expansion when the edges are not convincing or the engine call fails.
    let expansion = layout::webcam_rect_from_face(&found.face);
    let mut warnings: Vec<String> = Vec::new();
    let snap: Result<FindBoxResult> = engine
        .call("overlay.find_box", json!({"video_path": video, "times": times, "near": found.face}), &ctx.progress, &ctx.cancel)
        .await;
    engine.shutdown().await;
    let (webcam, rect_source, rect_confidence) = match snap {
        Ok(FindBoxResult { rect: Some(r), confidence }) if layout::accept_snapped_rect(&found.face, &r, confidence) => (r, "edges", confidence),
        Ok(FindBoxResult { confidence, .. }) => {
            warnings.push(format!("no clear overlay border found (edge confidence {confidence:.2}): the webcam rect is the face expanded 2.6x2.2, which may include a sliver of gameplay; check with `see frame`"));
            (expansion, "expansion", confidence)
        }
        Err(e) => {
            warnings.push(format!("edge snapping failed ({e}): the webcam rect is the face expanded 2.6x2.2; check with `see frame`"));
            (expansion, "expansion", 0.0)
        }
    };

    let canvas = Canvas::default();
    let split = layout::split_for_webcam(&webcam, sw, sh, &canvas);
    let (gameplay, clear) = layout::gameplay_rect(&webcam, split, sw, sh, &canvas);
    let (webcam_r, gameplay_r) = (layout::round_rect(&webcam), layout::round_rect(&gameplay));
    let split_r = (split * 1000.0).round() / 1000.0;
    let params = json!({"top": webcam_r, "bottom": gameplay_r, "split": split_r});

    // Validate through the real preset builder and collect quality warnings from it.
    let lp = LayoutParams { top: Some(webcam_r), bottom: Some(gameplay_r), split: Some(split_r), ..Default::default() };
    let built = layout::preset("split-vertical", sw, sh, &canvas, &lp)?;
    warnings.extend(built.regions.iter().flat_map(|r| layout::region_warnings(r, sw, sh, &canvas)));
    let position = layout::frame_position(&webcam);
    if position == "center" {
        warnings.push("the face is in the middle of the frame: this may be a talking head rather than a webcam overlay; check with `see frame`".to_string());
    }
    if !clear {
        warnings.push("the gameplay rect still overlaps the webcam (not enough room beside it): the face may appear twice; check the preview".to_string());
    }
    if found.hits * 3 < found.samples * 2 {
        warnings.push(format!("the face was found in only {} of {} samples; confirm with `see frame`", found.hits, found.samples));
    }

    let clip_ref = i.clip_id.as_ref().map_or("<clip_id>".to_string(), |c| c.to_string());
    let seg_flag = i.segment_id.as_ref().map(|s| format!(" --segment-id {s}")).unwrap_or_default();
    let mut set_layout_input = json!({"preset": "split-vertical", "params": params});
    let mut boxes_input = json!({"boxes": [{"role": "top", "rect": webcam_r}, {"role": "bottom", "rect": gameplay_r}], "split": split_r});
    if let Some(c) = &i.clip_id {
        set_layout_input["clip_id"] = json!(c);
        boxes_input["clip_id"] = json!(c);
    }
    if let Some(s) = &i.segment_id {
        set_layout_input["segment_id"] = json!(s);
        boxes_input["segment_id"] = json!(s);
    }
    let commands = vec![format!("rustler clip set-layout {clip_ref}{seg_flag} --preset split-vertical --params '{params}'")];
    let next = vec![
        format!("rustler see frame {} --t {:.0}", i.source_id, times.first().copied().unwrap_or(0.0)),
        format!("rustler render preview-frame {clip_ref} --out-t 1.0"),
    ];
    Ok(DetectWebcamOut {
        source_id: i.source_id,
        source_width: sw,
        source_height: sh,
        times,
        face_hits: found.hits,
        confidence: found.hits as f64 / found.samples as f64,
        face: layout::round_rect(&found.face),
        webcam: webcam_r,
        rect_source: rect_source.to_string(),
        rect_confidence: (rect_confidence * 1000.0).round() / 1000.0,
        expansion_webcam: layout::round_rect(&expansion),
        webcam_px: layout::rect_px(&webcam_r, sw, sh),
        position,
        gameplay: gameplay_r,
        gameplay_px: layout::rect_px(&gameplay_r, sw, sh),
        gameplay_clear_of_webcam: clear,
        gameplay_full_frame: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
        split: split_r,
        params,
        commands,
        set_layout_input,
        from_frame_boxes_input: boxes_input,
        warnings,
        next,
    })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

const READ: Annotations = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false };
const WRITE: Annotations = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
const DESTROY: Annotations = Annotations { read_only: false, destructive: true, network: false, costs_money: false, long_running: false };
const DETECT: Annotations = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: true };

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "layout.show",
        "Show a clip's layout regions: normalized and pixel rects, fit, z order, preset, quality warnings",
        &["clip_id"],
        READ,
        vec![Example { description: "regions of every segment", cli: "rustler layout show clp_x" }],
        layout_show,
    ));
    reg.register(typed(
        "layout.add_region",
        "Add a region (source rect drawn into a canvas rect) to a segment's layout",
        &["clip_id"],
        WRITE,
        vec![Example {
            description: "webcam picture-in-picture in the top-right corner",
            cli: "rustler layout add-region clp_x --src '{\"x\":0.75,\"y\":0.02,\"w\":0.23,\"h\":0.3}' --dst '{\"x\":0.62,\"y\":0.04,\"w\":0.34,\"h\":0.19}'",
        }],
        layout_add_region,
    ));
    reg.register(typed(
        "layout.edit_region",
        "Change a region's src/dst rect, fit or z; or nudge it by relative deltas",
        &["clip_id", "region_id"],
        WRITE,
        vec![
            Example {
                description: "move the selection 3% left and 2% up inside the frame",
                cli: "rustler layout edit-region clp_x top --nudge '{\"dx\":-0.03,\"dy\":-0.02}'",
            },
            Example {
                description: "give the bottom pane a new source rect",
                cli: "rustler layout edit-region clp_x bottom --src '{\"x\":0.1,\"y\":0,\"w\":0.6,\"h\":1}'",
            },
        ],
        layout_edit_region,
    ));
    reg.register(typed(
        "layout.remove_region",
        "Remove a region from a segment's layout",
        &["clip_id", "region_id"],
        DESTROY,
        vec![Example { description: "drop the inset", cli: "rustler layout remove-region clp_x inset" }],
        layout_remove_region,
    ));
    reg.register(typed(
        "layout.reorder",
        "Set the draw order of the regions (back to front)",
        &["clip_id"],
        WRITE,
        vec![Example {
            description: "draw the inset above the main region",
            cli: "rustler layout reorder clp_x --region-ids '[\"main\",\"inset\"]'",
        }],
        layout_reorder,
    ));
    reg.register(typed(
        "layout.from_frame_boxes",
        "Build a split or inset layout from rects drawn on a source frame (roles top/bottom/main/inset)",
        &["clip_id"],
        WRITE,
        vec![Example {
            description: "streamer clip: webcam on top, gameplay below",
            cli: "rustler layout from-frame-boxes clp_x --boxes '[{\"role\":\"top\",\"rect\":{\"x\":0.75,\"y\":0,\"w\":0.25,\"h\":0.33}},{\"role\":\"bottom\",\"rect\":{\"x\":0.1,\"y\":0,\"w\":0.6,\"h\":1}}]'",
        }],
        layout_from_frame_boxes,
    ));
    reg.register(typed(
        "layout.save_preset",
        "Save a clip segment's layout as a reusable custom preset in <project>/layouts/",
        &["name", "from_clip_id"],
        WRITE,
        vec![Example { description: "save the streamer split", cli: "rustler layout save-preset streamer-split clp_x --description \"webcam top, game bottom\"" }],
        layout_save_preset,
    ));
    reg.register(typed(
        "layout.apply_preset",
        "Apply a built-in or custom layout preset to one segment or all segments",
        &["clip_id", "name"],
        WRITE,
        vec![Example { description: "apply a saved preset", cli: "rustler layout apply-preset clp_x streamer-split" }],
        layout_apply_preset,
    ));
    reg.register(typed(
        "layout.delete_preset",
        "Delete a custom layout preset",
        &["name"],
        DESTROY,
        vec![Example { description: "remove a saved preset", cli: "rustler layout delete-preset streamer-split" }],
        layout_delete_preset,
    ));
    reg.register(typed(
        "layout.presets",
        "List built-in layout presets (with descriptions and required params) and custom presets",
        &[],
        READ,
        vec![Example { description: "all presets", cli: "rustler layout presets" }],
        layout_presets,
    ));
    reg.register(typed(
        "layout.detect_webcam",
        "Find a streamer webcam overlay (small consistent face) in a source, snap its rectangle to the overlay borders and suggest split-vertical params",
        &["source_id"],
        DETECT,
        vec![Example { description: "sample 6 frames and suggest the split", cli: "rustler layout detect-webcam src_x --clip-id clp_x" }],
        layout_detect_webcam,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_ids_are_checked() {
        assert!(valid_region_id("cam_1"));
        assert!(!valid_region_id(""));
        assert!(!valid_region_id("a b"));
    }
}
