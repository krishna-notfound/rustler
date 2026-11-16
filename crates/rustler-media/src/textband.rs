//! Burned-in text bands of a source (subtitles, titles, watermarks), engine-free.
//!
//! A Rust port of the band finder `reference.measure` uses (engine `reference.py`): per frame,
//! rows whose horizontal-edge density (|Sobel-x| > 160, dilated 7 px sideways) stands out from a
//! rolling-median baseline form a band when the run is 1.2-10 % of the frame tall and wider than
//! 10 % of it. Bands are clustered by vertical position over the samples; a cluster whose content
//! keeps changing between samples of the same shot (and not only when the picture moves) is a
//! **subtitle** band, a cluster that is up most of the time and does not change is **static** text
//! (logo, channel name, lower-third, or picture structure such as a table edge).
//!
//! These are measurements to combine with looking at a frame, not OCR: no text is read.

use crate::proc::{self, argv};
use rustler_core::{ErrorCode, RustlerError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// Width frames are scaled to before measuring.
pub const SCAN_WIDTH: usize = 640;

/// One grayscale frame.
#[derive(Clone, Debug)]
pub struct GrayFrame {
    /// Source seconds.
    pub t: f64,
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// A text-like band of one frame (fractions of the frame).
#[derive(Clone, Debug)]
pub struct FrameBand {
    pub y0: f64,
    pub y1: f64,
    pub x0: f64,
    pub x1: f64,
    /// 64 x 8 area-averaged edge mask: what the band looks like (compared between samples).
    pub sig: Vec<f32>,
    pub density: f64,
}

/// `subtitles` (content changes inside a shot) | `static` (stays the same).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Subtitles,
    Static,
}

/// A burned-in text band of the source, in fractions of the source frame.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct TextBand {
    pub kind: TextKind,
    pub y0: f64,
    pub y1: f64,
    pub x0: f64,
    pub x1: f64,
    /// Share of the samples the band is visible in (0..1).
    pub presence: f64,
    /// Share of consecutive same-shot sample pairs whose band content changed (0..1).
    pub change: f64,
    /// Source seconds of the first / last sample the band was seen in.
    pub first_t: f64,
    pub last_t: f64,
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0 + 0.0
}

/// Edge mask (|Sobel-x| > 160) dilated with a 1x7 horizontal kernel.
fn dilated_edges(f: &GrayFrame) -> Vec<u8> {
    let (w, h) = (f.w, f.h);
    let p = |x: usize, y: usize| f.px[y * w + x] as i32;
    let mut m = vec![0u8; w * h];
    for y in 0..h {
        let ya = y.saturating_sub(1);
        let yb = (y + 1).min(h - 1);
        for x in 0..w {
            let xa = x.saturating_sub(1);
            let xb = (x + 1).min(w - 1);
            let gx = (p(xb, ya) + 2 * p(xb, y) + p(xb, yb)) - (p(xa, ya) + 2 * p(xa, y) + p(xa, yb));
            if gx.abs() > 160 {
                m[y * w + x] = 1;
            }
        }
    }
    let mut d = vec![0u8; w * h];
    for y in 0..h {
        let row = &m[y * w..(y + 1) * w];
        for x in 0..w {
            let a = x.saturating_sub(3);
            let b = (x + 3).min(w - 1);
            if row[a..=b].iter().any(|&v| v != 0) {
                d[y * w + x] = 1;
            }
        }
    }
    d
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Text-like horizontal bands of one frame.
pub fn find_bands(f: &GrayFrame) -> Vec<FrameBand> {
    let (w, h) = (f.w, f.h);
    if w < 16 || h < 16 || f.px.len() < w * h {
        return Vec::new();
    }
    let d = dilated_edges(f);
    let p: Vec<f64> = (0..h).map(|y| d[y * w..(y + 1) * w].iter().map(|&v| v as f64).sum::<f64>() / w as f64).collect();
    // moving average (zero padded, like numpy "same")
    let k = (((h as f64 * 0.006) as usize) | 1).max(3);
    let half = k / 2;
    let ps: Vec<f64> = (0..h)
        .map(|y| {
            let mut s = 0.0;
            for j in 0..k {
                let yy = y as isize + j as isize - half as isize;
                if yy >= 0 && (yy as usize) < h {
                    s += p[yy as usize];
                }
            }
            s / k as f64
        })
        .collect();
    let win = ((h as f64 * 0.10) as usize).max(3);
    let base: Vec<f64> = (0..h)
        .map(|y| {
            let a = y.saturating_sub(win);
            let b = (y + win + 1).min(h);
            let mut v: Vec<f64> = ps[a..b].to_vec();
            median(&mut v)
        })
        .collect();
    let rows: Vec<bool> = (0..h).map(|y| ps[y] - base[y] > 0.04 && ps[y] > 0.07).collect();
    let gap = ((h as f64 * 0.012) as usize).max(1);
    let mut runs = Vec::new();
    let mut y = 0;
    while y < h {
        if rows[y] {
            let y0 = y;
            while y < h && (rows[y] || rows[(y + gap).min(h - 1)]) {
                y += 1;
            }
            runs.push((y0, y));
        }
        y += 1;
    }
    let mut out = Vec::new();
    for (y0, y1) in runs {
        let bh = y1 - y0;
        if (bh as f64) < h as f64 * 0.012 || (bh as f64) > h as f64 * 0.10 || bh == 0 {
            continue;
        }
        let cols: Vec<bool> = (0..w)
            .map(|x| (y0..y1).map(|yy| d[yy * w + x] as f64).sum::<f64>() / bh as f64 > 0.15)
            .collect();
        let (Some(xa), Some(xb)) = (cols.iter().position(|&c| c), cols.iter().rposition(|&c| c)) else { continue };
        if ((xb - xa) as f64) < w as f64 * 0.10 {
            continue;
        }
        // 64 x 8 area average of the band's edge mask
        let mut sig = vec![0f32; 64 * 8];
        for (r, s) in sig.chunks_mut(64).enumerate() {
            let ra = y0 + r * bh / 8;
            let rb = (y0 + (r + 1) * bh / 8).max(ra + 1).min(y1);
            for (c, cell) in s.iter_mut().enumerate() {
                let ca = c * w / 64;
                let cb = ((c + 1) * w / 64).max(ca + 1).min(w);
                let mut sum = 0u32;
                for yy in ra..rb {
                    for xx in ca..cb {
                        sum += d[yy * w + xx] as u32;
                    }
                }
                *cell = sum as f32 / ((rb - ra) * (cb - ca)) as f32;
            }
        }
        let density = (y0..y1).map(|yy| d[yy * w..(yy + 1) * w].iter().map(|&v| v as f64).sum::<f64>()).sum::<f64>() / (bh * w) as f64;
        out.push(FrameBand { y0: y0 as f64 / h as f64, y1: y1 as f64 / h as f64, x0: xa as f64 / w as f64, x1: xb as f64 / w as f64, sig, density });
    }
    out
}

fn sig_change(a: &[f32], b: &[f32]) -> f64 {
    let diff: f64 = a.iter().zip(b).map(|(x, y)| (x - y).abs() as f64).sum();
    let tot: f64 = a.iter().chain(b).map(|&x| x as f64).sum();
    diff / (tot + 1e-6) * 2.0
}

/// Mean absolute luma difference (0..255) of each row of two frames (every 4th pixel).
fn row_diffs(a: &GrayFrame, b: &GrayFrame) -> Vec<f64> {
    if a.w != b.w || a.h != b.h || a.px.len() < a.w * a.h || b.px.len() < b.w * b.h {
        return vec![255.0; a.h.max(1)];
    }
    (0..a.h)
        .map(|y| {
            let (mut s, mut n) = (0.0, 0.0);
            for x in (0..a.w).step_by(4) {
                let i = y * a.w + x;
                s += (a.px[i] as f64 - b.px[i] as f64).abs();
                n += 1.0;
            }
            if n > 0.0 { s / n } else { 0.0 }
        })
        .collect()
}

/// Mean of the row differences outside `y0..y1` (fractions): how much the picture around a band
/// moved, so the band's own change does not count as motion.
fn motion_outside(rows: &[f64], y0: f64, y1: f64) -> f64 {
    let h = rows.len() as f64;
    let (a, b) = (((y0 - 0.02) * h).floor().max(0.0) as usize, ((y1 + 0.02) * h).ceil().min(h) as usize);
    let (mut s, mut n) = (0.0, 0.0);
    for (y, d) in rows.iter().enumerate() {
        if y < a || y >= b {
            s += d;
            n += 1.0;
        }
    }
    if n > 0.0 { s / n } else { 0.0 }
}

/// A frame difference above this is a shot change (band changes across it do not count).
const CUT_DIFF: f64 = 28.0;

/// Clusters per-frame bands over the samples and names each cluster.
pub fn classify(frames: &[GrayFrame]) -> Vec<TextBand> {
    let per: Vec<Vec<FrameBand>> = frames.iter().map(find_bands).collect();
    let n = frames.len();
    if n < 2 {
        return Vec::new();
    }
    let rows: Vec<Vec<f64>> = (0..n).map(|i| if i + 1 < n { row_diffs(&frames[i], &frames[i + 1]) } else { Vec::new() }).collect();
    let mut items: Vec<(usize, f64, &FrameBand)> = Vec::new();
    for (si, bands) in per.iter().enumerate() {
        for b in bands {
            items.push((si, (b.y0 + b.y1) / 2.0, b));
        }
    }
    items.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut groups: Vec<Vec<(usize, f64, &FrameBand)>> = Vec::new();
    for it in items {
        match groups.last_mut() {
            Some(g) if it.1 - g.last().map_or(it.1, |l| l.1) <= 0.02 => g.push(it),
            _ => groups.push(vec![it]),
        }
    }
    let mut out = Vec::new();
    for g in groups {
        // one band per sample: the widest
        let mut by: std::collections::BTreeMap<usize, &FrameBand> = Default::default();
        for (si, _, b) in &g {
            let wider = by.get(si).is_none_or(|o| (b.x1 - b.x0) > (o.x1 - o.x0));
            if wider {
                by.insert(*si, b);
            }
        }
        if by.len() < 2 {
            continue;
        }
        let present: Vec<usize> = by.keys().copied().collect();
        let mut changes = Vec::new();
        let (mut mot_changed, mut mot_still) = (Vec::new(), Vec::new());
        for w in present.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b != a + 1 {
                continue;
            }
            let motion = motion_outside(&rows[a], by[&a].y0.min(by[&b].y0), by[&a].y1.max(by[&b].y1));
            if motion > CUT_DIFF {
                continue;
            }
            let ch = sig_change(&by[&a].sig, &by[&b].sig);
            changes.push(ch);
            if ch > 0.35 {
                mot_changed.push(motion);
            } else if ch < 0.2 {
                mot_still.push(motion);
            }
        }
        let frac = |f: &dyn Fn(f64) -> bool| if changes.is_empty() { 0.0 } else { changes.iter().filter(|c| f(**c)).count() as f64 / changes.len() as f64 };
        let change = frac(&|c| c > 0.35);
        let plateau = frac(&|c| c < 0.2);
        let motion_ratio = if !mot_changed.is_empty() && !mot_still.is_empty() {
            let m = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len() as f64;
            m(&mot_changed) / (m(&mot_still) + 0.5)
        } else {
            1.0
        };
        let mut y0s: Vec<f64> = present.iter().map(|i| by[i].y0).collect();
        let mut y1s: Vec<f64> = present.iter().map(|i| by[i].y1).collect();
        let mut x0s: Vec<f64> = present.iter().map(|i| by[i].x0).collect();
        let mut x1s: Vec<f64> = present.iter().map(|i| by[i].x1).collect();
        x0s.sort_by(|a, b| a.total_cmp(b));
        x1s.sort_by(|a, b| a.total_cmp(b));
        let pct = |v: &Vec<f64>, q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        // the band's full extent over time (two-line subtitles grow upwards)
        y0s.sort_by(|a, b| a.total_cmp(b));
        y1s.sort_by(|a, b| a.total_cmp(b));
        let (y0, y1) = (pct(&y0s, 0.1), pct(&y1s, 0.9));
        let (x0, x1) = (pct(&x0s, 0.1), pct(&x1s, 0.9));
        let presence = present.len() as f64 / n as f64;
        let cx = (x0 + x1) / 2.0;
        let density = {
            let mut d: Vec<f64> = present.iter().map(|i| by[i].density).collect();
            median(&mut d)
        };
        let subtitles = presence >= 0.25
            && change >= 0.2
            && plateau >= 0.15
            && motion_ratio < 2.5
            && (x1 - x0) >= 0.15
            && (cx - 0.5).abs() <= 0.2
            && density >= 0.12;
        let kind = if subtitles {
            TextKind::Subtitles
        } else if presence >= 0.5 && change < 0.15 && (x1 - x0) >= 0.1 {
            TextKind::Static
        } else {
            continue;
        };
        out.push(TextBand {
            kind,
            y0: r3(y0),
            y1: r3(y1),
            x0: r3(x0),
            x1: r3(x1),
            presence: r3(presence),
            change: r3(change),
            first_t: r3(frames[present[0]].t),
            last_t: r3(frames[*present.last().unwrap_or(&0)].t),
        });
    }
    // Subtitle bands stacked within 3 % of each other are one band (one- and two-line cues).
    let mut merged: Vec<TextBand> = Vec::new();
    for b in out {
        if let Some(m) = merged.iter_mut().find(|m| m.kind == b.kind && b.kind == TextKind::Subtitles && b.y0 <= m.y1 + 0.03 && b.y1 >= m.y0 - 0.03) {
            m.y0 = m.y0.min(b.y0);
            m.y1 = m.y1.max(b.y1);
            m.x0 = m.x0.min(b.x0);
            m.x1 = m.x1.max(b.x1);
            m.presence = m.presence.max(b.presence);
            m.change = m.change.max(b.change);
            m.first_t = m.first_t.min(b.first_t);
            m.last_t = m.last_t.max(b.last_t);
        } else {
            merged.push(b);
        }
    }
    merged
}

/// Scan height for a source of `src_w` x `src_h` at [`SCAN_WIDTH`] (even, 90..1080).
pub fn scan_height(src_w: u32, src_h: u32) -> usize {
    if src_w == 0 || src_h == 0 {
        return 360;
    }
    let h = (SCAN_WIDTH as f64 * src_h as f64 / src_w as f64 / 2.0).round() as usize * 2;
    h.clamp(90, 1080)
}

/// Grayscale frames of `media` every `every` seconds between `start` and `end` (source seconds),
/// scaled to [`SCAN_WIDTH`] x `scan_height`. One decode of the range.
pub async fn read_frames(ffmpeg: &Path, media: &Path, start: f64, end: f64, every: f64, h: usize) -> rustler_core::Result<Vec<GrayFrame>> {
    let every = every.max(0.1);
    let dur = (end - start).max(every);
    let mut args = argv!["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", format!("{:.3}", start.max(0.0)), "-t", format!("{dur:.3}"), "-i"];
    args.push(media.as_os_str().to_owned());
    args.extend(argv![
        "-an",
        "-vf",
        format!("fps={:.4},scale={SCAN_WIDTH}:{h}:flags=area,format=gray", 1.0 / every),
        "-f",
        "rawvideo",
        "-"
    ]);
    let out = proc::run(ffmpeg, &args, None, &CancellationToken::new()).await.map_err(|mut e| {
        e.stage = "caption.place".into();
        e
    })?;
    let size = SCAN_WIDTH * h;
    if out.stdout.len() < size {
        return Err(RustlerError::new(ErrorCode::Media, "caption.place", format!("no frames decoded from {} at {start:.2}-{end:.2}s", media.display())));
    }
    Ok(out
        .stdout
        .chunks_exact(size)
        .enumerate()
        .map(|(k, px)| GrayFrame { t: start + k as f64 * every, w: SCAN_WIDTH, h, px: px.to_vec() })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mid-grey frame with a striped "text line" between rows `y0..y1` whose stripe pattern
    /// depends on `phase` (different words), plus soft noise.
    fn frame(t: f64, text: Option<(usize, usize, usize)>, w: usize, h: usize) -> GrayFrame {
        let mut px = vec![110u8; w * h];
        for y in 0..h {
            for x in 0..w {
                px[y * w + x] = (100 + ((x * 7 + y * 3) % 11)) as u8;
            }
        }
        if let Some((y0, y1, phase)) = text {
            let (xa, xb) = (w / 4, w * 3 / 4);
            for y in y0..y1 {
                for x in xa..xb {
                    // word blocks of 6 px strokes with gaps that move with the phase
                    let k = (x + phase * 13) % 24;
                    let word_gap = ((x + phase * 37) / 60) % 5 == 0;
                    px[y * w + x] = if !word_gap && k < 3 { 250 } else { 20 };
                }
            }
        }
        GrayFrame { t, w, h, px }
    }

    #[test]
    fn changing_line_is_subtitles_and_fixed_line_is_static() {
        let (w, h) = (320, 180);
        // subtitles at rows 140..152 change every frame; a static title at rows 20..30
        let frames: Vec<GrayFrame> = (0..12)
            .map(|k| {
                // a new cue every 3 samples (1.5 s), like real subtitles
                let mut f = frame(k as f64 * 0.5, Some((140, 152, k / 3)), w, h);
                let fixed = frame(0.0, Some((20, 30, 0)), w, h);
                for y in 20..30 {
                    for x in 0..w {
                        f.px[y * w + x] = fixed.px[y * w + x];
                    }
                }
                f
            })
            .collect();
        let bands = classify(&frames);
        let sub = bands.iter().find(|b| b.kind == TextKind::Subtitles).unwrap_or_else(|| panic!("subtitle band: {bands:?}"));
        assert!(sub.y0 <= 140.0 / 180.0 + 0.02 && sub.y1 >= 152.0 / 180.0 - 0.02, "{sub:?}");
        let st = bands.iter().find(|b| b.kind == TextKind::Static).unwrap_or_else(|| panic!("static band: {bands:?}"));
        assert!(st.y1 < 0.25, "{st:?}");
        assert!(sub.presence > 0.9);
    }

    #[test]
    fn no_text_no_bands() {
        let frames: Vec<GrayFrame> = (0..6).map(|k| frame(k as f64, None, 320, 180)).collect();
        assert!(classify(&frames).is_empty());
        assert_eq!(scan_height(1920, 1080), 360);
        assert_eq!(scan_height(1280, 720), 360);
    }
}
