//! Command: clip.cover. Picks the best frame of a clip's latest render for a cover / thumbnail:
//! sharp (Laplacian variance), a face in a good place and facing the camera with the eyes
//! showing contrast (YuNet through the engine, when it is available), and not on a cut or inside
//! a transition. The JPG goes to `clips/<id>/cover.jpg`, the choice is recorded in the clip's
//! extras under `cover`, and `export.clip` copies it next to the video as `<name>.cover.jpg`
//! and points `<name>.post.json`'s `cover` at it.
//!
//! The frame comes from the render, so captions and the hook title are in the picture.
//!
//! Speed: `--auto` scores a fixed set of frames spread evenly over the render ([`SAMPLES`]; the
//! 12 frames of the render's QC contact sheet when `qc run` made one, so no frame is decoded
//! again), extracted by one ffmpeg call at [`SCORE_WIDTH`], with one engine `faces.track` call
//! for all of them. Frames where a mask is shown, and frames where a mask, card / image or text
//! overlay covers the face, are left out when other frames are free; the areas covered by those
//! layers never count toward the sharpness.

use crate::clip_cmds::log_event;
use crate::{extras, fx_cmds};
use chrono::Utc;
use image::GrayImage;
use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError, TimeMap};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::{Tool, ToolPaths};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// extras.json key.
pub const KEY: &str = "cover";
/// Width of the frames that are scored (the saved cover keeps the render's size).
const SCORE_WIDTH: u32 = 540;
/// Frames scored by `--auto` (evenly spread over the render) when no QC contact sheet is reused.
pub const SAMPLES: usize = 16;
/// The QC contact sheet layout (`rustler-media` qc: 12 tiles, 4 columns, 270 px wide).
const QC_TILES: usize = 12;
const QC_COLS: u32 = 4;
const QC_TILE_W: u32 = 270;
/// A face covered this much (share of its box) by a mask, card or text is not a cover.
pub const COVERED_FACE: f64 = 0.25;
/// Frames closer than this to a cut or inside a transition are not used (seconds).
const CUT_MARGIN: f64 = 0.15;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Image measures (pure, unit-tested)
// ---------------------------------------------------------------------------------------------

/// Variance of the 3x3 Laplacian of `g` inside `r = (x0, y0, x1, y1)`: high for a sharp picture,
/// low for a blurred or flat one.
pub fn laplacian_variance(g: &GrayImage, r: (u32, u32, u32, u32)) -> f64 {
    laplacian_variance_ex(g, r, &[])
}

/// [`laplacian_variance`] leaving out the pixels inside `skip` (pixel boxes `(x0, y0, x1, y1)`):
/// masks blur (low), text and cards are crisp graphics (high); neither is the picture.
pub fn laplacian_variance_ex(g: &GrayImage, r: (u32, u32, u32, u32), skip: &[(u32, u32, u32, u32)]) -> f64 {
    let (w, h) = g.dimensions();
    let (x0, y0) = (r.0.max(1), r.1.max(1));
    let (x1, y1) = (r.2.min(w.saturating_sub(1)), r.3.min(h.saturating_sub(1)));
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let px = |x: u32, y: u32| f64::from(g.get_pixel(x, y).0[0]);
    let (mut sum, mut sum2, mut n) = (0.0, 0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            if skip.iter().any(|b| x >= b.0 && x < b.2 && y >= b.1 && y < b.3) {
                continue;
            }
            let l = px(x - 1, y) + px(x + 1, y) + px(x, y - 1) + px(x, y + 1) - 4.0 * px(x, y);
            sum += l;
            sum2 += l * l;
            n += 1.0;
        }
    }
    if n < 1.0 {
        return 0.0;
    }
    let mean = sum / n;
    (sum2 / n - mean * mean).max(0.0)
}

/// Standard deviation of the luma in a square patch of radius `r` around `(cx, cy)`.
pub fn patch_std(g: &GrayImage, cx: f64, cy: f64, r: f64) -> f64 {
    let (w, h) = g.dimensions();
    let x0 = (cx - r).floor().max(0.0) as u32;
    let y0 = (cy - r).floor().max(0.0) as u32;
    let x1 = ((cx + r).ceil() as u32).min(w);
    let y1 = ((cy + r).ceil() as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let (mut s, mut s2, mut n) = (0.0, 0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let v = f64::from(g.get_pixel(x, y).0[0]);
            s += v;
            s2 += v * v;
            n += 1.0;
        }
    }
    let m = s / n;
    (s2 / n - m * m).max(0.0).sqrt()
}

/// A face as the engine reports it (normalised 0..1 of the frame; `lm` = YuNet's five landmarks:
/// right eye, left eye, nose, right mouth corner, left mouth corner).
#[derive(Deserialize, Clone, Debug)]
pub struct Face {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub lm: Option<Vec<[f64; 2]>>,
}

/// What was measured on one frame.
#[derive(Clone, Debug, Default)]
pub struct Cand {
    /// Edit-timeline time.
    pub out_t: f64,
    /// Time in the rendered file.
    pub file_t: f64,
    /// Laplacian variance (of the face box when there is a face, else of the middle of the frame).
    pub sharp: f64,
    /// 0..1: a face of a good size in a good place (0 = no face).
    pub face_fit: f64,
    /// Local contrast around the eyes (None without landmarks).
    pub eyes: Option<f64>,
    /// 0..1: nose between the eyes (facing the camera).
    pub frontal: Option<f64>,
    /// Final score 0..1 (filled by [`rank`]).
    pub score: f64,
    /// Layers shown on this frame (`mask`, `image`, `cutaway`, `text`).
    pub overlays: Vec<String>,
}

/// 0..1 for a face box: size between 2% and 50% of the frame, centre in the middle half
/// horizontally and above the caption band vertically.
pub fn face_fit(f: &Face) -> f64 {
    let area = f.w * f.h;
    let size = if area < 0.02 { area / 0.02 } else if area > 0.5 { 0.5 / area } else { 1.0 };
    let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
    let ramp = |v: f64, lo: f64, hi: f64, slack: f64| {
        if v < lo { 1.0 - ((lo - v) / slack).min(1.0) } else if v > hi { 1.0 - ((v - hi) / slack).min(1.0) } else { 1.0 }
    };
    size * ramp(cx, 0.3, 0.7, 0.25) * ramp(cy, 0.15, 0.55, 0.25)
}

/// Measures a frame (`gray`) given the engine's biggest face (if any).
pub fn measure(gray: &GrayImage, face: Option<&Face>) -> (f64, f64, Option<f64>, Option<f64>) {
    measure_ex(gray, face, &[])
}

/// [`measure`] with the layer boxes `covers` (normalised `(x0, y0, x1, y1)`) left out of the
/// sharpness.
pub fn measure_ex(gray: &GrayImage, face: Option<&Face>, covers: &[(f64, f64, f64, f64)]) -> (f64, f64, Option<f64>, Option<f64>) {
    let (w, h) = (f64::from(gray.width()), f64::from(gray.height()));
    let skip: Vec<(u32, u32, u32, u32)> = covers
        .iter()
        .map(|c| ((c.0.clamp(0.0, 1.0) * w) as u32, (c.1.clamp(0.0, 1.0) * h) as u32, (c.2.clamp(0.0, 1.0) * w).ceil() as u32, (c.3.clamp(0.0, 1.0) * h).ceil() as u32))
        .collect();
    let laplacian_variance = |g: &GrayImage, r: (u32, u32, u32, u32)| laplacian_variance_ex(g, r, &skip);
    match face {
        Some(f) => {
            let (px, py) = (f.w * 0.2, f.h * 0.2);
            let rect = (
                ((f.x - px).max(0.0) * w) as u32,
                ((f.y - py).max(0.0) * h) as u32,
                (((f.x + f.w + px).min(1.0)) * w) as u32,
                (((f.y + f.h + py).min(1.0)) * h) as u32,
            );
            let sharp = laplacian_variance(gray, rect);
            let (mut eyes, mut frontal) = (None, None);
            if let Some(lm) = f.lm.as_ref().filter(|l| l.len() >= 3) {
                let r = (f.w * w * 0.12).max(3.0);
                let a = patch_std(gray, lm[0][0] * w, lm[0][1] * h, r);
                let b = patch_std(gray, lm[1][0] * w, lm[1][1] * h, r);
                eyes = Some((a + b) / 2.0);
                let mid = (lm[0][0] + lm[1][0]) / 2.0;
                frontal = Some(1.0 - ((lm[2][0] - mid).abs() / (0.35 * f.w).max(1e-6)).min(1.0));
            }
            (sharp, face_fit(f), eyes, frontal)
        }
        // No face: the middle band, below where hook titles sit (about 10-28% of the height) and
        // above the captions (about 60-75%), so burned-in text does not decide the sharpness.
        None => (laplacian_variance(gray, ((w * 0.1) as u32, (h * 0.34) as u32, (w * 0.9) as u32, (h * 0.56) as u32)), 0.0, None, None),
    }
}

fn norm(v: f64, lo: f64, hi: f64) -> f64 {
    if hi - lo < 1e-9 { 1.0 } else { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) }
}

/// Fills `score` and sorts best first. With faces (`engine` true and at least one candidate has
/// one): 0.35 sharpness + 0.30 face size/position + 0.20 eye contrast + 0.15 frontal. Without:
/// sharpness only. Sharpness is log-scaled and compared across the candidates.
pub fn rank(cands: &mut [Cand], engine: bool) {
    if cands.is_empty() {
        return;
    }
    let sh: Vec<f64> = cands.iter().map(|c| (1.0 + c.sharp).ln()).collect();
    let (slo, shi) = (sh.iter().copied().fold(f64::MAX, f64::min), sh.iter().copied().fold(f64::MIN, f64::max));
    let faces = engine && cands.iter().any(|c| c.face_fit > 0.0);
    let eyes: Vec<f64> = cands.iter().filter_map(|c| c.eyes).collect();
    let (elo, ehi) = (eyes.iter().copied().fold(f64::MAX, f64::min), eyes.iter().copied().fold(f64::MIN, f64::max));
    for (c, s) in cands.iter_mut().zip(&sh) {
        let sharp = norm(*s, slo, shi);
        c.score = if faces {
            let e = c.eyes.map_or(0.0, |e| norm(e, elo, ehi));
            0.35 * sharp + 0.30 * c.face_fit + 0.20 * e + 0.15 * c.frontal.unwrap_or(0.0)
        } else {
            sharp
        };
    }
    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
}

/// `n` render-file times spread evenly: the middles of `n` equal slices of `file_dur` (the
/// QC contact sheet uses the same times for its 12 tiles).
pub fn sample_times(file_dur: f64, n: usize) -> Vec<f64> {
    if !(file_dur.is_finite() && file_dur > 0.0) || n == 0 {
        return Vec::new();
    }
    (0..n).map(|i| file_dur * (i as f64 + 0.5) / n as f64).collect()
}

/// Edit-timeline time whose rendered time is `file_t` (`map` = edit -> rendered, non-decreasing).
pub fn edit_time_of(map: impl Fn(f64) -> f64, file_t: f64, edit_dur: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, edit_dur.max(0.0));
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if map(mid) < file_t { lo = mid } else { hi = mid }
    }
    (lo + hi) / 2.0
}

/// A layer drawn over the picture: normalised box `(x0, y0, x1, y1)` shown during `[start, end)`
/// (edit-timeline seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub kind: String,
    pub bbox: (f64, f64, f64, f64),
    pub start: f64,
    pub end: f64,
}

/// Approximate box of a text overlay: centred at `position_y`, the font size a fraction of the
/// width, wrapped at 90% of the width (average glyph 0.55 em, line 1.25 em, box padding 0.5 em).
pub fn text_box(text: &str, position_y: f64, font_size_factor: f64, canvas_w: f64, canvas_h: f64) -> (f64, f64, f64, f64) {
    let fs = font_size_factor.clamp(0.01, 0.3);
    let chars = text.chars().count().max(1) as f64;
    let per_line = (0.9 / (0.55 * fs)).max(1.0);
    let lines = (chars / per_line).ceil().max(1.0);
    let width = (chars.min(per_line) * 0.55 * fs + fs).min(0.96);
    let height = (lines * 1.25 * fs + fs) * canvas_w / canvas_h.max(1.0);
    (0.5 - width / 2.0, position_y - height / 2.0, 0.5 + width / 2.0, position_y + height / 2.0)
}

/// Share (0..1) of the face box `f` covered by the boxes `covers` (overlaps added, capped at 1).
pub fn covered_share(f: &Face, covers: &[(f64, f64, f64, f64)]) -> f64 {
    let area = (f.w * f.h).max(1e-9);
    let sum: f64 = covers
        .iter()
        .map(|c| {
            let w = (c.2.min(f.x + f.w) - c.0.max(f.x)).max(0.0);
            let h = (c.3.min(f.y + f.h) - c.1.max(f.y)).max(0.0);
            w * h
        })
        .sum();
    (sum / area).min(1.0)
}

/// The 12 tiles of a QC contact sheet (`sheet`, 4 x 3 tiles 270 px wide with a label strip under
/// each) of a render `render_w` x `render_h`. `None` when the sheet does not have that layout.
pub fn sheet_tiles(sheet: &GrayImage, render_w: u32, render_h: u32) -> Option<Vec<GrayImage>> {
    if render_w == 0 || render_h == 0 {
        return None;
    }
    // ffmpeg scale=270:-2: the height rounded to an even number.
    let tile_h = ((f64::from(QC_TILE_W) * f64::from(render_h) / f64::from(render_w) / 2.0).round() * 2.0) as u32;
    let rows = (QC_TILES as u32).div_ceil(QC_COLS);
    let (w, h) = sheet.dimensions();
    if w != QC_COLS * QC_TILE_W || h % rows != 0 {
        return None;
    }
    let cell_h = h / rows;
    if cell_h <= tile_h || cell_h - tile_h > 80 {
        return None;
    }
    Some(
        (0..QC_TILES as u32)
            .map(|i| image::imageops::crop_imm(sheet, (i % QC_COLS) * QC_TILE_W, (i / QC_COLS) * cell_h, QC_TILE_W, tile_h).to_image())
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct CoverIn {
    pub clip_id: ClipId,
    /// Pick the best frame by measures (default when `out_t` is not given).
    pub auto: Option<bool>,
    /// Use the frame at this edit-timeline time (what `rustler clip show` prints) instead.
    pub out_t: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct CandidateView {
    pub out_t: f64,
    pub score: f64,
    pub sharpness: f64,
    pub face: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eye_contrast: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontal: Option<f64>,
    /// Layers shown on this frame (their areas do not count toward the sharpness).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct CoverOut {
    pub clip_id: ClipId,
    /// The cover JPG (absolute path); open it to look at it.
    pub image: PathBuf,
    /// Edit-timeline time of the chosen frame.
    pub out_t: f64,
    /// Time of the frame inside the rendered file.
    pub file_t: f64,
    /// auto | manual
    pub method: String,
    /// True when faces were measured (the engine was available).
    pub faces_measured: bool,
    /// auto: where the scored frames came from: `qc_contact_sheet` (the render's QC sheet, no
    /// decode) or `render` (one ffmpeg call); frames scored; frames left out (cut, mask, covered face).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames_scored: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<String>,
    /// Seconds the command took.
    pub elapsed_s: f64,
    /// Best candidates first (auto only).
    pub candidates: Vec<CandidateView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

#[derive(Deserialize)]
struct TrackOut {
    samples: Vec<TrackSample>,
}

#[derive(Deserialize)]
struct TrackSample {
    #[serde(default)]
    t: f64,
    #[serde(default)]
    faces: Vec<Face>,
}

fn view(c: &Cand) -> CandidateView {
    CandidateView {
        out_t: round2(c.out_t),
        score: (c.score * 1000.0).round() / 1000.0,
        sharpness: c.sharp.round(),
        face: c.face_fit > 0.0,
        eye_contrast: c.eyes.map(round2),
        frontal: c.frontal.map(round2),
        overlays: c.overlays.clone(),
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Rendered-file times of the cuts and transitions (centre, half width).
fn boundaries(clip: &rustler_core::Clip, tm: &TimeMap, tmap: &rustler_media::compile::TransitionMap) -> Vec<(f64, f64)> {
    let n = tm.segments().len();
    (0..n.saturating_sub(1))
        .map(|k| {
            let at = tm.segments()[k].out_end;
            let half = tmap.at_boundary(k).map_or(0.0, |(_, f)| f as f64 / clip.canvas.fps / 2.0);
            (tmap.map_time(at), half)
        })
        .collect()
}

async fn clip_cover(ctx: Ctx, i: CoverIn) -> Result<CoverOut> {
    const STAGE: &str = "clip.cover";
    let t0 = std::time::Instant::now();
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    if i.out_t.is_some() && i.auto == Some(true) {
        return Err(validation(STAGE, "give out_t or auto, not both"));
    }
    let render = crate::qc_cmds::pick_render(&clip, None).map_err(|mut e| {
        e.stage = STAGE.into();
        e
    })?;
    let video = store.resolve(&render.path);
    if !video.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("render file missing: {}", video.display()))
            .hint(format!("re-render: rustler render run {}", clip.id)));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    paths.require(Tool::Ffmpeg)?;
    paths.require(Tool::Ffprobe)?;
    let tm = TimeMap::from_segments(&clip.segments);
    let tmap = fx_cmds::transition_map(&store, &clip)?;
    let probe = rustler_media::probe(&paths, &video).await?;
    let file_dur = probe.duration;
    let edit_dur = tm.duration();
    let work = store.clip_dir(&clip.id).join("cover_work");
    std::fs::create_dir_all(&work).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("cannot create {}: {e}", work.display())))?;
    let qc_sheet = render.qc.as_ref().map(|q| store.resolve(q).with_extension("jpg")).filter(|p| p.is_file());
    let src = Frames { video: &video, render_id: &render.id, qc_sheet, size: if probe.width > 0 && probe.height > 0 { (probe.width, probe.height) } else { (clip.canvas.width, clip.canvas.height) } };
    let result = run(&ctx, &store, &paths, &clip, &i, &src, file_dur, edit_dur, &tm, &tmap, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    result.map(|mut o| {
        o.elapsed_s = round2(t0.elapsed().as_secs_f64());
        o
    })
}

/// Where `--auto` gets its frames.
struct Frames<'a> {
    video: &'a Path,
    render_id: &'a rustler_core::RenderId,
    /// The render's QC contact sheet, when `qc run` made one.
    qc_sheet: Option<PathBuf>,
    /// Render frame size.
    size: (u32, u32),
}

/// The layers drawn over the picture (masks, images / cards, picture-in-picture cutaways, text
/// overlays) with their edit-timeline windows.
fn overlays(store: &ProjectStore, clip: &rustler_core::Clip) -> Result<Vec<Overlay>> {
    let (t, _) = crate::timeline_doc::assemble(store, &clip.id)?;
    let env = crate::timeline_doc::DocEnv::new(store, &t)?;
    let bx = |r: &rustler_core::Rect| (r.x, r.y, r.x + r.w, r.y + r.h);
    let mut out = Vec::new();
    for m in &t.tracks.masks {
        if let Ok((s, e)) = env.window(&m.timing) {
            out.push(Overlay { kind: "mask".into(), bbox: bx(&m.rect), start: s, end: e });
        }
    }
    for m in &t.tracks.images {
        if let Ok((s, e)) = env.window(&m.timing) {
            out.push(Overlay { kind: "image".into(), bbox: bx(&m.rect), start: s, end: e });
        }
    }
    for m in &t.tracks.cutaways {
        if let (Some(r), Ok((s, e))) = (&m.rect, env.window(&m.timing)) {
            out.push(Overlay { kind: "cutaway".into(), bbox: bx(r), start: s, end: e });
        }
    }
    let (cw, ch) = (f64::from(clip.canvas.width), f64::from(clip.canvas.height));
    for x in &t.tracks.text {
        out.push(Overlay { kind: "text".into(), bbox: text_box(&x.text, x.position_y, x.font_size_factor, cw, ch), start: x.start, end: x.end });
    }
    Ok(out)
}

/// Grayscale frames at `times` of `video`, `width` wide: one ffmpeg call (`-ss` before each input).
async fn frames_at(paths: &ToolPaths, video: &Path, times: &[f64], width: u32, work: &Path, cancel: &tokio_util::sync::CancellationToken) -> Result<Vec<GrayImage>> {
    const STAGE: &str = "clip.cover";
    let mut cmd = tokio::process::Command::new(&paths.ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    for t in times {
        cmd.arg("-ss").arg(format!("{t:.3}")).arg("-i").arg(video);
    }
    let outs: Vec<PathBuf> = (0..times.len()).map(|k| work.join(format!("s{k:02}.jpg"))).collect();
    for (k, o) in outs.iter().enumerate() {
        cmd.args(["-map", &format!("{k}:v:0"), "-frames:v", "1", "-vf", &format!("scale={width}:-2"), "-q:v", "3"]).arg(o);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let child = cmd.spawn().map_err(|e| RustlerError::new(ErrorCode::DependencyMissing, STAGE, format!("cannot start ffmpeg: {e}")))?;
    let out = tokio::select! {
        r = child.wait_with_output() => r.map_err(|e| RustlerError::new(ErrorCode::Media, STAGE, format!("ffmpeg: {e}")))?,
        _ = cancel.cancelled() => return Err(RustlerError::new(ErrorCode::Cancelled, STAGE, "cancelled")),
    };
    if !out.status.success() {
        return Err(RustlerError::new(ErrorCode::Media, STAGE, format!("ffmpeg could not extract the frames: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    outs.iter()
        .map(|o| {
            image::open(o).map(|i| i.to_luma8()).map_err(|e| RustlerError::new(ErrorCode::Media, STAGE, format!("cannot read frame {}: {e}", o.display())))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn run(
    ctx: &Ctx,
    store: &ProjectStore,
    paths: &ToolPaths,
    clip: &rustler_core::Clip,
    i: &CoverIn,
    src: &Frames<'_>,
    file_dur: f64,
    edit_dur: f64,
    tm: &TimeMap,
    tmap: &rustler_media::compile::TransitionMap,
    work: &Path,
) -> Result<CoverOut> {
    const STAGE: &str = "clip.cover";
    let mut warnings = Vec::new();
    let (video, render_id) = (src.video, src.render_id);
    let (mut frames_from, mut frames_scored, mut left_out): (Option<String>, Option<usize>, Vec<String>) = (None, None, Vec::new());
    let clamp_file = |t: f64| t.clamp(0.0, (file_dur - 0.1).max(0.0));
    let (chosen, method, candidates, faces_measured) = if let Some(t) = i.out_t {
        if !(t.is_finite() && t >= 0.0 && t <= edit_dur + 0.05) {
            return Err(validation(STAGE, format!("out_t must be inside the clip (0 .. {edit_dur:.2}s)"))
                .hint("`rustler clip show <clip_id>` prints the timeline"));
        }
        (Cand { out_t: t, file_t: clamp_file(tmap.map_time(t)), ..Cand::default() }, "manual", Vec::new(), false)
    } else {
        let cuts = boundaries(clip, tm, tmap);
        // Frames: the QC contact sheet's 12 tiles when the render has one, else SAMPLES frames
        // from one ffmpeg call. Same evenly spread render times either way.
        let sheet = src.qc_sheet.as_ref().and_then(|p| image::open(p).ok()).and_then(|im| sheet_tiles(&im.to_luma8(), src.size.0, src.size.1));
        let (frames, file_times, from) = match sheet {
            Some(tiles) => (tiles, sample_times(file_dur, QC_TILES), "qc_contact_sheet"),
            None => {
                let ts: Vec<f64> = if edit_dur < 0.8 { vec![clamp_file(file_dur / 2.0)] } else { sample_times(file_dur, SAMPLES) };
                (frames_at(paths, video, &ts, SCORE_WIDTH, work, &ctx.cancel).await?, ts, "render")
            }
        };
        frames_from = Some(from.to_string());
        frames_scored = Some(frames.len());
        let layers = match overlays(store, clip) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!("layers not read, masks and overlays not avoided: {}", e.message));
                Vec::new()
            }
        };
        // Faces: one engine call for all the frames (same times as the frames).
        let mut faces: Vec<Option<Face>> = vec![None; file_times.len()];
        let mut engine_ok = false;
        match crate::analyze_cmds::spawn_engine(paths).await {
            Ok(e) => {
                let n = file_times.len() as f64;
                let fps = n / file_dur.max(0.1);
                let start = file_times.first().copied().unwrap_or(0.0);
                let params = json!({"video_path": video, "start": start, "end": start + n / fps, "fps": fps});
                match e.call::<_, TrackOut>("faces.track", params, &ctx.progress, &ctx.cancel).await {
                    Ok(r) => {
                        engine_ok = true;
                        for smp in r.samples {
                            if let Some(k) = file_times.iter().position(|t| (t - smp.t).abs() < 0.5 / fps) {
                                faces[k] = smp.faces.into_iter().max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)));
                            }
                        }
                    }
                    Err(err) => warnings.push(format!("faces not measured, the cover is chosen by sharpness only: {}", err.message)),
                }
                e.shutdown().await;
            }
            Err(e) => warnings.push(format!("faces not measured, the cover is chosen by sharpness only: {}", e.message)),
        }
        let mut all: Vec<(Cand, bool, bool)> = Vec::new(); // (cand, mask shown, face covered)
        for (k, (gray, ft)) in frames.iter().zip(&file_times).enumerate() {
            let ft = clamp_file(*ft);
            let out_t = edit_time_of(|t| tmap.map_time(t), ft, edit_dur).clamp(0.0, edit_dur);
            if cuts.iter().any(|(mid, half)| (ft - mid).abs() < half + CUT_MARGIN) {
                left_out.push(format!("{out_t:.2} s: next to a cut or inside a transition"));
                continue;
            }
            let shown: Vec<&Overlay> = layers.iter().filter(|o| out_t >= o.start - 0.05 && out_t < o.end + 0.05).collect();
            let boxes: Vec<(f64, f64, f64, f64)> = shown.iter().map(|o| o.bbox).collect();
            let masked = shown.iter().any(|o| o.kind == "mask");
            let face = faces[k].clone();
            let covered = face.as_ref().is_some_and(|f| covered_share(f, &boxes) >= COVERED_FACE);
            let (sharp, fit, eyes, frontal) = measure_ex(gray, face.as_ref(), &boxes);
            let mut kinds: Vec<String> = shown.iter().map(|o| o.kind.clone()).collect();
            kinds.dedup();
            all.push((Cand { out_t, file_t: ft, sharp, face_fit: fit, eyes, frontal, score: 0.0, overlays: kinds }, masked, covered));
        }
        // Frames with a mask shown, or with the face under a layer, only when nothing else is left.
        let clean = all.iter().filter(|(_, m, c)| !m && !c).count();
        let mut times: Vec<Cand> = Vec::new();
        for (c, masked, covered) in all {
            if clean > 0 && (masked || covered) {
                left_out.push(format!("{:.2} s: {}", c.out_t, if masked { "a mask is shown" } else { "the face is under a mask, card or text" }));
            } else {
                times.push(c);
            }
        }
        if clean == 0 && !times.is_empty() && times.iter().any(|c| !c.overlays.is_empty()) {
            warnings.push("every free frame shows a mask or has the face under a layer; the best of them was used: look at the cover".into());
        }
        if times.is_empty() {
            warnings.push("every sampled frame is next to a cut or inside a transition; using the middle of the clip".into());
            let t = edit_dur / 2.0;
            times.push(Cand { out_t: t, file_t: clamp_file(tmap.map_time(t)), ..Cand::default() });
        }
        if engine_ok && times.iter().all(|c| c.face_fit == 0.0) {
            warnings.push("no face found in any sampled frame; the cover is chosen by sharpness only".into());
        }
        rank(&mut times, engine_ok);
        let best = times[0].clone();
        let views: Vec<CandidateView> = times.iter().take(5).map(view).collect();
        (best, "auto", views, engine_ok)
    };

    // The cover itself: the chosen frame at the render's full size.
    // Forward slashes: the record is read on other machines and by agents.
    let rel = PathBuf::from(format!("clips/{}/cover.jpg", clip.id.as_str()));
    let out = if ctx.dry_run { work.join("cover.jpg") } else { store.resolve(&rel) };
    rustler_media::frame(paths, video, chosen.file_t, clip.canvas.width, &out).await?;
    if !ctx.dry_run {
        let record = json!({
            "file": rel,
            "out_t": round2(chosen.out_t),
            "file_t": round2(chosen.file_t),
            "method": method,
            "score": (chosen.score * 1000.0).round() / 1000.0,
            "faces_measured": faces_measured,
            // The render the frame was taken from: export.clip of another render leaves it out.
            "render_id": render_id,
            "created_at": Utc::now(),
        });
        extras::update(store, &clip.id, KEY, |_| Ok(Some(record)))?;
        log_event(ctx, store, STAGE, json!({"clip_id": clip.id, "out_t": chosen.out_t, "method": method}))?;
    } else {
        // Dry run: keep nothing in the project, but the frame is looked at before `work` is removed.
        let keep = std::env::temp_dir().join(format!("rustler-cover-{}.jpg", clip.id.as_str()));
        let _ = std::fs::copy(&out, &keep);
        return Ok(CoverOut {
            clip_id: clip.id.clone(),
            image: keep,
            out_t: round2(chosen.out_t),
            file_t: round2(chosen.file_t),
            method: method.into(),
            faces_measured,
            candidates,
            frames_from,
            frames_scored,
            left_out,
            elapsed_s: 0.0,
            dry_run: Some(true),
            warnings,
            next: Vec::new(),
        });
    }
    let image = store.clip_dir(&clip.id).join("cover.jpg"); // native separators for the caller
    Ok(CoverOut {
        clip_id: clip.id.clone(),
        image,
        out_t: round2(chosen.out_t),
        file_t: round2(chosen.file_t),
        method: method.into(),
        faces_measured,
        candidates,
        frames_from,
        frames_scored,
        left_out,
        elapsed_s: 0.0,
        dry_run: None,
        warnings,
        next: vec![
            "open the image and look at it; try another moment with `rustler clip cover <clip_id> --out-t <seconds>`".into(),
            format!("rustler export clip {}", clip.id),
        ],
    })
}

// ---------------------------------------------------------------------------------------------
// Export hook
// ---------------------------------------------------------------------------------------------

/// The render the clip's cover frame was taken from: the recorded `render_id`, or for a cover
/// recorded before that field existed, the latest render made before the cover (`clip.cover`
/// always uses the latest render). `None` without a cover or when it cannot be told.
pub(crate) fn cover_render(store: &ProjectStore, clip: &rustler_core::Clip) -> Option<rustler_core::RenderId> {
    let rec = extras::get::<Value>(store, &clip.id, KEY).ok().flatten()?;
    if let Some(id) = rec.get("render_id").and_then(|v| serde_json::from_value::<rustler_core::RenderId>(v.clone()).ok()) {
        return Some(id);
    }
    let at = rec.get("created_at").and_then(Value::as_str).and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok())?;
    clip.renders.iter().filter(|r| r.created_at <= at).max_by_key(|r| r.created_at).map(|r| r.id.clone())
}

/// Copies the clip's cover next to the exported video as `<name>.cover.jpg` and points the
/// post JSON's `cover` at it. A clip without a cover is left alone; a recorded cover whose file
/// is gone adds a warning.
pub(crate) fn export_cover(store: &ProjectStore, clip: &rustler_core::Clip, dir: &Path, name: &str, body: &mut Value, warnings: &mut Vec<String>) {
    let Ok(Some(rec)) = extras::get::<Value>(store, &clip.id, KEY) else { return };
    let Some(file) = rec.get("file").and_then(Value::as_str) else { return };
    let src = store.resolve(Path::new(file));
    if !src.is_file() {
        warnings.push(format!("the cover {} is missing; run `rustler clip cover {}` again", src.display(), clip.id));
        return;
    }
    let dest_name = format!("{name}.cover.jpg");
    match std::fs::copy(&src, dir.join(&dest_name)) {
        Ok(_) => {
            let mut rec = rec;
            rec["file"] = json!(dest_name);
            body["cover"] = rec;
        }
        Err(e) => warnings.push(format!("cannot copy the cover to {}: {e}", dir.join(&dest_name).display())),
    }
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    reg.register(typed(
        "clip.cover",
        "Pick the cover frame of the clip's latest render (sharp, face visible and facing the camera, not on a cut or transition) and save it as clips/<id>/cover.jpg; export copies it as <name>.cover.jpg and names it in the post JSON. Look at the image it returns",
        &["clip_id"],
        write,
        vec![
            Example { description: "auto-pick the best frame", cli: "rustler clip cover clp_x --auto" },
            Example { description: "use the frame at 6.5 s", cli: "rustler clip cover clp_x --out-t 6.5" },
        ],
        clip_cover,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn checker(w: u32, h: u32, cell: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, y| Luma([if (x / cell + y / cell) % 2 == 0 { 30 } else { 220 }]))
    }

    fn blur(g: &GrayImage) -> GrayImage {
        image::imageops::blur(g, 3.0)
    }

    #[test]
    fn sharp_beats_blurred_and_flat_is_zero() {
        let sharp = checker(200, 200, 4);
        let soft = blur(&sharp);
        let flat = GrayImage::from_pixel(200, 200, Luma([128]));
        let r = (0, 0, 200, 200);
        let (a, b, c) = (laplacian_variance(&sharp, r), laplacian_variance(&soft, r), laplacian_variance(&flat, r));
        assert!(a > b * 3.0, "sharp {a} vs blurred {b}");
        assert_eq!(c, 0.0);
        assert_eq!(laplacian_variance(&sharp, (50, 50, 50, 50)), 0.0, "empty rect");
    }

    #[test]
    fn patch_std_is_contrast() {
        let g = checker(100, 100, 5);
        assert!(patch_std(&g, 50.0, 50.0, 8.0) > 80.0);
        let flat = GrayImage::from_pixel(100, 100, Luma([100]));
        assert_eq!(patch_std(&flat, 50.0, 50.0, 8.0), 0.0);
    }

    #[test]
    fn face_fit_prefers_a_medium_centred_face() {
        let good = Face { x: 0.35, y: 0.2, w: 0.3, h: 0.2, score: 0.9, lm: None };
        let tiny = Face { x: 0.45, y: 0.3, w: 0.05, h: 0.05, score: 0.9, lm: None };
        let edge = Face { x: 0.85, y: 0.2, w: 0.14, h: 0.14, score: 0.9, lm: None };
        let low = Face { x: 0.35, y: 0.8, w: 0.3, h: 0.15, score: 0.9, lm: None };
        assert_eq!(face_fit(&good), 1.0);
        assert!(face_fit(&tiny) < 0.2);
        assert!(face_fit(&edge) < face_fit(&good) * 0.6);
        assert!(face_fit(&low) < face_fit(&good) * 0.6, "under the caption line: {}", face_fit(&low));
    }

    #[test]
    fn rank_prefers_face_sharp_open_eyes_and_falls_back_to_sharpness() {
        let mk = |out_t: f64, sharp: f64, fit: f64, eyes: Option<f64>, frontal: Option<f64>| Cand {
            out_t,
            file_t: out_t,
            sharp,
            face_fit: fit,
            eyes,
            frontal,
            score: 0.0,
            overlays: Vec::new(),
        };
        // 0: sharp but no face; 1: face, eyes closed (low contrast); 2: face, eyes open; 3: face, blurred
        let mut c = vec![
            mk(0.0, 900.0, 0.0, None, None),
            mk(1.0, 500.0, 1.0, Some(8.0), Some(1.0)),
            mk(2.0, 480.0, 1.0, Some(30.0), Some(0.9)),
            mk(3.0, 40.0, 1.0, Some(30.0), Some(0.9)),
        ];
        rank(&mut c, true);
        assert_eq!(c[0].out_t, 2.0, "{c:?}");
        assert_eq!(c[1].out_t, 1.0, "closed eyes rank below open eyes");
        assert!(c.iter().all(|x| (0.0..=1.0).contains(&x.score)));
        // no engine: sharpness only
        let mut s = vec![mk(0.0, 900.0, 0.0, None, None), mk(1.0, 500.0, 1.0, Some(8.0), Some(1.0))];
        rank(&mut s, false);
        assert_eq!(s[0].out_t, 0.0);
        // a single candidate is fine
        let mut one = vec![mk(0.0, 5.0, 0.0, None, None)];
        rank(&mut one, true);
        assert_eq!(one[0].score, 1.0);
    }

    #[test]
    fn sample_times_are_slice_middles_and_map_back_to_edit_time() {
        assert_eq!(sample_times(12.0, 4), vec![1.5, 4.5, 7.5, 10.5]);
        assert!(sample_times(0.0, 4).is_empty());
        // A render 1 s shorter than the edit (a 1 s overlap at 5 s): rendered = t - 1 after 6 s.
        let map = |t: f64| if t < 5.0 { t } else if t < 6.0 { 5.0 } else { t - 1.0 };
        assert!((edit_time_of(map, 3.0, 10.0) - 3.0).abs() < 1e-6);
        assert!((edit_time_of(map, 7.0, 10.0) - 8.0).abs() < 1e-6);
    }

    #[test]
    fn covered_share_and_text_box() {
        let f = Face { x: 0.4, y: 0.2, w: 0.2, h: 0.2, score: 0.9, lm: None };
        assert_eq!(covered_share(&f, &[]), 0.0);
        assert!((covered_share(&f, &[(0.5, 0.0, 1.0, 1.0)]) - 0.5).abs() < 1e-9);
        assert_eq!(covered_share(&f, &[(0.0, 0.0, 1.0, 1.0), (0.0, 0.0, 1.0, 1.0)]), 1.0);
        // A 0.1-wide hook title at 0.12: about 1-2 lines, centred, inside the top quarter.
        let b = text_box("He quit his job at 25", 0.12, 0.1, 1080.0, 1920.0);
        assert!(b.0 > 0.0 && b.2 < 1.0 && (b.0 + b.2 - 1.0).abs() < 1e-9, "{b:?}");
        assert!(b.1 > 0.0 && b.3 < 0.25 && b.1 < 0.12 && b.3 > 0.12, "{b:?}");
    }

    #[test]
    fn layer_areas_do_not_count_toward_sharpness() {
        // Flat picture with a crisp checker (a text box) in the top half.
        let mut g = GrayImage::from_pixel(100, 100, Luma([100]));
        for y in 10..40 {
            for x in 10..90 {
                g.put_pixel(x, y, Luma(if (x / 2 + y / 2) % 2 == 0 { [0] } else { [255] }));
            }
        }
        let all = laplacian_variance(&g, (0, 0, 100, 100));
        let without = laplacian_variance_ex(&g, (0, 0, 100, 100), &[(8, 8, 92, 42)]);
        assert!(all > 1000.0 && without < 1.0, "{all} {without}");
        assert_eq!(laplacian_variance_ex(&g, (0, 0, 100, 100), &[(0, 0, 100, 100)]), 0.0);
    }

    #[test]
    fn qc_sheet_is_cut_into_its_tiles() {
        // 4 x 3 tiles of 270 x 480 (a 1080x1920 render) with a 36 px label strip.
        let mut sheet = GrayImage::from_pixel(1080, 3 * (480 + 36), Luma([16]));
        for i in 0..12u32 {
            let (x0, y0) = ((i % 4) * 270, (i / 4) * 516);
            for y in y0..y0 + 480 {
                for x in x0..x0 + 270 {
                    sheet.put_pixel(x, y, Luma([(i * 20) as u8]));
                }
            }
        }
        let tiles = sheet_tiles(&sheet, 1080, 1920).unwrap();
        assert_eq!(tiles.len(), 12);
        assert_eq!(tiles[5].dimensions(), (270, 480));
        assert_eq!(tiles[5].get_pixel(100, 479).0[0], 100);
        assert_eq!(tiles[11].get_pixel(0, 0).0[0], 220);
        assert!(sheet_tiles(&sheet, 1920, 1080).is_none(), "another layout is not reused");
        assert!(sheet_tiles(&GrayImage::new(800, 600), 1080, 1920).is_none());
    }

    #[test]
    fn measure_uses_the_face_box_and_landmarks() {
        // dark frame, sharp checker only inside the face box
        let mut g = GrayImage::from_pixel(200, 400, Luma([100]));
        for y in 80..160 {
            for x in 60..140 {
                g.put_pixel(x, y, Luma(if (x / 4 + y / 4) % 2 == 0 { [20] } else { [230] }));
            }
        }
        let face = Face {
            x: 0.3,
            y: 0.2,
            w: 0.4,
            h: 0.2,
            score: 0.9,
            lm: Some(vec![[0.4, 0.25], [0.6, 0.25], [0.5, 0.3], [0.42, 0.35], [0.58, 0.35]]),
        };
        let (sharp, fit, eyes, frontal) = measure(&g, Some(&face));
        assert!(sharp > 1000.0, "{sharp}");
        assert!(fit > 0.9);
        assert!(eyes.unwrap() > 20.0);
        assert!(frontal.unwrap() > 0.99);
        let (s2, f2, e2, fr2) = measure(&g, None);
        assert!(s2 < sharp && f2 == 0.0 && e2.is_none() && fr2.is_none());
    }
}
