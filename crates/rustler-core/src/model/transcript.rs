//! Word-level transcripts and the sentence/clause units agents select by id.

use crate::ids::SourceId;
use crate::model::clip::CaptionScript;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Word {
    pub i: u32,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub conf: Option<f32>,
    pub speaker: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum WordTiming {
    Exact,
    /// e.g. YouTube captions.
    Approximate,
}

/// Schema "rustler.transcript@1". `engine` is "faster-whisper:<model>" or "youtube:<kind>".
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Transcript {
    pub schema: String,
    pub source_id: SourceId,
    pub language: String,
    pub engine: String,
    pub script: CaptionScript,
    pub words: Vec<Word>,
    pub word_timing: WordTiming,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Sentence,
    Clause,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Unit {
    pub id: u32,
    pub kind: UnitKind,
    pub word_start: u32,
    /// Inclusive.
    pub word_end: u32,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub boundary_strength: f32,
}
