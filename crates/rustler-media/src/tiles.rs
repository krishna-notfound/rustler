//! Labelled tile grids (contact sheets). (Task 6)

use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

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

/// Evenly spaced times in `[start, end]`; composed with image+imageproc, 36 px label
/// strip under each tile drawn with ab_glyph using `font`.
///
/// `labels` is `Send + Sync` (plan said `&dyn Fn(f64) -> String`) so that callers inside
/// `Send` command futures can hold it across `.await`.
pub async fn tiles(
    paths: &ToolPaths,
    video: &Path,
    spec: &TileSpec,
    labels: &(dyn Fn(f64) -> String + Send + Sync),
    font: &Path,
    out: &Path,
) -> rustler_core::Result<Vec<TileLabel>> {
    let _ = (paths, video, spec, labels, font, out);
    todo!("Task 6: tiles")
}
