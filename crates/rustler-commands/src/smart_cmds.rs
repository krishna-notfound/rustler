//! Commands: crop.track_active, layout.plan; the smart defaults that `clip.create` applies.
//!
//! Goal: a clip comes out right with no manual layout or framing calls, while the agent can
//! override every automatic decision. Everything here is lightweight (lite install: OpenCV +
//! YuNet in the engine, no diarization or recognition model):
//!
//! * **Light active speaker** (`crop.track_active`): the engine's `faces.active` gives faces with
//!   landmarks and a mouth-motion energy per face (mouth rectangle from the mouth corners, head
//!   motion removed). Here the energy is compared between the visible faces while somebody speaks
//!   (transcript words, else the audio silences), with hysteresis: a new face must lead clearly for
//!   at least [`MIN_TURN`] seconds, an ambiguous lead keeps the current face. The turns feed the
//!   smooth camera planner (`rustler_media::camera`) as speaker switches.
//! * **Shot plan** (`layout.plan`, smart defaults): shots from `scenes.json` plus a luma-normalised
//!   frame-difference scan (dark footage cuts too); per shot faces=1 -> center-crop + smooth track,
//!   faces>=2 -> center-crop + active speaker, no face / graphics -> blur-fit (zoom-fit when the
//!   content is centred). Segments are split only where the layout changes.
//! * **Cleanup**: filler words from `data/fillers_{en,hi}.txt` (conservative rules) and pauses
//!   longer than [`PAUSE_MIN_GAP`] trimmed to [`PAUSE_KEEP`].
//!
//! Each step of the smart defaults is its own clip revision, and the report lists every decision
//! with the exact command that undoes or overrides it.

use crate::analyze_cmds::spawn_engine;
use crate::clip_cmds::{load_transcript, scene_cut_events, tracking_crop_width};
use crate::extras;
use crate::project_cmds::log_event;
use crate::speaker_cmds::{ActiveFace, ActiveSample, TurnChoice, speaker_camera, speaker_events, speaker_face_samples};
use rustler_core::layout::{self, LayoutParams};
use rustler_core::{
    AudioEvents, Background, Canvas, Clip, ClipId, ErrorCode, Fit, Layout, ProjectStore, Rect, Region, RustlerError, Segment, SegmentId,
    Source, SourceId, TimeMap, Word, WordTiming,
};
use rustler_engine_bridge::EngineClient;
use rustler_media::camera::{self, CamEvent, CamFace, CamSample, CameraPlan, CameraStyle, EventKind, Landmarks, SwitchStyle};
use rustler_media::compile::CropKey;
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// Sampling rate of `faces.active` (faces + mouth motion).
pub const ACTIVE_FPS: f64 = 5.0;
/// Faces narrower than this share of the frame width are ignored (background people, posters).
pub const MIN_FACE_W: f64 = 0.025;
/// ... and faces narrower than this share of the largest face in the same sample.
pub const REL_FACE_W: f64 = 0.4;
/// Shortest speaker turn: a new face must lead this long before the camera switches.
pub const MIN_TURN: f64 = 1.0;
/// A lead counts only when the best face beats the runner-up by this share of its own score;
/// below it the choice is ambiguous and the current face is kept.
pub const AMBIGUITY_MARGIN: f64 = 0.3;
/// Mouth energy (after the per-track baseline) below this is no evidence of speech.
pub const MIN_ENERGY: f64 = 0.01;
/// Mouth energy is averaged over this window (seconds, centred).
pub const ENERGY_WINDOW: f64 = 1.0;
/// The current face missing this long (seconds) while other faces are visible: follow them.
pub const LOST_AFTER: f64 = 0.6;
/// Cut candidates from the scan: frame difference at least this ...
pub const CUT_MIN_DIFF: f64 = 0.3;
/// ... and this many times the local median difference (plus 0.08).
pub const CUT_RATIO: f64 = 2.5;
/// A layout change shorter than this (seconds) is absorbed by its neighbour (no flicker).
pub const MIN_SHOT: f64 = 1.2;
/// Graphics: share of flat 8x8 blocks and edge density (medians over the shot).
pub const GRAPHICS_FLAT: f64 = 0.55;
pub const GRAPHICS_EDGES: f64 = 0.008;
/// Pause trim: word gaps longer than this (seconds) ...
pub const PAUSE_MIN_GAP: f64 = 1.0;
/// ... are cut down to this much silence.
pub const PAUSE_KEEP: f64 = 0.15;
/// Guarded fillers ("~ you know") need this much silence before and after.
pub const FILLER_PAUSE: f64 = 0.3;
/// Key of the clip's `extras.json` with the last smart-defaults report.
pub const SMART_KEY: &str = "smart_defaults";

const FILLERS_EN: &str = include_str!("../data/fillers_en.txt");
const FILLERS_HI: &str = include_str!("../data/fillers_hi.txt");

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn verr(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Engine result (`faces.active`)
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct ScanFace {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub lm: Option<Landmarks>,
    /// Mouth-motion energy (head motion removed, contrast-normalised); 0 without `motion`.
    #[serde(default)]
    pub mouth: f64,
    /// Positional track: one face in one shot.
    #[serde(default)]
    pub track: u32,
}

impl ScanFace {
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
}

/// Graphics statistics of one frame.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct GraphicsStats {
    pub flat: f64,
    pub edges: f64,
    pub x_lo: f64,
    pub x_hi: f64,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct ScanSample {
    pub t: f64,
    pub faces: Vec<ScanFace>,
    #[serde(default)]
    pub g: Option<GraphicsStats>,
}

/// A local maximum of the frame-to-frame difference and its local median.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct ScanPeak {
    pub t: f64,
    pub diff: f64,
    #[serde(default)]
    pub base: f64,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct ScanResult {
    pub samples: Vec<ScanSample>,
    #[serde(default)]
    pub peaks: Vec<ScanPeak>,
}

/// Faces that count in a sample: wide enough in absolute terms and next to the largest face.
pub fn candidates(s: &ScanSample) -> Vec<&ScanFace> {
    let max_w = s.faces.iter().map(|f| f.w).fold(0.0, f64::max);
    s.faces.iter().filter(|f| f.w >= MIN_FACE_W && f.w >= REL_FACE_W * max_w).collect()
}

// ---------------------------------------------------------------------------------------------
// Speech activity
// ---------------------------------------------------------------------------------------------

/// Where the speech activity came from.
#[derive(Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SpeechSource {
    /// Transcript word timings.
    Transcript,
    /// Audio silences (`analyze audio`).
    Audio,
    /// Nothing known: every sample counts as speech (mouth motion alone decides).
    None,
}

impl SpeechSource {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Audio => "audio",
            Self::None => "mouth motion only",
        }
    }
}

/// Per sample: is somebody speaking? Words win (a sample within 0.15 s of a word), then the
/// audio silences, else everything counts as speech.
pub fn speech_mask(times: &[f64], words: &[Word], silences: Option<&[rustler_core::Silence]>) -> (Vec<bool>, SpeechSource) {
    if !words.is_empty() {
        let mask = times.iter().map(|&t| words.iter().any(|w| w.start - 0.15 <= t && t <= w.end + 0.15)).collect();
        return (mask, SpeechSource::Transcript);
    }
    if let Some(sil) = silences {
        let mask = times.iter().map(|&t| !sil.iter().any(|s| s.start <= t && t < s.end)).collect();
        return (mask, SpeechSource::Audio);
    }
    (vec![true; times.len()], SpeechSource::None)
}

/// Word starts that make good switch points (after a gap or after sentence punctuation).
pub fn switch_points(words: &[Word]) -> Vec<f64> {
    let mut out = Vec::new();
    for (k, w) in words.iter().enumerate() {
        let good = k == 0 || {
            let prev = &words[k - 1];
            w.start - prev.end >= 0.1 || prev.text.trim_end().ends_with(['.', '?', '!', ','])
        };
        if good {
            out.push(w.start);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Light active speaker (pure)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveParams {
    pub min_turn: f64,
    pub margin: f64,
    pub min_energy: f64,
    pub window: f64,
    pub lost_after: f64,
    /// Preferred switch times (word starts after a pause); a confirmed switch snaps to the nearest
    /// one within 0.6 s before / 0.3 s after the start of the lead.
    pub switch_points: Vec<f64>,
}

impl Default for ActiveParams {
    fn default() -> Self {
        Self {
            min_turn: MIN_TURN,
            margin: AMBIGUITY_MARGIN,
            min_energy: MIN_ENERGY,
            window: ENERGY_WINDOW,
            lost_after: LOST_AFTER,
            switch_points: Vec::new(),
        }
    }
}

/// Who is framed from `start` to `end` (source time).
#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ActiveTurn {
    pub start: f64,
    pub end: f64,
    /// Positional face track (one face in one shot), `None` when no face was visible at all.
    pub face: Option<u32>,
    /// Mean horizontal centre of that face during the turn (0..1).
    pub face_x: Option<f64>,
    /// Mean smoothed mouth energy of the face during the turn.
    pub score: f64,
    /// Mean lead over the runner-up face (0 when alone).
    pub margin: f64,
    /// `only-face`, `mouth-motion` (a clear lead held >= 1 s), `largest-face` (no clear evidence),
    /// `shot-change` (the framed face left the picture; the new shot's best face).
    pub reason: String,
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * q).round() as usize]
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    Some(percentile(&mut v, 0.5))
}

/// Per sample: smoothed mouth energy per present track (`None`: no speech evidence in the window),
/// and the raw weighted energy per track (0 outside speech).
type EnergyRows = (Vec<BTreeMap<u32, Option<f64>>>, Vec<BTreeMap<u32, f64>>);

/// Mouth energy minus the per-track baseline (capped p10), averaged over `window` during speech.
fn smoothed_energy(samples: &[ScanSample], speech: &[bool], window: f64) -> EnergyRows {
    // Per-track baseline (capped p10) and a speech-correlation weight.
    let mut all: BTreeMap<u32, Vec<f64>> = BTreeMap::new();
    for s in samples {
        for f in candidates(s) {
            all.entry(f.track).or_default().push(f.mouth);
        }
    }
    // Baseline: the track's p10, capped near the quietest track's floor so that a face that talks all
    // the time is not "normalised away" (it only removes chronic noise such as texture or codec).
    let p10: BTreeMap<u32, f64> = all.iter().map(|(k, v)| (*k, percentile(&mut v.clone(), 0.1))).collect();
    let floor = all.iter().filter(|(_, v)| v.len() >= 5).map(|(k, _)| p10[k]).fold(f64::INFINITY, f64::min);
    let floor = if floor.is_finite() { floor } else { 0.0 };
    let base: BTreeMap<u32, f64> = p10.iter().map(|(k, v)| (*k, v.min(2.0 * floor + 0.005))).collect();
    let mut on: BTreeMap<u32, (f64, usize)> = BTreeMap::new();
    let mut off: BTreeMap<u32, (f64, usize)> = BTreeMap::new();
    let energy: Vec<BTreeMap<u32, f64>> = samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            candidates(s)
                .into_iter()
                .map(|f| {
                    let e = (f.mouth - base.get(&f.track).copied().unwrap_or(0.0)).max(0.0);
                    let acc = if speech.get(i).copied().unwrap_or(true) { &mut on } else { &mut off };
                    let a = acc.entry(f.track).or_insert((0.0, 0));
                    a.0 += e;
                    a.1 += 1;
                    (f.track, e)
                })
                .collect()
        })
        .collect();
    // A face whose mouth moves as much in silence as during speech (chewing, laughing, nodding)
    // is less likely to be the voice.
    let weight: BTreeMap<u32, f64> = all
        .keys()
        .map(|k| {
            let (son, non) = on.get(k).copied().unwrap_or((0.0, 0));
            let (soff, noff) = off.get(k).copied().unwrap_or((0.0, 0));
            let w = if noff >= 5 && non >= 5 && soff / noff as f64 >= 0.8 * (son / non as f64) { 0.6 } else { 1.0 };
            (*k, w)
        })
        .collect();
    let half = window / 2.0;
    let mut out = Vec::with_capacity(samples.len());
    let mut lo = 0usize;
    for s in samples {
        while samples[lo].t < s.t - half - 1e-9 {
            lo += 1;
        }
        let mut row = BTreeMap::new();
        for f in candidates(s) {
            let (mut sum, mut n) = (0.0, 0usize);
            for j in lo..samples.len() {
                if samples[j].t > s.t + half + 1e-9 {
                    break;
                }
                if !speech.get(j).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(e) = energy[j].get(&f.track) {
                    sum += e;
                    n += 1;
                }
            }
            let w = weight.get(&f.track).copied().unwrap_or(1.0);
            row.insert(f.track, (n > 0).then(|| w * sum / n as f64));
        }
        out.push(row);
    }
    let raw = energy
        .iter()
        .enumerate()
        .map(|(j, r)| {
            let on = speech.get(j).copied().unwrap_or(true);
            r.iter().map(|(k, e)| (*k, if on { e * weight.get(k).copied().unwrap_or(1.0) } else { 0.0 })).collect()
        })
        .collect();
    (out, raw)
}

/// Ranked `(track, score)` of the present faces, best first (no evidence = -1).
fn ranked(row: &BTreeMap<u32, Option<f64>>) -> Vec<(u32, f64)> {
    let mut v: Vec<(u32, f64)> = row.iter().map(|(k, s)| (*k, s.unwrap_or(-1.0))).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

fn largest(s: &ScanSample) -> Option<u32> {
    candidates(s).into_iter().max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h))).map(|f| f.track)
}

/// Clear leader of a row: the best face beats the runner-up by `margin` of its score and has
/// at least `min_energy`.
fn clear_leader(row: &BTreeMap<u32, Option<f64>>, p: &ActiveParams) -> Option<(u32, f64)> {
    let r = ranked(row);
    let (best, top) = *r.first()?;
    if r.len() < 2 || top < p.min_energy {
        return None;
    }
    let second = r[1].1.max(0.0);
    (top - second >= p.margin * top).then_some((best, top - second))
}

fn snap(t: f64, points: &[f64]) -> f64 {
    points
        .iter()
        .copied()
        .filter(|&b| b >= t - 0.6 && b <= t + 0.3)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        .unwrap_or(t)
}

/// Decide who is framed when. `end` is the end of the range (source time). Pure.
pub fn decide_active(samples: &[ScanSample], speech: &[bool], end: f64, p: &ActiveParams) -> Vec<ActiveTurn> {
    if samples.is_empty() {
        return Vec::new();
    }
    let t0 = samples[0].t;
    let dt = if samples.len() > 1 { ((samples[samples.len() - 1].t - t0) / (samples.len() - 1) as f64).max(1e-3) } else { 0.2 };
    let (rows, raw) = smoothed_energy(samples, speech, p.window);
    // (start, track, reason)
    let mut cuts: Vec<(f64, Option<u32>, &'static str)> = Vec::new();
    let mut cur: Option<u32> = None;
    let mut cur_start = t0;
    let mut absent_since: Option<f64> = None;
    // challenger: (track, smoothed lead start, supporting samples, last support time, first and
    // last sample where its raw mouth energy clearly beats the current face's)
    let mut ch: Option<(u32, f64, usize, f64, Option<(f64, f64)>)> = None;
    let mut seen: HashSet<u32> = HashSet::new();
    for (i, s) in samples.iter().enumerate() {
        let row = &rows[i];
        if row.is_empty() {
            continue; // nobody on screen: hold
        }
        let t = s.t;
        let pick = || clear_leader(row, p).map(|(id, _)| (id, "mouth-motion")).or_else(|| largest(s).map(|id| (id, "largest-face")));
        match cur {
            None => {
                if let Some((id, why)) = pick() {
                    cur = Some(id);
                    cut_push(&mut cuts, t0, Some(id), why);
                    cur_start = t0;
                }
            }
            Some(c) if !row.contains_key(&c) => {
                let since = *absent_since.get_or_insert(t);
                let all_new = row.keys().all(|k| !seen.contains(k));
                if t - since >= p.lost_after || all_new {
                    if let Some((id, _)) = pick() {
                        let at = if all_new { t - dt / 2.0 } else { since };
                        let at = at.max(cur_start);
                        cur = Some(id);
                        cur_start = at;
                        cut_push(&mut cuts, at, Some(id), "shot-change");
                    }
                    absent_since = None;
                    ch = None;
                }
            }
            Some(c) => {
                absent_since = None;
                match clear_leader(row, p) {
                    Some((l, _)) if l != c => {
                        ch = match ch {
                            Some((id, st, sup, _, span)) if id == l => Some((id, st, sup + 1, t, span)),
                            _ => Some((l, t, 1, t, None)),
                        };
                    }
                    Some(_) => ch = None,
                    None => {}
                }
                if let Some((id, _, _, _, span)) = ch.as_mut() {
                    let (rc, rn) = (raw[i].get(id).copied().unwrap_or(0.0), raw[i].get(&c).copied().unwrap_or(0.0));
                    if rc >= p.min_energy * 0.5 && rc > rn * (1.0 + p.margin) {
                        *span = Some(span.map_or((t, t), |(a, _)| (a, t)));
                    }
                }
                if let Some((id, st, sup, last, span)) = ch {
                    let (r0, r1) = span.unwrap_or((t, t - 1.0));
                    if t - last > 0.6 * p.min_turn {
                        ch = None;
                    } else if t - st >= p.min_turn - 1e-9 && r1 - r0 + dt >= p.min_turn - 1e-9 && sup as f64 >= 0.6 * ((t - st) / dt + 1.0) {
                        let at = snap(r0, &p.switch_points).max(cur_start + p.min_turn).min(t);
                        cur = Some(id);
                        cur_start = at;
                        cut_push(&mut cuts, at, Some(id), "mouth-motion");
                        ch = None;
                    }
                }
            }
        }
        for k in row.keys() {
            seen.insert(*k);
        }
    }
    if cuts.is_empty() {
        return vec![ActiveTurn { start: t0, end, face: None, face_x: None, score: 0.0, margin: 0.0, reason: "no-face".into() }];
    }
    let mut turns = Vec::new();
    for (k, &(start, face, reason)) in cuts.iter().enumerate() {
        let stop = cuts.get(k + 1).map_or(end, |c| c.0);
        if stop - start <= 1e-6 {
            continue;
        }
        let (mut sx, mut nx, mut ss, mut ns, mut sm, mut alone) = (0.0, 0usize, 0.0, 0usize, 0.0, true);
        for (i, s) in samples.iter().enumerate().filter(|(_, s)| s.t >= start && s.t < stop) {
            let cands = candidates(s);
            if cands.len() > 1 {
                alone = false;
            }
            if let Some(f) = cands.iter().find(|f| Some(f.track) == face) {
                sx += f.cx();
                nx += 1;
            }
            if let Some(Some(sc)) = face.and_then(|id| rows[i].get(&id)) {
                let other = rows[i].iter().filter(|(k, _)| Some(**k) != face).filter_map(|(_, v)| *v).fold(0.0, f64::max);
                ss += sc;
                sm += sc - other;
                ns += 1;
            }
        }
        let reason = if alone && nx > 0 { "only-face" } else { reason };
        turns.push(ActiveTurn {
            start: round3(start),
            end: round3(stop),
            face,
            face_x: (nx > 0).then(|| round3(sx / nx as f64)),
            score: if ns > 0 { round3(ss / ns as f64) } else { 0.0 },
            margin: if ns > 0 && !alone { round3(sm / ns as f64) } else { 0.0 },
            reason: reason.to_string(),
        });
    }
    // Merge consecutive turns on the same face (e.g. a shot change back to the same track id).
    let mut merged: Vec<ActiveTurn> = Vec::new();
    for t in turns {
        match merged.last_mut() {
            Some(m) if m.face == t.face => m.end = t.end,
            _ => merged.push(t),
        }
    }
    merged
}

fn cut_push(cuts: &mut Vec<(f64, Option<u32>, &'static str)>, t: f64, face: Option<u32>, why: &'static str) {
    if let Some(last) = cuts.last_mut() {
        if (t - last.0).abs() < 1e-6 {
            *last = (t, face, why);
            return;
        }
    }
    cuts.push((t, face, why));
}

/// Engine samples as `speakers.active`-style samples (person id = positional track).
pub fn to_active_samples(samples: &[ScanSample]) -> Vec<ActiveSample> {
    samples
        .iter()
        .map(|s| ActiveSample {
            t: s.t,
            faces: s
                .faces
                .iter()
                .map(|f| ActiveFace { x: f.x, y: f.y, w: f.w, h: f.h, score: f.score, id: f.track, speak: 0.0, motion: f.mouth, track: f.track })
                .collect(),
        })
        .collect()
}

fn to_choices(turns: &[ActiveTurn]) -> Vec<TurnChoice> {
    turns
        .iter()
        .map(|t| TurnChoice {
            speaker: t.face.map_or_else(|| "none".to_string(), |f| format!("F{f}")),
            start: t.start,
            end: t.end,
            face_id: t.face,
            face_x: t.face_x,
            score: t.score,
            margin: t.margin,
            reason: t.reason.clone(),
        })
        .collect()
}

/// Shot cuts from the scan peaks (adaptive: absolute floor and a multiple of the local median).
pub fn scan_cuts(peaks: &[ScanPeak]) -> Vec<f64> {
    peaks.iter().filter(|p| p.diff >= CUT_MIN_DIFF && p.diff >= CUT_RATIO * p.base + 0.08).map(|p| p.t).collect()
}

/// Union of cut lists, sorted, duplicates within 0.3 s dropped.
pub fn merge_cuts(mut cuts: Vec<f64>) -> Vec<f64> {
    cuts.sort_by(|a, b| a.total_cmp(b));
    let mut out: Vec<f64> = Vec::new();
    for c in cuts {
        if out.last().is_none_or(|l| c - l > 0.3) {
            out.push(c);
        }
    }
    out
}

/// Camera path for a center-crop range following the active speaker. Keys are relative to
/// `start` (the segment start), samples are the scan samples inside the range.
pub fn active_camera(
    samples: &[ScanSample],
    turns: &[ActiveTurn],
    cuts: &[f64],
    start: f64,
    crop_w: f64,
    style: CameraStyle,
    switch: SwitchStyle,
    look_room: bool,
) -> CameraPlan {
    if samples.is_empty() {
        return CameraPlan::default();
    }
    let act = to_active_samples(samples);
    let choices = to_choices(turns);
    let face_samples = speaker_face_samples(&act, &choices);
    let with_lm: Vec<CamSample> = samples
        .iter()
        .map(|s| CamSample {
            t: s.t,
            faces: s.faces.iter().map(|f| CamFace { x: f.x, y: f.y, w: f.w, h: f.h, score: f.score, lm: f.lm }).collect(),
        })
        .collect();
    let cam = if look_room { camera::attach_landmarks(&face_samples, &with_lm) } else { camera::from_face_samples(&face_samples) };
    let mut events = speaker_events(&act, &choices);
    let (a, b) = (samples[0].t, samples[samples.len() - 1].t);
    for &c in cuts {
        if c > a + 0.05 && c < b && !events.iter().any(|e| e.kind == EventKind::ShotCut && (e.t - c).abs() < 0.3) {
            events.push(CamEvent { t: c, kind: EventKind::ShotCut });
        }
    }
    events.sort_by(|x, y| x.t.total_cmp(&y.t));
    let mut plan = camera::plan_detailed(&cam, crop_w, &events, &speaker_camera(style, look_room, switch));
    let shift = a - start;
    if shift.abs() > 1e-6 {
        for k in &mut plan.keys {
            k.t = round3(k.t + shift);
        }
    }
    plan
}

// ---------------------------------------------------------------------------------------------
// Shot plan (pure)
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum ShotClass {
    OneFace,
    MultiFace,
    NoFace,
    Graphics,
}

#[derive(Serialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum LayoutChoice {
    CenterCrop,
    BlurFit,
    /// The centred part of the frame (`cx`, width `w` of the source) scaled up on a blurred fill.
    ZoomFit { cx: f64, w: f64 },
}

impl LayoutChoice {
    pub fn name(&self) -> &'static str {
        match self {
            Self::CenterCrop => "center-crop",
            Self::BlurFit => "blur-fit",
            Self::ZoomFit { .. } => "zoom-fit",
        }
    }
    fn same_kind(&self, o: &Self) -> bool {
        self.name() == o.name()
    }
}

#[derive(Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Tracking {
    /// One face: the smooth camera follows it (look-room).
    Smooth,
    /// Two or more faces: the light active speaker picks who is framed.
    Active,
    None,
}

#[derive(Serialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct ShotPlan {
    /// Source time.
    pub start: f64,
    pub end: f64,
    pub class: ShotClass,
    /// Typical number of faces on screen.
    pub faces: u32,
    pub layout: LayoutChoice,
    pub tracking: Tracking,
    /// Mean centre of the faces (0..1) when there are faces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face_x: Option<f64>,
    pub reason: String,
}

fn classify(samples: &[&ScanSample], start: f64, end: f64) -> ShotPlan {
    let n = samples.len().max(1);
    let face_rows: Vec<Vec<&ScanFace>> = samples.iter().map(|s| candidates(s)).collect();
    let with_face: Vec<&Vec<&ScanFace>> = face_rows.iter().filter(|r| !r.is_empty()).collect();
    let presence = with_face.len() as f64 / n as f64;
    let multi = !with_face.is_empty() && with_face.iter().filter(|r| r.len() >= 2).count() as f64 / with_face.len() as f64 >= 0.4;
    let face_w = median(with_face.iter().map(|r| r.iter().map(|f| f.w).fold(0.0, f64::max)).collect()).unwrap_or(0.0);
    let face_x = median(with_face.iter().flat_map(|r| r.iter().map(|f| f.cx())).collect());
    let gs: Vec<GraphicsStats> = samples.iter().filter_map(|s| s.g).collect();
    let flat = median(gs.iter().map(|g| g.flat).collect()).unwrap_or(0.0);
    let edges = median(gs.iter().map(|g| g.edges).collect()).unwrap_or(0.0);
    let x_lo = median(gs.iter().map(|g| g.x_lo).collect()).unwrap_or(0.0);
    let x_hi = median(gs.iter().map(|g| g.x_hi).collect()).unwrap_or(1.0);
    let graphics = flat >= GRAPHICS_FLAT && edges >= GRAPHICS_EDGES;
    let faces_count = if with_face.is_empty() { 0 } else { median(with_face.iter().map(|r| r.len() as f64).collect()).unwrap_or(0.0).round() as u32 };
    let blurred = |class: ShotClass, why: String| {
        let centred = edges >= 0.004 && x_lo >= 0.15 && x_hi <= 0.85 && x_hi > x_lo;
        let layout = if centred {
            LayoutChoice::ZoomFit { cx: round3((x_lo + x_hi) / 2.0), w: round3((x_hi - x_lo + 0.12).clamp(0.5, 0.9)) }
        } else {
            LayoutChoice::BlurFit
        };
        let what = if centred { format!("{why}; content centred (x {x_lo:.2}-{x_hi:.2}) -> zoom-fit") } else { format!("{why} -> blur-fit") };
        ShotPlan { start, end, class, faces: faces_count, layout, tracking: Tracking::None, face_x: None, reason: what }
    };
    if samples.is_empty() {
        return blurred(ShotClass::NoFace, "no frames sampled".into());
    }
    if presence >= 0.5 && !(graphics && face_w < 0.06) {
        let (class, tracking, why) = if multi {
            (ShotClass::MultiFace, Tracking::Active, format!("{} faces in {:.0}% of samples -> center-crop + active speaker", faces_count.max(2), presence * 100.0))
        } else {
            (ShotClass::OneFace, Tracking::Smooth, format!("one face in {:.0}% of samples -> center-crop + smooth track", presence * 100.0))
        };
        return ShotPlan { start, end, class, faces: faces_count.max(1), layout: LayoutChoice::CenterCrop, tracking, face_x: face_x.map(round3), reason: why };
    }
    if graphics {
        let why = format!("graphics (flat {:.0}%, edges {:.1}%), faces in {:.0}% of samples", flat * 100.0, edges * 100.0, presence * 100.0);
        return blurred(ShotClass::Graphics, why);
    }
    blurred(ShotClass::NoFace, format!("faces in only {:.0}% of samples", presence * 100.0))
}

/// Per-shot layout plan for `start..end` (source time): split at `cuts`, classify each shot,
/// merge neighbours with the same layout kind and absorb shots shorter than [`MIN_SHOT`].
pub fn plan_shots(samples: &[ScanSample], cuts: &[f64], start: f64, end: f64) -> Vec<ShotPlan> {
    let mut bounds = vec![start];
    bounds.extend(cuts.iter().copied().filter(|&c| c > start + 0.3 && c < end - 0.3));
    bounds.push(end);
    let mut shots: Vec<ShotPlan> = bounds
        .windows(2)
        .map(|w| {
            let inside: Vec<&ScanSample> = samples.iter().filter(|s| s.t >= w[0] && s.t < w[1]).collect();
            classify(&inside, w[0], w[1])
        })
        .collect();
    loop {
        let before = shots.len();
        shots = merge_same(shots);
        // Absorb the shortest too-short shot into its longer neighbour.
        if let Some((k, _)) = shots.iter().enumerate().filter(|(_, s)| s.end - s.start < MIN_SHOT).min_by(|a, b| (a.1.end - a.1.start).total_cmp(&(b.1.end - b.1.start))) {
            if shots.len() > 1 {
                let into = if k == 0 {
                    1
                } else if k + 1 == shots.len() {
                    k - 1
                } else if shots[k - 1].end - shots[k - 1].start >= shots[k + 1].end - shots[k + 1].start {
                    k - 1
                } else {
                    k + 1
                };
                let short = shots.remove(k);
                let target = if into > k { into - 1 } else { into };
                let t = &mut shots[target];
                t.start = t.start.min(short.start);
                t.end = t.end.max(short.end);
                continue;
            }
        }
        if shots.len() == before {
            break;
        }
    }
    shots
}

fn merge_same(shots: Vec<ShotPlan>) -> Vec<ShotPlan> {
    let mut out: Vec<ShotPlan> = Vec::new();
    for s in shots {
        match out.last_mut() {
            Some(m) if m.layout.same_kind(&s.layout) => {
                if let (LayoutChoice::ZoomFit { cx: c1, w: w1 }, LayoutChoice::ZoomFit { cx: c2, w: w2 }) = (m.layout, s.layout) {
                    let lo = (c1 - w1 / 2.0).min(c2 - w2 / 2.0);
                    let hi = (c1 + w1 / 2.0).max(c2 + w2 / 2.0);
                    m.layout = LayoutChoice::ZoomFit { cx: round3((lo + hi) / 2.0), w: round3((hi - lo).clamp(0.5, 0.9)) };
                }
                if m.tracking != s.tracking && m.layout == LayoutChoice::CenterCrop {
                    m.tracking = Tracking::Active;
                    m.class = ShotClass::MultiFace;
                    m.reason = format!("{}; then {}", m.reason, s.reason);
                }
                m.faces = m.faces.max(s.faces);
                m.end = s.end;
            }
            _ => out.push(s),
        }
    }
    out
}

/// The zoom-fit layout: the source window `cx ± w/2` (full height) scaled to the canvas width on a
/// blurred fill.
pub fn zoom_fit_layout(src_w: u32, src_h: u32, canvas: &Canvas, cx: f64, w: f64) -> Layout {
    let w = w.clamp(0.2, 1.0);
    let x = (cx - w / 2.0).clamp(0.0, 1.0 - w);
    let aspect = w * src_w as f64 / src_h.max(1) as f64; // width / height of the window
    let ca = canvas.width as f64 / canvas.height.max(1) as f64;
    let dst = if aspect >= ca {
        let h = ca / aspect;
        Rect { x: 0.0, y: (1.0 - h) / 2.0, w: 1.0, h }
    } else {
        let dw = aspect / ca;
        Rect { x: (1.0 - dw) / 2.0, y: 0.0, w: dw, h: 1.0 }
    };
    Layout {
        preset: Some("zoom-fit".into()),
        regions: vec![Region { id: "main".into(), src: Rect { x, y: 0.0, w, h: 1.0 }, dst, z: 0, fit: Fit::Contain }],
        background: Background::BlurFill,
    }
}

fn build_layout(source: &Source, canvas: &Canvas, choice: LayoutChoice, focus_x: Option<f64>) -> Result<Layout> {
    let (w, h) = (source.probe.width, source.probe.height);
    match choice {
        LayoutChoice::CenterCrop => layout::preset("center-crop", w, h, canvas, &LayoutParams { focus_x: focus_x.map(|f| f.clamp(0.0, 1.0)), ..Default::default() }),
        LayoutChoice::BlurFit => layout::preset("blur-fit", w, h, canvas, &LayoutParams::default()),
        LayoutChoice::ZoomFit { cx, w: zw } => Ok(zoom_fit_layout(w, h, canvas, cx, zw)),
    }
}

// ---------------------------------------------------------------------------------------------
// Fillers and pauses (pure)
// ---------------------------------------------------------------------------------------------

/// Filler lists: plain words (removed anywhere) and guarded phrases (only between pauses).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fillers {
    pub plain: HashSet<String>,
    pub guarded: Vec<Vec<String>>,
}

pub fn norm_token(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Parses a filler file (see `data/fillers_en.txt`).
pub fn parse_fillers(text: &str, into: &mut Fillers) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(phrase) = line.strip_prefix('~') {
            let toks: Vec<String> = phrase.split_whitespace().map(norm_token).filter(|t| !t.is_empty()).collect();
            if !toks.is_empty() {
                into.guarded.push(toks);
            }
        } else {
            let t = norm_token(line);
            if !t.is_empty() {
                into.plain.insert(t);
            }
        }
    }
}

/// The built-in lists: English always, Hinglish when `hindi`.
pub fn builtin_fillers(hindi: bool) -> Fillers {
    let mut f = Fillers::default();
    parse_fillers(FILLERS_EN, &mut f);
    if hindi {
        parse_fillers(FILLERS_HI, &mut f);
    }
    f
}

/// True for a Hindi/Hinglish transcript (language code or Devanagari words).
pub fn is_hindi(language: &str, words: &[Word]) -> bool {
    let l = language.to_ascii_lowercase();
    ["hi", "ur", "mr", "pa"].iter().any(|c| l == *c || l.starts_with(&format!("{c}-")))
        || words.iter().any(|w| w.text.chars().any(|c| ('\u{0900}'..='\u{097F}').contains(&c)))
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct FillerHit {
    /// Transcript word indices (`Word.i`).
    pub words: Vec<u32>,
    pub text: String,
    /// Source time.
    pub start: f64,
    pub end: f64,
    /// `filler` (plain list) or `standalone` (guarded phrase between pauses).
    pub rule: String,
}

/// Filler words inside `start..end` (words sorted by time).
pub fn filler_hits(words: &[Word], start: f64, end: f64, f: &Fillers) -> Vec<FillerHit> {
    let inside: Vec<&Word> = words.iter().filter(|w| w.start >= start - 1e-6 && w.end <= end + 1e-6).collect();
    let toks: Vec<String> = inside.iter().map(|w| norm_token(&w.text)).collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < inside.len() {
        if toks[k].is_empty() {
            k += 1;
            continue;
        }
        if f.plain.contains(&toks[k]) {
            let w = inside[k];
            out.push(FillerHit { words: vec![w.i], text: w.text.clone(), start: w.start, end: w.end, rule: "filler".into() });
            k += 1;
            continue;
        }
        let mut matched = None;
        for phrase in &f.guarded {
            let n = phrase.len();
            if k + n > inside.len() || toks[k..k + n] != phrase[..] {
                continue;
            }
            let before_ok = k == 0 || inside[k].start - inside[k - 1].end >= FILLER_PAUSE;
            let after_ok = k + n == inside.len() || inside[k + n].start - inside[k + n - 1].end >= FILLER_PAUSE;
            if before_ok && after_ok {
                matched = Some(n);
                break;
            }
        }
        if let Some(n) = matched {
            let ws = &inside[k..k + n];
            out.push(FillerHit {
                words: ws.iter().map(|w| w.i).collect(),
                text: ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
                start: ws[0].start,
                end: ws[n - 1].end,
                rule: "standalone".into(),
            });
            k += n;
        } else {
            k += 1;
        }
    }
    out
}

/// Pause cuts: every word gap longer than `min_gap` inside `start..end` keeps `keep` seconds.
pub fn pause_cuts(words: &[Word], start: f64, end: f64, min_gap: f64, keep: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for pair in words.windows(2) {
        if pair[1].start <= start || pair[0].end >= end {
            continue;
        }
        let (g0, g1) = (pair[0].end.max(start), pair[1].start.min(end));
        if g1 - g0 > min_gap + 1e-9 {
            out.push((g0 + keep / 2.0, g1 - keep / 2.0));
        }
    }
    out
}

/// Shortest piece kept when cutting (matches the edit commands).
const MIN_SEG: f64 = 0.04;

/// Removes source ranges `cuts` from every segment (same rules as `edit.remove_words`: a segment
/// that splits gets new ids, a single surviving piece keeps its id).
pub fn cut_segments(segments: &[Segment], cuts: &[(f64, f64)]) -> Vec<Segment> {
    let mut sorted: Vec<(f64, f64)> = cuts.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in sorted {
        match merged.last_mut() {
            Some(l) if a <= l.1 + 1e-9 => l.1 = l.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut out = Vec::new();
    for seg in segments {
        let mine: Vec<(f64, f64)> = merged.iter().copied().filter(|(a, b)| *b > seg.start && *a < seg.end).collect();
        if mine.is_empty() {
            out.push(seg.clone());
            continue;
        }
        let mut ranges = Vec::new();
        let mut cursor = seg.start;
        for (a, b) in mine {
            if a - cursor >= MIN_SEG {
                ranges.push((cursor, a));
            }
            cursor = cursor.max(b);
        }
        if seg.end - cursor >= MIN_SEG {
            ranges.push((cursor, seg.end));
        }
        let single = ranges.len() == 1;
        for (s, e) in ranges {
            out.push(Segment {
                id: if single { seg.id.clone() } else { SegmentId::new() },
                source_id: seg.source_id.clone(),
                start: s,
                end: e,
                layout: seg.layout.clone(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Shared engine helpers
// ---------------------------------------------------------------------------------------------

fn video_of(store: &ProjectStore, source: &Source) -> PathBuf {
    source.proxy.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()).unwrap_or_else(|| store.resolve(&source.media_path))
}

async fn scan(engine: &EngineClient, ctx: &Ctx, video: &Path, start: f64, end: f64, with_scan: bool) -> Result<ScanResult> {
    let params = json!({"video_path": video, "start": start, "end": end, "fps": ACTIVE_FPS, "motion": true, "scan": with_scan});
    engine.call("faces.active", params, &ctx.progress, &ctx.cancel).await
}

/// Words and silences of a source for the speech mask and switch points.
struct SpeechInfo {
    words: Vec<Word>,
    silences: Option<Vec<rustler_core::Silence>>,
    language: String,
    approximate: bool,
}

fn speech_info(store: &ProjectStore, source: &SourceId) -> SpeechInfo {
    let t = load_transcript(store, source).ok().flatten();
    let silences = store.read_json::<AudioEvents>(&Path::new("analysis").join(source.as_str()).join("audio_events.json")).ok().map(|a| a.silences);
    match t {
        Some(t) => SpeechInfo {
            approximate: t.word_timing == WordTiming::Approximate,
            language: t.language.clone(),
            words: {
                let mut w = t.words;
                w.sort_by(|a, b| a.start.total_cmp(&b.start));
                w
            },
            silences,
        },
        None => SpeechInfo { words: Vec::new(), silences, language: String::new(), approximate: false },
    }
}

fn words_in(words: &[Word], start: f64, end: f64) -> Vec<Word> {
    words.iter().filter(|w| w.end > start - 0.5 && w.start < end + 0.5).cloned().collect()
}

/// Active-speaker turns + camera plan for one center-crop range.
fn track_range(
    samples: &[ScanSample],
    cuts: &[f64],
    info: &SpeechInfo,
    start: f64,
    end: f64,
    crop_w: f64,
    style: CameraStyle,
    switch: SwitchStyle,
    look_room: bool,
) -> (Vec<ActiveTurn>, CameraPlan, SpeechSource) {
    let inside: Vec<ScanSample> = samples.iter().filter(|s| s.t >= start - 1e-6 && s.t < end).cloned().collect();
    let words = words_in(&info.words, start, end);
    let times: Vec<f64> = inside.iter().map(|s| s.t).collect();
    let (speech, src) = speech_mask(&times, &words, info.silences.as_deref());
    let params = ActiveParams { switch_points: switch_points(&words), ..Default::default() };
    let turns = decide_active(&inside, &speech, end, &params);
    let plan = active_camera(&inside, &turns, cuts, start, crop_w, style, switch, look_room);
    (turns, plan, src)
}

// ---------------------------------------------------------------------------------------------
// crop.track_active
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct TrackActiveIn {
    pub clip_id: ClipId,
    /// All center-crop segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// Camera inside a turn: smooth (default) | locked | follow.
    pub style: Option<CameraStyle>,
    /// Speaker switch: auto (default; cut for wide reframes, else a short pan) | cut | pan.
    pub switch: Option<SwitchStyle>,
    /// Frame toward the side the speaker faces (default true).
    pub look_room: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct TrackedActiveSegment {
    pub segment_id: SegmentId,
    pub keys: Vec<CropKey>,
    pub turns: Vec<ActiveTurnOut>,
    /// Speaker switches (changes of the framed face).
    pub switches: usize,
    pub samples: usize,
    pub faces_found_ratio: f64,
    /// Where "somebody is speaking" came from: transcript | audio | none.
    pub speech: SpeechSource,
    pub style: CameraStyle,
    pub moves: usize,
    pub cuts: usize,
    pub look_room_shift: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct ActiveTurnOut {
    #[serde(flatten)]
    pub turn: ActiveTurn,
    /// Output (clip) time.
    pub out_start: f64,
    pub out_end: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct TrackActiveOut {
    pub clip_id: ClipId,
    pub segments: Vec<TrackedActiveSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn turns_out(turns: Vec<ActiveTurn>, tm: &TimeMap, seg: &Segment) -> Vec<ActiveTurnOut> {
    let mapped = tm.segments().iter().find(|m| m.segment_id == seg.id);
    let to_out = |t: f64| round3(mapped.map_or(t - seg.start, |m| m.out_start + (t - seg.start)));
    turns.into_iter().map(|t| ActiveTurnOut { out_start: to_out(t.start), out_end: to_out(t.end), turn: t }).collect()
}

fn switches(turns: &[ActiveTurn]) -> usize {
    turns.windows(2).filter(|w| w[0].face.is_some() && w[1].face.is_some() && w[0].face != w[1].face).count()
}

pub async fn crop_track_active(ctx: Ctx, i: TrackActiveIn) -> Result<TrackActiveOut> {
    let stage = "crop.track_active";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let mut segs: Vec<&Segment> = clip.segments.iter().filter(|s| i.segment_id.as_ref().is_none_or(|id| id == &s.id)).collect();
    if segs.is_empty() {
        return Err(RustlerError::new(ErrorCode::NotFound, stage, format!("clip {} has no such segment", clip.id))
            .hint("list segment ids with `rustler clip show <clip_id>`"));
    }
    let mut warnings = Vec::new();
    if i.segment_id.is_none() {
        let (crop, other): (Vec<&Segment>, Vec<&Segment>) = segs.into_iter().partition(|s| s.layout.preset.as_deref() == Some("center-crop"));
        if crop.is_empty() {
            return Err(verr(stage, format!("clip {} has no center-crop segment to track", clip.id))
                .hint("run `rustler clip set-layout <clip_id> --preset center-crop` first"));
        }
        warnings.extend(other.iter().map(|s| format!("segment {} skipped: layout {:?} is not center-crop", s.id, s.layout.preset)));
        segs = crop;
    }
    let widths: Vec<f64> = segs.iter().map(|s| tracking_crop_width(s, None)).collect::<Result<_>>()?;
    let next = vec![format!("rustler render preview-frame {} --out-t 1.0", clip.id), format!("rustler render run {} --quality draft", clip.id)];
    if ctx.dry_run {
        return Ok(TrackActiveOut { clip_id: clip.id, segments: Vec::new(), dry_run: Some(true), next, warnings });
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let tm = TimeMap::from_segments(&clip.segments);
    let style = i.style.unwrap_or_default();
    let switch = i.switch.unwrap_or_default();
    let look_room = i.look_room.unwrap_or(true);
    let mut out = Vec::new();
    let mut failure = None;
    for (seg, crop_w) in segs.iter().zip(widths) {
        let source = store.load_source(&seg.source_id)?;
        let res = match scan(&engine, &ctx, &video_of(&store, &source), seg.start, seg.end, false).await {
            Ok(r) => r,
            Err(e) => {
                failure = Some(e);
                break;
            }
        };
        let found = res.samples.iter().filter(|s| !s.faces.is_empty()).count();
        let ratio = if res.samples.is_empty() { 0.0 } else { found as f64 / res.samples.len() as f64 };
        if ratio < 0.3 {
            warnings.push(format!("segment {}: faces found in only {:.0}% of samples; the crop will mostly hold its position", seg.id, ratio * 100.0));
        }
        let info = speech_info(&store, &seg.source_id);
        let cuts: Vec<f64> = scene_cut_events(&store, &seg.source_id, seg.start, seg.end).iter().map(|e| e.t).collect();
        let (turns, plan, speech) = track_range(&res.samples, &cuts, &info, seg.start, seg.end, crop_w, style, switch, look_room);
        if speech == SpeechSource::None {
            warnings.push(format!("segment {}: no transcript or audio analysis; mouth motion alone decides who speaks", seg.id));
        }
        let ambiguous = turns.iter().filter(|t| t.reason == "largest-face").count();
        if ambiguous > 0 {
            warnings.push(format!("segment {}: {ambiguous} turn(s) without a clear talker; the largest face was framed (see turns[].reason)", seg.id));
        }
        out.push(TrackedActiveSegment {
            segment_id: seg.id.clone(),
            keys: plan.keys.clone(),
            switches: switches(&turns),
            turns: turns_out(turns, &tm, seg),
            samples: res.samples.len(),
            faces_found_ratio: round3(ratio),
            speech,
            style,
            moves: plan.moves,
            cuts: plan.cuts,
            look_room_shift: plan.look_room_shift,
        });
    }
    engine.shutdown().await;
    if let Some(e) = failure {
        return Err(e);
    }
    write_paths(&store, &clip.id, out.iter().map(|s| (s.segment_id.clone(), s.keys.clone())))?;
    log_event(&ctx, &store, stage, json!({"clip_id": clip.id, "segments": out.len()}))?;
    Ok(TrackActiveOut { clip_id: clip.id, segments: out, dry_run: None, next, warnings })
}

fn write_paths(store: &ProjectStore, clip: &ClipId, paths: impl Iterator<Item = (SegmentId, Vec<CropKey>)>) -> Result<()> {
    let paths: Vec<(SegmentId, Vec<CropKey>)> = paths.collect();
    extras::update(store, clip, "crop_paths", |cur| {
        let mut map = match cur {
            Some(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        for (id, keys) in &paths {
            map.insert(id.as_str().to_string(), serde_json::to_value(keys).expect("serializable"));
        }
        Ok(Some(serde_json::Value::Object(map)))
    })
}

// ---------------------------------------------------------------------------------------------
// Smart layout of a clip (layout.plan and clip.create)
// ---------------------------------------------------------------------------------------------

/// One planned piece of an existing segment.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct PlannedPiece {
    /// The existing segment this piece comes from.
    pub segment_id: SegmentId,
    pub source_id: SourceId,
    #[serde(flatten)]
    pub shot: ShotPlan,
    /// Output (clip) time of the piece in the current clip.
    pub out_start: f64,
    pub out_end: f64,
}

struct SegmentAnalysis {
    seg: Segment,
    samples: Vec<ScanSample>,
    cuts: Vec<f64>,
    shots: Vec<ShotPlan>,
}

/// Why a segment was not analysed (no layout decision for it).
fn skip_reason(store: &ProjectStore, source: &Source, canvas: &Canvas) -> Option<String> {
    if !source.probe.has_video || source.probe.width == 0 || source.probe.height == 0 {
        return Some(format!("source {} has no video", source.id));
    }
    let sa = source.probe.width as f64 / source.probe.height as f64;
    let ca = canvas.width as f64 / canvas.height.max(1) as f64;
    if sa <= ca * 1.05 {
        return Some(format!("source {} is already vertical; the frame is used as it is", source.id));
    }
    if !video_of(store, source).is_file() {
        return Some(format!("source {} media file is missing", source.id));
    }
    None
}

/// Scans and plans the given segments. `Err` only for engine failures; skipped segments land in
/// `skipped` with a reason.
async fn analyse_segments(
    ctx: &Ctx,
    store: &ProjectStore,
    clip: &Clip,
    segs: &[Segment],
    skipped: &mut Vec<(SegmentId, String)>,
) -> Result<Vec<SegmentAnalysis>> {
    let mut todo = Vec::new();
    for seg in segs {
        let source = store.load_source(&seg.source_id)?;
        match skip_reason(store, &source, &clip.canvas) {
            Some(why) => skipped.push((seg.id.clone(), why)),
            None => todo.push((seg.clone(), source)),
        }
    }
    if todo.is_empty() {
        return Ok(Vec::new());
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let mut out = Vec::new();
    let mut failure = None;
    for (seg, source) in todo {
        match scan(&engine, ctx, &video_of(store, &source), seg.start, seg.end, true).await {
            Ok(r) => {
                let mut cuts = scan_cuts(&r.peaks);
                cuts.extend(scene_cut_events(store, &seg.source_id, seg.start, seg.end).iter().map(|e| e.t));
                let cuts = merge_cuts(cuts);
                let shots = plan_shots(&r.samples, &cuts, seg.start, seg.end);
                out.push(SegmentAnalysis { seg, samples: r.samples, cuts, shots });
            }
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    engine.shutdown().await;
    if let Some(e) = failure {
        return Err(e);
    }
    Ok(out)
}

/// A decision made automatically, with the commands that undo or override it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Decision {
    /// layout | framing | filler | pause | skipped
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segment_ids: Vec<SegmentId>,
    /// Source time span the decision covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_end: Option<f64>,
    /// What was done.
    pub what: String,
    /// Why.
    pub why: String,
    /// Exact commands that undo or override this decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub override_with: Vec<String>,
}

/// One saved step of the smart defaults (a clip revision).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SmartStep {
    pub stage: String,
    /// Revision after this step.
    pub revision: u64,
    pub summary: String,
}

/// The report `clip.create` returns under `smart` (also stored in extras `smart_defaults`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct SmartReport {
    /// True when anything was changed.
    pub applied: bool,
    pub steps: Vec<SmartStep>,
    pub decisions: Vec<Decision>,
    /// Undo everything the smart defaults did (back to the raw clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_all: Option<String>,
    /// Create clips without smart defaults from now on.
    pub turn_off: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn layout_overrides(clip: &ClipId, seg: &SegmentId, shot: &ShotPlan) -> Vec<String> {
    let s = seg.as_str();
    match shot.layout {
        LayoutChoice::CenterCrop => {
            let mut v = vec![
                format!("rustler clip set-layout {clip} --segment-id {s} --preset blur-fit"),
                format!("rustler crop track-face {clip} --segment-id {s} --style locked"),
            ];
            if shot.tracking == Tracking::Active {
                v.insert(1, format!("rustler crop track-active {clip} --segment-id {s} --switch cut"));
                v.push(format!("rustler crop track-speaker {clip} --segment-id {s}"));
            } else {
                v.push(format!("rustler crop track-face {clip} --segment-id {s} --look-room false"));
            }
            v
        }
        LayoutChoice::BlurFit | LayoutChoice::ZoomFit { .. } => vec![
            format!("rustler clip set-layout {clip} --segment-id {s} --preset {}", if shot.layout == LayoutChoice::BlurFit { "center-crop" } else { "blur-fit" }),
            format!("rustler clip set-layout {clip} --segment-id {s} --preset center-crop"),
        ],
    }
}

/// Applies the shot plans: splits segments where the layout changes, sets layouts and writes the
/// crop paths (active speaker or smooth track). Returns the new segments and decisions.
fn apply_layout_plan(store: &ProjectStore, clip: &Clip, analyses: &[SegmentAnalysis]) -> Result<(Vec<Segment>, Vec<(SegmentId, Vec<CropKey>)>, Vec<Decision>, Vec<String>)> {
    let mut segments = Vec::new();
    let mut paths = Vec::new();
    let mut decisions = Vec::new();
    let mut warnings = Vec::new();
    for seg in &clip.segments {
        let Some(a) = analyses.iter().find(|a| a.seg.id == seg.id) else {
            segments.push(seg.clone());
            continue;
        };
        let source = store.load_source(&seg.source_id)?;
        let single = a.shots.len() == 1;
        for shot in &a.shots {
            let id = if single { seg.id.clone() } else { SegmentId::new() };
            let layout = build_layout(&source, &clip.canvas, shot.layout, shot.face_x)?;
            let piece = Segment { id: id.clone(), source_id: seg.source_id.clone(), start: shot.start, end: shot.end, layout };
            let mut what = format!("{} for source {:.2}-{:.2} s", shot.layout.name(), shot.start, shot.end);
            if shot.layout == LayoutChoice::CenterCrop {
                let crop_w = tracking_crop_width(&piece, None)?;
                let info = speech_info(store, &seg.source_id);
                let (turns, plan, speech) = track_range(&a.samples, &a.cuts, &info, shot.start, shot.end, crop_w, CameraStyle::Smooth, SwitchStyle::Auto, true);
                let sw = switches(&turns);
                if plan.keys.is_empty() {
                    warnings.push(format!("segment {id}: no face samples for tracking; static centre crop"));
                } else {
                    paths.push((id.clone(), plan.keys.clone()));
                }
                match shot.tracking {
                    Tracking::Active => {
                        what.push_str(&format!(
                            " + light active speaker ({} turn(s), {sw} switch(es), speech from {}; {} move(s), {} cut(s))",
                            turns.len(),
                            speech.name(),
                            plan.moves,
                            plan.cuts
                        ));
                    }
                    _ => what.push_str(&format!(" + smooth track with look-room ({} move(s), {} cut(s))", plan.moves, plan.cuts)),
                }
            }
            decisions.push(Decision {
                kind: "layout".into(),
                segment_ids: vec![id.clone()],
                source_start: Some(round3(shot.start)),
                source_end: Some(round3(shot.end)),
                what,
                why: shot.reason.clone(),
                override_with: layout_overrides(&clip.id, &id, shot),
            });
            segments.push(piece);
        }
    }
    Ok((segments, paths, decisions, warnings))
}

fn save_step(ctx: &Ctx, store: &ProjectStore, old: &Clip, mut new: Clip, stage: &str, summary: String, detail: serde_json::Value) -> Result<(Clip, SmartStep, Vec<String>)> {
    let base = old.revision;
    store.save_clip(&mut new, base)?;
    let warnings = extras::carry_crop_paths(store, old, &new)?;
    log_event(ctx, store, "clip.smart_defaults", json!({"clip_id": new.id, "stage": stage, "from_revision": base, "to_revision": new.revision, "detail": detail}))?;
    let step = SmartStep { stage: stage.to_string(), revision: new.revision, summary };
    Ok((new, step, warnings))
}

/// Remaps decision segment ids onto the final clip (pieces made by later cuts) by source overlap.
fn remap_ids(decisions: &mut [Decision], clip: &Clip) {
    for d in decisions.iter_mut().filter(|d| d.kind == "layout" || d.kind == "framing") {
        let (Some(a), Some(b)) = (d.source_start, d.source_end) else { continue };
        let ids: Vec<SegmentId> = clip.segments.iter().filter(|s| s.start < b - 1e-6 && s.end > a + 1e-6).map(|s| s.id.clone()).collect();
        if ids.is_empty() || ids == d.segment_ids {
            continue;
        }
        if let Some(old) = d.segment_ids.first().cloned() {
            let mut cmds = Vec::new();
            for c in &d.override_with {
                if c.contains(old.as_str()) {
                    for id in &ids {
                        cmds.push(c.replace(old.as_str(), id.as_str()));
                    }
                } else {
                    cmds.push(c.clone());
                }
            }
            d.override_with = cmds;
        }
        d.segment_ids = ids;
    }
}

/// What the smart defaults may do.
#[derive(Clone, Copy, Debug)]
pub struct SmartOptions {
    /// Plan layouts and framing (false when the caller chose a layout).
    pub layout: bool,
    pub cleanup: bool,
}

/// Applies the smart defaults to a freshly created clip (revision 1). Each step is its own revision.
pub async fn apply_smart_defaults(ctx: &Ctx, store: &ProjectStore, clip: Clip, opts: SmartOptions) -> Result<(Clip, SmartReport)> {
    let mut report = SmartReport {
        turn_off: "rustler config set clip.smart_defaults false (or pass --raw to clip create)".into(),
        ..Default::default()
    };
    let raw_revision = clip.revision;
    let mut clip = clip;

    // 1. Layout + framing.
    if opts.layout {
        let mut skipped = Vec::new();
        let segs = clip.segments.clone();
        match analyse_segments(ctx, store, &clip, &segs, &mut skipped).await {
            Ok(analyses) if !analyses.is_empty() => {
                let (segments, paths, decisions, warnings) = apply_layout_plan(store, &clip, &analyses)?;
                report.warnings.extend(warnings);
                let changed = segments != clip.segments;
                if changed {
                    let mut new = clip.clone();
                    new.segments = segments;
                    let summary = format!("{} segment(s): {}", new.segments.len(), new.segments.iter().map(|s| s.layout.preset.clone().unwrap_or_default()).collect::<Vec<_>>().join(", "));
                    let detail = json!({"segments": new.segments.len()});
                    let (saved, step, w) = save_step(ctx, store, &clip, new, "layout", summary, detail)?;
                    report.warnings.extend(w);
                    report.steps.push(step);
                    clip = saved;
                }
                if !paths.is_empty() {
                    write_paths(store, &clip.id, paths.into_iter())?;
                }
                report.decisions.extend(decisions);
            }
            Ok(_) => {}
            Err(e) => {
                report.decisions.push(Decision {
                    kind: "skipped".into(),
                    segment_ids: Vec::new(),
                    source_start: None,
                    source_end: None,
                    what: "no automatic layout or framing".into(),
                    why: format!("face/shot analysis unavailable: {}", e.message),
                    override_with: vec![format!("rustler setup run"), format!("rustler layout plan {} --apply", clip.id)],
                });
            }
        }
        for (id, why) in skipped {
            report.decisions.push(Decision {
                kind: "skipped".into(),
                segment_ids: vec![id],
                source_start: None,
                source_end: None,
                what: "layout kept as created".into(),
                why,
                override_with: Vec::new(),
            });
        }
    }

    // 2. Fillers, then 3. pauses (on the words that remain).
    if opts.cleanup {
        let mut by_source: BTreeMap<SourceId, SpeechInfo> = BTreeMap::new();
        for s in &clip.segments {
            by_source.entry(s.source_id.clone()).or_insert_with(|| speech_info(store, &s.source_id));
        }
        let usable: Vec<&SourceId> = by_source.iter().filter(|(_, i)| !i.words.is_empty() && !i.approximate).map(|(k, _)| k).collect();
        for (k, i) in &by_source {
            if i.words.is_empty() {
                report.decisions.push(skip_cleanup(k, "the source has no transcript words"));
            } else if i.approximate {
                report.decisions.push(skip_cleanup(k, "word timings are approximate (captions-based transcript); re-transcribe with `rustler analyze transcribe` to enable"));
            }
        }
        if !usable.is_empty() {
            // Fillers
            let mut hits_all: Vec<(SourceId, FillerHit)> = Vec::new();
            for seg in &clip.segments {
                let Some(info) = by_source.get(&seg.source_id).filter(|i| !i.words.is_empty() && !i.approximate) else { continue };
                let fl = builtin_fillers(is_hindi(&info.language, &info.words));
                for h in filler_hits(&info.words, seg.start, seg.end, &fl) {
                    hits_all.push((seg.source_id.clone(), h));
                }
            }
            if !hits_all.is_empty() {
                let cuts: Vec<(f64, f64)> = hits_all.iter().map(|(_, h)| (h.start, h.end)).collect();
                let before = clip.revision;
                let mut new = clip.clone();
                new.segments = cut_segments(&clip.segments, &cuts);
                if new.segments.is_empty() {
                    report.warnings.push("filler removal would remove the whole clip; skipped".into());
                } else if new.segments != clip.segments {
                    let summary = format!("removed {} filler(s)", hits_all.len());
                    let detail = json!({"removed": hits_all.iter().map(|(_, h)| json!({"words": h.words, "text": h.text})).collect::<Vec<_>>()});
                    let (saved, step, w) = save_step(ctx, store, &clip, new, "fillers", summary, detail)?;
                    report.warnings.extend(w);
                    report.steps.push(step);
                    clip = saved;
                    for (src, h) in &hits_all {
                        report.decisions.push(Decision {
                            kind: "filler".into(),
                            segment_ids: Vec::new(),
                            source_start: Some(round3(h.start)),
                            source_end: Some(round3(h.end)),
                            what: format!("removed \"{}\" (word {})", h.text, h.words.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("+")),
                            why: if h.rule == "filler" { "filler word list".into() } else { "standalone filler between pauses".into() },
                            override_with: vec![
                                format!("rustler history restore {} --revision {before}", clip.id),
                                format!("rustler edit set-range {} --segment-id <piece before it> --end {:.3} (source {src})", clip.id, h.end),
                            ],
                        });
                    }
                }
            }
            // Pauses on the words that remain.
            let removed: HashSet<(SourceId, u32)> = hits_all.iter().flat_map(|(s, h)| h.words.iter().map(move |i| (s.clone(), *i))).collect();
            let mut cuts = Vec::new();
            let mut found: Vec<(f64, f64, f64)> = Vec::new();
            for seg in &clip.segments {
                let Some(info) = by_source.get(&seg.source_id).filter(|i| !i.words.is_empty() && !i.approximate) else { continue };
                let words: Vec<Word> = info.words.iter().filter(|w| !removed.contains(&(seg.source_id.clone(), w.i))).cloned().collect();
                for (a, b) in pause_cuts(&words, seg.start, seg.end, PAUSE_MIN_GAP, PAUSE_KEEP) {
                    cuts.push((a, b));
                    found.push((a, b, b - a + PAUSE_KEEP));
                }
            }
            if !cuts.is_empty() {
                let before = clip.revision;
                let mut new = clip.clone();
                new.segments = cut_segments(&clip.segments, &cuts);
                if !new.segments.is_empty() && new.segments != clip.segments {
                    let removed_s: f64 = cuts.iter().map(|(a, b)| b - a).sum();
                    let summary = format!("trimmed {} pause(s), {:.2} s removed", cuts.len(), removed_s);
                    let detail = json!({"min_gap": PAUSE_MIN_GAP, "keep": PAUSE_KEEP, "removed_seconds": round3(removed_s)});
                    let (saved, step, w) = save_step(ctx, store, &clip, new, "pauses", summary, detail)?;
                    report.warnings.extend(w);
                    report.steps.push(step);
                    clip = saved;
                    for (a, b, gap) in found {
                        report.decisions.push(Decision {
                            kind: "pause".into(),
                            segment_ids: Vec::new(),
                            source_start: Some(round3(a)),
                            source_end: Some(round3(b)),
                            what: format!("cut a {gap:.2} s pause down to {PAUSE_KEEP} s"),
                            why: format!("word gap longer than {PAUSE_MIN_GAP} s"),
                            override_with: vec![
                                format!("rustler history restore {} --revision {before}", clip.id),
                                format!("rustler edit set-range {} --segment-id <piece before it> --end {:.3}", clip.id, b),
                            ],
                        });
                    }
                }
            }
        }
    }

    remap_ids(&mut report.decisions, &clip);
    report.applied = !report.steps.is_empty() || report.decisions.iter().any(|d| d.kind == "layout");
    if report.applied {
        report.undo_all = Some(format!("rustler history restore {} --revision {raw_revision}", clip.id));
    }
    if report.applied {
        // Kept with the clip so an agent can read the decisions later (`clip show` extras).
        let stored = serde_json::to_value(&report).expect("serializable");
        extras::update(store, &clip.id, SMART_KEY, |_| Ok(Some(stored)))?;
    }
    Ok((clip, report))
}

fn skip_cleanup(source: &SourceId, why: &str) -> Decision {
    Decision {
        kind: "skipped".into(),
        segment_ids: Vec::new(),
        source_start: None,
        source_end: None,
        what: format!("no filler or pause trim for source {source}"),
        why: why.to_string(),
        override_with: Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// layout.plan
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct LayoutPlanIn {
    pub clip_id: ClipId,
    /// Apply the plan (split segments where the layout changes, set layouts, write crop paths).
    /// Default false: only propose.
    pub apply: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct LayoutPlanOut {
    pub clip_id: ClipId,
    pub applied: bool,
    pub pieces: Vec<PlannedPiece>,
    /// Shot cuts found per segment (source time).
    pub cuts: BTreeMap<String, Vec<f64>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<Decision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub async fn layout_plan(ctx: Ctx, i: LayoutPlanIn) -> Result<LayoutPlanOut> {
    let stage = "layout.plan";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let apply = i.apply.unwrap_or(false);
    let mut skipped = Vec::new();
    let segs = clip.segments.clone();
    let analyses = analyse_segments(&ctx, &store, &clip, &segs, &mut skipped).await?;
    let mut warnings: Vec<String> = skipped.iter().map(|(id, why)| format!("segment {id} not planned: {why}")).collect();
    if analyses.is_empty() {
        return Err(verr(stage, format!("nothing to plan in clip {}", clip.id)).hint(warnings.join("; ")));
    }
    let tm = TimeMap::from_segments(&clip.segments);
    let mut pieces = Vec::new();
    let mut cuts = BTreeMap::new();
    for a in &analyses {
        let m = tm.segments().iter().find(|m| m.segment_id == a.seg.id);
        let to_out = |t: f64| round3(m.map_or(t - a.seg.start, |m| m.out_start + (t - a.seg.start)));
        for s in &a.shots {
            pieces.push(PlannedPiece { segment_id: a.seg.id.clone(), source_id: a.seg.source_id.clone(), shot: s.clone(), out_start: to_out(s.start), out_end: to_out(s.end) });
        }
        cuts.insert(a.seg.id.as_str().to_string(), a.cuts.iter().map(|c| round3(*c)).collect());
    }
    let mut out = LayoutPlanOut {
        clip_id: clip.id.clone(),
        applied: false,
        pieces,
        cuts,
        decisions: Vec::new(),
        revision: None,
        undo: None,
        next: vec![format!("rustler layout plan {} --apply", clip.id)],
        warnings: Vec::new(),
    };
    if !apply || ctx.dry_run {
        out.warnings = warnings;
        return Ok(out);
    }
    let (segments, paths, decisions, w) = apply_layout_plan(&store, &clip, &analyses)?;
    warnings.extend(w);
    let mut current = clip.clone();
    if segments != clip.segments {
        let mut new = clip.clone();
        new.segments = segments;
        let (saved, step, w) = save_step(&ctx, &store, &clip, new, "layout", "layout.plan applied".into(), json!({"via": stage}))?;
        warnings.extend(w);
        out.revision = Some(step.revision);
        out.undo = Some(format!("rustler history restore {} --revision {}", clip.id, clip.revision));
        current = saved;
    }
    if !paths.is_empty() {
        write_paths(&store, &current.id, paths.into_iter())?;
    }
    out.applied = true;
    out.decisions = decisions;
    out.next = vec![format!("rustler render preview-frame {} --out-t 1.0", clip.id), format!("rustler render run {} --quality draft", clip.id)];
    out.warnings = warnings;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let long = Annotations { long_running: true, ..Default::default() };
    reg.register(typed(
        "crop.track_active",
        "Light active speaker (lite install, no diarization): the center-crop follows whoever's mouth moves while somebody speaks (YuNet mouth landmarks + transcript/audio speech activity, hysteresis: >= 1 s turns, ambiguous moments keep the current face), with the smooth camera (style smooth|locked|follow, switch auto|cut|pan, look-room); reports the turns",
        &["clip_id"],
        long,
        vec![
            Example { description: "Frame whoever is talking in a two-person shot", cli: "rustler crop track-active clp_01k..." },
            Example { description: "Hard cuts between speakers, locked framing", cli: "rustler crop track-active clp_01k... --switch cut --style locked" },
        ],
        crop_track_active,
    ));
    reg.register(typed(
        "layout.plan",
        "Per-shot layout plan for a clip (shots from scenes.json + a dark-safe frame-difference scan; faces at 5 fps): one face -> center-crop + smooth track, two or more -> center-crop + active speaker, no face/graphics -> blur-fit (zoom-fit when centred). Proposes only; --apply splits segments where the layout changes and writes the crop paths",
        &["clip_id"],
        long,
        vec![
            Example { description: "See the plan", cli: "rustler layout plan clp_01k..." },
            Example { description: "Apply it", cli: "rustler layout plan clp_01k... --apply" },
        ],
        layout_plan,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(track: u32, x: f64, w: f64, mouth: f64) -> ScanFace {
        ScanFace { x, y: 0.2, w, h: w * 1.6, score: 0.9, lm: None, mouth, track }
    }

    /// Two faces at 5 fps; `talker(t)` says whose mouth moves (0 left, 1 right, None nobody).
    fn two(seconds: f64, talker: impl Fn(f64) -> Option<u32>) -> Vec<ScanSample> {
        let n = (seconds * 5.0) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / 5.0;
                let wob = if i % 2 == 0 { 0.06 } else { 0.02 };
                let m = |id| if talker(t) == Some(id) { 0.08 + wob } else { 0.01 };
                ScanSample { t, faces: vec![face(0, 0.15, 0.08, m(0)), face(1, 0.75, 0.08, m(1))], g: None }
            })
            .collect()
    }

    #[test]
    fn alternating_mouths_switch_with_hysteresis() {
        let s = two(9.0, |t| Some(if (3.0..6.0).contains(&t) { 1 } else { 0 }));
        let speech = vec![true; s.len()];
        let turns = decide_active(&s, &speech, 9.0, &ActiveParams::default());
        let faces: Vec<Option<u32>> = turns.iter().map(|t| t.face).collect();
        assert_eq!(faces, vec![Some(0), Some(1), Some(0)], "{turns:?}");
        assert!((turns[1].start - 3.0).abs() <= 0.6, "{turns:?}");
        assert!((turns[2].start - 6.0).abs() <= 0.6, "{turns:?}");
        assert!(turns.iter().all(|t| t.reason == "mouth-motion"), "{turns:?}");
    }

    #[test]
    fn short_bursts_and_ambiguity_keep_the_current_face() {
        // right face talks only 0.6 s (a backchannel): no switch
        let s = two(6.0, |t| Some(if (2.0..2.6).contains(&t) { 1 } else { 0 }));
        let turns = decide_active(&s, &vec![true; s.len()], 6.0, &ActiveParams::default());
        assert_eq!(turns.len(), 1, "{turns:?}");
        // both mouths move alike after 2 s: ambiguous, stays on the first talker
        let mut s = two(6.0, |_| Some(0));
        for x in s.iter_mut().filter(|x| x.t >= 2.0) {
            x.faces[1].mouth = x.faces[0].mouth;
        }
        let turns = decide_active(&s, &vec![true; s.len()], 6.0, &ActiveParams::default());
        assert_eq!(turns.len(), 1, "{turns:?}");
        assert_eq!(turns[0].face, Some(0));
    }

    #[test]
    fn mouth_motion_in_silence_does_not_count() {
        // right face moves its mouth only while nobody speaks (chewing); left talks
        let s = two(8.0, |t| Some(if (4.0..6.0).contains(&t) { 1 } else { 0 }));
        let speech: Vec<bool> = s.iter().map(|x| !(4.0..6.0).contains(&x.t)).collect();
        let turns = decide_active(&s, &speech, 8.0, &ActiveParams::default());
        assert!(turns.iter().all(|t| t.face == Some(0)), "{turns:?}");
    }

    #[test]
    fn a_single_face_is_the_only_face_and_a_cut_follows_the_new_shot() {
        let mut s: Vec<ScanSample> = (0..20).map(|i| ScanSample { t: i as f64 / 5.0, faces: vec![face(0, 0.4, 0.15, 0.02)], g: None }).collect();
        s.extend((20..40).map(|i| ScanSample { t: i as f64 / 5.0, faces: vec![face(3, 0.6, 0.3, 0.02)], g: None }));
        let turns = decide_active(&s, &vec![true; s.len()], 8.0, &ActiveParams::default());
        assert_eq!(turns.len(), 2, "{turns:?}");
        assert_eq!(turns[0].reason, "only-face");
        assert_eq!(turns[1].face, Some(3));
        assert!((turns[1].start - 4.0).abs() < 0.2, "{turns:?}");
    }

    #[test]
    fn switch_snaps_to_a_word_start() {
        let s = two(9.0, |t| Some(if (3.2..6.0).contains(&t) { 1 } else { 0 }));
        let p = ActiveParams { switch_points: vec![2.9, 6.1], ..Default::default() };
        let turns = decide_active(&s, &vec![true; s.len()], 9.0, &p);
        assert_eq!(turns[1].start, 2.9, "{turns:?}");
    }

    #[test]
    fn active_camera_puts_the_crop_on_the_talker() {
        let s = two(9.0, |t| Some(if (3.0..6.0).contains(&t) { 1 } else { 0 }));
        let turns = decide_active(&s, &vec![true; s.len()], 9.0, &ActiveParams::default());
        let plan = active_camera(&s, &turns, &[], 0.0, 0.3164, CameraStyle::Smooth, SwitchStyle::Cut, false);
        let at = |t: f64| plan.keys.iter().rev().find(|k| k.t <= t).map_or(plan.keys[0].x, |k| k.x);
        let inside = |t: f64, cx: f64| at(t) <= cx && cx <= at(t) + 0.3164;
        assert!(inside(1.5, 0.19), "{:?}", plan.keys);
        assert!(inside(4.8, 0.79), "{:?}", plan.keys);
        assert!(inside(8.0, 0.19), "{:?}", plan.keys);
    }

    fn g(flat: f64, edges: f64, x_lo: f64, x_hi: f64) -> Option<GraphicsStats> {
        Some(GraphicsStats { flat, edges, x_lo, x_hi })
    }

    #[test]
    fn shot_plan_classifies_and_splits_only_on_layout_change() {
        let mut s = Vec::new();
        for i in 0..60 {
            let t = i as f64 / 5.0;
            let (faces, gs) = if t < 4.0 {
                (vec![face(0, 0.4, 0.12, 0.02)], g(0.2, 0.05, 0.05, 0.95))
            } else if t < 8.0 {
                (vec![face(1, 0.15, 0.09, 0.02), face(2, 0.7, 0.09, 0.02)], g(0.2, 0.05, 0.05, 0.95))
            } else {
                (vec![], g(0.8, 0.03, 0.3, 0.7))
            };
            s.push(ScanSample { t, faces, g: gs });
        }
        let shots = plan_shots(&s, &[4.0, 8.0], 0.0, 12.0);
        assert_eq!(shots.len(), 2, "{shots:?}");
        assert_eq!(shots[0].layout, LayoutChoice::CenterCrop);
        assert_eq!(shots[0].tracking, Tracking::Active);
        assert_eq!((shots[0].start, shots[0].end), (0.0, 8.0));
        assert!(matches!(shots[1].layout, LayoutChoice::ZoomFit { .. }), "{shots:?}");
        assert_eq!(shots[1].class, ShotClass::Graphics);
    }

    #[test]
    fn short_shots_are_absorbed_and_wide_graphics_blur() {
        let mut s = Vec::new();
        for i in 0..50 {
            let t = i as f64 / 5.0;
            let (faces, gs) = if (5.0..5.8).contains(&t) { (vec![], g(0.7, 0.04, 0.02, 0.98)) } else { (vec![face(0, 0.4, 0.12, 0.0)], g(0.2, 0.05, 0.05, 0.95)) };
            s.push(ScanSample { t, faces, g: gs });
        }
        let shots = plan_shots(&s, &[5.0, 5.8], 0.0, 10.0);
        assert_eq!(shots.len(), 1, "{shots:?}");
        let wide: Vec<ScanSample> = (0..20).map(|i| ScanSample { t: i as f64 / 5.0, faces: vec![], g: g(0.7, 0.04, 0.02, 0.98) }).collect();
        assert_eq!(plan_shots(&wide, &[], 0.0, 4.0)[0].layout, LayoutChoice::BlurFit);
    }

    #[test]
    fn cut_detection_is_adaptive() {
        let peaks = [
            ScanPeak { t: 1.0, diff: 0.5, base: 0.02 },  // clean cut
            ScanPeak { t: 2.0, diff: 0.35, base: 0.2 },  // busy motion, not a cut
            ScanPeak { t: 3.0, diff: 0.2, base: 0.01 },  // too small
            ScanPeak { t: 4.0, diff: 0.45, base: 0.12 }, // cut in a lively shot
        ];
        assert_eq!(scan_cuts(&peaks), vec![1.0, 4.0]);
        assert_eq!(merge_cuts(vec![4.0, 1.0, 1.1, 7.0]), vec![1.0, 4.0, 7.0]);
    }

    fn w(i: u32, text: &str, s: f64, e: f64) -> Word {
        Word { i, text: text.into(), start: s, end: e, conf: None, speaker: None }
    }

    #[test]
    fn filler_files_load_and_rules_are_conservative() {
        let en = builtin_fillers(false);
        for x in ["um", "uh", "umm", "uhh", "hmm", "erm", "ah"] {
            assert!(en.plain.contains(x), "{x}");
        }
        assert!(en.guarded.contains(&vec!["you".to_string(), "know".to_string()]));
        assert!(en.guarded.contains(&vec!["like".to_string()]));
        assert!(!en.plain.contains("like") && !en.plain.contains("matlab"));
        let hi = builtin_fillers(true);
        assert!(hi.guarded.contains(&vec!["matlab".to_string()]));
        assert!(hi.guarded.iter().any(|p| p == &vec![norm_token("हाँ")]));

        let words = vec![
            w(0, "I", 0.0, 0.2),
            w(1, "Um,", 0.25, 0.5),
            w(2, "like", 0.55, 0.8), // inside a sentence: kept
            w(3, "this.", 0.85, 1.2),
            w(4, "You", 1.8, 2.0), // "you know" between pauses: removed
            w(5, "know", 2.05, 2.3),
            w(6, "it", 2.9, 3.1),
            w(7, "works", 3.15, 3.5),
        ];
        let hits = filler_hits(&words, 0.0, 4.0, &en);
        let texts: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, vec!["Um,", "You know"], "{hits:?}");
        assert_eq!(hits[1].words, vec![4, 5]);

        let hindi = vec![w(0, "matlab", 0.0, 0.3), w(1, "yeh", 0.35, 0.5), w(2, "na", 1.2, 1.4), w(3, "theek", 2.0, 2.3), w(4, "na", 2.35, 2.5), w(5, "hai", 2.55, 2.8)];
        let hits = filler_hits(&hindi, 0.0, 3.0, &hi);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].words, vec![2]); // the standalone "na", not "matlab yeh" or "theek na hai"
        assert!(is_hindi("hi", &[]) && is_hindi("en", &[w(0, "मतलब", 0.0, 0.1)]) && !is_hindi("en", &words));
    }

    #[test]
    fn pauses_longer_than_a_second_are_trimmed_to_keep() {
        let words = vec![w(0, "a", 0.0, 0.5), w(1, "b", 0.9, 1.2), w(2, "c", 3.0, 3.3), w(3, "d", 3.4, 4.0)];
        let cuts = pause_cuts(&words, 0.0, 4.0, PAUSE_MIN_GAP, PAUSE_KEEP);
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0].0 - 1.275).abs() < 1e-9 && (cuts[0].1 - 2.925).abs() < 1e-9, "{cuts:?}");
    }

    #[test]
    fn cut_segments_keeps_ids_for_single_pieces_and_mints_for_splits() {
        let seg = |s, e| Segment {
            id: SegmentId::new(),
            source_id: SourceId::new(),
            start: s,
            end: e,
            layout: Layout { preset: Some("center-crop".into()), regions: vec![], background: Background::Black },
        };
        let a = seg(0.0, 5.0);
        let out = cut_segments(&[a.clone()], &[(4.0, 6.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, a.id);
        assert_eq!(out[0].end, 4.0);
        let out = cut_segments(&[a.clone()], &[(1.0, 2.0), (1.5, 2.5)]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| s.id != a.id));
        assert_eq!((out[0].end, out[1].start), (1.0, 2.5));
    }

    #[test]
    fn zoom_fit_scales_the_centred_window_on_a_blurred_fill() {
        let l = zoom_fit_layout(1920, 1080, &Canvas { width: 1080, height: 1920, fps: 30.0 }, 0.5, 0.6);
        let r = &l.regions[0];
        assert!((r.src.x - 0.2).abs() < 1e-9 && (r.src.w - 0.6).abs() < 1e-9);
        assert_eq!(r.dst.w, 1.0);
        // 0.6 * 1920 = 1152 px wide window, 1080 tall -> 1080 x 1012.5 on the canvas
        assert!((r.dst.h * 1920.0 - 1012.5).abs() < 0.5, "{:?}", r.dst);
        assert_eq!(l.background, Background::BlurFill);
    }

    #[test]
    fn speech_mask_prefers_words_then_silences() {
        let times = [0.0, 1.0, 2.0];
        let (m, src) = speech_mask(&times, &[w(0, "x", 0.9, 1.1)], None);
        assert_eq!((m, src), (vec![false, true, false], SpeechSource::Transcript));
        let sil = [rustler_core::Silence { start: 1.5, end: 2.5 }];
        let (m, src) = speech_mask(&times, &[], Some(&sil));
        assert_eq!((m, src), (vec![true, true, false], SpeechSource::Audio));
        assert_eq!(speech_mask(&times, &[], None).1, SpeechSource::None);
    }
}
