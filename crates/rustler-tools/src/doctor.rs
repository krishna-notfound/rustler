//! `rustler doctor`: report which tools/encoders/fonts are available.

use crate::paths::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
pub struct ToolInfo {
    pub path: PathBuf,
    pub version: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
pub struct EncoderStatus {
    /// e.g. "h264_nvenc", "libx264".
    pub name: String,
    /// Passed the real 1-frame encode probe.
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct DoctorReport {
    pub os: String,
    pub arch: String,
    pub ffmpeg: Option<ToolInfo>,
    pub ffprobe: Option<ToolInfo>,
    pub ytdlp: Option<ToolInfo>,
    pub uv: Option<ToolInfo>,
    pub libass: bool,
    pub encoders: Vec<EncoderStatus>,
    pub nvidia_gpu: Option<String>,
    /// Result of engine `engine.capabilities`, when the engine is installed.
    pub engine: Option<serde_json::Value>,
    pub fonts: Vec<String>,
    pub ready: bool,
    pub missing: Vec<String>,
}

pub async fn doctor(paths: &ToolPaths) -> DoctorReport {
    let _ = paths;
    todo!("Task 4: doctor")
}
