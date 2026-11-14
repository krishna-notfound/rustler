//! Append-only project event log entries (`events.ndjson`).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    #[default]
    Agent,
    Human,
    System,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Event {
    pub ts: DateTime<Utc>,
    pub actor: Actor,
    pub kind: String,
    pub project_revision: u64,
    pub data: serde_json::Value,
}
