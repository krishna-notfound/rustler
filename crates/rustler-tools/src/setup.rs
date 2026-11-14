//! `rustler setup`: idempotent install of ffmpeg, yt-dlp, uv, the Python engine,
//! whisper models and fonts into the Rustler home.

use crate::paths::ToolPaths;
use rustler_registry::ProgressSink;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    #[default]
    Lite,
    Full,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpuMode {
    #[default]
    Auto,
    Cuda,
    None,
}

/// Default: Lite, Auto, `["small"]`, not forced.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct SetupOptions {
    pub profile: Profile,
    pub gpu: GpuMode,
    pub whisper_models: Vec<String>,
    pub force: bool,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            profile: Profile::Lite,
            gpu: GpuMode::Auto,
            whisper_models: vec!["small".to_string()],
            force: false,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
pub struct Component {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub sha256: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
pub struct SetupReport {
    pub installed: Vec<Component>,
    pub skipped: Vec<Component>,
    /// (component, error message)
    pub failed: Vec<(String, String)>,
}

pub async fn setup(
    paths: &ToolPaths,
    opts: &SetupOptions,
    progress: &ProgressSink,
) -> rustler_core::Result<SetupReport> {
    let _ = (paths, opts, progress);
    todo!("Task 4: setup")
}
