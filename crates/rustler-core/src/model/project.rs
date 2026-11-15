//! `project.json`: project identity, membership lists, permissions, settings.

use crate::ids::{ClipId, ProjectId, SourceId};
use crate::model::clip::{Canvas, CaptionScript};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Schema "rustler.project@1".
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Project {
    pub schema: String,
    pub id: ProjectId,
    pub name: String,
    pub revision: u64,
    pub created_at: DateTime<Utc>,
    pub sources: Vec<SourceId>,
    pub clips: Vec<ClipId>,
    pub permissions: Permissions,
    pub settings: ProjectSettings,
}

/// Default: network and downloads allowed, any fs root, no cookies, no spend.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Permissions {
    pub network: bool,
    pub downloads: bool,
    /// Empty = any path allowed.
    pub fs_roots: Vec<PathBuf>,
    pub cookies_file: Option<PathBuf>,
    pub max_spend_usd: f64,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            network: true,
            downloads: true,
            fs_roots: Vec::new(),
            cookies_file: None,
            max_spend_usd: 0.0,
        }
    }
}

/// Default: 1080x1920@30 canvas, Latin captions, whisper model "base" (the lite install fetches it; switch with `project set-settings`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct ProjectSettings {
    pub canvas: Canvas,
    pub caption_script: CaptionScript,
    pub default_whisper_model: String,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            canvas: Canvas::default(),
            caption_script: CaptionScript::Latin,
            default_whisper_model: "base".to_string(),
        }
    }
}
