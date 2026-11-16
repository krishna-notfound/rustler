//! Commands: grade.set, grade.show, grade.presets, grade.clear, grade.preview, grade.measure (f38).
//!
//! A colour grade lives in `clips/<id>/extras.json` under `grade`: a clip-wide [`Grade`] plus
//! per-segment overrides merged onto it ([`GradeDoc`]). Rustler never grades on its own: these
//! commands only store what the agent asks for, and `grade.measure` only reads frames and
//! suggests commands. Every change is a new clip revision with an extras snapshot, so
//! `history undo` brings the previous grade back. `render.run` compiles the grade per segment
//! onto the source picture, before layout scaling and before captions / text / images
//! ([`rustler_media::grade`]).

use crate::clip_cmds::log_event;
use crate::extras;
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, RustlerError, SegmentId, SourceId, TimeMap};
use rustler_media::compile::CompileExtras;
use rustler_media::grade::{self as mg, FrameStats, Grade, GradeDoc, Rgb};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// extras key of the grade.
pub const GRADE_KEY: &str = "grade";

fn verr(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn presets_hint() -> String {
    format!("presets: {} (`rustler grade presets`)", mg::preset_names().join(", "))
}

/// The stored grade of a clip (empty when none).
pub fn load_doc(store: &ProjectStore, clip: &ClipId) -> Result<GradeDoc> {
    Ok(extras::get(store, clip, GRADE_KEY)?.unwrap_or_default())
}

// ---------------------------------------------------------------------------------------------
// LUTs
// ---------------------------------------------------------------------------------------------

/// `asset:<id>`, a bare lut asset id or a `.cube` path -> the file (checked: exists, parses).
pub fn resolve_lut(project: Option<&Path>, home: &Path, s: &str) -> Result<PathBuf> {
    const STAGE: &str = "grade.lut";
    let s = s.trim();
    let path = if s.starts_with("asset:") {
        crate::asset_cmds::resolve_asset_path_in(project, home, s)?
    } else {
        let p = PathBuf::from(s);
        if p.is_file() {
            std::path::absolute(&p).unwrap_or(p)
        } else {
            crate::asset_cmds::resolve_asset_path_in(project, home, &format!("asset:{s}")).map_err(|_| {
                RustlerError::new(ErrorCode::NotFound, STAGE, format!("LUT '{s}' is neither a file nor an asset id"))
                    .hint("add it with `rustler asset add <file.cube> --kind lut`, then pass --lut asset:<id>")
            })?
        }
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| RustlerError::new(ErrorCode::NotFound, STAGE, format!("cannot read LUT {}: {e}", path.display())))?;
    mg::check_cube(&text).map_err(|m| verr(STAGE, format!("{}: {m}", path.display())).hint("only 3D .cube LUTs (LUT_3D_SIZE) are supported"))?;
    Ok(path)
}

/// The form a LUT reference is stored in: `asset:<id>` stays, a path becomes absolute.
fn stored_lut_ref(project: Option<&Path>, home: &Path, s: &str) -> Result<String> {
    let s = s.trim();
    let p = resolve_lut(project, home, s)?;
    if s.starts_with("asset:") {
        return Ok(s.to_string());
    }
    if !PathBuf::from(s).is_file() {
        return Ok(format!("asset:{s}"));
    }
    Ok(p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------------------------
// Render / effective view
// ---------------------------------------------------------------------------------------------

/// Per-segment grades into the compile extras (LUTs resolved to files). Warnings for overrides
/// whose segment no longer exists.
pub(crate) fn apply_to_compile(
    store: &ProjectStore,
    home: &Path,
    clip: &Clip,
    map: &Map<String, Value>,
    cx: &mut CompileExtras,
) -> Result<Vec<String>> {
    let Some(doc) = extras::from_map::<GradeDoc>(map, GRADE_KEY)? else { return Ok(Vec::new()) };
    let (grades, warnings) = resolve_segments(store, home, clip, &doc)?;
    for (id, g) in grades {
        cx.grades.insert(id, g);
    }
    Ok(warnings)
}

/// Merged grade per segment with LUT paths resolved (errors name the segment).
fn resolve_segments(store: &ProjectStore, home: &Path, clip: &Clip, doc: &GradeDoc) -> Result<(Vec<(SegmentId, Grade)>, Vec<String>)> {
    let mut out = Vec::new();
    for seg in &clip.segments {
        let Some(mut g) = doc.for_segment(seg.id.as_str()) else { continue };
        g.validate().map_err(|m| verr("grade", format!("grade of segment {}: {m}", seg.id)).hint(format!("fix it with `rustler grade set {} ...` or `rustler grade clear {}`", clip.id, clip.id)))?;
        if let Some(l) = g.lut_ref().map(str::to_string) {
            let p = resolve_lut(Some(&store.root), home, &l).map_err(|mut e| {
                e.message = format!("grade of segment {}: {}", seg.id, e.message);
                e.hint = Some(format!("re-add the LUT or drop it: `rustler grade set {} --lut none`", clip.id));
                e
            })?;
            g.lut = Some(p.to_string_lossy().into_owned());
        } else {
            g.lut = None;
        }
        out.push((seg.id.clone(), g));
    }
    let warnings = doc
        .segments
        .keys()
        .filter(|k| !clip.segments.iter().any(|s| s.id.as_str() == k.as_str()))
        .map(|k| format!("segment {k} has a grade override but is no longer in the clip; it is ignored (`rustler grade clear {} --segment-id {k}`)", clip.id))
        .collect();
    Ok((out, warnings))
}

/// What one segment renders with.
#[derive(Serialize, JsonSchema, Debug)]
pub struct SegGrade {
    pub segment_id: SegmentId,
    pub source_id: SourceId,
    /// Edit-timeline window of the segment.
    pub out_start: f64,
    pub out_end: f64,
    /// True when the segment has its own override.
    pub has_override: bool,
    /// The exact ffmpeg filter chain applied to this segment's source picture; null = no grade.
    pub filter: Option<String>,
}

/// Effective chains per segment exactly as `render.run` builds them (same pad labels, same
/// staged LUT names).
fn effective(store: &ProjectStore, home: &Path, clip: &Clip, doc: &GradeDoc) -> (Vec<SegGrade>, Vec<String>) {
    let mut warnings = Vec::new();
    let resolved = match resolve_segments(store, home, clip, doc) {
        Ok((g, w)) => {
            warnings.extend(w);
            g
        }
        Err(e) => {
            warnings.push(format!("{} (the render would fail)", e.message));
            Vec::new()
        }
    };
    let luts = mg::distinct_luts(resolved.iter().filter_map(|(_, g)| g.lut.as_ref()).map(PathBuf::from).collect::<Vec<_>>().iter());
    let tm = TimeMap::from_segments(&clip.segments);
    let segs = clip
        .segments
        .iter()
        .zip(tm.segments())
        .enumerate()
        .map(|(i, (s, m))| {
            let filter = resolved.iter().find(|(id, _)| id == &s.id).and_then(|(_, g)| {
                let lut = g.lut.as_ref().and_then(|l| luts.iter().position(|p| p == &PathBuf::from(l))).map(mg::staged_lut_name);
                mg::chain_for(g, lut.as_deref(), &format!("s{i}")).ok().flatten()
            });
            SegGrade {
                segment_id: s.id.clone(),
                source_id: s.source_id.clone(),
                out_start: m.out_start,
                out_end: m.out_end,
                has_override: doc.segments.get(s.id.as_str()).is_some_and(|g| !g.is_unset()),
                filter,
            }
        })
        .collect();
    (segs, warnings)
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct GradeOut {
    pub clip_id: ClipId,
    /// Clip revision after the command (`history undo` restores the previous grade).
    pub revision: u64,
    /// What is stored: the clip grade and the per-segment overrides.
    pub grade: GradeDoc,
    /// What each segment renders with (filter = exact ffmpeg chain, null = untouched).
    pub segments: Vec<SegGrade>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn out(ctx: &Ctx, store: &ProjectStore, clip: &Clip, revision: u64, doc: GradeDoc, mut warnings: Vec<String>, next: Vec<String>) -> GradeOut {
    let (segments, w) = effective(store, &ctx.home, clip, &doc);
    warnings.extend(w);
    GradeOut { clip_id: clip.id.clone(), revision, grade: doc, segments, dry_run: ctx.dry_run.then_some(true), warnings, next }
}

/// Stores `doc` as a new clip revision (extras snapshot first, so history undo restores the
/// previous grade). Returns the new revision; nothing is written in a dry run or when nothing
/// changed.
fn commit(ctx: &Ctx, store: &ProjectStore, stage: &str, clip: &Clip, doc: &GradeDoc, detail: Value) -> Result<(u64, bool)> {
    let old = load_doc(store, &clip.id)?;
    if ctx.dry_run || old == *doc {
        return Ok((clip.revision, false));
    }
    let value = if doc.is_empty() {
        None
    } else {
        Some(serde_json::to_value(doc).map_err(|e| RustlerError::new(ErrorCode::Internal, stage, e.to_string()))?)
    };
    crate::timeline_doc::snapshot_extras(store, &clip.id, clip.revision)?;
    let before: Option<Value> = extras::get(store, &clip.id, GRADE_KEY)?;
    extras::update(store, &clip.id, GRADE_KEY, |_| Ok(value))?;
    let mut c = clip.clone();
    if let Err(e) = store.save_clip(&mut c, clip.revision) {
        let _ = extras::update(store, &clip.id, GRADE_KEY, |_| Ok(before));
        return Err(e);
    }
    let mut detail = detail;
    if let Value::Object(m) = &mut detail {
        m.insert("clip_id".into(), json!(clip.id));
        m.insert("from_revision".into(), json!(clip.revision));
        m.insert("to_revision".into(), json!(c.revision));
    }
    log_event(ctx, store, stage, detail)?;
    Ok((c.revision, true))
}

fn check_segment(clip: &Clip, stage: &str, id: &SegmentId) -> Result<()> {
    if clip.segments.iter().any(|s| &s.id == id) {
        return Ok(());
    }
    let ids: Vec<&str> = clip.segments.iter().map(|s| s.id.as_str()).collect();
    Err(RustlerError::new(ErrorCode::NotFound, stage, format!("segment {id} is not in clip {}", clip.id)).hint(format!("segments: {}", ids.join(", "))))
}

fn preview_next(clip: &Clip, seg: Option<&SegmentId>) -> Vec<String> {
    let tm = TimeMap::from_segments(&clip.segments);
    let t = seg
        .and_then(|id| tm.segments().iter().find(|m| &m.segment_id == id))
        .map_or_else(|| tm.duration() / 2.0, |m| (m.out_start + m.out_end) / 2.0);
    vec![format!("rustler grade preview {} --out-t {t:.2}", clip.id), format!("rustler render run {} --quality draft", clip.id)]
}

// ---------------------------------------------------------------------------------------------
// grade.set
// ---------------------------------------------------------------------------------------------

/// A colour push: "r,g,b" text (e.g. "-0.05,0,0.06"), [r,g,b] or {"r":..,"g":..,"b":..}.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum RgbArg {
    Obj(Rgb),
    List(Vec<f64>),
    Text(String),
}

impl RgbArg {
    fn rgb(&self, name: &str) -> std::result::Result<Rgb, String> {
        let v: Vec<f64> = match self {
            RgbArg::Obj(c) => return Ok(*c),
            RgbArg::List(v) => v.clone(),
            RgbArg::Text(s) => s
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|x| x.trim().parse::<f64>().map_err(|_| format!("{name}: '{s}' is not r,g,b numbers")))
                .collect::<std::result::Result<_, _>>()?,
        };
        match v.as_slice() {
            [r, g, b] => Ok(Rgb::new(*r, *g, *b)),
            _ => Err(format!("{name} needs three numbers r,g,b (each -1..1)")),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SetIn {
    pub clip_id: ClipId,
    /// Grade only this segment (an override merged onto the clip grade); default: the whole clip.
    pub segment_id: Option<SegmentId>,
    /// Start from an empty grade instead of merging onto the stored one. With segment_id the
    /// override then also stops inheriting the clip grade (that segment gets only what you set).
    #[serde(default)]
    pub reset: bool,
    /// Built-in look: natural-pop | warm | cool | cinematic | moody | bright-clean | bw | vintage ("none" removes it).
    pub preset: Option<String>,
    /// -1..1: brighter (+) / darker (-) midtones; whites and blacks stay put.
    pub exposure: Option<f64>,
    /// 0.5..1.5 (1 = unchanged).
    pub contrast: Option<f64>,
    /// 0..2 (1 = unchanged, 0 = black and white).
    pub saturation: Option<f64>,
    /// -1..1: boosts muted colours more than saturated ones (skin-friendly).
    pub vibrance: Option<f64>,
    /// 2000..12000 K light colour (6500 = unchanged): lower = warmer picture, higher = cooler.
    pub temperature: Option<f64>,
    /// -1..1: green (-) / magenta (+).
    pub tint: Option<f64>,
    /// -1..1: crush (-) or lift / fade (+) the blacks.
    pub blacks: Option<f64>,
    /// Shadow colour push "r,g,b" (each -1..1), e.g. "-0.05,0,0.06" for teal shadows.
    pub shadows: Option<RgbArg>,
    /// Midtone colour push "r,g,b".
    pub midtones: Option<RgbArg>,
    /// Highlight colour push "r,g,b", e.g. "0.05,0.01,-0.05" for warm highlights.
    pub highlights: Option<RgbArg>,
    /// 3D LUT: asset:<id> (asset kind lut) or a .cube path; "none" removes it.
    pub lut: Option<String>,
    /// 0..1 mix of the LUT (default 1).
    pub lut_strength: Option<f64>,
    /// 0..1 strength of the whole grade (default 1; 0 = off).
    pub intensity: Option<f64>,
}

async fn grade_set(ctx: Ctx, i: SetIn) -> Result<GradeOut> {
    const STAGE: &str = "grade.set";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    if let Some(s) = &i.segment_id {
        check_segment(&clip, STAGE, s)?;
    }
    let mut doc = load_doc(&store, &clip.id)?;
    let current = match &i.segment_id {
        Some(s) => doc.segments.get(s.as_str()).cloned(),
        None => doc.clip.clone(),
    };
    let mut g = if i.reset { Grade::default() } else { current.unwrap_or_default() };
    let rgb = |a: &Option<RgbArg>, name: &str| -> Result<Option<Rgb>> {
        a.as_ref().map(|a| a.rgb(name)).transpose().map_err(|m| verr(STAGE, m))
    };
    let patch = Grade {
        preset: None,
        exposure: i.exposure,
        contrast: i.contrast,
        saturation: i.saturation,
        vibrance: i.vibrance,
        temperature: i.temperature,
        tint: i.tint,
        blacks: i.blacks,
        shadows: rgb(&i.shadows, "shadows")?,
        midtones: rgb(&i.midtones, "midtones")?,
        highlights: rgb(&i.highlights, "highlights")?,
        lut: None,
        lut_strength: i.lut_strength,
        intensity: i.intensity,
        standalone: (i.reset && i.segment_id.is_some()).then_some(true),
    };
    g = g.merged(&patch);
    match i.preset.as_deref().map(str::trim) {
        Some("none") | Some("") => g.preset = None,
        Some(p) => {
            let found = mg::find_preset(p).ok_or_else(|| verr(STAGE, format!("unknown preset '{p}'")).hint(presets_hint()))?;
            g.preset = Some(found.name.to_string());
        }
        None => {}
    }
    match i.lut.as_deref().map(str::trim) {
        Some("none") | Some("") => {
            // On an inheriting segment override "none" is kept so it blocks the clip LUT.
            let block = i.segment_id.is_some() && g.standalone != Some(true) && doc.clip.as_ref().is_some_and(|c| c.lut_ref().is_some());
            g.lut = block.then(|| "none".to_string());
            g.lut_strength = None;
        }
        Some(l) => g.lut = Some(stored_lut_ref(ctx.project_dir.as_deref(), &ctx.home, l).map_err(|mut e| {
            e.stage = STAGE.into();
            e
        })?),
        None => {}
    }
    g.validate().map_err(|m| verr(STAGE, m).hint("ranges: exposure/vibrance/tint/blacks -1..1, contrast 0.5..1.5, saturation 0..2, temperature 2000..12000, lut_strength/intensity 0..1"))?;
    let mut warnings = Vec::new();
    if g.is_unset() {
        warnings.push("nothing to set: the grade is empty (pass a preset or values)".into());
    }
    match &i.segment_id {
        Some(s) => {
            if g.is_unset() {
                doc.segments.remove(s.as_str());
            } else {
                doc.segments.insert(s.to_string(), g.clone());
            }
        }
        None => doc.clip = (!g.is_unset()).then_some(g.clone()),
    }
    let (revision, changed) = commit(&ctx, &store, STAGE, &clip, &doc, json!({"segment_id": i.segment_id, "grade": g}))?;
    if !changed && !ctx.dry_run {
        warnings.push("the grade is unchanged; nothing was saved".into());
    }
    if g.look().is_ok_and(|l| l.exposure > 0.35 || l.saturation > 1.4 || (l.temperature - 6500.0).abs() > 2500.0) {
        warnings.push("strong settings: look at `grade preview` before rendering (subtle grades usually look better)".into());
    }
    let clip = store.load_clip(&clip.id)?;
    Ok(out(&ctx, &store, &clip, revision, doc, warnings, preview_next(&clip, i.segment_id.as_ref())))
}

// ---------------------------------------------------------------------------------------------
// grade.show / grade.clear / grade.presets
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ClipIn {
    pub clip_id: ClipId,
}

async fn grade_show(ctx: Ctx, i: ClipIn) -> Result<GradeOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let doc = load_doc(&store, &clip.id)?;
    let next = if doc.is_empty() {
        vec![format!("rustler grade measure {}", clip.id), "rustler grade presets".to_string()]
    } else {
        preview_next(&clip, None)
    };
    Ok(out(&ctx, &store, &clip, clip.revision, doc, Vec::new(), next))
}

#[derive(Deserialize, JsonSchema)]
pub struct ClearIn {
    pub clip_id: ClipId,
    /// Remove only this segment's override; default: the whole grade (clip + every override).
    pub segment_id: Option<SegmentId>,
}

async fn grade_clear(ctx: Ctx, i: ClearIn) -> Result<GradeOut> {
    const STAGE: &str = "grade.clear";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let mut doc = load_doc(&store, &clip.id)?;
    match &i.segment_id {
        Some(s) => {
            if doc.segments.remove(s.as_str()).is_none() {
                return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("segment {s} has no grade override"))
                    .hint(format!("`rustler grade show {}` lists them", clip.id)));
            }
        }
        None => doc = GradeDoc::default(),
    }
    let mut warnings = Vec::new();
    let (revision, changed) = commit(&ctx, &store, STAGE, &clip, &doc, json!({"segment_id": i.segment_id}))?;
    if !changed && !ctx.dry_run {
        warnings.push("the clip had no grade; nothing was saved".into());
    }
    let clip = store.load_clip(&clip.id)?;
    Ok(out(&ctx, &store, &clip, revision, doc, warnings, vec![format!("rustler history undo {}", clip.id)]))
}

#[derive(Deserialize, JsonSchema)]
pub struct NoIn {}

#[derive(Serialize, JsonSchema)]
pub struct PresetView {
    pub name: String,
    pub description: String,
    /// The numbers the preset sets (explicit grade.set values override them).
    pub grade: Grade,
    /// The ffmpeg chain the preset alone compiles to.
    pub filter: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct PresetsOut {
    pub presets: Vec<PresetView>,
    pub note: String,
}

async fn grade_presets(_ctx: Ctx, _i: NoIn) -> Result<PresetsOut> {
    let presets = mg::presets()
        .into_iter()
        .map(|p| PresetView {
            name: p.name.into(),
            description: p.description.into(),
            filter: mg::chain_for(&Grade { preset: Some(p.name.into()), ..Default::default() }, None, "s0").ok().flatten(),
            grade: p.grade,
        })
        .collect();
    Ok(PresetsOut {
        presets,
        note: "nothing is graded by default; pick one only when the footage needs it, start with --intensity 0.5..0.8 and check `grade preview`".into(),
    })
}

// ---------------------------------------------------------------------------------------------
// grade.preview
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct PreviewIn {
    pub clip_id: ClipId,
    /// Edit-timeline time (seconds) of the frame.
    pub out_t: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct PreviewOut {
    /// Side-by-side jpg: left = before (no grade), right = after (graded). Look at it.
    pub image: PathBuf,
    pub before: PathBuf,
    pub after: PathBuf,
    pub out_t: f64,
    /// The chain of the segment under out_t (null = that segment is not graded).
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

async fn grade_preview(ctx: Ctx, i: PreviewIn) -> Result<PreviewOut> {
    const STAGE: &str = "grade.preview";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let doc = load_doc(&store, &clip.id)?;
    let (segs, mut warnings) = effective(&store, &ctx.home, &clip, &doc);
    let filter = segs.iter().find(|s| i.out_t >= s.out_start && i.out_t < s.out_end).or(segs.last()).and_then(|s| s.filter.clone());
    if filter.is_none() {
        warnings.push("the segment at out_t has no grade: both halves are the same".into());
    }
    let before = crate::render_cmds::preview_frame_image_with(&ctx, &clip.id, i.out_t, false, "_before").await?;
    let after = crate::render_cmds::preview_frame_image_with(&ctx, &clip.id, i.out_t, true, "_after").await?;
    let image = after.with_file_name(
        after.file_name().map(|n| n.to_string_lossy().replace("_after.jpg", "_grade.jpg")).unwrap_or_else(|| "grade.jpg".into()),
    );
    let paths = ToolPaths::from_home(&ctx.home);
    mg::side_by_side(&paths.ffmpeg, &before, &after, &image).await.map_err(|mut e| {
        e.stage = STAGE.into();
        e
    })?;
    Ok(PreviewOut { image, before, after, out_t: i.out_t, filter, warnings })
}

// ---------------------------------------------------------------------------------------------
// grade.measure
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct MeasureIn {
    /// A clip id (frames of its segments) or a source id.
    pub id: String,
    /// Times to sample: edit-timeline seconds for a clip, source seconds for a source (default:
    /// the middle of each segment, or 5 spread over the source).
    #[serde(default)]
    pub t: Vec<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct MeasuredFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<SegmentId>,
    pub source_id: SourceId,
    /// The time asked for (edit timeline for a clip, source time for a source).
    pub t: f64,
    pub source_t: f64,
    pub stats: FrameStats,
}

#[derive(Serialize, JsonSchema)]
pub struct MeasureOut {
    pub id: String,
    /// Plain-language read of the picture (averaged over the frames).
    pub read: Vec<String>,
    /// Commands that would address it; nothing is applied (the agent decides).
    pub suggestions: Vec<String>,
    pub overall: FrameStats,
    pub frames: Vec<MeasuredFrame>,
    /// Segments that differ from the rest (shot matching in a multi-camera clip).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matching: Vec<String>,
    pub note: String,
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round_stats(s: FrameStats) -> FrameStats {
    FrameStats {
        luma_mean: r2(s.luma_mean),
        luma_median: r2(s.luma_median),
        subject_luma: r2(s.subject_luma),
        black_pct: r2(s.black_pct),
        white_pct: r2(s.white_pct),
        saturation: r2(s.saturation),
        neutral_r: r2(s.neutral_r),
        neutral_g: r2(s.neutral_g),
        neutral_b: r2(s.neutral_b),
        neutral_share: r2(s.neutral_share),
    }
}

/// Exposure (our gamma curve) that moves median luma `from` to `to` (0..255), rounded to 0.05.
fn exposure_between(from: f64, to: f64) -> f64 {
    let (a, b) = ((from / 255.0).clamp(0.02, 0.98), (to / 255.0).clamp(0.02, 0.98));
    let e = -(b.ln() / a.ln()).log2() / 1.2;
    (e.clamp(-0.5, 0.5) * 20.0).round() / 20.0
}

async fn grade_measure(ctx: Ctx, i: MeasureIn) -> Result<MeasureOut> {
    const STAGE: &str = "grade.measure";
    let store = ctx.store()?;
    let paths = ToolPaths::from_home(&ctx.home);
    if !paths.ffmpeg.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, "ffmpeg is not installed").hint("run `rustler setup`"));
    }
    for t in &i.t {
        if !(t.is_finite() && *t >= 0.0) {
            return Err(verr(STAGE, "t must be >= 0"));
        }
    }
    let id = i.id.trim().to_string();
    // (segment, source, asked t, source t)
    let mut plan: Vec<(Option<SegmentId>, SourceId, f64, f64)> = Vec::new();
    let mut clip_id: Option<ClipId> = None;
    if id.starts_with("clp_") {
        let cid: ClipId = id.parse().map_err(|_| verr(STAGE, format!("'{id}' is not a clip id")))?;
        let clip = store.load_clip(&cid)?;
        let tm = TimeMap::from_segments(&clip.segments);
        let maps = tm.segments();
        let times: Vec<f64> = if i.t.is_empty() {
            if maps.len() == 1 {
                let m = &maps[0];
                [0.2, 0.5, 0.8].iter().map(|f| m.out_start + (m.out_end - m.out_start) * f).collect()
            } else {
                maps.iter().take(12).map(|m| (m.out_start + m.out_end) / 2.0).collect()
            }
        } else {
            i.t.clone()
        };
        for t in times {
            let Some(m) = maps.iter().find(|m| t >= m.out_start && t < m.out_end).or_else(|| maps.last().filter(|m| t <= m.out_end + 1e-6)) else {
                return Err(verr(STAGE, format!("t {t:.2} is past the clip end ({:.2}s)", tm.duration())));
            };
            plan.push((Some(m.segment_id.clone()), m.source_id.clone(), t, m.src_start + (t - m.out_start)));
        }
        clip_id = Some(clip.id);
    } else if id.starts_with("src_") {
        let sid: SourceId = id.parse().map_err(|_| verr(STAGE, format!("'{id}' is not a source id")))?;
        let src = store.load_source(&sid)?;
        let d = src.probe.duration.max(0.1);
        let times: Vec<f64> = if i.t.is_empty() { [0.1, 0.3, 0.5, 0.7, 0.9].iter().map(|f| d * f).collect() } else { i.t.clone() };
        for t in times {
            plan.push((None, sid.clone(), t, t.min((d - 0.05).max(0.0))));
        }
    } else {
        return Err(verr(STAGE, format!("'{id}' is not a clip (clp_...) or source (src_...) id")));
    }

    let mut frames = Vec::new();
    for (seg, src, t, st) in plan {
        let s = store.load_source(&src)?;
        let media = store.resolve(&s.media_path);
        let stats = mg::measure_frame(&paths.ffmpeg, &media, st).await?;
        frames.push(MeasuredFrame { segment_id: seg, source_id: src, t: r2(t), source_t: r2(st), stats: round_stats(stats) });
    }
    let all: Vec<FrameStats> = frames.iter().map(|f| f.stats).collect();
    let overall = mg::mean_stats(&all);
    let (read, flags) = mg::hints(&overall);
    let target = clip_id.as_ref().map_or_else(|| "<clip_id>".to_string(), |c| c.to_string());
    let mut suggestions = Vec::new();
    if !flags.is_empty() {
        suggestions.push(format!("rustler grade set {target} {}", flags.join(" ")));
    }
    // Shot matching: segments whose brightness or white balance stand out from the rest.
    let mut matching = Vec::new();
    if let Some(cid) = &clip_id {
        let mut per_seg: Vec<(SegmentId, FrameStats)> = Vec::new();
        for f in &frames {
            if let Some(s) = &f.segment_id
                && !per_seg.iter().any(|(id, _)| id == s)
            {
                let mine: Vec<FrameStats> = frames.iter().filter(|x| x.segment_id.as_ref() == Some(s)).map(|x| x.stats).collect();
                per_seg.push((s.clone(), mg::mean_stats(&mine)));
            }
        }
        if per_seg.len() > 1 {
            // Brightness of what is lit (a dark studio background would drown the median).
            let level = |s: &FrameStats| if overall.black_pct > 35.0 { s.subject_luma } else { s.luma_median };
            let med = level(&overall).max(1.0);
            let warm_of = |s: &FrameStats| (s.neutral_share >= 0.01).then(|| s.neutral_r - s.neutral_b);
            let warms: Vec<f64> = per_seg.iter().filter_map(|(_, s)| warm_of(s)).collect();
            let warm_avg = if warms.is_empty() { None } else { Some(warms.iter().sum::<f64>() / warms.len() as f64) };
            for (sid, s) in &per_seg {
                let mut flags = Vec::new();
                let mut why = Vec::new();
                let rel = (level(s) - med) / med;
                if rel.abs() > 0.2 && s.white_pct < 2.0 {
                    // Half-way and capped: a wide shot is often darker by design; preview it.
                    let e = (exposure_between(level(s).max(1.0), med) * 10.0).round() / 20.0;
                    let e = e.clamp(-0.2, 0.2);
                    if e.abs() >= 0.05 {
                        why.push(format!("{:.0}% {} than the clip", rel.abs() * 100.0, if rel < 0.0 { "darker" } else { "brighter" }));
                        flags.push(format!("--exposure {e}"));
                    }
                }
                if let (Some(w), Some(avg)) = (warm_of(s), warm_avg)
                    && (w - avg).abs() > 8.0
                {
                    let k = (6500.0 + (w - avg) * 25.0).clamp(4500.0, 8500.0);
                    why.push(format!("{} than the other shots", if w > avg { "warmer" } else { "cooler" }));
                    flags.push(format!("--temperature {:.0}", (k / 100.0).round() * 100.0));
                }
                if !flags.is_empty() {
                    matching.push(format!("{sid}: {}", why.join(", ")));
                    suggestions.push(format!("rustler grade set {cid} --segment-id {sid} {}", flags.join(" ")));
                }
            }
            if !matching.is_empty() {
                matching.push("match only shots of the same scene and camera setup (a wide shot is often darker by design); the segment suggestions go half-way: preview, then adjust".into());
            }
        }
        suggestions.push(format!("rustler grade preview {cid} --out-t {:.2}", frames.first().map_or(0.0, |f| f.t)));
    }
    Ok(MeasureOut {
        id,
        read,
        suggestions,
        overall: round_stats(overall),
        frames,
        matching,
        note: "measurements only: nothing was changed. Grade only when the footage needs it; look at `grade preview` before rendering".into(),
    })
}

/// Keeps per-segment overrides on their picture after a timeline edit that re-cuts segments
/// (split, remove-pauses, remove-words give new ids): a new id without an override gets the one
/// of the old segment it overlaps most (same source); ids gone from the clip are dropped (the
/// edit's history snapshot keeps them for `history undo`).
pub fn carry_segment_grades(store: &ProjectStore, old: &Clip, new: &Clip) -> Result<()> {
    let doc = load_doc(store, &new.id)?;
    if doc.segments.is_empty() {
        return Ok(());
    }
    let mut d = doc.clone();
    d.segments.retain(|k, _| new.segments.iter().any(|s| s.id.as_str() == k.as_str()));
    for seg in &new.segments {
        if d.segments.contains_key(seg.id.as_str()) || old.segments.iter().any(|o| o.id == seg.id) {
            continue;
        }
        let overlap = |o: &rustler_core::Segment| (o.end.min(seg.end) - o.start.max(seg.start)).max(0.0);
        if let Some(o) = old
            .segments
            .iter()
            .filter(|o| o.source_id == seg.source_id && doc.segments.contains_key(o.id.as_str()) && overlap(o) > 0.0)
            .max_by(|a, b| overlap(a).total_cmp(&overlap(b)))
        {
            d.segments.insert(seg.id.to_string(), doc.segments[o.id.as_str()].clone());
        }
    }
    if d != doc {
        let v = if d.is_empty() { None } else { Some(serde_json::to_value(&d).map_err(|e| RustlerError::new(ErrorCode::Internal, "grade", e.to_string()))?) };
        extras::update(store, &new.id, GRADE_KEY, |_| Ok(v))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    let read = Annotations { read_only: true, ..write };
    reg.register(typed(
        "grade.set",
        "Colour-grade a clip or one segment (merges onto the stored grade; --reset starts empty): preset, exposure, contrast, saturation, vibrance, temperature (K), tint, blacks, shadows/midtones/highlights r,g,b, a .cube LUT (+ strength), intensity. Applied to the source picture before layout and captions; never applied unless asked",
        &["clip_id"],
        write,
        vec![
            Example { description: "a subtle look", cli: "rustler grade set clp_x --preset natural-pop" },
            Example { description: "cinematic at 60 %", cli: "rustler grade set clp_x --preset cinematic --intensity 0.6" },
            Example { description: "match a darker, warmer camera", cli: "rustler grade set clp_x --segment-id seg_b --exposure 0.15 --temperature 7000" },
            Example { description: "teal shadows / warm highlights by hand", cli: "rustler grade set clp_x --shadows -0.05,0,0.06 --highlights 0.05,0.01,-0.05" },
            Example { description: "a LUT from the asset library at 50 %", cli: "rustler grade set clp_x --lut asset:lut-film --lut-strength 0.5" },
        ],
        grade_set,
    ));
    reg.register(typed(
        "grade.show",
        "Show a clip's stored grade (clip + per-segment overrides) and the exact ffmpeg filter chain each segment renders with",
        &["clip_id"],
        read,
        vec![Example { description: "what is graded and how", cli: "rustler grade show clp_x" }],
        grade_show,
    ));
    reg.register(typed(
        "grade.presets",
        "List the built-in grade presets with one-line descriptions, their numbers and filter chains",
        &[],
        read,
        vec![Example { description: "all presets", cli: "rustler grade presets" }],
        grade_presets,
    ));
    reg.register(typed(
        "grade.clear",
        "Remove a clip's whole grade, or one segment's override (--segment-id); undo with history undo",
        &["clip_id"],
        Annotations { destructive: true, ..write },
        vec![
            Example { description: "ungraded again", cli: "rustler grade clear clp_x" },
            Example { description: "drop one segment's override", cli: "rustler grade clear clp_x --segment-id seg_b" },
        ],
        grade_clear,
    ));
    reg.register(typed(
        "grade.preview",
        "Write a side-by-side before | after jpg of the clip frame at out_t (the full render graph with and without the grade); look at it before rendering",
        &["clip_id"],
        read,
        vec![Example { description: "before/after at 2 s", cli: "rustler grade preview clp_x --out-t 2" }],
        grade_preview,
    ));
    reg.register(typed(
        "grade.measure",
        "Measure frames of a clip or source (brightness, clipped blacks/whites, saturation, white-balance cast) and get plain-language hints, shot-matching notes and suggested grade commands; never changes anything",
        &["id"],
        read,
        vec![
            Example { description: "read a clip's segments", cli: "rustler grade measure clp_x" },
            Example { description: "a source at chosen times", cli: "rustler grade measure src_x --t 12.5 --t 40" },
        ],
        grade_measure,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_args_parse() {
        assert_eq!(RgbArg::Text("-0.05, 0,0.06".into()).rgb("s").unwrap(), Rgb::new(-0.05, 0.0, 0.06));
        assert_eq!(RgbArg::Text("[0.1,0.2,0.3]".into()).rgb("s").unwrap(), Rgb::new(0.1, 0.2, 0.3));
        assert_eq!(RgbArg::List(vec![0.1, 0.0, -0.1]).rgb("s").unwrap(), Rgb::new(0.1, 0.0, -0.1));
        assert!(RgbArg::Text("0.1,0.2".into()).rgb("s").is_err());
        assert!(RgbArg::Text("a,b,c".into()).rgb("s").is_err());
    }

    #[test]
    fn exposure_match() {
        assert_eq!(exposure_between(100.0, 100.0), 0.0);
        assert!(exposure_between(60.0, 100.0) > 0.0);
        assert!(exposure_between(140.0, 100.0) < 0.0);
        assert!(exposure_between(5.0, 250.0) <= 0.5);
    }
}
