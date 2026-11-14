//! Face samples -> smooth horizontal crop keys (pure functions, no I/O).
//!
//! Chain (offline, so no causal-latency penalty): pick the primary face per sample, fill gaps by
//! holding the last centre, median filter, One-Euro filter, dead zone, velocity clamp, then
//! convert to the crop's left edge and emit a key only when it moved.
//!
//! The One-Euro filter, dead-zone and velocity-clamp stages are a Rust port of the logic in
//! autoclip `backend/autoclip/pipeline/reframe/smoothing.py` (MIT licence), re-expressed in
//! fractions of the frame width instead of pixels. Reference: Casiez, Roussel & Vogel,
//! "1 Euro Filter" (CHI 2012).

use serde::{Deserialize, Serialize};

/// A detected face, normalized 0..1 of the frame (`x`,`y` = top-left corner).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct FaceBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub score: f64,
}

impl FaceBox {
    fn centre_x(&self) -> f64 {
        self.x + self.w / 2.0
    }
    fn area(&self) -> f64 {
        self.w * self.h
    }
}

/// All faces found at source time `t` (seconds), as produced by the engine's `faces.track`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct FaceSample {
    pub t: f64,
    pub faces: Vec<FaceBox>,
}

/// Crop keyframes are the render compiler's type, so face paths feed `compile_with` directly.
pub use crate::compile::CropKey;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SmoothParams {
    /// Median filter window in samples (odd; 0 or 1 disables).
    pub median_window: usize,
    /// One-Euro minimum cutoff (Hz). Lower filters harder at rest.
    pub min_cutoff: f64,
    /// One-Euro speed coefficient. Speed is measured in 1/1000 frame-widths per second.
    pub beta: f64,
    /// Movement below this many frame-widths is ignored.
    pub dead_zone: f64,
    /// Ceiling on crop travel in frame-widths per second.
    pub max_speed: f64,
}

impl Default for SmoothParams {
    fn default() -> Self {
        Self {
            median_window: 5,
            min_cutoff: 0.6,
            beta: 0.02,
            dead_zone: 0.02,
            max_speed: 0.25,
        }
    }
}

/// How long a lost face's last centre is held before the crop drifts back to the middle.
const HOLD_SECONDS: f64 = 3.0;
/// A face counts as "similar in size" to the largest when its area is at least this fraction.
const SIMILAR_AREA: f64 = 0.5;
/// Keys closer than this to the previous key are not emitted.
const KEY_EPSILON: f64 = 0.002;
const DERIVATIVE_CUTOFF: f64 = 1.0;

/// Smooth `samples` into crop keys for a crop that is `crop_w_norm` of the frame width.
/// Key times are relative to the first sample; the first key is always at `t = 0`.
pub fn crop_keys(samples: &[FaceSample], crop_w_norm: f64, params: &SmoothParams) -> Vec<CropKey> {
    if samples.is_empty() {
        return Vec::new();
    }
    let t0 = samples[0].t;
    let times: Vec<f64> = samples.iter().map(|s| s.t - t0).collect();
    let raw = primary_centres(samples);
    let filled = fill_gaps(&times, &raw);
    let medianed = median_filter(&filled, params.median_window);
    let smoothed = smooth(&times, &medianed, params);

    let crop_w = crop_w_norm.clamp(0.0, 1.0);
    let max_left = (1.0 - crop_w).max(0.0);
    let mut keys: Vec<CropKey> = Vec::new();
    for (&t, &centre) in times.iter().zip(&smoothed) {
        let x = (centre - crop_w / 2.0).clamp(0.0, max_left);
        match keys.last() {
            Some(last) if (x - last.x).abs() <= KEY_EPSILON => {}
            _ => keys.push(CropKey { t, x }),
        }
    }
    keys
}

/// Primary face centre per sample. The largest face wins, but among faces of similar size the
/// one nearest the previous primary is kept, so two similar faces trading places don't make the
/// crop jump.
fn primary_centres(samples: &[FaceSample]) -> Vec<Option<f64>> {
    let mut prev: Option<f64> = None;
    samples
        .iter()
        .map(|s| {
            let max_area = s.faces.iter().map(FaceBox::area).fold(0.0, f64::max);
            let pick = if max_area <= 0.0 {
                None
            } else {
                let similar = s
                    .faces
                    .iter()
                    .filter(|f| f.area() >= SIMILAR_AREA * max_area)
                    .map(FaceBox::centre_x);
                match prev {
                    Some(p) => similar.min_by(|a, b| (a - p).abs().total_cmp(&(b - p).abs())),
                    None => s
                        .faces
                        .iter()
                        .max_by(|a, b| a.area().total_cmp(&b.area()))
                        .map(FaceBox::centre_x),
                }
            };
            if pick.is_some() {
                prev = pick;
            }
            pick
        })
        .collect()
}

/// Hold the last centre across gaps for up to [`HOLD_SECONDS`], then fall back to 0.5.
/// Leading gaps take the first detection when it is within the same window.
fn fill_gaps(times: &[f64], raw: &[Option<f64>]) -> Vec<f64> {
    let first = raw.iter().position(Option::is_some);
    let mut last: Option<(f64, f64)> = None; // (time, centre)
    let mut out = Vec::with_capacity(raw.len());
    for (&t, r) in times.iter().zip(raw) {
        let v = match (r, last) {
            (Some(c), _) => {
                last = Some((t, *c));
                *c
            }
            (None, Some((lt, lc))) if t - lt <= HOLD_SECONDS => lc,
            (None, None) => match first {
                Some(fi) if times[fi] - t <= HOLD_SECONDS => raw[fi].unwrap_or(0.5),
                _ => 0.5,
            },
            (None, Some(_)) => 0.5,
        };
        out.push(v);
    }
    out
}

fn median_filter(v: &[f64], window: usize) -> Vec<f64> {
    if window <= 1 || v.len() < 2 {
        return v.to_vec();
    }
    let half = window / 2;
    (0..v.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(v.len());
            let mut w: Vec<f64> = v[lo..hi].to_vec();
            w.sort_by(f64::total_cmp);
            let n = w.len();
            if n % 2 == 1 {
                w[n / 2]
            } else {
                (w[n / 2 - 1] + w[n / 2]) / 2.0
            }
        })
        .collect()
}

fn alpha(cutoff: f64, dt: f64) -> f64 {
    let tau = 1.0 / (2.0 * std::f64::consts::PI * cutoff);
    1.0 / (1.0 + tau / dt)
}

/// One-Euro filter, then dead zone, then velocity clamp (port of autoclip `smooth_series`).
fn smooth(times: &[f64], values: &[f64], p: &SmoothParams) -> Vec<f64> {
    let mut out = Vec::with_capacity(values.len());
    let mut held = 0.0;
    // One-Euro state.
    let mut filtered = 0.0;
    let mut deriv_filtered = 0.0;
    let mut last_sample = 0.0;
    let mut last_time = 0.0;
    for (i, (&t, &raw)) in times.iter().zip(values).enumerate() {
        if i == 0 {
            filtered = raw;
            held = raw;
            last_sample = raw;
            last_time = t;
            out.push(held);
            continue;
        }
        let dt = t - last_time;
        let f = if dt <= 0.0 {
            // Non-increasing timestamp: restart the filter on this sample.
            filtered = raw;
            deriv_filtered = 0.0;
            raw
        } else {
            let deriv = (raw - last_sample) / dt;
            deriv_filtered += alpha(DERIVATIVE_CUTOFF, dt) * (deriv - deriv_filtered);
            let cutoff = p.min_cutoff + p.beta * deriv_filtered.abs() * 1000.0;
            filtered += alpha(cutoff, dt) * (raw - filtered);
            filtered
        };
        let step_dt = (t - last_time).max(1e-6);
        last_sample = raw;
        last_time = t;

        if (f - held).abs() >= p.dead_zone {
            let max_step = p.max_speed * step_dt;
            let delta = f - held;
            held = if delta.abs() > max_step {
                held + max_step.copysign(delta)
            } else {
                f
            };
        }
        out.push(held);
    }
    out
}
