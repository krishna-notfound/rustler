//! Layout presets: compute normalized regions for a source on the canvas.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::error::Result;
use crate::model::{Canvas, Layout, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PRESETS: &[&str] = &["center-crop", "blur-fit", "split-vertical"];

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
#[serde(default)]
pub struct LayoutParams {
    /// 0..1 horizontal center for center-crop (default 0.5).
    pub focus_x: Option<f64>,
    /// split-vertical: source rect for the top pane.
    pub top: Option<Rect>,
    /// split-vertical: source rect for the bottom pane.
    pub bottom: Option<Rect>,
    /// split-vertical: top pane height fraction (default 0.5).
    pub split: Option<f64>,
}

/// `Validation` for unknown presets (listing `PRESETS`) and missing params
/// ("split-vertical needs --top and --bottom").
pub fn preset(
    name: &str,
    src_w: u32,
    src_h: u32,
    canvas: &Canvas,
    params: &LayoutParams,
) -> Result<Layout> {
    let _ = (name, src_w, src_h, canvas, params);
    todo!("Task 3: layout::preset")
}
