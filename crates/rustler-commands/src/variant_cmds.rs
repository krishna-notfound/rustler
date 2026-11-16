//! Commands: variant.create, variant.list, variant.compare, render.batch, export.batch (f22).
//!
//! *Variants* are clones of a clip that differ in caption style, layout preset and/or hook title,
//! so the best version can be picked by eye (`variant.compare` composes a labelled grid of preview
//! frames). A variant is a normal clip; `extras.json` carries `variant_of` (the base clip id) and
//! `variant_info` (the knobs). *Batch* commands render and export many clips with per-clip results
//! (a failing clip never stops the rest).
//!
//! The commands reuse `render.run`, `qc.run`, `export.clip`, `layout.apply_preset` and
//! `overlay.add_text` through a private registry, so the logic lives in one place.

use crate::brief_cmds::{self, RuleStatus};
use crate::clip_cmds::log_event;
use crate::render_cmds::preview_frame_image;
use crate::{extras, layout_cmds, overlay_cmds, qc_cmds, render_cmds};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use chrono::Utc;
use image::{Rgb, RgbImage, imageops};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, QcStatus, RenderId, RenderQuality, RustlerError, TimeMap};
use rustler_media::qc::{QcView, default_font};
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, ProgressSink, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

type Result<T> = rustler_core::Result<T>;

/// extras.json: base clip id of a variant.
pub const VARIANT_OF: &str = "variant_of";
/// extras.json: what differs in a variant (`caption_style`, `layout`, `hook`, `n`).
pub const VARIANT_INFO: &str = "variant_info";
/// Most variants one `variant.create` call makes.
const MAX_VARIANTS: usize = 24;
/// Clips in one `variant.compare` grid.
const MAX_COMPARE: usize = 12;
/// NVENC allows a couple of concurrent sessions on consumer GPUs.
const NVENC_PARALLEL: usize = 2;
const MAX_PARALLEL: usize = 4;

/// Default `render.batch` concurrency: 2 with a GPU encoder (NVENC session limit), else
/// `min(2, cores / 4)` so software x264 does not starve the machine (at least 1).
pub fn default_parallel(nvenc: bool, cores: usize) -> usize {
    if nvenc { NVENC_PARALLEL } else { (cores / 4).clamp(1, 2) }
}
/// Slack when comparing file times against the render time.
const FRESH_SLACK: Duration = Duration::from_secs(2);

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

/// A registry with the commands the batch code calls.
fn inner() -> Registry {
    let mut r = Registry::new();
    render_cmds::register(&mut r);
    qc_cmds::register(&mut r);
    overlay_cmds::register(&mut r);
    layout_cmds::register(&mut r);
    r
}

fn silent(ctx: &Ctx) -> Ctx {
    let mut c = ctx.clone();
    c.progress = ProgressSink::noop();
    c
}

fn clip_label(store: &ProjectStore, id: &ClipId) -> String {
    store.load_clip(id).map(|c| c.name).unwrap_or_else(|_| id.to_string())
}

// ---------------------------------------------------------------------------------------------
// Variants
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct VariantInfo {
    pub clip_id: ClipId,
    pub name: String,
    pub caption_style: String,
    /// Layout preset of the first segment (null for hand-edited regions).
    pub layout: Option<String>,
    pub hook: Option<String>,
    pub duration: f64,
    pub renders: usize,
}

fn info_of(store: &ProjectStore, clip: &Clip) -> VariantInfo {
    let x = extras::load(store, &clip.id).unwrap_or_default();
    VariantInfo {
        clip_id: clip.id.clone(),
        name: clip.name.clone(),
        caption_style: clip.captions.preset.clone(),
        layout: clip.segments.first().and_then(|s| s.layout.preset.clone()),
        hook: x.get(VARIANT_INFO).and_then(|i| i.get("hook")).and_then(Value::as_str).map(String::from),
        duration: TimeMap::from_segments(&clip.segments).duration(),
        renders: clip.renders.len(),
    }
}

/// The base clip a clip is a variant of (itself when it is not a variant).
fn root_of(store: &ProjectStore, id: &ClipId) -> Result<ClipId> {
    let x = extras::load(store, id)?;
    Ok(x.get(VARIANT_OF).and_then(Value::as_str).and_then(|s| s.parse().ok()).unwrap_or_else(|| id.clone()))
}

/// Variants of `root` in creation order.
fn variants_of(store: &ProjectStore, root: &ClipId) -> Result<Vec<Clip>> {
    let project = store.load_project()?;
    let mut out = Vec::new();
    for id in &project.clips {
        if id == root {
            continue;
        }
        let x = extras::load(store, id)?;
        if x.get(VARIANT_OF).and_then(Value::as_str) == Some(root.as_str()) {
            out.push(store.load_clip(id)?);
        }
    }
    out.sort_by_key(|c| c.created_at);
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
pub struct VariantCreateIn {
    pub clip_id: ClipId,
    /// Caption style presets to try (see caption.list_styles).
    pub caption_styles: Option<Vec<String>>,
    /// Layout presets to try: built-in or custom (see layout.presets).
    pub layouts: Option<Vec<String>>,
    /// Hook titles (hook-top overlays, <= 120 chars; <= 7 words reads best) to try.
    pub hooks: Option<Vec<String>>,
}

#[derive(Serialize, JsonSchema)]
pub struct VariantCreateOut {
    pub base: ClipId,
    pub variants: Vec<VariantInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn dedup(v: &Option<Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v.iter().flatten() {
        let s = s.trim().to_string();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

async fn variant_create(ctx: Ctx, i: VariantCreateIn) -> Result<VariantCreateOut> {
    const STAGE: &str = "variant.create";
    let store = ctx.store()?;
    let base = store.load_clip(&i.clip_id)?;
    let styles = dedup(&i.caption_styles);
    let layouts = dedup(&i.layouts);
    let hooks = dedup(&i.hooks);
    if styles.is_empty() && layouts.is_empty() && hooks.is_empty() {
        return Err(validation(STAGE, "nothing to vary: give caption_styles, layouts and/or hooks")
            .hint(format!("rustler variant create {} --caption-styles '[\"hormozi-pop\",\"clean-white\"]' --layouts '[\"center-crop\",\"blur-fit\"]'", base.id)));
    }
    let count = styles.len().max(1) * layouts.len().max(1) * hooks.len().max(1);
    if count > MAX_VARIANTS {
        return Err(validation(STAGE, format!("{count} combinations is more than the limit of {MAX_VARIANTS}")).hint("pick fewer styles/layouts/hooks"));
    }
    for s in &styles {
        crate::asset_cmds::check_caption_style(&ctx, STAGE, s)?;
    }
    for h in &hooks {
        overlay_cmds::check_text(STAGE, h)?;
    }
    let reg = inner();
    // Preflight every layout on the base clip without saving, so nothing is half-created.
    let mut dry = silent(&ctx);
    dry.dry_run = true;
    for l in &layouts {
        reg.dispatch("layout.apply_preset", json!({"clip_id": base.id, "name": l}), dry.clone()).await.map_err(|mut e| {
            e.message = format!("layout '{l}' cannot be applied to {}: {}", base.id, e.message);
            e
        })?;
    }

    let root = root_of(&store, &base.id)?;
    let root_name = if root == base.id { base.name.clone() } else { clip_label(&store, &root) };
    let mut n = variants_of(&store, &root)?.len();
    let mut warnings = Vec::new();
    if !styles.is_empty() && !base.captions.enabled {
        warnings.push("captions are off on the base clip; caption styles will not show until `caption set --enabled true`".to_string());
    }

    // Combination list: (style, layout, hook) with `None` = keep the base's.
    let opts = |v: &[String]| -> Vec<Option<String>> { if v.is_empty() { vec![None] } else { v.iter().cloned().map(Some).collect() } };
    let mut combos = Vec::new();
    for s in opts(&styles) {
        for l in opts(&layouts) {
            for h in opts(&hooks) {
                combos.push((s.clone(), l.clone(), h));
            }
        }
    }

    if ctx.dry_run {
        let variants = combos
            .iter()
            .enumerate()
            .map(|(k, (s, l, h))| VariantInfo {
                clip_id: ClipId::new(),
                name: format!("{root_name}-v{}", n + k + 1),
                caption_style: s.clone().unwrap_or_else(|| base.captions.preset.clone()),
                layout: l.clone().or_else(|| base.segments.first().and_then(|g| g.layout.preset.clone())),
                hook: h.clone(),
                duration: TimeMap::from_segments(&base.segments).duration(),
                renders: 0,
            })
            .collect();
        warnings.push("dry run: nothing was created (clip ids are placeholders)".to_string());
        return Ok(VariantCreateOut { base: base.id, variants, dry_run: Some(true), warnings, next: Vec::new() });
    }

    let base_extras = extras::load(&store, &base.id)?;
    let mut created: Vec<ClipId> = Vec::new();
    let mut infos = Vec::new();
    for (style, layout, hook) in combos {
        n += 1;
        let mut c = base.clone();
        c.id = ClipId::new();
        c.name = format!("{root_name}-v{n}");
        c.revision = 0;
        c.renders = Vec::new();
        c.created_at = Utc::now();
        if let Some(s) = &style {
            c.captions.preset = s.clone();
        }
        store.save_clip(&mut c, 0)?;
        created.push(c.id.clone());

        let mut x = base_extras.clone();
        x.insert(VARIANT_OF.into(), json!(root.as_str()));
        x.insert(
            VARIANT_INFO.into(),
            json!({"n": n, "caption_style": style, "layout": layout, "hook": hook, "from": base.id.as_str()}),
        );
        if hook.is_some() {
            // The new hook replaces the base's hook title.
            if let Some(Value::Array(list)) = x.get_mut("overlays") {
                list.retain(|o| o.get("style").and_then(Value::as_str) != Some("hook-top"));
                if list.is_empty() {
                    x.remove("overlays");
                }
            }
        }
        extras::save(&store, &c.id, &x)?;

        if let Some(l) = &layout {
            reg.dispatch("layout.apply_preset", json!({"clip_id": c.id, "name": l}), silent(&ctx)).await?;
        }
        if let Some(h) = &hook {
            reg.dispatch("overlay.add_text", json!({"clip_id": c.id, "text": h, "style": "hook-top"}), silent(&ctx)).await?;
        }
        let fresh = store.load_clip(&c.id)?;
        let mut info = info_of(&store, &fresh);
        info.hook = hook;
        infos.push(info);
    }

    let mut project = store.load_project()?;
    let rev = project.revision;
    project.clips.extend(created.iter().cloned());
    store.save_project(&mut project, rev)?;
    log_event(&ctx, &store, "variant.create", json!({"base": base.id, "variants": created}))?;

    let ids: Vec<String> = created.iter().map(|c| c.to_string()).collect();
    Ok(VariantCreateOut {
        base: base.id.clone(),
        variants: infos,
        dry_run: None,
        warnings,
        next: vec![
            format!("rustler variant compare --clip-ids '{}'", json!(ids)),
            format!("rustler render batch --clip-ids '{}' --quality draft", json!(ids)),
        ],
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct VariantListIn {
    /// The base clip or any of its variants.
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema)]
pub struct VariantListOut {
    pub base: ClipId,
    pub variants: Vec<VariantInfo>,
}

async fn variant_list(ctx: Ctx, i: VariantListIn) -> Result<VariantListOut> {
    let store = ctx.store()?;
    store.load_clip(&i.clip_id)?;
    let root = root_of(&store, &i.clip_id)?;
    let variants = variants_of(&store, &root)?.iter().map(|c| info_of(&store, c)).collect();
    Ok(VariantListOut { base: root, variants })
}

// ---------------------------------------------------------------------------------------------
// variant.compare
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct VariantCompareIn {
    /// 2..=12 clips (e.g. the base and its variants).
    pub clip_ids: Vec<ClipId>,
    /// Output time of the frame every clip is shown at (default 1.0 s; clamped into each clip).
    pub out_t: Option<f64>,
    /// Width of one tile in pixels (default 360).
    pub tile_width: Option<u32>,
    /// Tiles per row (default 3).
    pub cols: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
pub struct CompareItem {
    pub clip_id: ClipId,
    pub label: String,
    pub frame: PathBuf,
}

#[derive(Serialize, JsonSchema)]
pub struct VariantCompareOut {
    /// Absolute path of the labelled grid jpg; open it and pick.
    pub image: PathBuf,
    pub out_t: f64,
    pub items: Vec<CompareItem>,
    pub next: Vec<String>,
}

/// Truncates `text` with an ellipsis so it fits `max_w` pixels at `scale`.
fn fit_text(font: &FontArc, scale: PxScale, text: &str, max_w: f32) -> String {
    let sf = font.as_scaled(scale);
    let width = |s: &str| s.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum::<f32>();
    if width(text) <= max_w {
        return text.to_string();
    }
    let mut out: String = text.to_string();
    while !out.is_empty() && width(&format!("{out}…")) > max_w {
        out.pop();
    }
    format!("{out}…")
}

async fn variant_compare(ctx: Ctx, i: VariantCompareIn) -> Result<VariantCompareOut> {
    const STAGE: &str = "variant.compare";
    let store = ctx.store()?;
    if i.clip_ids.len() < 2 || i.clip_ids.len() > MAX_COMPARE {
        return Err(validation(STAGE, format!("give 2..={MAX_COMPARE} clip ids, got {}", i.clip_ids.len())));
    }
    let out_t = i.out_t.unwrap_or(1.0);
    let tile_w = i.tile_width.unwrap_or(360).clamp(120, 1080);
    let cols = i.cols.unwrap_or(3).clamp(1, 6).min(i.clip_ids.len() as u32);
    let paths = ToolPaths::from_home(&ctx.home);
    let font_path = default_font(&paths).ok_or_else(|| {
        RustlerError::new(ErrorCode::DependencyMissing, STAGE, "no font available for the labels").hint("run `rustler setup` (installs fonts)")
    })?;
    let font = FontArc::try_from_vec(std::fs::read(&font_path).map_err(|e| {
        RustlerError::new(ErrorCode::NotFound, STAGE, format!("font not readable: {}: {e}", font_path.display()))
    })?)
    .map_err(|e| validation(STAGE, format!("invalid font file: {e}")))?;

    let mut tiles: Vec<(RgbImage, String, String)> = Vec::new();
    let mut items = Vec::new();
    for (k, id) in i.clip_ids.iter().enumerate() {
        ctx.progress.emit(ProgressEvent {
            stage: "variant.compare".into(),
            pct: Some(k as f32 / i.clip_ids.len() as f32),
            message: Some(id.to_string()),
            eta_s: None,
        });
        let clip = store.load_clip(id)?;
        let frame = preview_frame_image(&ctx, id, out_t).await?;
        let img = image::open(&frame)
            .map_err(|e| RustlerError::new(ErrorCode::Media, STAGE, format!("bad preview frame {}: {e}", frame.display())))?
            .to_rgb8();
        let h = (img.height() as f64 * tile_w as f64 / img.width().max(1) as f64).round().max(1.0) as u32;
        let tile = imageops::resize(&img, tile_w, h, imageops::FilterType::Triangle);
        let info = info_of(&store, &clip);
        let line2 = format!("{} · {}{}", info.caption_style, info.layout.as_deref().unwrap_or("custom"), info.hook.as_deref().map(|h| format!(" · \"{h}\"")).unwrap_or_default());
        let label = format!("{} | {line2}", clip.name);
        tiles.push((tile, clip.name.clone(), line2));
        items.push(CompareItem { clip_id: id.clone(), label, frame });
    }

    const STRIP: u32 = 56;
    let tile_h = tiles.iter().map(|t| t.0.height()).max().unwrap_or(0);
    let cell_h = tile_h + STRIP;
    let rows = (tiles.len() as u32).div_ceil(cols);
    let mut sheet = RgbImage::from_pixel(cols * tile_w, rows * cell_h, Rgb([16, 16, 16]));
    let scale = PxScale::from((tile_w as f32 / 18.0).clamp(12.0, 22.0));
    for (k, (tile, name, line2)) in tiles.iter().enumerate() {
        let (col, row) = (k as u32 % cols, k as u32 / cols);
        let (x0, y0) = (col * tile_w, row * cell_h);
        imageops::replace(&mut sheet, tile, i64::from(x0), i64::from(y0));
        draw_filled_rect_mut(&mut sheet, Rect::at(x0 as i32, (y0 + tile_h) as i32).of_size(tile_w, STRIP), Rgb([0, 0, 0]));
        let max_w = tile_w as f32 - 12.0;
        let line1 = fit_text(&font, scale, &format!("{}. {name}", k + 1), max_w);
        draw_text_mut(&mut sheet, Rgb([255, 255, 255]), x0 as i32 + 6, (y0 + tile_h) as i32 + 6, scale, &font, &line1);
        let line2 = fit_text(&font, scale, line2, max_w);
        draw_text_mut(&mut sheet, Rgb([170, 200, 255]), x0 as i32 + 6, (y0 + tile_h) as i32 + 6 + scale.y as i32 + 4, scale, &font, &line2);
    }
    let dir = std::path::absolute(&store.root).unwrap_or_else(|_| store.root.clone()).join("variants");
    std::fs::create_dir_all(&dir).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("cannot create {}: {e}", dir.display())))?;
    let image = dir.join(format!("compare_{}.jpg", Utc::now().format("%Y%m%d_%H%M%S_%3f")));
    sheet
        .save(&image)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("cannot write {}: {e}", image.display())))?;
    Ok(VariantCompareOut {
        image,
        out_t,
        items,
        next: vec!["open the image, pick the best variant, then `rustler render run <clip_id>` and `rustler qc run <clip_id>`".to_string()],
    })
}

// ---------------------------------------------------------------------------------------------
// Batch helpers
// ---------------------------------------------------------------------------------------------

fn resolve_ids(store: &ProjectStore, stage: &str, ids: &Option<Vec<ClipId>>, all: bool) -> Result<Vec<ClipId>> {
    let mut out: Vec<ClipId> = Vec::new();
    if all {
        out = store.load_project()?.clips;
    } else if let Some(v) = ids {
        for id in v {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
    }
    if out.is_empty() {
        return Err(validation(stage, if all { "the project has no clips" } else { "give clip_ids or all: true" })
            .hint("`rustler clip list` shows the clips"));
    }
    Ok(out)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// True when the clip's latest render (of `quality` when given) exists on disk and neither the clip
/// definition nor its extras changed after the render (or its QC, which re-saves the clip).
pub fn render_is_fresh(store: &ProjectStore, clip: &Clip, quality: Option<RenderQuality>) -> bool {
    let Some(r) = clip.renders.iter().rev().find(|r| quality.is_none_or(|q| q == r.quality)) else { return false };
    if !store.resolve(&r.path).is_file() {
        return false;
    }
    let mut reference = SystemTime::from(r.created_at);
    if let Some(q) = r.qc.as_ref().and_then(|q| mtime(&store.resolve(q))) {
        reference = reference.max(q);
    }
    let dir = store.clip_dir(&clip.id);
    ["clip.json", "extras.json"]
        .iter()
        .all(|f| mtime(&dir.join(f)).is_none_or(|t| t <= reference + FRESH_SLACK))
}

// ---------------------------------------------------------------------------------------------
// render.batch
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct RenderBatchIn {
    /// Clips to render (or `all: true`).
    pub clip_ids: Option<Vec<ClipId>>,
    /// Render every clip of the project.
    #[serde(default)]
    pub all: bool,
    /// draft | final (default final)
    pub quality: Option<RenderQuality>,
    /// Renders at once (default 2 with NVENC, else min(2, cores/4); capped at 2 with NVENC, else at 4).
    pub parallel: Option<u32>,
    /// Skip clips that already have an up-to-date render of this quality (default false).
    pub only_missing: Option<bool>,
    /// Use a hardware encoder when one passes a probe (default true).
    pub prefer_gpu: Option<bool>,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct BatchRender {
    pub clip_id: ClipId,
    pub name: String,
    /// rendered | skipped | failed | cancelled
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_id: Option<RenderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RustlerError>,
}

#[derive(Serialize, JsonSchema)]
pub struct RenderBatchOut {
    pub requested: usize,
    pub rendered: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Renders that ran at once.
    pub parallel: usize,
    pub encoder: String,
    pub results: Vec<BatchRender>,
    pub next: Vec<String>,
}

async fn render_batch(ctx: Ctx, i: RenderBatchIn) -> Result<RenderBatchOut> {
    const STAGE: &str = "render.batch";
    let store = ctx.store()?;
    let ids = resolve_ids(&store, STAGE, &i.clip_ids, i.all)?;
    let quality = i.quality.unwrap_or(RenderQuality::Final);
    let prefer_gpu = i.prefer_gpu.unwrap_or(true);
    let paths = ToolPaths::from_home(&ctx.home);
    let encoder = rustler_media::pick_encoder(&paths, quality, prefer_gpu).await.name;
    let cap = if encoder.contains("nvenc") { NVENC_PARALLEL } else { MAX_PARALLEL };
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let default = default_parallel(encoder.contains("nvenc"), cores);
    let parallel = (i.parallel.map(|p| p as usize).unwrap_or(default)).clamp(1, cap);
    let only_missing = i.only_missing.unwrap_or(false);

    let total = ids.len();
    let reg = Arc::new(inner());
    let sem = Arc::new(Semaphore::new(parallel));
    let mut set: JoinSet<(usize, BatchRender)> = JoinSet::new();
    for (idx, id) in ids.iter().cloned().enumerate() {
        let (reg, sem, ctx, store) = (reg.clone(), sem.clone(), ctx.clone(), store.clone());
        set.spawn(async move {
            let name = clip_label(&store, &id);
            let mut r = BatchRender {
                clip_id: id.clone(),
                name,
                status: "failed".into(),
                render_id: None,
                path: None,
                encoder: None,
                duration: None,
                reason: None,
                error: None,
            };
            let Ok(_permit) = sem.acquire().await else { return (idx, r) };
            if ctx.cancel.is_cancelled() {
                r.status = "cancelled".into();
                return (idx, r);
            }
            if only_missing {
                if let Ok(clip) = store.load_clip(&id) {
                    if render_is_fresh(&store, &clip, Some(quality)) {
                        r.status = "skipped".into();
                        r.reason = Some("already has an up-to-date render of this quality".into());
                        return (idx, r);
                    }
                }
            }
            let input = json!({"clip_id": id, "quality": quality, "prefer_gpu": prefer_gpu});
            match reg.dispatch("render.run", input, silent(&ctx)).await {
                Ok(v) => {
                    r.status = "rendered".into();
                    r.render_id = v.get("render_id").and_then(Value::as_str).and_then(|s| s.parse().ok());
                    r.path = v.get("path").and_then(Value::as_str).map(PathBuf::from);
                    r.encoder = v.get("encoder").and_then(Value::as_str).map(String::from);
                    r.duration = v.get("duration").and_then(Value::as_f64);
                }
                Err(e) => {
                    r.status = if e.code == ErrorCode::Cancelled { "cancelled" } else { "failed" }.into();
                    r.error = Some(e);
                }
            }
            (idx, r)
        });
    }
    let mut results: Vec<Option<BatchRender>> = vec![None; total];
    let mut done = 0usize;
    while let Some(joined) = set.join_next().await {
        done += 1;
        match joined {
            Ok((idx, r)) => {
                ctx.progress.emit(ProgressEvent {
                    stage: "render.batch".into(),
                    pct: Some(done as f32 / total as f32),
                    message: Some(format!("{} ({done}/{total}): {}", r.name, r.status)),
                    eta_s: None,
                });
                results[idx] = Some(r);
            }
            Err(e) => {
                ctx.progress.emit(ProgressEvent { stage: "render.batch".into(), pct: None, message: Some(format!("a render task crashed: {e}")), eta_s: None });
            }
        }
    }
    let results: Vec<BatchRender> = results
        .into_iter()
        .zip(&ids)
        .map(|(r, id)| {
            r.unwrap_or_else(|| BatchRender {
                clip_id: id.clone(),
                name: id.to_string(),
                status: "failed".into(),
                render_id: None,
                path: None,
                encoder: None,
                duration: None,
                reason: Some("the render task crashed".into()),
                error: None,
            })
        })
        .collect();
    let count = |s: &str| results.iter().filter(|r| r.status == s).count();
    let (rendered, skipped) = (count("rendered"), count("skipped"));
    let failed = results.len() - rendered - skipped;
    if !ctx.dry_run && rendered > 0 {
        log_event(&ctx, &store, "render.batch", json!({"rendered": rendered, "failed": failed, "quality": quality}))?;
    }
    let mut next = Vec::new();
    if rendered > 0 {
        next.push("rustler export batch --all true   # runs QC and the brief check, exports the passing clips".to_string());
    }
    for r in results.iter().filter(|r| r.status == "failed") {
        next.push(format!("rustler render run {}   # failed: {}", r.clip_id, r.error.as_ref().map_or("", |e| e.message.as_str())));
    }
    Ok(RenderBatchOut { requested: total, rendered, skipped, failed, parallel, encoder, results, next })
}

// ---------------------------------------------------------------------------------------------
// export.batch
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ExportBatchIn {
    pub clip_ids: Option<Vec<ClipId>>,
    #[serde(default)]
    pub all: bool,
    /// Run QC where missing and skip clips whose QC fails (default true).
    pub require_qc: Option<bool>,
    /// Run the brief check and skip clips that fail it, when the project has a brief (default true).
    pub require_brief: Option<bool>,
    /// Export even when the clip was edited after its last render (default false).
    pub allow_stale: Option<bool>,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Exported {
    pub clip_id: ClipId,
    pub name: String,
    pub video: PathBuf,
    pub report: PathBuf,
    pub qc_passed: bool,
    /// pass | warn | fail | null when no brief
    pub brief: Option<RuleStatus>,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SkippedClip {
    pub clip_id: ClipId,
    pub name: String,
    pub reasons: Vec<String>,
    /// Commands that fix the reasons.
    pub fix: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ExportBatchOut {
    pub exported: Vec<Exported>,
    pub skipped: Vec<SkippedClip>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn qc_failures(q: &QcView) -> (Vec<String>, Vec<String>) {
    let bad: Vec<_> = q.report.checks.iter().filter(|c| c.status == QcStatus::Fail).collect();
    (
        bad.iter().map(|c| format!("QC check '{}' failed", c.name)).collect(),
        bad.iter().filter_map(|c| c.fix_hint.as_ref().map(|h| format!("{}: {h}", c.name))).collect(),
    )
}

async fn export_batch(ctx: Ctx, i: ExportBatchIn) -> Result<ExportBatchOut> {
    const STAGE: &str = "export.batch";
    let store = ctx.store()?;
    let ids = resolve_ids(&store, STAGE, &i.clip_ids, i.all)?;
    let require_qc = i.require_qc.unwrap_or(true);
    let want_brief = i.require_brief.unwrap_or(true);
    let allow_stale = i.allow_stale.unwrap_or(false);
    let brief = if want_brief { brief_cmds::load_brief(&store)? } else { None };
    let mut warnings = Vec::new();
    if want_brief && brief.is_none() {
        warnings.push("the project has no brief; only QC was required (set one with `rustler brief set`)".to_string());
    }
    let reg = inner();
    let mut exported = Vec::new();
    let mut skipped = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        ctx.progress.emit(ProgressEvent { stage: "export.batch".into(), pct: Some(k as f32 / ids.len() as f32), message: Some(id.to_string()), eta_s: None });
        let mut clip = match store.load_clip(id) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(SkippedClip { clip_id: id.clone(), name: id.to_string(), reasons: vec![e.message], fix: vec!["rustler clip list".into()] });
                continue;
            }
        };
        let mut reasons: Vec<String> = Vec::new();
        let mut fix: Vec<String> = Vec::new();
        if clip.renders.is_empty() {
            skipped.push(SkippedClip {
                clip_id: id.clone(),
                name: clip.name.clone(),
                reasons: vec!["no render yet".into()],
                fix: vec![format!("rustler render run {id}")],
            });
            continue;
        }
        if !allow_stale && !render_is_fresh(&store, &clip, None) {
            reasons.push("the clip was edited after its last render (or the render file is missing)".into());
            fix.push(format!("rustler render run {id}"));
        }
        // QC (runs one when the latest render has none).
        let mut qc_passed = true;
        if require_qc && reasons.is_empty() {
            if clip.renders.last().is_some_and(|r| r.qc.is_none()) {
                if let Err(e) = reg.dispatch("qc.run", json!({"clip_id": id, "allow_fail": true}), silent(&ctx)).await {
                    reasons.push(format!("QC could not run: {}", e.message));
                    fix.extend(e.hint);
                }
                if let Ok(c) = store.load_clip(id) {
                    clip = c;
                }
            }
            if let Some(q) = clip.renders.last().and_then(|r| r.qc.as_ref()).and_then(|p| store.read_json::<QcView>(p).ok()) {
                qc_passed = q.report.passed;
                if !q.report.passed {
                    let (r, f) = qc_failures(&q);
                    reasons.extend(r);
                    fix.extend(f);
                }
            }
        }
        // Brief.
        let mut brief_status = None;
        if brief.is_some() {
            {
                match brief_cmds::run_and_save(&store, id, !ctx.dry_run) {
                    Ok(Some(c)) => {
                        brief_status = Some(c.status);
                        if c.status == RuleStatus::Fail {
                            for r in c.rules.iter().filter(|r| r.status == RuleStatus::Fail) {
                                reasons.push(format!("brief: {}", r.message));
                                fix.extend(r.fix.iter().cloned());
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => reasons.push(format!("brief check failed to run: {}", e.message)),
                }
            }
        }
        if !reasons.is_empty() {
            skipped.push(SkippedClip { clip_id: id.clone(), name: clip.name.clone(), reasons, fix });
            continue;
        }
        let input = json!({"clip_id": id, "allow_fail": !require_qc});
        match reg.dispatch("export.clip", input, silent(&ctx)).await {
            Ok(v) => exported.push(Exported {
                clip_id: id.clone(),
                name: clip.name.clone(),
                video: v.get("video").and_then(Value::as_str).map(PathBuf::from).unwrap_or_default(),
                report: v.get("report").and_then(Value::as_str).map(PathBuf::from).unwrap_or_default(),
                qc_passed: v.get("qc_passed").and_then(Value::as_bool).unwrap_or(qc_passed),
                brief: brief_status,
            }),
            Err(e) => skipped.push(SkippedClip {
                clip_id: id.clone(),
                name: clip.name.clone(),
                reasons: vec![format!("export failed: {}", e.message)],
                fix: e.hint.into_iter().collect(),
            }),
        }
    }
    if !ctx.dry_run {
        log_event(&ctx, &store, "export.batch", json!({"exported": exported.len(), "skipped": skipped.len()}))?;
    }
    let mut next = Vec::new();
    if !skipped.is_empty() {
        next.push("fix the skipped clips (see `fix`), re-render, then run `rustler export batch` again".to_string());
    }
    if !exported.is_empty() {
        next.push("publishing is manual: upload the exported files with each clip's `rustler brief description <clip_id>` text".to_string());
    }
    Ok(ExportBatchOut { exported, skipped, warnings, next })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let read = Annotations { read_only: true, ..Annotations::default() };
    let long = Annotations { long_running: true, ..Annotations::default() };
    reg.register(typed(
        "variant.create",
        "Clone a clip once per combination of caption style x layout x hook title (names <name>-v<n>, extras copied) to compare versions",
        &["clip_id"],
        Annotations::default(),
        vec![Example {
            description: "2 styles x 2 layouts = 4 variants",
            cli: "rustler variant create clp_X --caption-styles '[\"hormozi-pop\",\"clean-white\"]' --layouts '[\"center-crop\",\"blur-fit\"]'",
        }],
        variant_create,
    ));
    reg.register(typed(
        "variant.list",
        "List the variants of a base clip (accepts the base or any variant)",
        &["clip_id"],
        read,
        vec![Example { description: "variants", cli: "rustler variant list clp_X" }],
        variant_list,
    ));
    reg.register(typed(
        "variant.compare",
        "Preview frames of several clips at the same output time composed into one labelled grid image; open it to pick the best",
        &[],
        long,
        vec![Example { description: "compare at 1.5 s", cli: "rustler variant compare --clip-ids '[\"clp_a\",\"clp_b\"]' --out-t 1.5" }],
        variant_compare,
    ));
    reg.register(typed(
        "render.batch",
        "Render many clips (clip_ids or all) with bounded concurrency (1 by default, max 2 on NVENC); a failing clip never stops the rest",
        &[],
        long,
        vec![
            Example { description: "render all clips as drafts", cli: "rustler render batch --all true --quality draft" },
            Example { description: "two at a time, skipping ones already rendered", cli: "rustler render batch --all true --parallel 2 --only-missing true" },
        ],
        render_batch,
    ));
    reg.register(typed(
        "export.batch",
        "Export many clips: runs QC and the brief check, exports the passing ones and lists the skipped with reasons and fixes",
        &[],
        long,
        vec![Example { description: "export everything that passes", cli: "rustler export batch --all true" }],
        export_batch,
    ));
}
