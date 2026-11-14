//! Engine child-process client.

use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// Child process, pending-request map and reader task (fields defined by Task 5).
pub struct EngineClient {
    _private: (),
}

impl EngineClient {
    /// `<venv_python> -m rustler_engine`, cwd `home/engine`, env `HF_HOME`;
    /// `DependencyMissing` if the venv is missing.
    pub async fn spawn(paths: &ToolPaths) -> rustler_core::Result<Self> {
        let _ = paths;
        todo!("Task 5: EngineClient::spawn")
    }

    /// Spawn with an explicit interpreter/engine dir (tests use `uv run`).
    pub async fn spawn_with(
        python: &Path,
        engine_dir: &Path,
        env: &[(String, String)],
    ) -> rustler_core::Result<Self> {
        let _ = (python, engine_dir, env);
        todo!("Task 5: EngineClient::spawn_with")
    }

    /// Child exit mid-call -> `Internal` "engine crashed" + last stderr lines, retryable.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> rustler_core::Result<R> {
        let _ = (method, params, progress, cancel, &self._private);
        todo!("Task 5: EngineClient::call")
    }

    pub async fn shutdown(self) {
        todo!("Task 5: EngineClient::shutdown")
    }
}
