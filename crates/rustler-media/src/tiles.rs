//! Labelled tile grids (contact sheets). (Task 6)

use crate::frames::frame;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{Rgb, RgbImage, imageops};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use rustler_core::{ErrorCode, RustlerError};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

const LABEL_STRIP: u32 = 36;

/// Default: count 12, cols 4, tile_width 480.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TileSpec {
    pub start: f64,
    pub end: f64,
    pub count: u32,
    pub cols: u32,
    pub tile_width: u32,
}

impl Default for TileSpec {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: 0.0,
            count: 12,
            cols: 4,
            tile_width: 480,
        }
    }
}

/// e.g. `"00:47:05.2 · words spoken around t"`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TileLabel {
    pub t: f64,
    pub text: String,
}

fn validation(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, "tiles", msg)
}

/// Scratch directory removed on drop (tempfile is a dev-dependency only).
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> rustler_core::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("rustler-tiles-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "tiles", format!("cannot create temp dir: {e}"))
        })?;
        Ok(Self(dir))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `HH:MM:SS.t` (tenths).
fn fmt_time(t: f64) -> String {
    let tenths = (t.max(0.0) * 10.0).round() as u64;
    let (s, d) = (tenths / 10, tenths % 10);
    format!("{:02}:{:02}:{:02}.{d}", s / 3600, (s / 60) % 60, s % 60)
}

/// Truncate `text` with an ellipsis so it fits in `max_w` pixels at `scale`.
fn fit_text(font: &FontArc, scale: PxScale, text: &str, max_w: f32) -> String {
    let sf = font.as_scaled(scale);
    let width = |s: &str| -> f32 { s.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum() };
    if width(text) <= max_w {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cand: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if width(&cand) <= max_w {
            return cand;
        }
    }
    String::new()
}

/// Evenly spaced times in `[start, end]`; composed with image+imageproc, 36 px label
/// strip under each tile drawn with ab_glyph using `font`.
///
/// `labels` is `Send + Sync` (plan said `&dyn Fn(f64) -> String`) so that callers inside
/// `Send` command futures can hold it across `.await`.
///
/// Times are the midpoints of `count` equal slices of the range, so the last tile never
/// lands on the final (possibly absent) frame. The output format follows `out`'s extension.
pub async fn tiles(
    paths: &ToolPaths,
    video: &Path,
    spec: &TileSpec,
    labels: &(dyn Fn(f64) -> String + Send + Sync),
    font: &Path,
    out: &Path,
) -> rustler_core::Result<Vec<TileLabel>> {
    if spec.count == 0 || spec.cols == 0 || spec.tile_width < 16 {
        return Err(validation("count, cols must be >= 1 and tile_width >= 16"));
    }
    if !(spec.start.is_finite() && spec.end.is_finite()) || spec.start < 0.0 || spec.end <= spec.start {
        return Err(validation("need 0 <= start < end"));
    }
    let font_bytes = std::fs::read(font).map_err(|e| {
        RustlerError::new(ErrorCode::NotFound, "tiles", format!("font not readable: {}: {e}", font.display()))
            .hint("run `rustler setup` to install fonts")
    })?;
    let font = FontArc::try_from_vec(font_bytes)
        .map_err(|e| RustlerError::new(ErrorCode::Validation, "tiles", format!("invalid font file: {e}")))?;

    let tmp = TempDir::new()?;

    let span = spec.end - spec.start;
    let mut result = Vec::with_capacity(spec.count as usize);
    let mut frames: Vec<RgbImage> = Vec::with_capacity(spec.count as usize);
    for i in 0..spec.count {
        let t = spec.start + span * (f64::from(i) + 0.5) / f64::from(spec.count);
        let jpg = tmp.path().join(format!("t{i:03}.jpg"));
        frame(paths, video, t, spec.tile_width, &jpg).await?;
        let img = image::open(&jpg)
            .map_err(|e| RustlerError::new(ErrorCode::Media, "tiles", format!("bad frame image: {e}")))?
            .to_rgb8();
        frames.push(img);
        let words = labels(t);
        let text = if words.trim().is_empty() {
            fmt_time(t)
        } else {
            format!("{} · {}", fmt_time(t), words.trim())
        };
        result.push(TileLabel { t, text });
    }

    let tile_w = spec.tile_width;
    let tile_h = frames.iter().map(|f| f.height()).max().unwrap_or(0);
    let cell_h = tile_h + LABEL_STRIP;
    let rows = spec.count.div_ceil(spec.cols);
    let mut sheet = RgbImage::from_pixel(spec.cols * tile_w, rows * cell_h, Rgb([16, 16, 16]));
    let scale = PxScale::from(20.0);
    for (i, (img, label)) in frames.iter().zip(&result).enumerate() {
        let (col, row) = (i as u32 % spec.cols, i as u32 / spec.cols);
        let (x0, y0) = (col * tile_w, row * cell_h);
        imageops::replace(&mut sheet, img, i64::from(x0), i64::from(y0));
        draw_filled_rect_mut(
            &mut sheet,
            Rect::at(x0 as i32, (y0 + tile_h) as i32).of_size(tile_w, LABEL_STRIP),
            Rgb([0, 0, 0]),
        );
        let text = fit_text(&font, scale, &label.text, tile_w as f32 - 12.0);
        draw_text_mut(
            &mut sheet,
            Rgb([255, 255, 255]),
            x0 as i32 + 6,
            (y0 + tile_h) as i32 + 7,
            scale,
            &font,
            &text,
        );
    }

    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    sheet
        .save(out)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "tiles", format!("cannot write {}: {e}", out.display())))?;
    Ok(result)
}
