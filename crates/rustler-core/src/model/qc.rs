//! QC report for one render (schema "rustler.qc@1").

use crate::ids::{ClipId, RenderId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum QcStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct QcCheck {
    pub name: String,
    pub status: QcStatus,
    pub value: serde_json::Value,
    pub expected: String,
    pub fix_hint: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct QcReport {
    pub schema: String,
    pub clip_id: ClipId,
    pub render_id: RenderId,
    pub passed: bool,
    pub checks: Vec<QcCheck>,
    pub contact_sheet: PathBuf,
}
