//! v2 layer kinds (design §5.1): image layers, masks, cutaways and caption word overrides.
//!
//! Each kind is stored under its own `extras.json` key ([`IMAGES_KEY`], [`MASKS_KEY`],
//! [`CUTAWAYS_KEY`], [`CAPTION_WORDS_KEY`]) by the `image.*` / `mask.*` / `cutaway.*` /
//! `caption.word*` commands; the timeline v2 document assembles the same keys.
//!
//! Timing: an item has `out_start`/`out_end` on the *edit timeline* (the clip's TimeMap, like
//! overlays and effects) or word anchors (`start_word` / `end_word` = `{source_id, word_i}`). Word
//! anchors are resolved through the TimeMap at every render ([`resolve_timing`]), then through the
//! [`TransitionMap`] like every other edit-timeline time. An anchor whose word was cut out by a
//! re-cut does not resolve: the item is dropped with a warning, never placed at a wrong time.
//!
//! The compiler ([`crate::compile::build_graph`]) takes the items with their timing already
//! resolved (`out_start`/`out_end` set) and composites them in the same single graph, z-order:
//! video -> full cutaways -> masks -> camera effects -> PiP cutaways -> images below the captions
//! (by `z`) -> captions -> text overlays -> images with `above_captions` (by `z`) -> progress bar.

use crate::compile::{AUDIO_FORMAT, TransitionMap, ffmpeg_color, fps_str, frame_at, num, num6, sample_at, secs, tb_str};
use rustler_core::{Fit, Rect, SourceId, TimeMap, Word};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// extras.json keys.
pub const IMAGES_KEY: &str = "images";
pub const MASKS_KEY: &str = "masks";
pub const CUTAWAYS_KEY: &str = "cutaways";
pub const CAPTION_WORDS_KEY: &str = "caption_word_overrides";

/// Smallest rect side (fraction of the canvas).
pub const MIN_RECT_SIDE: f64 = 0.02;
/// Image file extensions an image layer accepts.
pub const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];
/// Still-image extensions (looped with `-loop 1`).
const STILL_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

// ---------------------------------------------------------------------------------------------
// Stored types
// ---------------------------------------------------------------------------------------------

/// A transcript word: `transcript get <source_id>` prints the indices.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Hash)]
pub struct Anchor {
    pub source_id: SourceId,
    /// Word index in the source transcript (`word_index` is accepted too).
    #[serde(alias = "word_index")]
    pub word_i: u32,
}

/// When an item shows: edit-timeline seconds, or transcript words that follow re-cuts.
/// `start_word` wins over `out_start`, `end_word` over `out_end`. A `start_word` without an end
/// lasts until the end of that word. Missing start = 0, missing end = end of the clip.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct Timing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_end: Option<f64>,
    /// Starts at the start of this word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_word: Option<Anchor>,
    /// Ends at the end of this word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_word: Option<Anchor>,
}

impl Timing {
    pub fn out(start: f64, end: f64) -> Self {
        Self { out_start: Some(start), out_end: Some(end), start_word: None, end_word: None }
    }
    pub fn has_anchor(&self) -> bool {
        self.start_word.is_some() || self.end_word.is_some()
    }
    /// Resolved `(start, end)` for the compiler (anchors must be resolved already).
    fn window(&self, edit_duration: f64) -> (f64, f64) {
        (self.out_start.unwrap_or(0.0), self.out_end.unwrap_or(edit_duration).min(edit_duration))
    }
}

/// Entrance / exit animation of an image layer.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnimKind {
    #[default]
    None,
    /// Alpha fade.
    Fade,
    /// Scale 50% -> 112% -> 100% (in) / shrink away (out).
    Pop,
    /// Slides up into place while fading in (out: keeps rising while fading out).
    SlideUp,
}

fn default_anim_duration() -> f64 {
    0.3
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct Anim {
    #[serde(rename = "in", default)]
    pub in_: AnimKind,
    #[serde(default)]
    pub out: AnimKind,
    /// Seconds per animation (default 0.3; at most half the layer).
    #[serde(default = "default_anim_duration")]
    pub duration: f64,
}

impl Default for Anim {
    fn default() -> Self {
        Self { in_: AnimKind::None, out: AnimKind::None, duration: default_anim_duration() }
    }
}

fn one() -> f64 {
    1.0
}
fn contain() -> Fit {
    Fit::Contain
}
fn cover() -> Fit {
    Fit::Cover
}
fn yes() -> bool {
    true
}

/// A still image or GIF on the canvas (logo, sticker, screenshot, agent-made card).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct ImageLayer {
    pub id: String,
    #[serde(flatten)]
    pub timing: Timing,
    /// Library asset id when it came from the asset library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Absolute path of the image (png, jpg, webp, gif, bmp).
    pub file: PathBuf,
    /// Box on the canvas, normalized 0..1; the image is fitted inside (`fit`).
    pub rect: Rect,
    /// Extra scale around the rect centre (default 1).
    #[serde(default = "one")]
    pub scale: f64,
    /// 0..1 (default 1).
    #[serde(default = "one")]
    pub opacity: f64,
    /// Degrees clockwise (default 0).
    #[serde(default)]
    pub rotation: f64,
    /// contain (default: whole image inside the rect) | cover (fills the rect, cropped).
    #[serde(default = "contain")]
    pub fit: Fit,
    #[serde(default)]
    pub anim: Anim,
    /// Stacking order among images (higher = on top).
    #[serde(default)]
    pub z: i32,
    /// Draw above the captions and text overlays (default: below the captions).
    #[serde(default)]
    pub above_captions: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MaskKind {
    /// Box blur of the area.
    Blur,
    /// Solid box (`color`, default black).
    Box,
    /// Big pixels.
    Pixelate,
}

impl MaskKind {
    pub fn default_strength(self) -> f64 {
        match self {
            Self::Blur => 20.0,
            Self::Pixelate => 24.0,
            Self::Box => 0.0,
        }
    }
}

/// Hides part of the source picture (pop-ups, logos, burned-in subtitles). Drawn on the video
/// before effects, images and captions.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct MaskLayer {
    pub id: String,
    #[serde(flatten)]
    pub timing: Timing,
    pub rect: Rect,
    pub kind: MaskKind,
    /// blur: radius in pixels (default 20); pixelate: block size in pixels (default 24); 1..100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
    /// box: `#RRGGBB` (default `#000000`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl MaskLayer {
    pub fn strength_or_default(&self) -> f64 {
        self.strength.unwrap_or_else(|| self.kind.default_strength())
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CutawayMode {
    /// Replaces the whole picture for the range (speech keeps playing).
    Full,
    /// Picture-in-picture inside `rect`.
    Pip,
}

/// What a cutaway shows: another imported source, a library asset or a file.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct CutawayMedia {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Absolute path (asset / file / downloaded URL). For `source_id` the render resolves the
    /// source's media file and fills this in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Seconds into the media where the cutaway starts (default 0).
    #[serde(rename = "in", default)]
    pub in_: f64,
}

/// B-roll over the speaker: full-frame replacement or picture-in-picture.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Cutaway {
    pub id: String,
    #[serde(flatten)]
    pub timing: Timing,
    pub media: CutawayMedia,
    pub mode: CutawayMode,
    /// pip: box on the canvas (normalized 0..1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// cover (default: fills, cropped) | contain (whole picture, black bars in full mode).
    #[serde(default = "cover")]
    pub fit: Fit,
    /// Keep only the clip's speech (default true); false mixes the cutaway's own audio in.
    #[serde(default = "yes")]
    pub mute: bool,
}

/// Per-word caption look (keywords in a colour, bigger, with an emoji, or hidden).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct CaptionWordOverride {
    pub word_anchor: Anchor,
    /// `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Size factor (0.5..2.5; 1 = preset size).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// Appended after the word (e.g. "🔥").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Leave the word out of the captions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide: bool,
}

// ---------------------------------------------------------------------------------------------
// Validation and resolution (pure)
// ---------------------------------------------------------------------------------------------

/// `Err(reason)` unless the rect lies inside 0..1 with both sides > 0.02.
pub fn check_rect(r: &Rect) -> Result<(), String> {
    let all_finite = [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite());
    if !all_finite {
        return Err("rect values must be finite numbers".into());
    }
    if r.w <= MIN_RECT_SIDE || r.h <= MIN_RECT_SIDE {
        return Err(format!("rect w and h must be > {MIN_RECT_SIDE} (got w {} h {})", r.w, r.h));
    }
    const EPS: f64 = 1e-9;
    if r.x < -EPS || r.y < -EPS || r.x + r.w > 1.0 + EPS || r.y + r.h > 1.0 + EPS {
        return Err(format!(
            "rect must lie inside 0..1 (x {} + w {} = {:.4}, y {} + h {} = {:.4})",
            r.x,
            r.y,
            r.x + r.w,
            r.y,
            r.h,
            r.y + r.h
        ));
    }
    Ok(())
}

/// Lower-case extension of `p`.
pub fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// True for still images (looped), false for GIFs and videos.
pub fn is_still(p: &Path) -> bool {
    STILL_EXTS.contains(&ext_of(p).as_str())
}

pub fn is_gif(p: &Path) -> bool {
    ext_of(p) == "gif"
}

/// Edit-timeline `(start, end)` of every occurrence of a word in the clip, in timeline order.
/// A word belongs to the segment holding its midpoint (like the captions); its span is clamped to
/// that segment.
pub fn word_spans(a: &Anchor, tm: &TimeMap, words: &HashMap<SourceId, Vec<Word>>) -> Result<Vec<(f64, f64)>, String> {
    let ws = words.get(&a.source_id).ok_or_else(|| format!("source {} has no transcript", a.source_id))?;
    let w = ws
        .iter()
        .find(|w| w.i == a.word_i)
        .or_else(|| ws.get(a.word_i as usize))
        .ok_or_else(|| format!("word {} does not exist in source {}", a.word_i, a.source_id))?;
    let mid = (w.start + w.end) / 2.0;
    let spans: Vec<(f64, f64)> = tm
        .segments()
        .iter()
        .filter(|s| s.source_id == a.source_id && mid >= s.src_start && mid < s.src_end)
        .map(|s| {
            let st = w.start.max(s.src_start);
            let en = w.end.min(s.src_end).max(st + 0.02);
            (s.out_start + (st - s.src_start), (s.out_start + (en - s.src_start)).min(s.out_end.max(s.out_start + 0.02)))
        })
        .collect();
    if spans.is_empty() {
        return Err(format!(
            "word {} ('{}') of source {} is not in the clip any more (cut out by an edit)",
            a.word_i,
            w.text.trim(),
            a.source_id
        ));
    }
    Ok(spans)
}

/// Edit-timeline `(start, end)` of an item. `Err(reason)` when an anchor does not resolve (word
/// cut out, unknown) or the result is empty; such items are dropped, never placed elsewhere.
pub fn resolve_timing(t: &Timing, tm: &TimeMap, words: &HashMap<SourceId, Vec<Word>>) -> Result<(f64, f64), String> {
    let duration = tm.duration();
    let start = match &t.start_word {
        Some(a) => word_spans(a, tm, words)?[0].0,
        None => t.out_start.unwrap_or(0.0),
    };
    let end = match (&t.end_word, &t.start_word, t.out_end) {
        (Some(a), _, _) => {
            let spans = word_spans(a, tm, words)?;
            // First occurrence that ends after the start.
            spans.iter().map(|s| s.1).find(|e| *e > start + 1e-6).unwrap_or(spans[0].1)
        }
        (None, Some(a), None) => word_spans(a, tm, words)?[0].1,
        (None, _, Some(e)) => e,
        (None, None, None) => duration,
    };
    if !(start.is_finite() && end.is_finite()) || start < 0.0 {
        return Err("start and end must be finite and start >= 0".into());
    }
    if end <= start + 1e-6 {
        return Err(format!("ends ({end:.2}s) before it starts ({start:.2}s)"));
    }
    if start >= duration {
        return Err(format!("starts at {start:.2}s, after the clip ends ({duration:.2}s)"));
    }
    Ok((start, end.min(duration)))
}

/// `t` with its anchors resolved to edit-timeline seconds (what the compiler takes).
pub fn resolved(t: &Timing, tm: &TimeMap, words: &HashMap<SourceId, Vec<Word>>) -> Result<Timing, String> {
    let (s, e) = resolve_timing(t, tm, words)?;
    Ok(Timing::out(s, e))
}

/// Validation of one image layer's values (rect, scale, opacity, rotation, animation).
pub fn check_image(i: &ImageLayer) -> Result<(), String> {
    check_rect(&i.rect)?;
    if !(i.scale.is_finite() && (0.05..=5.0).contains(&i.scale)) {
        return Err("scale must be between 0.05 and 5".into());
    }
    if !(i.opacity.is_finite() && (0.0..=1.0).contains(&i.opacity)) {
        return Err("opacity must be between 0 and 1".into());
    }
    if !i.rotation.is_finite() || i.rotation.abs() > 360.0 {
        return Err("rotation must be between -360 and 360 degrees".into());
    }
    if !(i.anim.duration.is_finite() && (0.0..=5.0).contains(&i.anim.duration)) {
        return Err("anim duration must be between 0 and 5 seconds".into());
    }
    if i.file.as_os_str().is_empty() {
        return Err("image has no file".into());
    }
    Ok(())
}

pub fn check_mask(m: &MaskLayer) -> Result<(), String> {
    check_rect(&m.rect)?;
    if let Some(s) = m.strength
        && !(s.is_finite() && (1.0..=100.0).contains(&s))
    {
        return Err("strength must be between 1 and 100".into());
    }
    if let Some(c) = &m.color
        && ffmpeg_color(c).is_none()
    {
        return Err(format!("color must be #RRGGBB (got '{c}')"));
    }
    Ok(())
}

pub fn check_cutaway(c: &Cutaway) -> Result<(), String> {
    if !(c.media.in_.is_finite() && c.media.in_ >= 0.0) {
        return Err("media in (src_start) must be >= 0".into());
    }
    match (c.mode, &c.rect) {
        (CutawayMode::Pip, None) => return Err("a pip cutaway needs a rect".into()),
        (_, Some(r)) => check_rect(r)?,
        _ => {}
    }
    if c.media.source_id.is_none() && c.media.asset.is_none() && c.media.file.is_none() {
        return Err("a cutaway needs media: source_id, asset or file".into());
    }
    Ok(())
}

pub fn check_word_override(o: &CaptionWordOverride) -> Result<(), String> {
    if let Some(c) = &o.color
        && ffmpeg_color(c).is_none()
    {
        return Err(format!("color must be #RRGGBB (got '{c}')"));
    }
    if let Some(s) = o.scale
        && !(s.is_finite() && (0.5..=2.5).contains(&s))
    {
        return Err("scale must be between 0.5 and 2.5".into());
    }
    if let Some(e) = &o.emoji {
        let n = e.chars().count();
        if n == 0 || n > 8 || e.chars().any(|c| matches!(c, '{' | '}' | '\\') || c.is_control()) {
            return Err("emoji must be 1..8 characters without { } \\".into());
        }
    }
    if o.color.is_none() && o.scale.is_none() && o.emoji.is_none() && !o.hide {
        return Err("nothing to change: give color, scale, emoji or hide".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Graph building (crate-internal; called from compile::build_graph)
// ---------------------------------------------------------------------------------------------

/// An extra ffmpeg input a layer reads (`opts` go before `-i path`).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerInput {
    pub opts: Vec<String>,
    pub path: PathBuf,
}

/// The open video chain after `[vcat]`: filters are appended with `,`; an overlay closes it under
/// a label and opens a new chain from the overlay.
pub(crate) struct VChain {
    open: String,
    n: usize,
}

impl VChain {
    pub(crate) fn new(start: String) -> Self {
        Self { open: start, n: 0 }
    }

    pub(crate) fn push(&mut self, filter: &str) {
        let _ = write!(self.open, ",{filter}");
    }

    fn close(&mut self, parts: &mut Vec<String>) -> String {
        let l = format!("[vl{}]", self.n);
        self.n += 1;
        parts.push(format!("{}{l}", self.open));
        l
    }

    /// `[cur][top]overlay=args`.
    fn overlay(&mut self, parts: &mut Vec<String>, top: &str, args: &str) {
        let l = self.close(parts);
        self.open = format!("{l}{top}overlay={args}");
    }

    /// Splits the picture, runs `chain` on the copy and lays it back at (x, y).
    fn patch(&mut self, parts: &mut Vec<String>, id: &str, chain: &str, x: u32, y: u32, enable: &str) {
        let l = self.close(parts);
        parts.push(format!("{l}split=2[{id}a][{id}b]"));
        parts.push(format!("[{id}b]{chain}[{id}c]"));
        self.open = format!("[{id}a][{id}c]overlay=x={x}:y={y}:{enable}");
    }

    pub(crate) fn into_open(self) -> String {
        self.open
    }
}

/// Pixel box of a normalized rect on the canvas: even x/y/w/h, kept inside the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PxBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

fn even_round(v: f64) -> u32 {
    let r = v.max(0.0).round() as u32;
    r - r % 2
}

pub(crate) fn px_box(r: &Rect, cw: u32, ch: u32) -> PxBox {
    let x = even_round(r.x * cw as f64).min(cw.saturating_sub(2));
    let y = even_round(r.y * ch as f64).min(ch.saturating_sub(2));
    let w = even_round(r.w * cw as f64).clamp(2, cw - x);
    let h = even_round(r.h * ch as f64).clamp(2, ch - y);
    PxBox { x, y, w: w - w % 2, h: h - h % 2 }
}

/// Everything the layer builders need from the main graph.
pub(crate) struct LayerCtx<'a> {
    pub tmap: &'a TransitionMap,
    pub fps: f64,
    pub cw: u32,
    pub ch: u32,
    pub total_frames: u64,
    pub total_samples: u64,
    /// ffmpeg input index of the first layer input.
    pub base_input: usize,
    pub inputs: Vec<LayerInput>,
    /// Audio chains (without output label) mixed like sound effects.
    pub audio_chains: Vec<String>,
}

impl LayerCtx<'_> {
    /// Rendered frame window `[f0, f1)` of an item; `None` when it falls outside the clip or
    /// inside the hidden half of a transition.
    fn frames(&self, t: &Timing) -> Option<(u64, u64)> {
        let (s, e) = t.window(self.tmap.edit_duration());
        let (a, b) = self.tmap.map_interval(s, e)?;
        let (f0, f1) = (frame_at(a, self.fps), frame_at(b, self.fps).min(self.total_frames));
        (f1 > f0).then_some((f0, f1))
    }

    fn enable(f0: u64, f1: u64) -> String {
        format!("enable='between(n,{f0},{})'", f1 - 1)
    }

    fn add_input(&mut self, opts: Vec<String>, path: &Path) -> usize {
        self.inputs.push(LayerInput { opts, path: path.to_path_buf() });
        self.base_input + self.inputs.len() - 1
    }

    /// Input options for a visual file shown for `len` seconds starting `media_in` into it.
    fn visual_opts(&self, path: &Path, media_in: f64, len: f64) -> Vec<String> {
        let t = secs(len + 0.1);
        if is_still(path) {
            vec!["-loop".into(), "1".into(), "-framerate".into(), fps_str(self.fps), "-t".into(), t]
        } else if is_gif(path) {
            vec!["-ignore_loop".into(), "0".into(), "-t".into(), t]
        } else if media_in > 0.0 {
            vec!["-ss".into(), secs(media_in), "-t".into(), t]
        } else {
            vec!["-t".into(), t]
        }
    }

    /// `[k:v]` at the canvas frame rate with frame-index timestamps from 0.
    fn video_head(&self, k: usize) -> String {
        format!("[{k}:v]fps={},settb={},setpts=N", fps_str(self.fps), tb_str(self.fps))
    }

    pub(crate) fn full_cutaways(&mut self, parts: &mut Vec<String>, vc: &mut VChain, cutaways: &[Cutaway]) {
        for (j, c) in cutaways.iter().enumerate().filter(|(_, c)| c.mode == CutawayMode::Full) {
            self.cutaway(parts, vc, j, c);
        }
    }

    pub(crate) fn pip_cutaways(&mut self, parts: &mut Vec<String>, vc: &mut VChain, cutaways: &[Cutaway]) {
        for (j, c) in cutaways.iter().enumerate().filter(|(_, c)| c.mode == CutawayMode::Pip) {
            self.cutaway(parts, vc, j, c);
        }
    }

    fn cutaway(&mut self, parts: &mut Vec<String>, vc: &mut VChain, j: usize, c: &Cutaway) {
        let Some(path) = c.media.file.as_deref() else { return };
        let Some((f0, f1)) = self.frames(&c.timing) else { return };
        let (cw, ch) = (self.cw, self.ch);
        let nf = f1 - f0;
        let len = nf as f64 / self.fps;
        let k = self.add_input(self.visual_opts(path, c.media.in_, len), path);
        let (bw, bh, bx, by) = match (c.mode, &c.rect) {
            (CutawayMode::Pip, Some(r)) => {
                let b = px_box(r, cw, ch);
                (b.w, b.h, b.x, b.y)
            }
            _ => (cw, ch, 0, 0),
        };
        let fit = match (c.fit, c.mode) {
            (Fit::Cover, _) => format!(
                "scale=w={bw}:h={bh}:force_original_aspect_ratio=increase:force_divisible_by=2,crop={bw}:{bh}"
            ),
            (Fit::Contain, CutawayMode::Full) => format!(
                "scale=w={bw}:h={bh}:force_original_aspect_ratio=decrease:force_divisible_by=2,\
                 pad={bw}:{bh}:(ow-iw)/2:(oh-ih)/2:color=black"
            ),
            (Fit::Contain, CutawayMode::Pip) => {
                format!("scale=w={bw}:h={bh}:force_original_aspect_ratio=decrease:force_divisible_by=2")
            }
        };
        let label = format!("[cut{j}]");
        parts.push(format!(
            "{},{fit},setsar=1,format=yuv420p,tpad=stop_mode=clone:stop_duration={},trim=end_frame={nf},setpts=PTS+{f0}{label}",
            self.video_head(k),
            num(len + 0.1)
        ));
        let (x, y) = match (c.mode, c.fit) {
            (CutawayMode::Pip, Fit::Contain) => (format!("{bx}+({bw}-w)/2"), format!("{by}+({bh}-h)/2")),
            _ => (bx.to_string(), by.to_string()),
        };
        vc.overlay(parts, &label, &format!("x={x}:y={y}:eof_action=pass:{}", Self::enable(f0, f1)));
        if !c.mute {
            let s = f0 as f64 / self.fps;
            self.audio_chains.push(format!(
                "[{k}:a:0]aresample=48000,{AUDIO_FORMAT},atrim=end_sample={},asetpts=N/SR/TB,\
                 adelay=delays={}S:all=1,apad,atrim=end_sample={},asetpts=N/SR/TB",
                sample_at(len),
                sample_at(s),
                self.total_samples
            ));
        }
    }

    pub(crate) fn masks(&mut self, parts: &mut Vec<String>, vc: &mut VChain, masks: &[MaskLayer]) {
        for (j, m) in masks.iter().enumerate() {
            let Some((f0, f1)) = self.frames(&m.timing) else { continue };
            let b = px_box(&m.rect, self.cw, self.ch);
            let en = Self::enable(f0, f1);
            match m.kind {
                MaskKind::Box => {
                    let c = m.color.as_deref().and_then(ffmpeg_color).unwrap_or_else(|| "0x000000".into());
                    vc.push(&format!("drawbox=x={}:y={}:w={}:h={}:color={c}@1:t=fill:{en}", b.x, b.y, b.w, b.h));
                }
                MaskKind::Blur => {
                    let side = b.w.min(b.h);
                    let r = (m.strength_or_default().round() as u32).clamp(1, (side / 2).saturating_sub(1).max(1));
                    let rc = (r / 2).min((side / 4).saturating_sub(1));
                    let chain = format!(
                        "crop={}:{}:{}:{},boxblur=luma_radius={r}:luma_power=3:chroma_radius={rc}:chroma_power=3",
                        b.w, b.h, b.x, b.y
                    );
                    vc.patch(parts, &format!("mk{j}"), &chain, b.x, b.y, &en);
                }
                MaskKind::Pixelate => {
                    let block = (m.strength_or_default().round() as u32).max(2);
                    let (sw, sh) = ((b.w / block).max(1), (b.h / block).max(1));
                    let chain = format!(
                        "crop={}:{}:{}:{},scale={sw}:{sh}:flags=area,scale={}:{}:flags=neighbor",
                        b.w, b.h, b.x, b.y, b.w, b.h
                    );
                    vc.patch(parts, &format!("mk{j}"), &chain, b.x, b.y, &en);
                }
            }
        }
    }

    /// Images of one tier (`above` = above the captions), in `z` order (stable).
    pub(crate) fn images(&mut self, parts: &mut Vec<String>, vc: &mut VChain, images: &[ImageLayer], above: bool) {
        let mut order: Vec<(usize, &ImageLayer)> =
            images.iter().enumerate().filter(|(_, i)| i.above_captions == above).collect();
        order.sort_by_key(|(j, i)| (i.z, *j));
        for (j, im) in order {
            let Some((f0, f1)) = self.frames(&im.timing) else { continue };
            let nf = f1 - f0;
            let len = nf as f64 / self.fps;
            let k = self.add_input(self.visual_opts(&im.file, 0.0, len), &im.file);
            let b = px_box(&im.rect, self.cw, self.ch);
            let (cx, cy) = (b.x as f64 + b.w as f64 / 2.0, b.y as f64 + b.h as f64 / 2.0);
            let bw = ((b.w as f64 * im.scale).round() as u32).max(2);
            let bh = ((b.h as f64 * im.scale).round() as u32).max(2);
            let d = im.anim.duration.clamp(0.0, len / 2.0);
            let mut chain = format!("{},format=rgba", self.video_head(k));
            match im.fit {
                Fit::Contain => {
                    let _ = write!(chain, ",scale=w={bw}:h={bh}:force_original_aspect_ratio=decrease");
                }
                Fit::Cover => {
                    let _ = write!(chain, ",scale=w={bw}:h={bh}:force_original_aspect_ratio=increase,crop={bw}:{bh}");
                }
            }
            if im.rotation.abs() > 1e-6 {
                let a = num6(im.rotation.to_radians());
                let _ = write!(chain, ",rotate=a={a}:c=none:ow=rotw({a}):oh=roth({a})");
            }
            if im.opacity < 1.0 {
                let _ = write!(chain, ",colorchannelmixer=aa={}", num(im.opacity));
            }
            let anim_on = d > 0.0;
            if anim_on {
                match im.anim.in_ {
                    AnimKind::Fade | AnimKind::SlideUp => {
                        let _ = write!(chain, ",fade=t=in:st=0:d={}:alpha=1", num(d));
                    }
                    AnimKind::Pop => {
                        let _ = write!(chain, ",fade=t=in:st=0:d={}:alpha=1", num(d * 0.4));
                    }
                    AnimKind::None => {}
                }
                if matches!(im.anim.out, AnimKind::Fade | AnimKind::SlideUp | AnimKind::Pop) {
                    let _ = write!(chain, ",fade=t=out:st={}:d={}:alpha=1", num(len - d), num(d));
                }
            }
            // Pop changes the frame size every frame: it must be the last filter before overlay.
            let pin = anim_on && im.anim.in_ == AnimKind::Pop;
            let pout = anim_on && im.anim.out == AnimKind::Pop;
            if pin || pout {
                let a = num(d * 0.7);
                let dd = num(d);
                let s_in = if pin {
                    format!("if(lt(t,{a}),0.5+0.62*t/{a},if(lt(t,{dd}),1.12-0.12*(t-{a})/({dd}-{a}),1))")
                } else {
                    "1".to_string()
                };
                let s_out = if pout {
                    format!("if(gt(t,{}),max(0.05,1-(t-{})/{dd}),1)", num(len - d), num(len - d))
                } else {
                    "1".to_string()
                };
                let s = format!("({s_in})*({s_out})");
                let _ = write!(chain, ",scale=w='max(2,iw*{s})':h='max(2,ih*{s})':eval=frame");
            }
            let label = format!("[img{j}]");
            let _ = write!(chain, ",setpts=PTS+{f0}{label}");
            parts.push(chain);

            let (s, e) = (f0 as f64 / self.fps, f1 as f64 / self.fps);
            let off = num((self.ch as f64 * 0.06).round());
            let mut y = format!("{}-h/2", num(cy));
            if anim_on && im.anim.in_ == AnimKind::SlideUp {
                let _ = write!(y, "+{off}*(1-min(1,(t-{})/{}))", num6(s), num(d));
            }
            if anim_on && im.anim.out == AnimKind::SlideUp {
                let _ = write!(y, "-{off}*max(0,(t-{})/{})", num6(e - d), num(d));
            }
            vc.overlay(
                parts,
                &label,
                &format!("x='{}-w/2':y='{y}':eof_action=pass:{}", num(cx), Self::enable(f0, f1)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_checks() {
        assert!(check_rect(&Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }).is_ok());
        assert!(check_rect(&Rect { x: 0.7, y: 0.02, w: 0.25, h: 0.1 }).is_ok());
        for r in [
            Rect { x: 0.9, y: 0.0, w: 0.2, h: 0.2 },
            Rect { x: -0.1, y: 0.0, w: 0.2, h: 0.2 },
            Rect { x: 0.0, y: 0.0, w: 0.02, h: 0.5 },
            Rect { x: 0.0, y: 0.0, w: 0.5, h: 0.0 },
            Rect { x: f64::NAN, y: 0.0, w: 0.5, h: 0.5 },
        ] {
            assert!(check_rect(&r).is_err(), "{r:?}");
        }
    }

    #[test]
    fn px_box_is_even_and_inside() {
        let b = px_box(&Rect { x: 0.75, y: 0.031, w: 0.25, h: 0.1 }, 1080, 1920);
        assert_eq!((b.x % 2, b.y % 2, b.w % 2, b.h % 2), (0, 0, 0, 0));
        assert!(b.x + b.w <= 1080 && b.y + b.h <= 1920, "{b:?}");
        assert_eq!(b.x, 810);
    }

    #[test]
    fn timing_serde_flattens() {
        let m: MaskLayer = serde_json::from_value(serde_json::json!({
            "id": "msk_1", "out_start": 1.0, "out_end": 2.0, "rect": {"x":0.1,"y":0.1,"w":0.2,"h":0.2}, "kind": "blur"
        }))
        .unwrap();
        assert_eq!(m.timing, Timing::out(1.0, 2.0));
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["out_start"], 1.0);
        assert!(v.get("start_word").is_none());
        let a: Anchor = serde_json::from_value(serde_json::json!({"source_id": "src_x", "word_index": 3})).unwrap();
        assert_eq!(a.word_i, 3);
    }
}
