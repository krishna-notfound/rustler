//! Per-invocation context passed to every command.

use crate::progress::ProgressSink;
use rustler_core::{Actor, ErrorCode, ProjectStore, RustlerError};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Ctx {
    /// Resolved project dir (`--project`, `RUSTLER_PROJECT`, or cwd with project.json).
    pub project_dir: Option<PathBuf>,
    /// Tools home (`RUSTLER_HOME`, default `~/.rustler`).
    pub home: PathBuf,
    pub actor: Actor,
    pub dry_run: bool,
    pub yes: bool,
    pub progress: ProgressSink,
    pub cancel: CancellationToken,
    pub max_seconds: Option<u64>,
}

impl Ctx {
    /// Context with defaults: actor Agent, no dry-run, no `--yes`, no-op progress,
    /// fresh cancellation token, no time limit.
    pub fn new(project_dir: Option<PathBuf>, home: PathBuf) -> Self {
        Self {
            project_dir,
            home,
            actor: Actor::Agent,
            dry_run: false,
            yes: false,
            progress: ProgressSink::noop(),
            cancel: CancellationToken::new(),
            max_seconds: None,
        }
    }

    /// Opens the project store; `NotFound` "no project" when no project dir is set.
    pub fn store(&self) -> rustler_core::Result<ProjectStore> {
        match &self.project_dir {
            Some(dir) => ProjectStore::open(dir),
            None => Err(
                RustlerError::new(ErrorCode::NotFound, "project", "no project")
                    .hint("run `rustler project create` or pass --project"),
            ),
        }
    }
}
