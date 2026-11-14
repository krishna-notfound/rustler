//! Clip model: segments with layouts, caption track, audio settings, renders.

use crate::ids::{ClipId, RenderId, SegmentId, SourceId};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Output canvas. Default 1080x1920 @ 30 fps.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 1080,
            height: 1920,
            fps: 30.0,
        }
    }
}

/// Normalized rectangle, all values in 0..1 of the reference frame.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    Cover,
    Contain,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    Black,
    BlurFill,
}

/// A source rectangle placed into a destination rectangle of the canvas.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Region {
    pub id: String,
    pub src: Rect,
    pub dst: Rect,
    pub z: i32,
    pub fit: Fit,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Layout {
    pub preset: Option<String>,
    pub regions: Vec<Region>,
    pub background: Background,
}

/// A source-time range `[start, end)` of one source, with its layout.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Segment {
    pub id: SegmentId,
    pub source_id: SourceId,
    pub start: f64,
    pub end: f64,
    pub layout: Layout,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptionScript {
    #[default]
    Latin,
    Devanagari,
    AsTranscribed,
}

/// Default: enabled, "bold-karaoke", Latin, position_y 0.70, 3 words per line.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct CaptionTrack {
    pub enabled: bool,
    pub preset: String,
    pub script: CaptionScript,
    pub position_y: f64,
    pub max_words_per_line: u32,
}

impl Default for CaptionTrack {
    fn default() -> Self {
        Self {
            enabled: true,
            preset: "bold-karaoke".to_string(),
            script: CaptionScript::Latin,
            position_y: 0.70,
            max_words_per_line: 3,
        }
    }
}

/// Default: loudnorm on, I = -14 LUFS, TP = -2 dBTP, LRA = 11.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct AudioSettings {
    pub loudnorm: bool,
    pub target_lufs: f64,
    pub true_peak: f64,
    pub lra: f64,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            loudnorm: true,
            target_lufs: -14.0,
            true_peak: -2.0,
            lra: 11.0,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RenderQuality {
    Draft,
    Final,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct RenderRecord {
    pub id: RenderId,
    pub path: PathBuf,
    pub quality: RenderQuality,
    pub encoder: String,
    pub created_at: DateTime<Utc>,
    pub qc: Option<PathBuf>,
}

/// `clips/<id>/clip.json`, schema "rustler.clip@1".
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Clip {
    pub schema: String,
    pub id: ClipId,
    pub name: String,
    pub revision: u64,
    pub canvas: Canvas,
    pub segments: Vec<Segment>,
    pub captions: CaptionTrack,
    pub audio: AudioSettings,
    pub renders: Vec<RenderRecord>,
    pub created_at: DateTime<Utc>,
}
