//! Imported media sources and their probe data.

use crate::ids::SourceId;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Seconds in source time.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceOrigin {
    File {
        path: PathBuf,
    },
    Url {
        url: String,
        section: Option<TimeRange>,
        original_offset: f64,
    },
}

/// ffprobe summary. `width`/`height` are display dims after rotation.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct MediaProbe {
    pub duration: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub width: u32,
    pub height: u32,
    pub rotation: i32,
    pub fps: f64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

/// `sources/<id>/source.json`. Paths are project-relative when inside the project dir.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Source {
    pub id: SourceId,
    pub name: String,
    pub origin: SourceOrigin,
    pub media_path: PathBuf,
    pub probe: MediaProbe,
    pub proxy: Option<PathBuf>,
    pub audio_wav: Option<PathBuf>,
    pub imported_at: DateTime<Utc>,
}
