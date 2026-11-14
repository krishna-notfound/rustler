//! Layout presets: compute normalized regions for a source on the canvas.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::error::{ErrorCode, Result, RustlerError};
use crate::model::{Background, Canvas, Fit, Layout, Rect, Region};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PRESETS: &[&str] = &["center-crop", "blur-fit", "split-vertical"];

const STAGE: &str = "layout";

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

fn validation(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg)
}

/// `Validation` for unknown presets (listing `PRESETS`) and missing params
/// ("split-vertical needs --top and --bottom").
pub fn preset(name: &str, src_w: u32, src_h: u32, canvas: &Canvas, params: &LayoutParams) -> Result<Layout> {
    if !PRESETS.contains(&name) {
        return Err(validation(format!("unknown layout preset '{name}'; valid presets: {}", PRESETS.join(", ")))
            .hint("pass one of the listed preset names"));
    }
    if src_w == 0 || src_h == 0 {
        return Err(validation(format!("source dimensions must be non-zero, got {src_w}x{src_h}")));
    }
    if canvas.width == 0 || canvas.height == 0 {
        return Err(validation(format!("canvas dimensions must be non-zero, got {}x{}", canvas.width, canvas.height)));
    }
    let full = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let sa = src_w as f64 / src_h as f64;
    let ca = canvas.width as f64 / canvas.height as f64;

    match name {
        "center-crop" => {
            let focus = params.focus_x.unwrap_or(0.5);
            if !(0.0..=1.0).contains(&focus) {
                return Err(validation(format!("focus_x must be within 0..1, got {focus}")));
            }
            let src = if sa > ca {
                // Source is wider than the canvas: crop horizontally around the focus.
                let w = ca / sa;
                Rect { x: (focus - w / 2.0).clamp(0.0, 1.0 - w), y: 0.0, w, h: 1.0 }
            } else {
                // Source is narrower (or equal): crop vertically, centered.
                let h = sa / ca;
                Rect { x: 0.0, y: (1.0 - h) / 2.0, w: 1.0, h }
            };
            Ok(Layout {
                preset: Some(name.to_string()),
                regions: vec![Region { id: "main".into(), src, dst: full, z: 0, fit: Fit::Cover }],
                background: Background::Black,
            })
        }
        "blur-fit" => {
            // Contain: scale the whole source to the canvas width (or height when it is too tall).
            let dst = if sa >= ca {
                let h = ca / sa;
                Rect { x: 0.0, y: (1.0 - h) / 2.0, w: 1.0, h }
            } else {
                let w = sa / ca;
                Rect { x: (1.0 - w) / 2.0, y: 0.0, w, h: 1.0 }
            };
            Ok(Layout {
                preset: Some(name.to_string()),
                regions: vec![Region { id: "main".into(), src: full, dst, z: 0, fit: Fit::Contain }],
                background: Background::BlurFill,
            })
        }
        _ => {
            let (Some(top), Some(bottom)) = (params.top, params.bottom) else {
                return Err(validation("split-vertical needs --top and --bottom")
                    .hint("pass normalized source rects, e.g. --top 0.75,0,0.25,0.33 --bottom 0,0,0.75,1"));
            };
            let split = params.split.unwrap_or(0.5);
            if !(split > 0.0 && split < 1.0) {
                return Err(validation(format!("split must be within (0, 1), got {split}")));
            }
            for (label, r) in [("top", top), ("bottom", bottom)] {
                let ok = r.w > 0.0
                    && r.h > 0.0
                    && r.x >= 0.0
                    && r.y >= 0.0
                    && r.x + r.w <= 1.0 + 1e-9
                    && r.y + r.h <= 1.0 + 1e-9;
                if !ok {
                    return Err(validation(format!("{label} rect must lie within 0..1 with positive size, got {r:?}")));
                }
            }
            Ok(Layout {
                preset: Some(name.to_string()),
                regions: vec![
                    Region {
                        id: "top".into(),
                        src: top,
                        dst: Rect { x: 0.0, y: 0.0, w: 1.0, h: split },
                        z: 0,
                        fit: Fit::Cover,
                    },
                    Region {
                        id: "bottom".into(),
                        src: bottom,
                        dst: Rect { x: 0.0, y: split, w: 1.0, h: 1.0 - split },
                        z: 0,
                        fit: Fit::Cover,
                    },
                ],
                background: Background::Black,
            })
        }
    }
}
