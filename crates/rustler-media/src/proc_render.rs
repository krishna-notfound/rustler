//! Small tokio::process helper for the render path (Task 9). Kept local so the renderer does not
//! depend on `rustler_tools::run` while that lands in parallel. Arguments are argv arrays, never a
//! shell string.

use rustler_core::{ErrorCode, RustlerError};
use rustler_tools::ToolPaths;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const TAIL_LINES: usize = 20;

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// `RUSTLER_HOME`, else `~/.rustler` (USERPROFILE on Windows, HOME elsewhere).
pub(crate) fn default_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("RUSTLER_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(h));
    }
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    }?;
    Some(PathBuf::from(home).join(".rustler"))
}

/// Resolves a tool binary: the path in `paths` if it exists, else `<home>/bin/<name>(.exe)`
/// where home is `paths.home`, then `RUSTLER_HOME` / `~/.rustler`.
pub(crate) fn resolve_tool(
    configured: &Path,
    home: &Path,
    name: &str,
) -> rustler_core::Result<PathBuf> {
    if !configured.as_os_str().is_empty() && configured.is_file() {
        return Ok(configured.to_path_buf());
    }
    let mut candidates = vec![home.join("bin").join(exe(name))];
    if let Some(h) = default_home() {
        candidates.push(h.join("bin").join(exe(name)));
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::DependencyMissing,
            "render",
            format!("{name} not found in {}", home.join("bin").display()),
        )
        .hint("run `rustler setup`")
    })
}

pub(crate) fn resolve_ffmpeg(paths: &ToolPaths) -> rustler_core::Result<PathBuf> {
    resolve_tool(&paths.ffmpeg, &paths.home, "ffmpeg")
}

pub(crate) struct RunOutcome {
    pub status: ExitStatus,
    /// Last stderr lines that were not consumed as progress (for error messages).
    pub tail: Vec<String>,
}

/// Runs `program args` with `cwd`, streaming stderr lines to `on_line`. Lines for which
/// `on_line` returns `false` are kept in the error tail. Cancellation kills the child.
pub(crate) async fn run_streaming(
    program: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&str) -> bool,
) -> rustler_core::Result<RunOutcome> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console flash when run from a GUI host.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| {
        RustlerError::new(
            ErrorCode::DependencyMissing,
            "render",
            format!("failed to start {}: {e}", program.display()),
        )
        .hint("run `rustler setup`")
    })?;
    let stderr = child.stderr.take().expect("stderr piped");
    let mut lines = BufReader::new(stderr).lines();
    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL_LINES);
    let push_tail = |l: &str, tail: &mut VecDeque<String>| {
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(l.to_string());
    };

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(RustlerError::new(ErrorCode::Cancelled, "render", "cancelled"));
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        if !on_line(&l) {
                            push_tail(&l, &mut tail);
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }
    let status = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(RustlerError::new(ErrorCode::Cancelled, "render", "cancelled"));
        }
        s = child.wait() => s.map_err(|e| RustlerError::new(ErrorCode::Media, "render", format!("waiting for ffmpeg: {e}")))?,
    };
    Ok(RunOutcome {
        status,
        tail: tail.into_iter().collect(),
    })
}
