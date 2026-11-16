//! Offline camera planner: face samples -> crop keys that move like a camera operator.
//!
//! The whole segment is known in advance, so the path is planned, not filtered:
//!
//! 1. **Subject**: the primary face per sample (largest; among faces of similar size the one
//!    nearest the previous pick), gaps held at the last position.
//! 2. **Outlier rejection**: a median filter on the face centre, so a one- or two-sample detection
//!    jump (a poster face, a false positive) never reaches the camera.
//! 3. **Look-room**: the framing target is shifted toward the side the face points, measured from
//!    the YuNet landmarks (nose tip relative to the eye midpoint), smoothed over ~2 s.
//! 4. **Shots**: the timeline is split at source shot cuts (a sustained jump of the face) and at
//!    the caller's events (speaker switches). A shot cut is always a crop cut; a speaker switch is
//!    a hard cut or a short eased pan (`switch`).
//! 5. **Hold or move** inside a shot: the camera stays locked while the subject stays inside a
//!    safe box around the framing target; it moves only when the subject leaves the box for long
//!    enough (or is about to leave the frame), to a robust estimate of where the subject will
//!    rest, with a minimum-jerk ease (zero velocity and acceleration at both ends) whose
//!    duration respects velocity and acceleration limits. Moves start a little before the subject
//!    crosses the box edge (anticipation).
//! 6. **Keys**: one key per hold boundary, dense keys (`key_rate` per second) during moves so the
//!    renderer's per-frame linear interpolation reproduces the eased curve; nothing while holding.

use crate::compile::CropKey;
use crate::facepath::{FaceBox, FaceSample};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// YuNet's five landmarks `[x, y]` (normalized 0..1 of the frame): right eye, left eye, nose
/// tip, right mouth corner, left mouth corner (the subject's right/left).
pub type Landmarks = [[f64; 2]; 5];

/// A detected face with optional landmarks (`faces.track` output; `lm` since 2026-11-16).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct CamFace {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lm: Option<Landmarks>,
}

impl CamFace {
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    fn area(&self) -> f64 {
        self.w * self.h
    }
    /// Head turn from the landmarks: the nose tip's offset from the eye midpoint over the eye
    /// distance (floored at 0.3 of the box width so profiles don't explode). Positive = the face
    /// points toward the image's right. `None` without landmarks.
    pub fn yaw(&self) -> Option<f64> {
        let lm = self.lm?;
        let (re, le, nose) = (lm[0], lm[1], lm[2]);
        let mid = (re[0] + le[0]) / 2.0;
        let eyes = (le[0] - re[0]).abs().max(0.3 * self.w).max(1e-6);
        let y = (nose[0] - mid) / eyes;
        y.is_finite().then(|| y.clamp(-1.0, 1.0))
    }
}

impl From<FaceBox> for CamFace {
    fn from(b: FaceBox) -> Self {
        Self { x: b.x, y: b.y, w: b.w, h: b.h, score: b.score, lm: None }
    }
}

/// All faces at source time `t` (seconds).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct CamSample {
    pub t: f64,
    pub faces: Vec<CamFace>,
}

impl From<&FaceSample> for CamSample {
    fn from(s: &FaceSample) -> Self {
        Self { t: s.t, faces: s.faces.iter().copied().map(CamFace::from).collect() }
    }
}

/// Box-only samples (no landmarks, so no look-room) as planner input.
pub fn from_face_samples(samples: &[FaceSample]) -> Vec<CamSample> {
    samples.iter().map(CamSample::from).collect()
}

/// Copies landmarks from `with_lm` (a `faces.track` pass over the same range) onto the boxes of
/// `samples`: same time (within 0.1 s) and the best-overlapping box (IoU >= 0.3).
pub fn attach_landmarks(samples: &[FaceSample], with_lm: &[CamSample]) -> Vec<CamSample> {
    samples
        .iter()
        .map(|s| {
            let near = with_lm
                .iter()
                .min_by(|a, b| (a.t - s.t).abs().total_cmp(&(b.t - s.t).abs()))
                .filter(|o| (o.t - s.t).abs() <= 0.1);
            let faces = s
                .faces
                .iter()
                .map(|b| {
                    let mut f = CamFace::from(*b);
                    if let Some(o) = near {
                        f.lm = o
                            .faces
                            .iter()
                            .map(|c| (iou(&f, c), c))
                            .filter(|(v, _)| *v >= 0.3)
                            .max_by(|a, b| a.0.total_cmp(&b.0))
                            .and_then(|(_, c)| c.lm);
                    }
                    f
                })
                .collect();
            CamSample { t: s.t, faces }
        })
        .collect()
}

fn iou(a: &CamFace, b: &CamFace) -> f64 {
    let ix = ((a.x + a.w).min(b.x + b.w) - a.x.max(b.x)).max(0.0);
    let iy = ((a.y + a.h).min(b.y + b.h) - a.y.max(b.y)).max(0.0);
    let inter = ix * iy;
    let union = a.area() + b.area() - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// How the camera behaves inside a shot.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum CameraStyle {
    /// One fixed framing per shot (moves only at cuts and speaker switches).
    Locked,
    /// Hold-or-move: locked until the subject leaves a Â±18% safe box, then an eased move.
    #[default]
    Smooth,
    /// Tighter safe box (Â±7%) and quicker eased moves, for subjects who move a lot.
    Follow,
}

/// How a speaker switch moves the camera.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SwitchStyle {
    /// Cut when the move is wider than `switch_cut_over` of the frame, else pan.
    #[default]
    Auto,
    /// Always an eased pan of at most `switch_pan_max` seconds.
    Pan,
    /// Always a hard cut at the turn start.
    Cut,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventKind {
    /// The source cuts to another shot: the crop cuts with it.
    ShotCut,
    /// Another person starts talking (a sentence boundary): pan or cut per [`SwitchStyle`].
    SpeakerSwitch,
}

/// A point where the framing may jump, in source time (same clock as the samples).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CamEvent {
    pub t: f64,
    pub kind: EventKind,
}

/// Camera-planner tuning. Distances in fractions of the crop width are marked `_frac`; speeds
/// and accelerations are in source frame widths per second (squared).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraParams {
    pub style: CameraStyle,
    /// Shift the framing toward the side the face points.
    pub look_room: bool,
    /// Look-room shift at a clear head turn, fraction of the crop width.
    pub look_room_frac: f64,
    /// Yaw (see [`CamFace::yaw`]) below which a face counts as frontal (no look-room).
    pub yaw_dead: f64,
    /// Yaw at which the full look-room is applied.
    pub yaw_full: f64,
    /// Median window (samples) for the yaw.
    pub yaw_window: usize,
    /// Half-width of the safe box around the framing target, fraction of the crop width.
    pub safe_frac: f64,
    /// Beyond this distance from the crop centre (fraction of the crop width) the face is about
    /// to leave the frame: move without waiting for `min_out`.
    pub hard_frac: f64,
    /// The subject must stay outside the safe box this long (s) before the camera moves.
    pub min_out: f64,
    /// Look-ahead (s) for the new rest position (median of the target).
    pub lookahead: f64,
    /// Start a move this long (s) before the subject crosses the safe-box edge.
    pub anticipation: f64,
    /// A confirmed move starts with the subject's motion (its onset + 0.1 s reaction), at most
    /// this long (s) before the box crossing, so a quick lean is followed, not caught up with.
    pub max_lead: f64,
    pub max_velocity: f64,
    pub max_accel: f64,
    /// Shortest eased move (s).
    pub min_move: f64,
    /// Median window (samples) on the face centre; removes one/two-sample detection jumps.
    pub median_window: usize,
    /// Keys per second during a move.
    pub key_rate: f64,
    /// A sustained jump of the face centre between two samples larger than this (frame widths)
    /// is a source shot cut (half of it when the face size also changes by 35%+).
    pub jump: f64,
    pub switch: SwitchStyle,
    /// Auto switch: cut when the reframe is wider than this (frame widths).
    pub switch_cut_over: f64,
    /// Longest speaker-switch pan (s).
    pub switch_pan_max: f64,
    /// The face box stays at least this share of the crop width inside both crop edges (wins
    /// over look-room and the safe box; impossible only when the face is wider than the crop
    /// allows or sits at the frame edge).
    pub margin_frac: f64,
    /// Margin (share of the crop width) that new framings aim for; a move is triggered only below
    /// `margin_frac`, so the gap between the two keeps box jitter from re-triggering moves.
    pub comfort_frac: f64,
    /// Half-window (s) over which the face's box width is taken at its (robust) largest for both
    /// margins: detection boxes jitter in size from sample to sample.
    pub margin_window: f64,
}

impl Default for CameraParams {
    fn default() -> Self {
        Self::for_style(CameraStyle::Smooth)
    }
}

impl CameraParams {
    pub fn for_style(style: CameraStyle) -> Self {
        let smooth = Self {
            style,
            look_room: true,
            look_room_frac: 0.10,
            yaw_dead: 0.10,
            yaw_full: 0.35,
            yaw_window: 9,
            safe_frac: 0.18,
            hard_frac: 0.32,
            min_out: 0.5,
            lookahead: 1.5,
            anticipation: 0.15,
            max_lead: 0.5,
            max_velocity: 0.35,
            max_accel: 0.8,
            min_move: 0.5,
            median_window: 5,
            key_rate: 30.0,
            jump: 0.2,
            switch: SwitchStyle::Auto,
            switch_cut_over: 0.40,
            switch_pan_max: 0.6,
            margin_frac: 0.08,
            comfort_frac: 0.14,
            margin_window: 0.5,
        };
        match style {
            CameraStyle::Smooth | CameraStyle::Locked => smooth,
            CameraStyle::Follow => Self {
                safe_frac: 0.07,
                min_out: 0.2,
                lookahead: 0.6,
                anticipation: 0.1,
                max_velocity: 0.6,
                max_accel: 2.0,
                min_move: 0.35,
                ..smooth
            },
        }
    }
}

/// Planner result: the keys plus what the camera does.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct CameraPlan {
    pub keys: Vec<CropKey>,
    /// Eased moves (reframes and speaker-switch pans).
    pub moves: usize,
    /// Hard cuts of the crop (source shot cuts, speaker-switch cuts).
    pub cuts: usize,
    /// Mean look-room shift, signed fraction of the crop width (> 0 = framed toward the right).
    pub look_room_shift: f64,
}

/// Plan crop keys for a crop `crop_w` of the frame width. Key times are relative to the first
/// sample (the first key is at `t = 0`); `x` is the crop's left edge.
pub fn plan(samples: &[CamSample], crop_w: f64, events: &[CamEvent], p: &CameraParams) -> Vec<CropKey> {
    plan_detailed(samples, crop_w, events, p).keys
}

/// Minimum-jerk ease 0..1 (`10u^3 - 15u^4 + 6u^5`): zero velocity and acceleration at both ends.
pub fn ease(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * u * (10.0 + u * (-15.0 + 6.0 * u))
}

/// Duration of a minimum-jerk move of `d` under the limits (peak velocity `1.875 d / T`, peak
/// acceleration `5.7735 d / T^2`).
pub fn move_time(d: f64, max_velocity: f64, max_accel: f64, min_move: f64) -> f64 {
    let d = d.abs();
    let tv = 1.875 * d / max_velocity.max(1e-6);
    let ta = (5.7735 * d / max_accel.max(1e-6)).sqrt();
    tv.max(ta).max(min_move)
}

#[derive(Clone, Copy, Debug)]
struct Move {
    start: f64,
    /// 0 = a cut.
    dur: f64,
    from: f64,
    to: f64,
    /// A later cut can end a move early (the new shot replaces it).
    stop: f64,
}

impl Move {
    fn end(&self) -> f64 {
        self.start + self.dur
    }
    fn at(&self, t: f64) -> f64 {
        if self.dur <= 0.0 {
            return if t < self.start { self.from } else { self.to };
        }
        self.from + (self.to - self.from) * ease((t - self.start) / self.dur)
    }
}

pub fn plan_detailed(samples: &[CamSample], crop_w: f64, events: &[CamEvent], p: &CameraParams) -> CameraPlan {
    if samples.is_empty() {
        return CameraPlan::default();
    }
    let crop_w = crop_w.clamp(0.01, 1.0);
    let t0 = samples[0].t;
    let times: Vec<f64> = samples.iter().map(|s| s.t - t0).collect();
    let n = times.len();
    let (lo, hi) = (crop_w / 2.0, 1.0 - crop_w / 2.0);
    let clamp_c = |c: f64| if lo >= hi { 0.5 } else { c.clamp(lo, hi) };

    let prim = primary_faces(samples);
    let cx = median_filter(&fill(&prim.iter().map(|f| f.map(|f| f.cx())).collect::<Vec<_>>(), 0.5), p.median_window);
    let widths = median_filter(&fill(&prim.iter().map(|f| f.map(|f| f.w)).collect::<Vec<_>>(), 0.1), p.median_window);
    let raw = RawPath {
        cx: fill(&prim.iter().map(|f| f.map(|f| f.cx())).collect::<Vec<_>>(), 0.5),
        cy: fill(&prim.iter().map(|f| f.map(|f| f.y + f.h / 2.0)).collect::<Vec<_>>(), 0.5),
        yaw: prim.iter().map(|f| f.and_then(|f| f.yaw())).collect(),
    };
    let bounds = boundaries(&times, &cx, &widths, &raw, events, t0, p);
    let mut edges: Vec<usize> = std::iter::once(0).chain(bounds.iter().map(|b| b.0)).collect();
    edges.push(n);
    // Look-room per shot: the yaw is smoothed within a shot, never across a cut.
    let mut shift = vec![0.0; n];
    if p.look_room {
        for w in edges.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b {
                continue;
            }
            let yaw = median_filter(&fill(&raw.yaw[a..b], 0.0), p.yaw_window);
            for (j, y) in yaw.iter().enumerate() {
                let st = ((y.abs() - p.yaw_dead) / (p.yaw_full - p.yaw_dead).max(1e-6)).clamp(0.0, 1.0);
                // A big close-up has no room to spare: keep the back of the head (about 0.9 face
                // widths behind the box centre in profile) 2% of the frame inside the crop.
                let cap = ((crop_w / 2.0 - 0.9 * widths[a + j] - 0.02) / crop_w).max(0.0);
                shift[a + j] = (st * p.look_room_frac).min(cap).copysign(*y);
            }
        }
    }
    // Where the crop centre may be so the face keeps a margin inside both crop edges. Detection
    // boxes jitter in size from sample to sample, and a head moving toward an edge should be seen
    // coming, so the face's extent for the checks is its robust envelope over a short window
    // (`reach`): `allowed` (the hard limit that triggers a move) and `fits` (an old framing carried
    // over a cut). Framings are aimed (`comfort`, a wider margin) on the face where it is, with its
    // robust largest width (`aim`), so a lean does not bias the rest position toward where the face
    // was, and a face that just got its margin back is not at the limit again with the next box.
    let reach = face_extents(&times, &prim, &cx, &edges, p.margin_window, true);
    let aim = face_extents(&times, &prim, &cx, &edges, p.margin_window, false);
    let range = |ext: &[Option<(f64, f64)>], m: f64| -> Vec<(f64, f64)> {
        let m = m.max(0.0) * crop_w;
        ext.iter()
            .map(|e| match *e {
                None => (f64::NEG_INFINITY, f64::INFINITY),
                Some((left, right)) => {
                    let (lo_c, hi_c) = (right + m - crop_w / 2.0, left - m + crop_w / 2.0);
                    if lo_c > hi_c { ((left + right) / 2.0, (left + right) / 2.0) } else { (lo_c, hi_c) }
                }
            })
            // Inside the frame: a face at the frame edge cannot get its margin.
            .map(|(a, b)| (clamp_c(a), clamp_c(b)))
            .collect()
    };
    let comfort_frac = p.margin_frac.max(p.comfort_frac);
    let allowed = range(&reach, p.margin_frac);
    let rest_range = range(&aim, p.margin_frac);
    let fits_range = range(&reach, comfort_frac);
    let comfort = range(&reach, comfort_frac);
    let target: Vec<f64> = cx.iter().zip(&shift).zip(&comfort).map(|((c, s), &(a, b))| (c + s * crop_w).clamp(a, b)).collect();
    let safe = p.safe_frac * crop_w;
    let hard = p.hard_frac * crop_w;

    // Pieces between boundaries; each enters with the boundary's (time, kind).
    let mut starts: Vec<(usize, Option<(f64, EventKind)>)> = vec![(0, None)];
    starts.extend(bounds.iter().map(|&(i, t, k)| (i, Some((t, k)))));
    let mut moves: Vec<Move> = Vec::new();
    let mut cuts = 0usize;
    let mut cam: Option<f64> = None;
    let mut first = 0.5;
    let mut busy = 0.0f64;
    for (k, &(a, entry)) in starts.iter().enumerate() {
        let b = starts.get(k + 1).map_or(n, |s| s.0);
        if a >= b {
            continue;
        }
        let rest0 = match p.style {
            CameraStyle::Locked => median(&target[a..b]),
            _ => median(&window(&times, &target, a, b, times[a] + p.lookahead)),
        };
        // The old framing carries over a cut only when it frames the new shot comfortably (a
        // reframe at a source cut is free; one shortly after it is a visible move).
        let fits = |x: f64| x >= fits_range[a].0 - 0.004 && x <= fits_range[a].1 + 0.004;
        // A move still running at a shot cut belongs to the old shot: it stops at the cut, and the
        // new shot starts from where the camera is then (or cuts when that framing does not fit).
        if let (Some(_), Some((te, EventKind::ShotCut))) = (cam, entry) {
            if let Some(m) = moves.last_mut().filter(|m| m.dur > 0.0 && m.end() > te && m.start < te) {
                m.stop = te;
                cam = Some(m.at(te));
                busy = te;
            }
        }
        let c = match cam {
            None => {
                first = rest0;
                rest0
            }
            Some(c) if (rest0 - c).abs() <= safe && fits(c) => c,
            Some(c) => {
                let (te, kind) = entry.unwrap_or((times[a], EventKind::ShotCut));
                let d = (rest0 - c).abs();
                let cut = match kind {
                    EventKind::ShotCut => true,
                    EventKind::SpeakerSwitch => match p.switch {
                        SwitchStyle::Cut => true,
                        SwitchStyle::Pan => false,
                        SwitchStyle::Auto => d > p.switch_cut_over,
                    },
                };
                if cut {
                    let mut from = c;
                    if let Some(last) = moves.last_mut().filter(|m| m.end() > te && m.start < te) {
                        last.stop = te; // the cut replaces the rest of the move
                        from = last.at(te);
                    }
                    let t = te.max(moves.last().map_or(0.0, |m| m.start.min(m.stop)));
                    moves.push(Move { start: t, dur: 0.0, from, to: rest0, stop: t });
                    busy = t;
                    cuts += 1;
                } else {
                    let dur = move_time(d, p.max_velocity * 4.0, p.max_accel * 10.0, 0.35).min(p.switch_pan_max);
                    let start = (te - dur / 2.0).max(busy).max(0.0);
                    moves.push(Move { start, dur, from: c, to: rest0, stop: start + dur });
                    busy = start + dur;
                }
                rest0
            }
        };
        let mut c = c;
        if p.style != CameraStyle::Locked {
            hold_or_move(&times, &target, (&allowed, &rest_range), a, b, &mut c, &mut busy, &mut moves, p, safe, hard);
        }
        cam = Some(c);
    }

    let to_left = |c: f64| round((c - crop_w / 2.0).clamp(0.0, (1.0 - crop_w).max(0.0)), 5);
    let mut keys: Vec<CropKey> = vec![CropKey { t: 0.0, x: to_left(first) }];
    let push = |keys: &mut Vec<CropKey>, t: f64, x: f64| {
        let t = round(t, 4);
        if let Some(l) = keys.last() {
            if t < l.t || (t == l.t && x == l.x) {
                return;
            }
            // Only drop exact repeats; a repeated x at a new time is a hold boundary.
        }
        keys.push(CropKey { t, x });
    };
    for m in &moves {
        if m.dur <= 0.0 {
            push(&mut keys, m.start, to_left(m.from));
            push(&mut keys, m.start, to_left(m.to));
            continue;
        }
        let stop = m.stop.min(m.end());
        push(&mut keys, m.start, to_left(m.from));
        let mut j = 1;
        loop {
            let t = m.start + j as f64 / p.key_rate;
            if t >= stop - 1e-6 {
                break;
            }
            push(&mut keys, t, to_left(m.at(t)));
            j += 1;
        }
        push(&mut keys, stop, to_left(m.at(stop)));
    }
    // Hold keys equal to the previous key and not followed by a move are redundant.
    let mut out: Vec<CropKey> = Vec::with_capacity(keys.len());
    for (i, k) in keys.iter().enumerate() {
        let prev_same = out.last().is_some_and(|l: &CropKey| l.x == k.x);
        let next_moves = keys.get(i + 1).is_some_and(|nk| nk.x != k.x);
        if prev_same && !next_moves {
            continue;
        }
        out.push(*k);
    }
    let shift_mean = if n > 0 { shift.iter().sum::<f64>() / n as f64 } else { 0.0 };
    CameraPlan {
        keys: out,
        moves: moves.iter().filter(|m| m.dur > 0.0).count(),
        cuts,
        look_room_shift: round(shift_mean, 3),
    }
}

#[allow(clippy::too_many_arguments)]
fn hold_or_move(
    times: &[f64],
    target: &[f64],
    // (margin limit that triggers a move, margin limit for the new rest position)
    (allowed, rest_range): (&[(f64, f64)], &[(f64, f64)]),
    a: usize,
    b: usize,
    cam: &mut f64,
    busy: &mut f64,
    moves: &mut Vec<Move>,
    p: &CameraParams,
    safe: f64,
    hard: f64,
) {
    // Tolerance on the face margin (frame widths): detection jitter must not trigger moves.
    const MARGIN_TOL: f64 = 0.004;
    let outside = |j: usize, c: f64| c < allowed[j].0 - MARGIN_TOL || c > allowed[j].1 + MARGIN_TOL;
    let mut i = a;
    while i < b {
        if times[i] < *busy {
            i += 1;
            continue;
        }
        let d = target[i] - *cam;
        // The face is too near a crop edge for a sustained moment (two samples): move now.
        let squeezed = outside(i, *cam) && (i + 1 >= b || outside(i + 1, *cam));
        if d.abs() <= safe && !squeezed {
            i += 1;
            continue;
        }
        let side = if d.abs() > 1e-9 { d.signum() } else { 1.0 };
        // Is the subject really out (most of the next `min_out`), or about to leave the frame?
        let horizon = times[i] + p.min_out;
        let (mut out_n, mut tot) = (0usize, 0usize);
        let mut last_t = times[i];
        for j in i..b {
            if times[j] > horizon + 1e-9 {
                break;
            }
            tot += 1;
            last_t = times[j];
            if (target[j] - *cam) * side > safe {
                out_n += 1;
            }
        }
        let covered = last_t - times[i] >= p.min_out * 0.75 || b == times.len();
        let persistent = covered && out_n as f64 >= 0.75 * tot as f64;
        if !(persistent || d.abs() > hard || squeezed) {
            i += 1;
            continue;
        }
        let new = median(&window(times, target, i, b, times[i] + p.lookahead)).clamp(rest_range[i].0, rest_range[i].1);
        let min_step = if squeezed { MARGIN_TOL } else { safe * 0.5 };
        if (new - *cam).abs() <= min_step {
            i += 1;
            continue;
        }
        // When did the subject cross the box edge? (Linear between the two samples.)
        let tx = if i > a {
            let d0 = target[i - 1] - *cam;
            let edge = safe * side;
            let f = if (d - d0).abs() > 1e-12 { ((edge - d0) / (d - d0)).clamp(0.0, 1.0) } else { 1.0 };
            times[i - 1] + f * (times[i] - times[i - 1])
        } else {
            times[i]
        };
        // Onset: walk back while the target keeps moving toward this side.
        let mut on = i;
        while on > a && (target[on] - target[on - 1]) * side > 0.004 {
            on -= 1;
        }
        let lead = (times[on] + 0.1).max(tx - p.max_lead);
        let start = (tx - p.anticipation).min(lead).max(*busy).max(times[a].min(tx)).max(0.0);
        let same_dir = |m: &Move| m.dur > 0.0 && m.stop >= m.end() && (m.to - m.from) * (new - m.to) > 0.0;
        match moves.last_mut().filter(|m| same_dir(m) && start <= m.end() + 0.1) {
            // Leaving again right after a move in the same direction: extend it into one move
            // instead of stop-and-go.
            Some(m) => {
                m.to = new;
                m.dur = move_time(new - m.from, p.max_velocity, p.max_accel, p.min_move).max(m.dur);
                m.stop = m.end();
                *busy = m.end();
            }
            None => {
                let mut m = Move { start, dur: move_time(new - *cam, p.max_velocity, p.max_accel, p.min_move), from: *cam, to: new, stop: 0.0 };
                // Keep the face inside the frame during the move: speed up when needed.
                // At most 1/0.6 faster than the limits allow (a lag is better than a whip).
                let floor = (m.dur * 0.6).max(p.min_move * 0.6);
                for _ in 0..4 {
                    let lost = (i..b)
                        .take_while(|&j| times[j] <= m.end())
                        .any(|j| (target[j] - m.at(times[j])).abs() > hard);
                    if !lost || m.dur <= floor {
                        break;
                    }
                    m.dur = (m.dur * 0.8).max(floor);
                }
                m.stop = m.end();
                *busy = m.end();
                moves.push(m);
            }
        }
        *cam = new;
        i += 1;
    }
}

/// Piece boundaries `(first sample index, transition time (relative), kind)`: detected shot cuts
/// in the face path merged with the caller's events.
/// Unfiltered per-sample subject features (cut detection looks at single steps).
struct RawPath {
    cx: Vec<f64>,
    cy: Vec<f64>,
    yaw: Vec<Option<f64>>,
}

impl RawPath {
    /// A shot cut between two people who face each other (a reaction shot in a podcast): the face
    /// barely moves in the frame, but the head direction flips from one sample to the next and
    /// stays flipped, and the face box steps. A real head turn takes longer than one sample step.
    fn pose_flip(&self, i: usize) -> bool {
        let n = self.yaw.len();
        if i < 2 || i + 1 >= n {
            return false;
        }
        let (Some(y0), Some(a), Some(b), Some(y3)) = (self.yaw[i - 2], self.yaw[i - 1], self.yaw[i], self.yaw[i + 1]) else {
            return false;
        };
        let flip = a * b < 0.0 && a.abs() > 0.2 && b.abs() > 0.2 && (a - b).abs() > 0.7;
        let stable = y0 * a > 0.0 && y3 * b > 0.0;
        let stepped = (self.cx[i] - self.cx[i - 1]).abs() + (self.cy[i] - self.cy[i - 1]).abs() > 0.02;
        flip && stable && stepped
    }
}

fn boundaries(
    times: &[f64],
    cx: &[f64],
    widths: &[f64],
    raw: &RawPath,
    events: &[CamEvent],
    t0: f64,
    p: &CameraParams,
) -> Vec<(usize, f64, EventKind)> {
    const MERGE_S: f64 = 0.75;
    let n = times.len();
    let explicit: Vec<CamEvent> = events
        .iter()
        .map(|e| CamEvent { t: e.t - t0, kind: e.kind })
        .filter(|e| e.t > 0.0 && e.t <= times[n - 1] + 1e-9)
        .collect();
    let mut used = vec![false; explicit.len()];
    let mut out: Vec<(usize, f64, EventKind)> = Vec::new();
    for i in 1..n {
        let dx = (cx[i] - cx[i - 1]).abs();
        let wr = widths[i].max(widths[i - 1]) / widths[i].min(widths[i - 1]).max(1e-6);
        if !(dx > p.jump || (dx > p.jump * 0.5 && wr > 1.35) || raw.pose_flip(i)) {
            continue;
        }
        let td = (times[i - 1] + times[i]) / 2.0;
        let near: Vec<usize> = (0..explicit.len()).filter(|&k| (explicit[k].t - td).abs() <= MERGE_S).collect();
        near.iter().for_each(|&k| used[k] = true);
        let pick = near
            .iter()
            .copied()
            .find(|&k| explicit[k].kind == EventKind::ShotCut)
            .or_else(|| near.iter().copied().min_by(|&a, &b| (explicit[a].t - td).abs().total_cmp(&(explicit[b].t - td).abs())));
        match pick {
            Some(k) => out.push((i, explicit[k].t, explicit[k].kind)),
            None => out.push((i, td, EventKind::ShotCut)),
        }
    }
    for (k, e) in explicit.iter().enumerate() {
        if used[k] {
            continue;
        }
        // The piece starts where the face path actually changes near the event (a caller may
        // switch faces a little before the turn starts), else at the event.
        let snap = (1..n)
            .filter(|&j| (times[j] - e.t).abs() <= 0.5)
            .map(|j| (j, (cx[j] - cx[j - 1]).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .filter(|&(_, d)| d > 0.01)
            .map(|(j, _)| j);
        let i = snap.unwrap_or_else(|| times.partition_point(|&t| t < e.t));
        if i > 0 && i < n {
            out.push((i, e.t, e.kind));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then((a.2 != EventKind::ShotCut).cmp(&(b.2 != EventKind::ShotCut))));
    out.dedup_by_key(|b| b.0);
    out
}

/// Primary face per sample: the largest, but among faces of similar size the one nearest the
/// previous pick (so two similar faces trading places don't make the crop jump).
fn primary_faces(samples: &[CamSample]) -> Vec<Option<CamFace>> {
    const SIMILAR_AREA: f64 = 0.5;
    let mut prev: Option<f64> = None;
    samples
        .iter()
        .map(|s| {
            let max_area = s.faces.iter().map(CamFace::area).fold(0.0, f64::max);
            if max_area <= 0.0 {
                return None;
            }
            let pick = match prev {
                Some(pc) => s
                    .faces
                    .iter()
                    .filter(|f| f.area() >= SIMILAR_AREA * max_area)
                    .min_by(|a, b| (a.cx() - pc).abs().total_cmp(&(b.cx() - pc).abs())),
                None => s.faces.iter().max_by(|a, b| a.area().total_cmp(&b.area())),
            }
            .copied();
            if let Some(f) = pick {
                prev = Some(f.cx());
            }
            pick
        })
        .collect()
}

/// Robust horizontal extent `(left, right)` of the primary face per sample, from the samples of
/// the same piece (between `edges`) within `half` seconds, each box at its median-filtered centre
/// `cx` (a detection jump is ignored) with its own width (YuNet's box width jitters by 10-25 %
/// from sample to sample; a margin measured on the median width leaves the real boxes short of
/// it). The most extreme value in five is dropped, so a one-sample false size never widens it.
/// `spread`: the outermost edges of those boxes (where the face goes in the window); else the
/// face where it is now with the largest width. `None` where the sample has no face.
fn face_extents(times: &[f64], prim: &[Option<CamFace>], cx: &[f64], edges: &[usize], half: f64, spread: bool) -> Vec<Option<(f64, f64)>> {
    let mut out = vec![None; prim.len()];
    for w in edges.windows(2) {
        let (a, b) = (w[0], w[1]);
        for i in a..b {
            if prim[i].is_none() {
                continue;
            }
            // Boxes of the same face: a detection jump (far from the filtered centre) is left out.
            let near: Vec<usize> = (a..b)
                .filter(|&j| (times[j] - times[i]).abs() <= half + 1e-9)
                .filter(|&j| prim[j].is_some_and(|g| (g.cx() - cx[j]).abs() <= 0.5 * g.w))
                .collect();
            if near.is_empty() {
                let w = prim[i].map_or(0.0, |g| g.w);
                out[i] = Some((cx[i] - w / 2.0, cx[i] + w / 2.0));
                continue;
            }
            let drop = (near.len() / 5).min(near.len() - 1);
            let half_w = |j: usize| prim[j].map_or(0.0, |g| g.w / 2.0);
            if spread {
                // The raw boxes: their position jitters too.
                let mut ls: Vec<f64> = near.iter().map(|&j| prim[j].map_or(cx[j], |g| g.x)).collect();
                let mut rs: Vec<f64> = near.iter().map(|&j| prim[j].map_or(cx[j], |g| g.x + g.w)).collect();
                ls.sort_by(f64::total_cmp);
                rs.sort_by(|x, y| y.total_cmp(x));
                out[i] = Some((ls[drop], rs[drop]));
            } else {
                let mut ws: Vec<f64> = near.iter().map(|&j| half_w(j)).collect();
                ws.sort_by(|x, y| y.total_cmp(x));
                out[i] = Some((cx[i] - ws[drop], cx[i] + ws[drop]));
            }
        }
    }
    out
}

/// Gaps hold the last value; leading gaps take the first value; nothing at all -> `default`.
fn fill(v: &[Option<f64>], default: f64) -> Vec<f64> {
    let first = v.iter().flatten().next().copied().unwrap_or(default);
    let mut last = first;
    v.iter()
        .map(|x| {
            if let Some(x) = x {
                last = *x;
            }
            last
        })
        .collect()
}

fn median_filter(v: &[f64], window: usize) -> Vec<f64> {
    if window <= 1 || v.len() < 3 {
        return v.to_vec();
    }
    let half = window / 2;
    (0..v.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(v.len());
            median(&v[lo..hi])
        })
        .collect()
}

fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.5;
    }
    let mut w = v.to_vec();
    w.sort_by(f64::total_cmp);
    let n = w.len();
    if n % 2 == 1 { w[n / 2] } else { (w[n / 2 - 1] + w[n / 2]) / 2.0 }
}

/// Target values at indices `a..b` with time <= `until` (at least one).
fn window(times: &[f64], target: &[f64], a: usize, b: usize, until: f64) -> Vec<f64> {
    let mut w: Vec<f64> = (a..b).take_while(|&j| times[j] <= until + 1e-9).map(|j| target[j]).collect();
    if w.is_empty() {
        w.push(target[a]);
    }
    w
}

fn round(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

// ---------------------------------------------------------------------------------------------
// Motion metrics (QC `camera_motion`)
// ---------------------------------------------------------------------------------------------

/// How a crop path moves, measured the way the renderer plays it (linear between keys,
/// sampled at 30 fps). Speeds in source frame widths per second.
#[derive(Serialize, Clone, Copy, PartialEq, Debug, Default)]
pub struct MotionStats {
    pub duration_s: f64,
    /// Separate camera moves (runs of motion).
    pub moves: usize,
    /// Hard jumps (two keys at the same time).
    pub cuts: usize,
    pub max_velocity: f64,
    /// At 10 Hz, after each cut.
    pub max_accel: f64,
    pub max_jerk: f64,
    /// Wasted back-and-forth motion: per 1 s window, path length minus net travel, averaged per
    /// second. 0 for eased one-way moves.
    pub jitter: f64,
    /// Direction changes larger than 0.4% of the frame.
    pub reversals: usize,
    /// Moves that start at full speed (velocity jumps from rest within one frame): robotic.
    pub abrupt_starts: usize,
}

/// Motion stats of one crop path (keys of [`plan`] or any stored path).
pub fn motion_stats(keys: &[CropKey]) -> MotionStats {
    const FPS: f64 = 30.0;
    const MOVING: f64 = 0.01;
    const REVERSAL: f64 = 0.004;
    let mut keys: Vec<CropKey> = keys.iter().copied().filter(|k| k.t.is_finite() && k.x.is_finite()).collect();
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut st = MotionStats::default();
    if keys.len() < 2 {
        st.duration_s = 0.0;
        return st;
    }
    // Split into cut-free pieces.
    let mut pieces: Vec<Vec<CropKey>> = vec![vec![keys[0]]];
    for w in keys.windows(2) {
        if w[1].t - w[0].t < 1e-3 {
            if (w[1].x - w[0].x).abs() > 1e-6 {
                st.cuts += 1;
            }
            pieces.push(vec![w[1]]);
        } else {
            pieces.last_mut().unwrap().push(w[1]);
        }
    }
    st.duration_s = keys[keys.len() - 1].t - keys[0].t;
    for piece in &pieces {
        if piece.len() < 2 {
            continue;
        }
        let (a, b) = (piece[0].t, piece[piece.len() - 1].t);
        let at = |t: f64| -> f64 {
            let k = piece.partition_point(|k| k.t <= t);
            if k == 0 {
                return piece[0].x;
            }
            if k >= piece.len() {
                return piece[piece.len() - 1].x;
            }
            let (p0, p1) = (piece[k - 1], piece[k]);
            p0.x + (p1.x - p0.x) * (t - p0.t) / (p1.t - p0.t)
        };
        let frames = ((b - a) * FPS).floor() as usize;
        let xs: Vec<f64> = (0..=frames).map(|f| at(a + f as f64 / FPS)).collect();
        let v: Vec<f64> = xs.windows(2).map(|w| (w[1] - w[0]) * FPS).collect();
        let mut moving = false;
        for (f, &vf) in v.iter().enumerate() {
            st.max_velocity = st.max_velocity.max(vf.abs());
            let now = vf.abs() > MOVING;
            if now && !moving {
                st.moves += 1;
                if vf.abs() > 0.15 && (f == 0 || v[f - 1].abs() < 0.02) {
                    st.abrupt_starts += 1;
                }
            }
            moving = now;
        }
        // Jitter: path minus net travel per 1 s window.
        let win = FPS as usize;
        let mut wasted = 0.0;
        for chunk in xs.chunks(win + 1) {
            if chunk.len() < 2 {
                continue;
            }
            let path: f64 = chunk.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            wasted += path - (chunk[chunk.len() - 1] - chunk[0]).abs();
        }
        st.jitter += wasted;
        // Reversals (zigzag with hysteresis).
        let (mut dir, mut ext) = (0i8, xs[0]);
        for &x in &xs {
            match dir {
                0 if (x - ext).abs() > REVERSAL => {
                    dir = if x > ext { 1 } else { -1 };
                    ext = x;
                }
                1 if x > ext => ext = x,
                -1 if x < ext => ext = x,
                1 | -1 if (x - ext).abs() > REVERSAL => {
                    st.reversals += 1;
                    dir = -dir;
                    ext = x;
                }
                _ => {}
            }
        }
        // Acceleration and jerk at 10 Hz.
        let step = 0.1;
        let n10 = ((b - a) / step).floor() as usize;
        let x10: Vec<f64> = (0..=n10).map(|k| at(a + k as f64 * step)).collect();
        let v10: Vec<f64> = x10.windows(2).map(|w| (w[1] - w[0]) / step).collect();
        let a10: Vec<f64> = v10.windows(2).map(|w| (w[1] - w[0]) / step).collect();
        for w in a10.windows(2) {
            st.max_jerk = st.max_jerk.max(((w[1] - w[0]) / step).abs());
        }
        for x in &a10 {
            st.max_accel = st.max_accel.max(x.abs());
        }
    }
    if st.duration_s > 0.0 {
        st.jitter /= st.duration_s;
    }
    st.max_velocity = round(st.max_velocity, 4);
    st.max_accel = round(st.max_accel, 3);
    st.max_jerk = round(st.max_jerk, 2);
    st.jitter = round(st.jitter, 5);
    st.duration_s = round(st.duration_s, 3);
    st
}
