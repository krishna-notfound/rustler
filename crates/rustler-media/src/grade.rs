//! Colour grading (f38): the stored [`Grade`], built-in presets, the FFmpeg filter chain a grade
//! compiles to, `.cube` LUT checks and cheap frame statistics for `grade.measure`.
//!
//! A grade is a handful of optional numbers; unset = neutral, and a neutral grade compiles to no
//! filter at all. The filters run in this order (only the ones the grade needs):
//!
//! 1. `colorcorrect` (tint + shadows / midtones / highlights as chroma offsets ramped by luma; YUV)
//! 2. `colortemperature` (white balance, lightness preserved)
//! 3. `curves` master curve (exposure, contrast, blacks; monotone `pchip`, 0 and 1 stay put)
//! 4. `colorchannelmixer` (saturation around Rec.709 luma)
//! 5. `vibrance`
//! 6. `lut3d` (blended with `blend` when `lut_strength` < 1)
//!
//! `intensity` scales the whole grade toward neutral (it changes the numbers, not the graph), so a
//! grade at 0.5 is exactly half as strong. The render puts the chain on each segment's source
//! picture before layout scaling, crops and composition, and before captions, text and images.

use rustler_core::{ErrorCode, RustlerError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

use crate::proc::{self, argv};

/// Shadows / midtones / highlights colour push, each channel -1..1 (0 = none).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
pub struct Rgb {
    #[serde(default)]
    pub r: f64,
    #[serde(default)]
    pub g: f64,
    #[serde(default)]
    pub b: f64,
}

impl Rgb {
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }
    fn scaled(self, k: f64) -> Self {
        Self { r: self.r * k, g: self.g * k, b: self.b * k }
    }
    fn is_zero(&self) -> bool {
        self.r.abs() < 1e-6 && self.g.abs() < 1e-6 && self.b.abs() < 1e-6
    }
}

/// A colour grade. Every field is optional; unset = neutral.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct Grade {
    /// Built-in look the numbers below start from (`grade.presets`); explicit fields win.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// -1..1: brightens (+) or darkens (-) the midtones; black and white points stay put.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f64>,
    /// 0.5..1.5 (1 = unchanged): S-curve around mid grey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f64>,
    /// 0..2 (1 = unchanged, 0 = black and white).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f64>,
    /// -1..1: boosts (+) muted colours more than already saturated ones (skin-friendly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrance: Option<f64>,
    /// 2000..12000 Kelvin light colour (6500 = unchanged); lower = warmer picture, higher = cooler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// -1..1: green (-) / magenta (+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<f64>,
    /// -1..1: crushes (-) or lifts / fades (+) the black point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blacks: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<Rgb>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midtones: Option<Rgb>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlights: Option<Rgb>,
    /// A `.cube` 3D LUT: `asset:<id>` from the asset library or a file path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<String>,
    /// 0..1 mix of the LUT with the picture before it (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut_strength: Option<f64>,
    /// 0..1 strength of the whole grade (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f64>,
    /// Segment overrides only: true = ignore the clip grade (default: merged onto it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standalone: Option<bool>,
}

/// What the `grade` extras key holds: a clip-wide grade plus per-segment overrides (merged field
/// by field onto the clip grade).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct GradeDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Grade>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub segments: std::collections::BTreeMap<String, Grade>,
}

impl GradeDoc {
    pub fn is_empty(&self) -> bool {
        self.clip.as_ref().is_none_or(Grade::is_unset) && self.segments.values().all(Grade::is_unset)
    }

    /// The grade a segment renders with: its override merged onto the clip grade.
    pub fn for_segment(&self, seg: &str) -> Option<Grade> {
        match (&self.clip, self.segments.get(seg)) {
            (None, None) => None,
            (Some(c), None) => Some(c.clone()),
            (None, Some(s)) => Some(s.clone()),
            (Some(_), Some(s)) if s.standalone == Some(true) => Some(s.clone()),
            (Some(c), Some(s)) => Some(c.merged(s)),
        }
    }
}

/// One built-in look.
pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub grade: Grade,
}

const fn g() -> Grade {
    Grade {
        preset: None,
        exposure: None,
        contrast: None,
        saturation: None,
        vibrance: None,
        temperature: None,
        tint: None,
        blacks: None,
        shadows: None,
        midtones: None,
        highlights: None,
        lut: None,
        lut_strength: None,
        intensity: None,
        standalone: None,
    }
}

/// The built-in presets (tuned on real podcast footage: skin stays natural, whites do not clip).
pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "natural-pop",
            description: "subtle: a little contrast and vibrance; safe default when footage looks flat",
            grade: Grade { contrast: Some(1.08), vibrance: Some(0.3), saturation: Some(1.03), ..g() },
        },
        Preset {
            name: "warm",
            description: "warmer white balance and a touch of vibrance; friendly, golden-hour feel",
            grade: Grade { temperature: Some(5500.0), vibrance: Some(0.1), ..g() },
        },
        Preset {
            name: "cool",
            description: "cooler white balance; clean, techy, night feel",
            grade: Grade { temperature: Some(8200.0), saturation: Some(0.95), ..g() },
        },
        Preset {
            name: "cinematic",
            description: "teal shadows, warm highlights, a bit more contrast (teal-orange film look)",
            grade: Grade {
                contrast: Some(1.12),
                saturation: Some(0.95),
                shadows: Some(Rgb::new(-0.06, 0.0, 0.07)),
                highlights: Some(Rgb::new(0.05, 0.01, -0.05)),
                ..g()
            },
        },
        Preset {
            name: "moody",
            description: "darker, desaturated, deeper blacks, cool shadows; serious or dramatic talk",
            grade: Grade {
                exposure: Some(-0.12),
                contrast: Some(1.12),
                saturation: Some(0.72),
                blacks: Some(-0.35),
                shadows: Some(Rgb::new(-0.02, 0.0, 0.03)),
                ..g()
            },
        },
        Preset {
            name: "bright-clean",
            description: "lifted midtones, neutral white balance, gentle vibrance; vlog / tutorial look",
            grade: Grade { exposure: Some(0.25), contrast: Some(1.03), vibrance: Some(0.15), ..g() },
        },
        Preset {
            name: "bw",
            description: "black and white with a little extra contrast",
            grade: Grade { saturation: Some(0.0), contrast: Some(1.15), ..g() },
        },
        Preset {
            name: "vintage",
            description: "faded blacks, softer colour, warm highlights; retro film feel",
            grade: Grade {
                blacks: Some(0.35),
                contrast: Some(0.95),
                saturation: Some(0.8),
                temperature: Some(5800.0),
                highlights: Some(Rgb::new(0.05, 0.03, -0.06)),
                shadows: Some(Rgb::new(0.0, 0.02, 0.06)),
                ..g()
            },
        },
    ]
}

pub fn find_preset(name: &str) -> Option<Preset> {
    let n = name.trim().to_ascii_lowercase().replace('_', "-");
    presets().into_iter().find(|p| p.name == n)
}

pub fn preset_names() -> Vec<&'static str> {
    presets().iter().map(|p| p.name).collect()
}

/// Every number a grade compiles from, after preset + overrides + intensity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub exposure: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub vibrance: f64,
    pub temperature: f64,
    pub tint: f64,
    pub blacks: f64,
    pub shadows: Rgb,
    pub midtones: Rgb,
    pub highlights: Rgb,
    pub lut_strength: f64,
}

const NEUTRAL_K: f64 = 6500.0;
const EPS: f64 = 1e-4;

fn check(name: &str, v: Option<f64>, lo: f64, hi: f64) -> Result<(), String> {
    match v {
        Some(x) if !(x.is_finite() && x >= lo - 1e-9 && x <= hi + 1e-9) => Err(format!("{name} must be between {lo} and {hi} (got {x})")),
        _ => Ok(()),
    }
}

impl Grade {
    /// True when no field is set at all.
    pub fn is_unset(&self) -> bool {
        *self == Grade::default()
    }

    /// `other`'s set fields over `self`'s.
    pub fn merged(&self, other: &Grade) -> Grade {
        macro_rules! pick {
            ($($f:ident),*) => { Grade { $($f: other.$f.clone().or_else(|| self.$f.clone()),)* } };
        }
        pick!(preset, exposure, contrast, saturation, vibrance, temperature, tint, blacks, shadows, midtones, highlights, lut, lut_strength, intensity, standalone)
    }

    /// Range and preset checks; the message names the field.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(p) = &self.preset
            && find_preset(p).is_none()
        {
            return Err(format!("unknown preset '{p}' (presets: {})", preset_names().join(", ")));
        }
        check("exposure", self.exposure, -1.0, 1.0)?;
        check("contrast", self.contrast, 0.5, 1.5)?;
        check("saturation", self.saturation, 0.0, 2.0)?;
        check("vibrance", self.vibrance, -1.0, 1.0)?;
        check("temperature", self.temperature, 2000.0, 12000.0)?;
        check("tint", self.tint, -1.0, 1.0)?;
        check("blacks", self.blacks, -1.0, 1.0)?;
        check("lut_strength", self.lut_strength, 0.0, 1.0)?;
        check("intensity", self.intensity, 0.0, 1.0)?;
        for (n, c) in [("shadows", self.shadows), ("midtones", self.midtones), ("highlights", self.highlights)] {
            if let Some(c) = c {
                for (ch, v) in [("r", c.r), ("g", c.g), ("b", c.b)] {
                    check(&format!("{n}.{ch}"), Some(v), -1.0, 1.0)?;
                }
            }
        }
        if let Some(l) = &self.lut
            && l.trim().is_empty()
        {
            return Err("lut must be asset:<id> or a .cube path".into());
        }
        Ok(())
    }

    /// Preset + explicit fields + intensity, as plain numbers.
    pub fn look(&self) -> Result<Look, String> {
        self.validate()?;
        let base = match &self.preset {
            Some(p) => find_preset(p).map(|p| p.grade).unwrap_or_default(),
            None => Grade::default(),
        };
        let m = base.merged(self);
        let k = m.intensity.unwrap_or(1.0);
        Ok(Look {
            exposure: m.exposure.unwrap_or(0.0) * k,
            contrast: 1.0 + (m.contrast.unwrap_or(1.0) - 1.0) * k,
            saturation: 1.0 + (m.saturation.unwrap_or(1.0) - 1.0) * k,
            vibrance: m.vibrance.unwrap_or(0.0) * k,
            temperature: NEUTRAL_K + (m.temperature.unwrap_or(NEUTRAL_K) - NEUTRAL_K) * k,
            tint: m.tint.unwrap_or(0.0) * k,
            blacks: m.blacks.unwrap_or(0.0) * k,
            shadows: m.shadows.unwrap_or_default().scaled(k),
            midtones: m.midtones.unwrap_or_default().scaled(k),
            highlights: m.highlights.unwrap_or_default().scaled(k),
            lut_strength: if m.lut_ref().is_some() { m.lut_strength.unwrap_or(1.0) * k } else { 0.0 },
        })
    }

    /// The LUT reference after preset merge (presets carry none, so this is the field).
    pub fn lut_ref(&self) -> Option<&str> {
        // "none" on a segment override blocks the clip's LUT.
        self.lut.as_deref().map(str::trim).filter(|s| !s.is_empty() && *s != "none")
    }
}

fn n(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

impl Look {
    /// Master tone curve value at `x` (0..1): blacks, then exposure (gamma), then contrast.
    pub fn tone(&self, x: f64) -> f64 {
        let mut y = x;
        // Blacks: + lifts the floor (fade), - pulls a soft toe down.
        if self.blacks > 0.0 {
            let t = 0.12 * self.blacks;
            y = t + y * (1.0 - t);
        } else if self.blacks < 0.0 {
            let t = 0.08 * -self.blacks;
            y = ((y - t) / (1.0 - t)).max(0.0);
        }
        // Exposure as a power curve: midtones move, 0 and 1 stay.
        y = y.powf(2f64.powf(-self.exposure * 1.2));
        // Contrast: f(x) = x + (c-1)(x-0.5)·4x(1-x), slope c at mid grey, monotone for c in 0.5..1.5.
        y += (self.contrast - 1.0) * (y - 0.5) * 4.0 * y * (1.0 - y);
        y.clamp(0.0, 1.0)
    }

    fn tone_is_identity(&self) -> bool {
        self.blacks.abs() < EPS && self.exposure.abs() < EPS && (self.contrast - 1.0).abs() < EPS
    }

    /// True when the look changes nothing (no filter needed), LUT aside.
    pub fn is_neutral(&self) -> bool {
        self.tone_is_identity()
            && (self.saturation - 1.0).abs() < EPS
            && self.vibrance.abs() < EPS
            && (self.temperature - NEUTRAL_K).abs() < 1.0
            && self.tint.abs() < EPS
            && self.shadows.is_zero()
            && self.midtones.is_zero()
            && self.highlights.is_zero()
            && self.lut_strength < EPS
    }

    /// Chroma offsets (U = blue axis, V = red axis; -0.5..0.5 units) at black and at white from
    /// the shadows / midtones / highlights pushes and the tint.
    pub fn toning(&self) -> ((f64, f64), (f64, f64)) {
        // An RGB push -> Rec.709 chroma; 0.1 on a channel moves chroma by about 6 code values.
        const K: f64 = 0.5;
        let uv = |c: Rgb| (K * (-0.1146 * c.r - 0.3854 * c.g + 0.5 * c.b), K * (0.5 * c.r - 0.4542 * c.g - 0.0458 * c.b));
        let (mu, mv) = uv(self.midtones);
        // + tint = magenta (+U +V), - = green.
        let (tu, tv) = (0.04 * self.tint, 0.04 * self.tint);
        let (su, sv) = uv(self.shadows);
        let (hu, hv) = uv(self.highlights);
        ((su + mu + tu, sv + mv + tv), (hu + mu + tu, hv + mv + tv))
    }

    /// Filters before the LUT, comma-joined; empty when neutral.
    fn pre_lut(&self) -> Vec<String> {
        let mut f = Vec::new();
        // Toning by luma (`colorcorrect` works on YUV: chroma offsets ramp from the shadow values
        // at black to the highlight values at white), so saturated colours keep their hue (a
        // per-channel filter such as colorbalance greys out pure yellows when it pushes blue
        // into the shadows). It runs first, on the decoded YUV, so the RGB filters below need
        // one conversion only. Midtones and tint shift both ends.
        let (lo, hi) = self.toning();
        if lo.0.abs() >= EPS || lo.1.abs() >= EPS || hi.0.abs() >= EPS || hi.1.abs() >= EPS {
            let c = |v: f64| n(v.clamp(-1.0, 1.0));
            f.push(format!("colorcorrect=rl={}:bl={}:rh={}:bh={}", c(lo.1), c(lo.0), c(hi.1), c(hi.0)));
        }
        if (self.temperature - NEUTRAL_K).abs() >= 1.0 {
            f.push(format!("colortemperature=temperature={}:pl=1", n(self.temperature.round())));
        }
        if !self.tone_is_identity() {
            let pts: Vec<String> = (0..=16).map(|i| i as f64 / 16.0).map(|x| format!("{}/{}", n(x), n(self.tone(x)))).collect();
            f.push(format!("curves=master='{}':interp=pchip", pts.join(" ")));
        }
        if (self.saturation - 1.0).abs() >= EPS {
            let s = self.saturation;
            let (lr, lg, lb) = (0.2126, 0.7152, 0.0722);
            let row = |own: f64, a: f64, b: f64| (n(own + (1.0 - own) * s), n(a * (1.0 - s)), n(b * (1.0 - s)));
            let (rr, rg, rb) = row(lr, lg, lb);
            let (gg, gr, gb) = row(lg, lr, lb);
            let (bb, br, bg) = row(lb, lr, lg);
            f.push(format!("colorchannelmixer=rr={rr}:rg={rg}:rb={rb}:gr={gr}:gg={gg}:gb={gb}:br={br}:bg={bg}:bb={bb}"));
        }
        if self.vibrance.abs() >= EPS {
            f.push(format!("vibrance=intensity={}", n(self.vibrance)));
        }
        f
    }

    /// The filter chain for this look. `lut_file` is the (staged) LUT name the graph may use;
    /// `label` makes the blend pad names unique in a bigger graph. `None` when neutral. The text
    /// starts with a filter (no leading comma) and has no output label.
    pub fn chain(&self, lut_file: Option<&str>, label: &str) -> Option<String> {
        let mut parts = self.pre_lut();
        let lut = lut_file.filter(|_| self.lut_strength >= EPS);
        let mut out = parts.join(",");
        if let Some(file) = lut {
            let lut = format!("lut3d=file='{}':interp=tetrahedral", file.replace('\\', "/").replace('\'', ""));
            if self.lut_strength >= 1.0 - EPS {
                parts.push(lut);
                out = parts.join(",");
            } else {
                // Original and LUT'd picture mixed by blend opacity.
                let head = if out.is_empty() { String::new() } else { format!("{out},") };
                out = format!(
                    "{head}split=2[{label}ga][{label}gb];[{label}gb]{lut}[{label}gc];[{label}ga][{label}gc]blend=all_mode=normal:all_opacity={}",
                    n(self.lut_strength)
                );
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

/// Chain of a stored grade (LUT referenced as `lut_file`), or `None` when it changes nothing.
pub fn chain_for(grade: &Grade, lut_file: Option<&str>, label: &str) -> Result<Option<String>, String> {
    Ok(grade.look()?.chain(lut_file, label))
}

// ---------------------------------------------------------------------------------------------
// .cube LUTs
// ---------------------------------------------------------------------------------------------

/// Checks a `.cube` text: a `LUT_3D_SIZE N` header (2..=256) and exactly N³ RGB rows. Returns N.
pub fn check_cube(text: &str) -> Result<usize, String> {
    let mut size = None;
    let mut rows = 0usize;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let up = l.to_ascii_uppercase();
        if let Some(v) = up.strip_prefix("LUT_3D_SIZE") {
            let s: usize = v.trim().parse().map_err(|_| format!("bad LUT_3D_SIZE line: {l}"))?;
            if !(2..=256).contains(&s) {
                return Err(format!("LUT_3D_SIZE {s} is outside 2..256"));
            }
            size = Some(s);
            continue;
        }
        if up.starts_with("LUT_1D_SIZE") {
            return Err("1D LUTs are not supported; use a 3D .cube (LUT_3D_SIZE)".into());
        }
        if up.starts_with("TITLE") || up.starts_with("DOMAIN_") || up.starts_with("LUT_") {
            continue;
        }
        let nums: Vec<&str> = l.split_whitespace().collect();
        if nums.len() != 3 || nums.iter().any(|x| x.parse::<f64>().map_or(true, |v| !v.is_finite())) {
            return Err(format!("not an RGB row: {l}"));
        }
        rows += 1;
    }
    let s = size.ok_or("missing LUT_3D_SIZE header (only 3D .cube LUTs are supported)")?;
    if rows != s * s * s {
        return Err(format!("LUT_3D_SIZE {s} needs {} rows, found {rows}", s * s * s));
    }
    Ok(s)
}

/// A `.cube` of size `n` from `f(r, g, b) -> (r, g, b)` on 0..1 (red changes fastest).
pub fn cube_from_fn(title: &str, n: usize, f: impl Fn(f64, f64, f64) -> (f64, f64, f64)) -> String {
    let mut s = format!("TITLE \"{title}\"\nLUT_3D_SIZE {n}\n");
    let d = (n - 1) as f64;
    for b in 0..n {
        for gg in 0..n {
            for r in 0..n {
                let (x, y, z) = f(r as f64 / d, gg as f64 / d, b as f64 / d);
                let _ = writeln!(s, "{:.6} {:.6} {:.6}", x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), z.clamp(0.0, 1.0));
            }
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Measuring frames
// ---------------------------------------------------------------------------------------------

/// Statistics of one small RGB frame (values 0..255).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameStats {
    /// Mean Rec.709 luma.
    pub luma_mean: f64,
    /// Median luma.
    pub luma_median: f64,
    /// Median luma of the pixels that are not near black (the subject on a dark background).
    #[serde(default)]
    pub subject_luma: f64,
    /// Percent of pixels at black (luma <= 4).
    pub black_pct: f64,
    /// Percent of pixels at white (any channel >= 252).
    pub white_pct: f64,
    /// Mean saturation 0..1 ((max-min)/max) of pixels that are not near black.
    pub saturation: f64,
    /// Mean R, G, B of near-neutral midtone pixels (white balance reference); 0 when none.
    pub neutral_r: f64,
    pub neutral_g: f64,
    pub neutral_b: f64,
    /// Share (0..1) of pixels used for the neutral means.
    pub neutral_share: f64,
}

/// Stats of packed rgb24 pixels.
pub fn stats_rgb24(px: &[u8]) -> FrameStats {
    let count = px.len() / 3;
    if count == 0 {
        return FrameStats::default();
    }
    let mut hist = [0u64; 256];
    let (mut lsum, mut black, mut white) = (0.0, 0u64, 0u64);
    let (mut ssum, mut scount) = (0.0, 0u64);
    let (mut nr, mut ng, mut nb, mut ncount) = (0.0, 0.0, 0.0, 0u64);
    for p in px.chunks_exact(3) {
        let (r, gg, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let l = 0.2126 * r + 0.7152 * gg + 0.0722 * b;
        lsum += l;
        hist[l.round().clamp(0.0, 255.0) as usize] += 1;
        if l <= 4.0 {
            black += 1;
        }
        if p.iter().any(|&c| c >= 252) {
            white += 1;
        }
        let (mx, mn) = (r.max(gg).max(b), r.min(gg).min(b));
        if mx > 24.0 {
            let s = (mx - mn) / mx;
            ssum += s;
            scount += 1;
            if s < 0.2 && (40.0..=230.0).contains(&l) {
                nr += r;
                ng += gg;
                nb += b;
                ncount += 1;
            }
        }
    }
    let half = count as u64 / 2;
    let mut acc = 0;
    let mut median = 0.0;
    for (i, h) in hist.iter().enumerate() {
        acc += h;
        if acc > half {
            median = i as f64;
            break;
        }
    }
    // Median of the non-black pixels (luma > 16).
    let lit: u64 = hist[17..].iter().sum();
    let mut subject = 0.0;
    let mut acc = 0;
    for (i, h) in hist.iter().enumerate().skip(17) {
        acc += h;
        if acc * 2 > lit {
            subject = i as f64;
            break;
        }
    }
    let c = count as f64;
    let nc = ncount.max(1) as f64;
    FrameStats {
        luma_mean: lsum / c,
        luma_median: median,
        subject_luma: subject,
        black_pct: black as f64 * 100.0 / c,
        white_pct: white as f64 * 100.0 / c,
        saturation: if scount > 0 { ssum / scount as f64 } else { 0.0 },
        neutral_r: if ncount > 0 { nr / nc } else { 0.0 },
        neutral_g: if ncount > 0 { ng / nc } else { 0.0 },
        neutral_b: if ncount > 0 { nb / nc } else { 0.0 },
        neutral_share: ncount as f64 / c,
    }
}

/// Averages of several frames (equal weight).
pub fn mean_stats(all: &[FrameStats]) -> FrameStats {
    if all.is_empty() {
        return FrameStats::default();
    }
    let k = all.len() as f64;
    let avg = |f: fn(&FrameStats) -> f64| all.iter().map(f).sum::<f64>() / k;
    let with_neutral: Vec<&FrameStats> = all.iter().filter(|s| s.neutral_share > 0.0).collect();
    let navg = |f: fn(&FrameStats) -> f64| {
        if with_neutral.is_empty() { 0.0 } else { with_neutral.iter().map(|s| f(s)).sum::<f64>() / with_neutral.len() as f64 }
    };
    FrameStats {
        luma_mean: avg(|s| s.luma_mean),
        luma_median: avg(|s| s.luma_median),
        subject_luma: avg(|s| s.subject_luma),
        black_pct: avg(|s| s.black_pct),
        white_pct: avg(|s| s.white_pct),
        saturation: avg(|s| s.saturation),
        neutral_r: navg(|s| s.neutral_r),
        neutral_g: navg(|s| s.neutral_g),
        neutral_b: navg(|s| s.neutral_b),
        neutral_share: avg(|s| s.neutral_share),
    }
}

/// Plain-language read of the stats plus suggested `grade set` flags (never applied).
pub fn hints(s: &FrameStats) -> (Vec<String>, Vec<String>) {
    let mut words = Vec::new();
    let mut flags = Vec::new();
    // On a mostly black frame (dark studio background) judge the lit part, not the average.
    let dark_bg = s.black_pct > 35.0;
    let (bright, what) = if dark_bg { (s.subject_luma, "lit-area median luma") } else { (s.luma_median.max(s.luma_mean), "median luma") };
    let lvl = if dark_bg { s.subject_luma } else { s.luma_median };
    if s.white_pct > 2.0 {
        words.push(format!("highlights clipped ({:.1}% of pixels at white): do not raise exposure", s.white_pct));
        if bright > 120.0 {
            flags.push("--exposure -0.1".to_string());
        }
    } else if bright < 45.0 {
        words.push(format!("dark ({what} {lvl:.0}/255)"));
        flags.push("--exposure 0.2".into());
    } else if bright < 70.0 {
        words.push(format!("slightly dark ({what} {lvl:.0}/255)"));
        flags.push("--exposure 0.1".into());
    } else if bright > 190.0 {
        words.push(format!("bright ({what} {lvl:.0}/255)"));
        flags.push("--exposure -0.1".into());
    } else {
        words.push(format!("exposure looks fine ({what} {lvl:.0}/255)"));
    }
    if s.black_pct > 35.0 {
        words.push(format!(
            "{:.0}% of the frame is black (a dark background?): judge the face, not the average",
            s.black_pct
        ));
    }
    if s.saturation < 0.12 {
        words.push(format!("muted colours (saturation {:.2})", s.saturation));
        flags.push("--vibrance 0.25".into());
    } else if s.saturation > 0.5 {
        words.push(format!("very saturated (saturation {:.2})", s.saturation));
        flags.push("--saturation 0.9".into());
    }
    if s.neutral_share >= 0.01 {
        let warm = s.neutral_r - s.neutral_b;
        let green = s.neutral_g - (s.neutral_r + s.neutral_b) / 2.0;
        if warm > 18.0 {
            words.push(format!("warm cast (neutral greys {:.0} more red than blue)", warm));
            flags.push("--temperature 7300".into());
        } else if warm > 9.0 {
            words.push(format!("slightly warm cast (neutral greys {:.0} more red than blue)", warm));
            flags.push("--temperature 6900".into());
        } else if warm < -14.0 {
            words.push(format!("cool / blue cast (neutral greys {:.0} more blue than red)", -warm));
            flags.push("--temperature 5600".into());
        } else if warm < -7.0 {
            words.push(format!("slightly cool cast (neutral greys {:.0} more blue than red)", -warm));
            flags.push("--temperature 6000".into());
        }
        if green > 6.0 {
            words.push(format!("green tint ({green:.0})"));
            flags.push("--tint 0.15".into());
        } else if green < -6.0 {
            words.push(format!("magenta tint ({:.0})", -green));
            flags.push("--tint -0.15".into());
        }
        if warm.abs() <= 7.0 && green.abs() <= 6.0 {
            words.push("white balance looks neutral".into());
        }
    } else {
        words.push("too few neutral pixels to judge white balance; look at the frame".into());
    }
    (words, flags)
}

/// Grabs one frame of `media` at source time `t`, scaled to 160 px wide, as rgb24 stats.
pub async fn measure_frame(ffmpeg: &Path, media: &Path, t: f64) -> rustler_core::Result<FrameStats> {
    let mut args = argv!["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", format!("{:.3}", t.max(0.0)), "-i"];
    args.push(media.as_os_str().to_owned());
    args.extend(argv!["-frames:v", "1", "-vf", "scale=160:-2,format=rgb24", "-f", "rawvideo", "-"]);
    let out = proc::run(ffmpeg, &args, None, &CancellationToken::new()).await.map_err(|mut e| {
        e.stage = "grade.measure".into();
        e
    })?;
    if out.stdout.len() < 3 {
        return Err(RustlerError::new(ErrorCode::Media, "grade.measure", format!("no frame at t={t:.3}s of {}", media.display())));
    }
    Ok(stats_rgb24(&out.stdout))
}

/// `left` | `right` side by side in one jpg, 960 px high.
pub async fn side_by_side(ffmpeg: &Path, left: &Path, right: &Path, out: &Path) -> rustler_core::Result<()> {
    let mut args = argv!["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"];
    args.push(left.as_os_str().to_owned());
    args.push("-i".into());
    args.push(right.as_os_str().to_owned());
    args.extend(argv!["-filter_complex", "[0:v][1:v]hstack=inputs=2,scale=-2:960,format=yuvj420p", "-frames:v", "1", "-q:v", "3"]);
    args.push(out.as_os_str().to_owned());
    let _ = std::fs::remove_file(out);
    proc::run(ffmpeg, &args, None, &CancellationToken::new()).await.map_err(|mut e| {
        e.stage = "grade.preview".into();
        e
    })?;
    if !out.is_file() {
        return Err(RustlerError::new(ErrorCode::Media, "grade.preview", "ffmpeg wrote no before/after image"));
    }
    Ok(())
}

/// Staged name of the `k`-th distinct LUT of a render.
pub fn staged_lut_name(k: usize) -> String {
    format!("grade_lut{k}.cube")
}

/// Distinct LUT files in first-use order.
pub fn distinct_luts<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        if !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_compiles_to_nothing() {
        assert_eq!(chain_for(&Grade::default(), None, "s0").unwrap(), None);
        let g = Grade { exposure: Some(0.0), contrast: Some(1.0), temperature: Some(6500.0), intensity: Some(1.0), ..Default::default() };
        assert_eq!(chain_for(&g, None, "s0").unwrap(), None);
        // Intensity 0 turns any grade off; a LUT without a staged file adds nothing.
        let g = Grade { preset: Some("cinematic".into()), intensity: Some(0.0), ..Default::default() };
        assert_eq!(chain_for(&g, None, "s0").unwrap(), None);
    }

    #[test]
    fn ranges_are_validated() {
        for bad in [
            Grade { exposure: Some(1.5), ..Default::default() },
            Grade { contrast: Some(0.2), ..Default::default() },
            Grade { saturation: Some(-0.1), ..Default::default() },
            Grade { temperature: Some(900.0), ..Default::default() },
            Grade { tint: Some(f64::NAN), ..Default::default() },
            Grade { intensity: Some(1.2), ..Default::default() },
            Grade { lut_strength: Some(-1.0), ..Default::default() },
            Grade { shadows: Some(Rgb::new(0.0, 2.0, 0.0)), ..Default::default() },
            Grade { preset: Some("nope".into()), ..Default::default() },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert!(Grade { preset: Some("Natural_Pop".into()), ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn presets_are_valid_and_distinct() {
        let ps = presets();
        assert!(ps.len() >= 8);
        let mut chains = Vec::new();
        for p in &ps {
            p.grade.validate().unwrap_or_else(|e| panic!("{}: {e}", p.name));
            let c = chain_for(&Grade { preset: Some(p.name.into()), ..Default::default() }, None, "s0").unwrap();
            let c = c.unwrap_or_else(|| panic!("{} compiles to nothing", p.name));
            assert!(!chains.contains(&c), "{} duplicates another preset", p.name);
            chains.push(c);
        }
    }

    #[test]
    fn chain_text_and_intensity() {
        let g = Grade { temperature: Some(5000.0), tint: Some(0.5), saturation: Some(0.0), ..Default::default() };
        let c = chain_for(&g, None, "s0").unwrap().unwrap();
        // Tint +0.5 = magenta: +U +V at both ends, first in the chain.
        assert!(c.starts_with("colorcorrect=rl=0.02:bl=0.02:rh=0.02:bh=0.02,colortemperature=temperature=5000:pl=1,"), "{c}");
        // Teal shadows / warm highlights: blue up + red down at black, the opposite at white.
        let tl = Grade { shadows: Some(Rgb::new(-0.1, 0.0, 0.1)), highlights: Some(Rgb::new(0.1, 0.0, -0.1)), ..Default::default() };
        let ((su, sv), (hu, hv)) = tl.look().unwrap().toning();
        assert!(su > 0.0 && sv < 0.0 && hu < 0.0 && hv > 0.0);
        // Saturation 0: every row is the luma weights.
        assert!(c.contains("colorchannelmixer=rr=0.2126:rg=0.7152:rb=0.0722:gr=0.2126:gg=0.7152:gb=0.0722:br=0.2126:bg=0.7152:bb=0.0722"), "{c}");
        let half = Grade { intensity: Some(0.5), ..g.clone() };
        let c = chain_for(&half, None, "s0").unwrap().unwrap();
        assert!(c.contains("temperature=5750"), "{c}");
        // Explicit fields win over the preset.
        let l = Grade { preset: Some("bw".into()), saturation: Some(0.5), ..Default::default() }.look().unwrap();
        assert!((l.saturation - 0.5).abs() < 1e-9 && (l.contrast - 1.15).abs() < 1e-9);
    }

    #[test]
    fn tone_curve_is_monotone_and_pinned() {
        for (e, c, b) in [(1.0, 1.5, -1.0), (-1.0, 0.5, 1.0), (0.3, 1.2, 0.0), (0.0, 1.0, 0.4)] {
            let l = Grade { exposure: Some(e), contrast: Some(c), blacks: Some(b), ..Default::default() }.look().unwrap();
            let mut prev = -1.0;
            for i in 0..=100 {
                let y = l.tone(i as f64 / 100.0);
                assert!(y >= prev - 1e-9, "not monotone at {i} for {e},{c},{b}");
                prev = y;
            }
            assert!(l.tone(1.0) > 0.999, "white stays white");
        }
        let up = Grade { exposure: Some(0.3), ..Default::default() }.look().unwrap();
        assert!(up.tone(0.5) > 0.55 && up.tone(0.0) == 0.0);
        let c = Grade { exposure: Some(0.3), ..Default::default() }.look().unwrap().chain(None, "s0").unwrap();
        assert!(c.starts_with("curves=master='0/0 0.0625/") && c.ends_with(":interp=pchip"), "{c}");
    }

    #[test]
    fn lut_chain_and_strength() {
        let g = Grade { lut: Some("asset:lut-x".into()), ..Default::default() };
        assert_eq!(chain_for(&g, Some("grade_lut0.cube"), "s0").unwrap().unwrap(), "lut3d=file='grade_lut0.cube':interp=tetrahedral");
        let g = Grade { lut: Some("x.cube".into()), lut_strength: Some(0.5), contrast: Some(1.1), ..Default::default() };
        let c = chain_for(&g, Some("grade_lut1.cube"), "s3").unwrap().unwrap();
        assert!(c.contains(",split=2[s3ga][s3gb];[s3gb]lut3d=file='grade_lut1.cube':interp=tetrahedral[s3gc];[s3ga][s3gc]blend=all_mode=normal:all_opacity=0.5"), "{c}");
        let g = Grade { lut: Some("x.cube".into()), lut_strength: Some(0.0), ..Default::default() };
        assert_eq!(chain_for(&g, Some("grade_lut0.cube"), "s0").unwrap(), None);
    }

    #[test]
    fn segment_override_merges_onto_clip() {
        let doc = GradeDoc {
            clip: Some(Grade { preset: Some("warm".into()), exposure: Some(0.1), ..Default::default() }),
            segments: [("seg_2".to_string(), Grade { exposure: Some(-0.2), ..Default::default() })].into_iter().collect(),
        };
        assert_eq!(doc.for_segment("seg_1").unwrap().exposure, Some(0.1));
        let s2 = doc.for_segment("seg_2").unwrap();
        assert_eq!((s2.exposure, s2.preset.as_deref()), (Some(-0.2), Some("warm")));
        assert!(!doc.is_empty());
        assert!(GradeDoc::default().is_empty());
    }

    #[test]
    fn cube_check_and_generate() {
        let c = cube_from_fn("id", 3, |r, g, b| (r, g, b));
        assert_eq!(check_cube(&c).unwrap(), 3);
        assert!(check_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(check_cube("0 0 0\n").is_err());
        assert!(check_cube("LUT_1D_SIZE 4\n").is_err());
    }

    #[test]
    fn stats_and_hints() {
        // Mid grey with a warm cast: R 142, G 128, B 118.
        let px: Vec<u8> = (0..1000).flat_map(|_| [142u8, 128, 118]).collect();
        let s = stats_rgb24(&px);
        assert!((s.neutral_r - 142.0).abs() < 1e-9 && s.neutral_share > 0.99);
        let (words, flags) = hints(&s);
        assert!(words.iter().any(|w| w.contains("warm cast")), "{words:?}");
        assert!(flags.contains(&"--temperature 7300".to_string()));
        // Clipped whites never suggest more exposure.
        let px: Vec<u8> = (0..1000).flat_map(|i| if i % 10 == 0 { [255u8, 255, 255] } else { [30, 30, 30] }).collect();
        let (words, flags) = hints(&stats_rgb24(&px));
        assert!(words[0].contains("clipped"), "{words:?}");
        assert!(!flags.iter().any(|f| f == "--exposure 0.2" || f == "--exposure 0.1"), "{flags:?}");
    }

    /// `cargo test -p rustler-media --lib grade -- --ignored --nocapture` prints every preset
    /// chain (used to tune the presets on real frames).
    #[test]
    #[ignore]
    fn print_preset_chains() {
        for p in presets() {
            let g = Grade { preset: Some(p.name.into()), ..Default::default() };
            println!("PRESET {}\t{}", p.name, chain_for(&g, None, "s0").unwrap().unwrap());
        }
    }
}
