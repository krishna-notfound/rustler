//! Progress reporting. Progress goes to stderr as NDJSON, never stdout.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProgressEvent {
    pub stage: String,
    /// 0.0..=1.0 when known.
    pub pct: Option<f32>,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct ProgressSink {
    f: Arc<dyn Fn(ProgressEvent) + Send + Sync>,
}

impl ProgressSink {
    pub fn new(f: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        Self { f: Arc::new(f) }
    }

    /// Writes `{"event":"progress","stage":…,"pct":…,"message":…}` lines to stderr.
    pub fn stderr_ndjson() -> Self {
        Self::new(|e: ProgressEvent| {
            let line = serde_json::json!({
                "event": "progress",
                "stage": e.stage,
                "pct": e.pct,
                "message": e.message,
            });
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "{line}");
        })
    }

    pub fn noop() -> Self {
        Self::new(|_| {})
    }

    pub fn emit(&self, e: ProgressEvent) {
        (self.f)(e)
    }
}

impl Default for ProgressSink {
    fn default() -> Self {
        Self::noop()
    }
}

impl fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProgressSink")
    }
}
