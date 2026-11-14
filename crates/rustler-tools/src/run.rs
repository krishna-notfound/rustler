//! Subprocess helper. Arguments are always argv arrays (never through a shell).

use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Output};
use tokio_util::sync::CancellationToken;

/// Runs `cmd` to completion, capturing output. Media/DependencyMissing errors carry the
/// last 20 stderr lines in the message; cancellation kills the child (`Cancelled`).
pub async fn run(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
) -> rustler_core::Result<Output> {
    let _ = (cmd, args, cwd, cancel);
    todo!("Task 4: run")
}

/// Like [`run`] but streams stderr line by line to `on_stderr_line`.
pub async fn run_streaming(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
    on_stderr_line: impl FnMut(&str),
) -> rustler_core::Result<ExitStatus> {
    let _ = (cmd, args, cwd, cancel, on_stderr_line);
    todo!("Task 4: run_streaming")
}
