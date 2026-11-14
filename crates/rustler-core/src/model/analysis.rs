//! Analysis outputs: scenes and audio events (source time, seconds).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct Scene {
    pub start: f64,
    pub end: f64,
}

/// `analysis/<id>/scenes.json`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct Scenes {
    pub scenes: Vec<Scene>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct LoudnessPoint {
    pub t: f64,
    pub lufs: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct Silence {
    pub start: f64,
    pub end: f64,
}

/// `analysis/<id>/audio_events.json`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct AudioEvents {
    /// 1 s buckets of momentary loudness.
    pub curve: Vec<LoudnessPoint>,
    pub peaks: Vec<LoudnessPoint>,
    pub silences: Vec<Silence>,
}
