//! Command: clip.cover. Picks the best frame of a clip's latest render for a cover / thumbnail:
//! sharp (Laplacian variance), a face in a good place and facing the camera with the eyes
//! showing contrast (YuNet through the engine, when it is available), and not on a cut or inside
//! a transition. The JPG goes to `clips/<id>/cover.jpg`, the choice is recorded in the clip's
//! extras under `cover`, and `export.clip` copies it next to the video as `<name>.cover.jpg`
//! and points `<name>.post.json`'s `cover` at it.
//!
//! The frame comes from the render, so captions and the hook title are in the picture.

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
/// Most frames looked at.
const MAX_CANDIDATES: usize = 40;
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
            let l = px(x - 1, y) + px(x + 1, y) + px(x, y - 1) + px(x, y + 1) - 4.0 * px(x, y);
            sum += l;
            sum2 += l * l;
            n += 1.0;
        }
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
    let (w, h) = (f64::from(gray.width()), f64::from(gray.height()));
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
        None => (laplacian_variance(gray, ((w * 0.1) as u32, (h * 0.2) as u32, (w * 0.9) as u32, (h * 0.6) as u32)), 0.0, None, None),
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

/// Edit-timeline times to look at: every `step` seconds from 0.3 s to 0.3 s before the end, at
/// most [`MAX_CANDIDATES`]; a clip shorter than 0.8 s gives its middle.
pub fn candidate_times(edit_duration: f64) -> Vec<f64> {
    if edit_duration < 0.8 {
        return vec![edit_duration / 2.0];
    }
    let step = (edit_duration / MAX_CANDIDATES as f64).max(0.5);
    let mut v = Vec::new();
    let mut t = 0.3;
    while t <= edit_duration - 0.3 + 1e-9 {
        v.push(t);
        t += step;
    }
    v
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
    let result = run(&ctx, &store, &paths, &clip, &i, &video, file_dur, edit_dur, &tm, &tmap, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[allow(clippy::too_many_arguments)]
async fn run(
    ctx: &Ctx,
    store: &ProjectStore,
    paths: &ToolPaths,
    clip: &rustler_core::Clip,
    i: &CoverIn,
    video: &Path,
    file_dur: f64,
    edit_dur: f64,
    tm: &TimeMap,
    tmap: &rustler_media::compile::TransitionMap,
    work: &Path,
) -> Result<CoverOut> {
    const STAGE: &str = "clip.cover";
    let mut warnings = Vec::new();
    let clamp_file = |t: f64| t.clamp(0.0, (file_dur - 0.1).max(0.0));
    let (chosen, method, candidates, faces_measured) = if let Some(t) = i.out_t {
        if !(t.is_finite() && t >= 0.0 && t <= edit_dur + 0.05) {
            return Err(validation(STAGE, format!("out_t must be inside the clip (0 .. {edit_dur:.2}s)"))
                .hint("`rustler clip show <clip_id>` prints the timeline"));
        }
        (Cand { out_t: t, file_t: clamp_file(tmap.map_time(t)), ..Cand::default() }, "manual", Vec::new(), false)
    } else {
        let cuts = boundaries(clip, tm, tmap);
        let mut times: Vec<Cand> = candidate_times(edit_dur)
            .into_iter()
            .map(|t| Cand { out_t: t, file_t: clamp_file(tmap.map_time(t)), ..Cand::default() })
            .filter(|c| !cuts.iter().any(|(mid, half)| (c.file_t - mid).abs() < half + CUT_MARGIN))
            .collect();
        if times.is_empty() {
            warnings.push("every sampled frame is next to a cut or inside a transition; using the middle of the clip".into());
            let t = edit_dur / 2.0;
            times.push(Cand { out_t: t, file_t: clamp_file(tmap.map_time(t)), ..Cand::default() });
        }
        // The engine is optional: without it only sharpness counts.
        let engine = match crate::analyze_cmds::spawn_engine(paths).await {
            Ok(e) => Some(e),
            Err(e) => {
                warnings.push(format!("faces not measured, the cover is chosen by sharpness only: {}", e.message));
                None
            }
        };
        let mut engine_ok = engine.is_some();
        for (n, c) in times.iter_mut().enumerate() {
            let jpg = work.join(format!("c{n}.jpg"));
            rustler_media::frame(paths, video, c.file_t, SCORE_WIDTH, &jpg).await?;
            let gray = image::open(&jpg)
                .map_err(|e| RustlerError::new(ErrorCode::Media, STAGE, format!("cannot read frame {}: {e}", jpg.display())))?
                .to_luma8();
            let mut face = None;
            if let (Some(e), true) = (&engine, engine_ok) {
                let params = json!({"video_path": video, "start": c.file_t, "end": c.file_t + 1.0, "fps": 1.0});
                match e.call::<_, TrackOut>("faces.track", params, &ctx.progress, &ctx.cancel).await {
                    Ok(r) => {
                        face = r.samples.into_iter().next().and_then(|s| {
                            s.faces.into_iter().max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
                        })
                    }
                    Err(err) => {
                        engine_ok = false;
                        warnings.push(format!("faces not measured, the cover is chosen by sharpness only: {}", err.message));
                    }
                }
            }
            let (sharp, fit, eyes, frontal) = measure(&gray, face.as_ref());
            *c = Cand { sharp, face_fit: fit, eyes, frontal, ..c.clone() };
        }
        if let Some(e) = engine {
            e.shutdown().await;
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
            dry_run: Some(true),
            warnings,
            next: Vec::new(),
        });
    }
    let image = store.resolve(&rel);
    Ok(CoverOut {
        clip_id: clip.id.clone(),
        image,
        out_t: round2(chosen.out_t),
        file_t: round2(chosen.file_t),
        method: method.into(),
        faces_measured,
        candidates,
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
    fn candidate_times_stay_inside_and_are_capped() {
        let t = candidate_times(30.0);
        assert!((t[0] - 0.3).abs() < 1e-9 && *t.last().unwrap() <= 29.7 + 1e-9);
        assert!(t.len() <= MAX_CANDIDATES + 1);
        assert_eq!(candidate_times(0.5), vec![0.25]);
        let long = candidate_times(600.0);
        assert!(long.len() <= MAX_CANDIDATES + 1, "{}", long.len());
        assert!(candidate_times(3.0).windows(2).all(|w| (w[1] - w[0] - 0.5).abs() < 1e-9));
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
