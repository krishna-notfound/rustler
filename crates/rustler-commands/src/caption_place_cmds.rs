//! Command: caption.place (f40) — where the captions can go without covering a face or the
//! source's own burned-in subtitles.
//!
//! Per segment it measures, in canvas coordinates (through the segment's layout regions, fit and
//! face-tracked crop path):
//! - **burned-in source text** — an engine-free edge-density scan of the source range
//!   ([`rustler_media::textband`], cached in `analysis/<source>/text_bands.json`) plus, when
//!   `analyze ocr` has run, its `burned-in-captions` band ([`crate::understand_cmds::summarize_ocr`]);
//! - **faces** — `analysis/<source>/faces.json` (`analyze faces`) when present, else the engine's
//!   `faces.track` on the segment's source range (1 sample/s).
//!
//! Then it scores every caption baseline `position_y` 0.10..0.85 (the platform safe zone) per
//! segment and over the clip (weighted by segment length) and suggests the best one. Nothing
//! changes unless the agent passes `--apply true` (then one new clip revision; `history undo`).
//! Captions are one clip-wide position (the frozen caption model), so segments whose own best
//! position differs are listed with what still collides there.
//!
//! [`subtitle_spans`] gives QC (`caption_over_source_text`) the same subtitle bands.

use crate::clip_cmds::log_event;
use crate::understand_cmds::{FacesResult, OcrResult, summarize_ocr};
use rustler_core::{Clip, ClipId, ErrorCode, Fit, ProjectStore, Region, RustlerError, Segment, Source, SourceId, TimeMap};
use rustler_media::facepath::CropKey;
use rustler_media::qc::{SAFE_ZONE_MAX_Y, SAFE_ZONE_MIN_Y, SourceTextSpan, caption_band};
use rustler_media::textband::{self, TextBand, TextKind};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

const STAGE: &str = "caption.place";
/// Default seconds between scanned source frames.
pub const DEFAULT_EVERY: f64 = 0.5;
/// Face samples per second when the engine is asked.
const FACE_FPS: f64 = 1.0;
/// Kept free around a source text band.
const TEXT_PAD: f64 = rustler_media::qc::SOURCE_TEXT_MARGIN;

fn verr(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg)
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0 + 0.0
}

// ---------------------------------------------------------------------------------------------
// Source text bands (scan + cache + OCR)
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
struct BandCache {
    #[serde(default)]
    entries: Vec<CacheEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    start: f64,
    end: f64,
    every: f64,
    frames: usize,
    bands: Vec<TextBand>,
}

fn cache_rel(src: &SourceId) -> PathBuf {
    Path::new("analysis").join(src.as_str()).join("text_bands.json")
}

/// Burned-in text bands of `source` between `start` and `end` (source seconds): the edge scan
/// (cached) plus the OCR caption band when `analysis/<source>/ocr.json` exists.
pub(crate) async fn source_bands(store: &ProjectStore, ffmpeg: &Path, source: &Source, start: f64, end: f64, every: f64) -> Result<(Vec<TextBand>, Vec<String>)> {
    let mut notes = Vec::new();
    let key = |x: f64| (x * 1000.0).round() as i64;
    let rel = cache_rel(&source.id);
    let mut cache: BandCache = if store.resolve(&rel).is_file() { store.read_json(&rel).unwrap_or_default() } else { BandCache::default() };
    let hit = cache.entries.iter().find(|e| key(e.start) == key(start) && key(e.end) == key(end) && key(e.every) == key(every)).cloned();
    let mut bands = match hit {
        Some(e) => e.bands,
        None => {
            let media = store.resolve(&source.media_path);
            let h = textband::scan_height(source.probe.width, source.probe.height);
            let frames = textband::read_frames(ffmpeg, &media, start, end, every, h).await?;
            let bands = textband::classify(&frames);
            cache.entries.retain(|e| !(key(e.start) == key(start) && key(e.end) == key(end) && key(e.every) == key(every)));
            cache.entries.push(CacheEntry { start: r3(start), end: r3(end), every, frames: frames.len(), bands: bands.clone() });
            if cache.entries.len() > 200 {
                cache.entries.remove(0);
            }
            let _ = store.write_json(&rel, &cache);
            bands
        }
    };
    if let Some(b) = ocr_caption_band(store, source, start, end) {
        notes.push(format!("OCR (analyze ocr) found burned-in captions at y {:.2}-{:.2} of source {}", b.y0, b.y1, source.id));
        bands.push(b);
    }
    Ok((bands, notes))
}

/// The `burned-in-captions` band `analyze ocr` found, limited to lines seen in `start..end`.
fn ocr_caption_band(store: &ProjectStore, source: &Source, start: f64, end: f64) -> Option<TextBand> {
    let rel = Path::new("analysis").join(source.id.as_str()).join("ocr.json");
    if !store.resolve(&rel).is_file() {
        return None;
    }
    let res: OcrResult = store.read_json(&rel).ok()?;
    let (_, regions) = summarize_ocr(&res.frames, source.probe.duration, res.every);
    let region = regions.iter().find(|r| r.kind == "burned-in-captions")?;
    let lines: Vec<(f64, &crate::understand_cmds::OcrLine)> = res
        .frames
        .iter()
        .filter(|f| f.t_end.unwrap_or(f.t) >= start && f.t <= end)
        .flat_map(|f| f.lines.iter().map(move |l| (f.t, l)))
        .filter(|(_, l)| ((l.bbox.y + l.bbox.h / 2.0) - region.y).abs() <= 0.06)
        .collect();
    if lines.is_empty() {
        return None;
    }
    let fold = |f: &dyn Fn(&crate::understand_cmds::OcrLine) -> f64, min: bool| {
        lines.iter().map(|(_, l)| f(l)).fold(if min { f64::MAX } else { f64::MIN }, |a, b| if min { a.min(b) } else { a.max(b) })
    };
    Some(TextBand {
        kind: TextKind::Subtitles,
        y0: r3(fold(&|l| l.bbox.y, true)),
        y1: r3(fold(&|l| l.bbox.y + l.bbox.h, false)),
        x0: r3(fold(&|l| l.bbox.x, true)),
        x1: r3(fold(&|l| l.bbox.x + l.bbox.w, false)),
        presence: 1.0,
        change: 1.0,
        first_t: r3(lines.iter().map(|(t, _)| *t).fold(f64::MAX, f64::min)),
        last_t: r3(lines.iter().map(|(t, _)| *t).fold(f64::MIN, f64::max)),
    })
}

// ---------------------------------------------------------------------------------------------
// Source -> canvas
// ---------------------------------------------------------------------------------------------

/// Crop path x at segment-relative time `t` (linear between keys).
fn crop_x_at(keys: &[CropKey], t: f64) -> Option<f64> {
    let first = keys.first()?;
    if t <= first.t {
        return Some(first.x);
    }
    let k = keys.partition_point(|k| k.t <= t);
    if k >= keys.len() {
        return keys.last().map(|k| k.x);
    }
    let (a, b) = (keys[k - 1], keys[k]);
    let span = b.t - a.t;
    Some(if span <= 0.0 { b.x } else { a.x + (b.x - a.x) * (t - a.t) / span })
}

/// Maps a source rect `(x0, y0, x1, y1)` (fractions of the source) through `region` onto the
/// canvas; the visible part as fractions of the canvas, `None` when nothing of it shows.
pub(crate) fn map_rect(region: &Region, src_x: Option<f64>, src_px: (f64, f64), canvas_px: (f64, f64), r: (f64, f64, f64, f64)) -> Option<(f64, f64, f64, f64)> {
    let (sw_px, sh_px) = src_px;
    let (cw, ch) = canvas_px;
    let mut s = region.src;
    if let Some(x) = src_x {
        s.x = x;
    }
    let d = region.dst;
    if s.w <= 0.0 || s.h <= 0.0 || sw_px <= 0.0 || sh_px <= 0.0 {
        return None;
    }
    let (rw, rh) = (s.w * sw_px, s.h * sh_px);
    let (dw, dh) = (d.w * cw, d.h * ch);
    let scale = match region.fit {
        Fit::Cover => (dw / rw).max(dh / rh),
        Fit::Contain => (dw / rw).min(dh / rh),
    };
    let (disp_w, disp_h) = (rw * scale, rh * scale);
    let off_x = d.x * cw + (dw - disp_w) / 2.0;
    let off_y = d.y * ch + (dh - disp_h) / 2.0;
    let px = |u: f64| off_x + (u - s.x) * sw_px * scale;
    let py = |v: f64| off_y + (v - s.y) * sh_px * scale;
    let vis_x0 = off_x.max(d.x * cw);
    let vis_x1 = (off_x + disp_w).min((d.x + d.w) * cw);
    let vis_y0 = off_y.max(d.y * ch);
    let vis_y1 = (off_y + disp_h).min((d.y + d.h) * ch);
    let x0 = px(r.0).max(vis_x0);
    let x1 = px(r.2).min(vis_x1);
    let y0 = py(r.1).max(vis_y0);
    let y1 = py(r.3).min(vis_y1);
    (x1 > x0 && y1 > y0).then(|| (x0 / cw, y0 / ch, x1 / cw, y1 / ch))
}

/// A source text band on the canvas.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct CanvasText {
    /// subtitles | static
    pub kind: TextKind,
    /// Canvas band, fractions of the frame height.
    pub y0: f64,
    pub y1: f64,
    /// The band in the source frame.
    pub source_y0: f64,
    pub source_y1: f64,
    /// Share of the scanned frames it was visible in.
    pub presence: f64,
}

/// Where the source text of `bands` lands for `seg` (the main region follows its crop path at
/// the middle of the segment). Text shown by several regions is listed once per region.
fn text_on_canvas(clip: &Clip, seg: &Segment, source: &Source, bands: &[TextBand], crop: Option<&Vec<CropKey>>) -> Vec<CanvasText> {
    let src_px = (source.probe.width as f64, source.probe.height as f64);
    let canvas_px = (clip.canvas.width as f64, clip.canvas.height as f64);
    let main = seg.layout.regions.first().map(|r| r.id.clone());
    let mid = (seg.end - seg.start) / 2.0;
    let mut out = Vec::new();
    for b in bands {
        for r in &seg.layout.regions {
            let x = if Some(&r.id) == main.as_ref() { crop.and_then(|k| crop_x_at(k, mid)) } else { None };
            if let Some((x0, y0, x1, y1)) = map_rect(r, x, src_px, canvas_px, (b.x0, b.y0, b.x1, b.y1)) {
                // A sliver of a wide band at the edge of a crop is not readable text.
                if (x1 - x0) < 0.08 {
                    continue;
                }
                out.push(CanvasText { kind: b.kind, y0: r3(y0), y1: r3(y1), source_y0: b.y0, source_y1: b.y1, presence: b.presence });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Faces
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct TrackResult {
    samples: Vec<rustler_media::facepath::FaceSample>,
}

/// Face boxes `(t, [(x, y, w, h)])` of the source between `start` and `end`, from faces.json.
fn faces_from_json(store: &ProjectStore, src: &SourceId, start: f64, end: f64) -> Option<Vec<(f64, Vec<(f64, f64, f64, f64)>)>> {
    let rel = Path::new("analysis").join(src.as_str()).join("faces.json");
    if !store.resolve(&rel).is_file() {
        return None;
    }
    let f: FacesResult = store.read_json(&rel).ok()?;
    let v: Vec<_> = f
        .samples
        .iter()
        .filter(|s| s.t >= start && s.t < end)
        .map(|s| (s.t, s.faces.iter().map(|b| (b.x, b.y, b.w, b.h)).collect()))
        .collect();
    (!v.is_empty()).then_some(v)
}

/// Faces of one segment on the canvas.
#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct FaceSpan {
    /// faces.json | engine | none
    pub from: String,
    pub samples: usize,
    /// Samples with at least one visible face.
    pub with_face: usize,
    /// Typical top / bottom of the visible faces (10th / 90th percentile), fractions of the frame height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<f64>,
    /// Per sample: the visible faces' vertical extents.
    #[serde(skip)]
    pub per_sample: Vec<Vec<(f64, f64)>>,
}

fn faces_on_canvas(clip: &Clip, seg: &Segment, source: &Source, samples: &[(f64, Vec<(f64, f64, f64, f64)>)], crop: Option<&Vec<CropKey>>, from: &str) -> FaceSpan {
    let src_px = (source.probe.width as f64, source.probe.height as f64);
    let canvas_px = (clip.canvas.width as f64, clip.canvas.height as f64);
    let main = seg.layout.regions.first().map(|r| r.id.clone());
    let mut per_sample = Vec::new();
    for (t, faces) in samples {
        let rel_t = t - seg.start;
        let mut vis = Vec::new();
        for &(x, y, w, h) in faces {
            for r in &seg.layout.regions {
                let cx = if Some(&r.id) == main.as_ref() { crop.and_then(|k| crop_x_at(k, rel_t)) } else { None };
                let Some(full) = map_rect_unclipped(r, cx, src_px, canvas_px, (x, y, x + w, y + h)) else { continue };
                if let Some((x0, y0, x1, y1)) = map_rect(r, cx, src_px, canvas_px, (x, y, x + w, y + h)) {
                    let area = (x1 - x0) * (y1 - y0);
                    if full > 0.0 && area / full >= 0.4 {
                        vis.push((y0, y1));
                    }
                }
            }
        }
        per_sample.push(vis);
    }
    let mut tops: Vec<f64> = per_sample.iter().flatten().map(|f| f.0).collect();
    let mut bottoms: Vec<f64> = per_sample.iter().flatten().map(|f| f.1).collect();
    tops.sort_by(|a, b| a.total_cmp(b));
    bottoms.sort_by(|a, b| a.total_cmp(b));
    let pct = |v: &Vec<f64>, q: f64| (!v.is_empty()).then(|| r3(v[((v.len() - 1) as f64 * q).round() as usize]));
    FaceSpan {
        from: from.into(),
        samples: per_sample.len(),
        with_face: per_sample.iter().filter(|s| !s.is_empty()).count(),
        top: pct(&tops, 0.1),
        bottom: pct(&bottoms, 0.9),
        per_sample,
    }
}

/// Area (canvas fraction units) of the mapped rect before clipping to what is visible.
fn map_rect_unclipped(region: &Region, src_x: Option<f64>, src_px: (f64, f64), canvas_px: (f64, f64), r: (f64, f64, f64, f64)) -> Option<f64> {
    let mut s = region.src;
    if let Some(x) = src_x {
        s.x = x;
    }
    let d = region.dst;
    let (rw, rh) = (s.w * src_px.0, s.h * src_px.1);
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let (dw, dh) = (d.w * canvas_px.0, d.h * canvas_px.1);
    let scale = match region.fit {
        Fit::Cover => (dw / rw).max(dh / rh),
        Fit::Contain => (dw / rw).min(dh / rh),
    };
    let w = (r.2 - r.0) * src_px.0 * scale / canvas_px.0;
    let h = (r.3 - r.1) * src_px.1 * scale / canvas_px.1;
    Some(w * h)
}

// ---------------------------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hits {
    subtitles: bool,
    face_ratio: f64,
    static_text: bool,
}

fn hits_at(y: f64, text: &[CanvasText], faces: &FaceSpan) -> Hits {
    let (lo, hi) = caption_band(y);
    let over = |a: f64, b: f64| a - TEXT_PAD < hi && b + TEXT_PAD > lo;
    let subtitles = text.iter().any(|t| t.kind == TextKind::Subtitles && over(t.y0, t.y1));
    let static_text = text.iter().any(|t| t.kind == TextKind::Static && over(t.y0, t.y1));
    let n = faces.per_sample.len();
    let hit = faces.per_sample.iter().filter(|s| s.iter().any(|(a, b)| *a < hi && *b > lo)).count();
    Hits { subtitles, face_ratio: if n == 0 { 0.0 } else { hit as f64 / n as f64 }, static_text }
}

/// Below this baseline the captions get close to the platform caption/username area, above
/// `HIGH_Y` close to the top UI; both are allowed up to the safe-zone edges (0.10 / 0.85) when
/// that clears a face or subtitles, at a growing cost.
const LOW_Y: f64 = 0.78;
const HIGH_Y: f64 = 0.22;

fn cost(h: Hits, y: f64, pref: f64) -> f64 {
    let edge = if y > LOW_Y {
        15.0 * (y - LOW_Y) / (SAFE_ZONE_MAX_Y - LOW_Y)
    } else if y < HIGH_Y {
        15.0 * (HIGH_Y - y) / (HIGH_Y - SAFE_ZONE_MIN_Y)
    } else {
        0.0
    };
    (if h.subtitles { 100.0 } else { 0.0 }) + 60.0 * h.face_ratio + (if h.static_text { 8.0 } else { 0.0 }) + 30.0 * (y - pref).abs() + edge
}

fn candidates() -> Vec<f64> {
    let lo = (SAFE_ZONE_MIN_Y * 100.0).round() as i64;
    let hi = (SAFE_ZONE_MAX_Y * 100.0).round() as i64;
    (lo..=hi).map(|k| k as f64 / 100.0).collect()
}

/// Free stretches of position_y (no subtitles, faces in at most 20 % of samples) as `[from, to]`.
fn safe_ranges(text: &[CanvasText], faces: &FaceSpan) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    for y in candidates() {
        let h = hits_at(y, text, faces);
        if !h.subtitles && h.face_ratio <= 0.2 {
            match out.last_mut() {
                Some(r) if (y - r[1] - 0.01).abs() < 1e-6 => r[1] = y,
                _ => out.push([y, y]),
            }
        }
    }
    out
}

fn describe(h: Hits) -> Vec<String> {
    let mut v = Vec::new();
    if h.subtitles {
        v.push("covers the source's burned-in subtitles (double captions)".to_string());
    }
    if h.face_ratio > 0.2 {
        v.push(format!("covers a face in {:.0}% of samples", h.face_ratio * 100.0));
    }
    if h.static_text {
        v.push("touches static on-screen text (title, logo or watermark)".to_string());
    }
    v
}

// ---------------------------------------------------------------------------------------------
// Measurement of a clip
// ---------------------------------------------------------------------------------------------

/// One segment's measurement.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SegPlace {
    pub segment_id: String,
    pub out_start: f64,
    pub out_end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Burned-in source text on the canvas in this segment.
    pub source_text: Vec<CanvasText>,
    pub faces: FaceSpan,
    /// Free position_y stretches here (no subtitles, faces in <= 20 % of samples).
    pub safe_ranges: Vec<[f64; 2]>,
    /// Best position_y for this segment alone.
    pub best_y: f64,
    /// What the current position covers here (empty = fine).
    pub at_current: Vec<String>,
    /// What the suggested position covers here (empty = fine).
    pub at_suggested: Vec<String>,
}

pub(crate) struct Measured {
    pub segments: Vec<SegPlace>,
    pub warnings: Vec<String>,
    pub method: Vec<String>,
}

fn crop_paths(store: &ProjectStore, clip: &ClipId) -> BTreeMap<String, Vec<CropKey>> {
    crate::extras::get(store, clip, "crop_paths").ok().flatten().unwrap_or_default()
}

/// Source text (and, with `faces`, faces) of every segment on the canvas.
pub(crate) async fn measure(ctx: &Ctx, store: &ProjectStore, paths: &ToolPaths, clip: &Clip, every: f64, faces: bool) -> Result<Measured> {
    let tm = TimeMap::from_segments(&clip.segments);
    let crops = crop_paths(store, &clip.id);
    let mut sources: HashMap<SourceId, Source> = HashMap::new();
    let mut warnings = Vec::new();
    let mut method = vec![format!(
        "source text: edge-density scan of the source every {every} s (rows whose |Sobel-x| edge density stands out, clustered over samples; changing content = subtitles, unchanging = static text; no OCR), cached in analysis/<source>/text_bands.json; plus `analyze ocr` caption bands when ocr.json exists"
    )];
    let mut engine: Option<rustler_engine_bridge::EngineClient> = None;
    let mut engine_failed = false;
    let mut used_json = false;
    let mut used_engine = false;
    let mut out = Vec::new();
    for (seg, m) in clip.segments.iter().zip(tm.segments()) {
        if !sources.contains_key(&seg.source_id) {
            sources.insert(seg.source_id.clone(), store.load_source(&seg.source_id)?);
        }
        let source = &sources[&seg.source_id];
        if !source.probe.has_video || seg.layout.regions.is_empty() {
            continue;
        }
        let crop = crops.get(seg.id.as_str());
        let (bands, notes) = match source_bands(store, &paths.ffmpeg, source, seg.start, seg.end, every).await {
            Ok(b) => b,
            Err(e) => {
                warnings.push(format!("segment {}: source text not measured ({})", seg.id, e.message));
                (Vec::new(), Vec::new())
            }
        };
        for n in notes {
            if !method.contains(&n) {
                method.push(n);
            }
        }
        let text = text_on_canvas(clip, seg, source, &bands, crop);
        let mut face = FaceSpan { from: "none".into(), ..Default::default() };
        // Segments under 0.5 s carry too little weight to sample (and may yield no frame).
        if faces && seg.end - seg.start >= 0.5 {
            if let Some(s) = faces_from_json(store, &seg.source_id, seg.start, seg.end) {
                used_json = true;
                face = faces_on_canvas(clip, seg, source, &s, crop, "faces.json");
            } else if !engine_failed {
                if engine.is_none() {
                    match crate::analyze_cmds::spawn_engine(paths).await {
                        Ok(e) => engine = Some(e),
                        Err(e) => {
                            engine_failed = true;
                            warnings.push(format!("faces not measured: the engine could not start ({}); run `rustler analyze faces <source>` or check `rustler doctor`", e.message));
                        }
                    }
                }
                if let Some(e) = &engine {
                    let video = source.proxy.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()).unwrap_or_else(|| store.resolve(&source.media_path));
                    let fps = if seg.end - seg.start < 3.0 { 2.0 } else { FACE_FPS };
                    let params = json!({"video_path": video, "start": seg.start, "end": seg.end, "fps": fps});
                    let res: Result<TrackResult> = e.call("faces.track", params, &ctx.progress, &ctx.cancel).await;
                    match res {
                        Ok(r) => {
                            used_engine = true;
                            let s: Vec<_> = r.samples.iter().map(|s| (s.t, s.faces.iter().map(|b| (b.x, b.y, b.w, b.h)).collect())).collect();
                            face = faces_on_canvas(clip, seg, source, &s, crop, "engine");
                        }
                        Err(err) => warnings.push(format!("segment {}: faces not measured ({})", seg.id, err.message)),
                    }
                }
            }
        }
        out.push(SegPlace {
            segment_id: seg.id.to_string(),
            out_start: r2(m.out_start),
            out_end: r2(m.out_end),
            layout: seg.layout.preset.clone(),
            safe_ranges: safe_ranges(&text, &face),
            source_text: text,
            faces: face,
            best_y: 0.0,
            at_current: Vec::new(),
            at_suggested: Vec::new(),
        });
    }
    if let Some(e) = engine {
        e.shutdown().await;
    }
    if used_json {
        method.push("faces: analysis/<source>/faces.json (analyze faces)".into());
    }
    if used_engine {
        method.push(format!("faces: engine faces.track on each segment's source range ({FACE_FPS} sample/s)"));
    }
    method.push("both mapped onto the canvas through each segment's layout regions, fit and face-tracked crop path".into());
    Ok(Measured { segments: out, warnings, method })
}

/// Picks the clip-wide position_y (weighted by segment length) and fills each segment's best y
/// and collisions. Returns the suggestion.
fn decide(segs: &mut [SegPlace], current: f64) -> f64 {
    let cands = candidates();
    let mut best = (f64::MAX, current);
    for &y in &cands {
        let total: f64 = segs
            .iter()
            .map(|s| (s.out_end - s.out_start).max(0.05) * cost(hits_at(y, &s.source_text, &s.faces), y, current))
            .sum();
        if total < best.0 - 1e-9 {
            best = (total, y);
        }
    }
    let y = best.1;
    for s in segs.iter_mut() {
        let mut b = (f64::MAX, current);
        for &c in &cands {
            let k = cost(hits_at(c, &s.source_text, &s.faces), c, current);
            if k < b.0 - 1e-9 {
                b = (k, c);
            }
        }
        s.best_y = b.1;
        s.at_current = describe(hits_at(current, &s.source_text, &s.faces));
        s.at_suggested = describe(hits_at(y, &s.source_text, &s.faces));
    }
    y
}

/// Burned-in source subtitles on the canvas per output span, for QC's `caption_over_source_text`.
pub(crate) async fn subtitle_spans(store: &ProjectStore, paths: &ToolPaths, clip: &Clip) -> Result<Vec<SourceTextSpan>> {
    let tm = TimeMap::from_segments(&clip.segments);
    let crops = crop_paths(store, &clip.id);
    let mut out = Vec::new();
    for (seg, m) in clip.segments.iter().zip(tm.segments()) {
        let source = store.load_source(&seg.source_id)?;
        if !source.probe.has_video || seg.layout.regions.is_empty() {
            continue;
        }
        let (bands, _) = source_bands(store, &paths.ffmpeg, &source, seg.start, seg.end, DEFAULT_EVERY).await?;
        for t in text_on_canvas(clip, seg, &source, &bands, crops.get(seg.id.as_str())) {
            if t.kind == TextKind::Subtitles {
                out.push(SourceTextSpan { segment_id: seg.id.to_string(), out_start: m.out_start, out_end: m.out_end, y0: t.y0, y1: t.y1 });
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// caption.place
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceIn {
    pub clip_id: ClipId,
    /// Set the suggested position_y on the clip (one new revision, `history undo` reverts it).
    /// Default false: measure and suggest only.
    #[serde(default)]
    pub apply: bool,
    /// Seconds between scanned source frames (default 0.5; 0.2..5).
    pub every: Option<f64>,
    /// Measure faces too (faces.json, else the engine's faces.track). Default true.
    pub faces: Option<bool>,
    /// With apply: fail with revision_conflict unless the clip is at this revision.
    pub base_revision: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct PlaceOut {
    pub clip_id: ClipId,
    pub revision: u64,
    pub captions_enabled: bool,
    pub current_position_y: f64,
    /// Best clip-wide caption baseline (0 top .. 1 bottom) inside the safe zone 0.10..0.85.
    pub suggested_position_y: f64,
    /// True when the current position covers no source subtitles and faces in <= 20 % of samples anywhere.
    pub current_ok: bool,
    /// What the suggestion still covers (segment: problem), when no single position clears everything.
    pub remaining_conflicts: Vec<String>,
    pub applied: bool,
    pub segments: Vec<SegPlace>,
    /// How each number was measured.
    pub method: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

async fn caption_place(ctx: Ctx, i: PlaceIn) -> Result<PlaceOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let every = i.every.unwrap_or(DEFAULT_EVERY);
    if !(0.2..=5.0).contains(&every) {
        return Err(verr("every must be between 0.2 and 5 seconds"));
    }
    if let Some(b) = i.base_revision
        && b != clip.revision
    {
        return Err(RustlerError::new(ErrorCode::RevisionConflict, STAGE, format!("clip {} is at revision {} but base_revision was {b}", clip.id, clip.revision))
            .hint("reload the clip (`rustler clip show <clip_id>`) and retry with its revision"));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let m = measure(&ctx, &store, &paths, &clip, every, i.faces.unwrap_or(true)).await?;
    let mut segs = m.segments;
    let mut warnings = m.warnings;
    let current = clip.captions.position_y;
    let suggested = decide(&mut segs, current);
    let current_ok = segs.iter().all(|s| s.at_current.is_empty() || s.at_current.iter().all(|c| c.starts_with("touches static")));
    let remaining: Vec<String> = segs
        .iter()
        .flat_map(|s| s.at_suggested.iter().map(move |c| format!("{} ({:.1}-{:.1} s): {c}; its own best position is {:.2}", s.segment_id, s.out_start, s.out_end, s.best_y)))
        .collect();
    if !clip.captions.enabled {
        warnings.push(format!("captions are off on this clip; the suggestion applies when you turn them on (`rustler caption set {} --enabled true`)", clip.id));
    }
    if segs.iter().all(|s| s.source_text.is_empty() && s.faces.with_face == 0) {
        warnings.push("no burned-in text and no faces found on the canvas: any position works; look at a frame to be sure".into());
    }
    let mut revision = clip.revision;
    let mut applied = false;
    let change = (suggested - current).abs() >= 0.005;
    if i.apply && change && !ctx.dry_run {
        let mut c = clip.clone();
        c.captions.position_y = suggested;
        crate::extras::save_clip(&store, &mut c, clip.revision)?;
        revision = c.revision;
        applied = true;
        log_event(&ctx, &store, STAGE, json!({"clip_id": clip.id, "from": current, "position_y": suggested, "to_revision": revision}))?;
    }
    // A frame worth looking at: the middle of the first segment with source text, else of the clip.
    let look = segs
        .iter()
        .find(|s| !s.source_text.is_empty())
        .or_else(|| segs.first())
        .map_or(0.5, |s| r2((s.out_start + s.out_end) / 2.0));
    let mut next = Vec::new();
    if applied {
        next.push(format!("rustler render preview-frame {} --out-t {look:.2}", clip.id));
        next.push(format!("undo it: rustler history undo {}", clip.id));
    } else if change {
        next.push(format!("rustler caption place {} --apply true", clip.id));
        next.push(format!("or set it yourself: rustler caption set {} --position-y {suggested:.2}", clip.id));
    } else {
        next.push(format!("the current position {current:.2} is already the best; look: rustler render preview-frame {} --out-t {look:.2}", clip.id));
    }
    if segs.iter().any(|s| s.source_text.iter().any(|t| t.kind == TextKind::Subtitles)) {
        next.push(format!(
            "or hide the source subtitles instead: rustler mask add {} --kind blur --rect '{{\"x\":0,\"y\":Y0,\"w\":1,\"h\":H}}' --start S --end E (canvas band per segment above)",
            clip.id
        ));
    }
    Ok(PlaceOut {
        clip_id: clip.id.clone(),
        revision,
        captions_enabled: clip.captions.enabled,
        current_position_y: current,
        suggested_position_y: suggested,
        current_ok,
        remaining_conflicts: remaining,
        applied,
        segments: segs,
        method: m.method,
        warnings,
        next,
    })
}

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "caption.place",
        "Measure where captions can go: burned-in source subtitles/text and faces per segment on the canvas, then suggest a safe caption position_y (0.10-0.85) that avoids both; --apply true sets it (one undoable revision). Never changes anything without --apply",
        &["clip_id"],
        Annotations { long_running: true, ..Annotations::default() },
        vec![
            Example { description: "see where the captions can go", cli: "rustler caption place clp_X" },
            Example { description: "move them there", cli: "rustler caption place clp_X --apply true" },
        ],
        caption_place,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_core::Rect;

    fn region(src: Rect, dst: Rect, fit: Fit) -> Region {
        Region { id: "main".into(), src, dst, z: 0, fit }
    }

    #[test]
    fn blur_fit_maps_source_bottom_into_the_middle_band() {
        // 16:9 source contained in the middle of a 9:16 canvas (the stage shot of trial 3).
        let r = region(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, Rect { x: 0.0, y: 0.341796875, w: 1.0, h: 0.31640625 }, Fit::Contain);
        let (_, y0, _, y1) = map_rect(&r, None, (1280.0, 720.0), (1080.0, 1920.0), (0.2, 0.85, 0.8, 0.92)).unwrap();
        assert!((y0 - (0.3418 + 0.85 * 0.3164)).abs() < 0.01, "{y0}");
        assert!((y1 - (0.3418 + 0.92 * 0.3164)).abs() < 0.01, "{y1}");
    }

    #[test]
    fn center_crop_hides_text_outside_the_crop() {
        let r = region(Rect { x: 0.4, y: 0.0, w: 0.31640625, h: 1.0 }, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, Fit::Cover);
        // text entirely left of the crop
        assert!(map_rect(&r, None, (1920.0, 1080.0), (1080.0, 1920.0), (0.05, 0.8, 0.3, 0.85)).is_none());
        // the crop path moves the window onto it
        assert!(map_rect(&r, Some(0.0), (1920.0, 1080.0), (1080.0, 1920.0), (0.05, 0.8, 0.3, 0.85)).is_some());
    }

    fn seg(text: Vec<(TextKind, f64, f64)>, faces: Vec<(f64, f64)>, a: f64, b: f64) -> SegPlace {
        let source_text = text.into_iter().map(|(kind, y0, y1)| CanvasText { kind, y0, y1, source_y0: 0.0, source_y1: 0.0, presence: 1.0 }).collect();
        let per_sample: Vec<Vec<(f64, f64)>> = (0..10).map(|_| faces.clone()).collect();
        SegPlace {
            segment_id: "s".into(),
            out_start: a,
            out_end: b,
            layout: None,
            source_text,
            faces: FaceSpan { from: "test".into(), samples: 10, with_face: if faces.is_empty() { 0 } else { 10 }, top: None, bottom: None, per_sample },
            safe_ranges: Vec::new(),
            best_y: 0.0,
            at_current: Vec::new(),
            at_suggested: Vec::new(),
        }
    }

    #[test]
    fn suggestion_clears_subtitles_and_faces_and_stays_close() {
        // subtitles at 0.60-0.64 under the default 0.70 (band 0.62-0.74); face 0.20-0.45.
        let mut s = vec![seg(vec![(TextKind::Subtitles, 0.60, 0.64)], vec![(0.20, 0.45)], 0.0, 4.0)];
        let y = decide(&mut s, 0.70);
        let (lo, hi) = caption_band(y);
        assert!(lo >= 0.649 || hi <= 0.591, "y {y} band {lo}-{hi}");
        assert!(lo >= 0.45 || hi <= 0.20, "clear of the face: {y}");
        assert!((y - 0.70).abs() <= 0.10, "stays near the current spot: {y}");
        assert!(!s[0].at_current.is_empty() && s[0].at_suggested.is_empty(), "{:?}", s[0]);
        // Nothing in the way: the current position stays.
        let mut free = vec![seg(vec![], vec![(0.2, 0.4)], 0.0, 3.0)];
        assert_eq!(decide(&mut free, 0.70), 0.70);
        assert!(free[0].at_current.is_empty());
    }

    #[test]
    fn longer_segment_wins_a_conflict_and_the_other_is_reported() {
        // seg A (long) has subtitles at 0.75-0.82; seg B (short) has a face low at 0.50-0.66.
        let mut s = vec![seg(vec![(TextKind::Subtitles, 0.75, 0.82)], vec![], 0.0, 10.0), seg(vec![], vec![(0.50, 0.66)], 10.0, 11.0)];
        let y = decide(&mut s, 0.78);
        assert!(s[0].at_suggested.is_empty(), "the long segment is clear at {y}: {:?}", s[0].at_suggested);
        let r = safe_ranges(&s[0].source_text, &s[0].faces);
        assert!(r.iter().any(|r| y >= r[0] && y <= r[1]), "{r:?} {y}");
    }
}
