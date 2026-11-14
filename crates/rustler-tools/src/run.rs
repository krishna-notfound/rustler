//! Subprocess helper. Arguments are always argv arrays (never through a shell).

use rustler_core::{ErrorCode, RustlerError};
use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Output, Stdio};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// Extra environment variables for a child process.
pub type EnvVars<'a> = &'a [(OsString, OsString)];

/// How many trailing stderr lines go into error messages.
const TAIL_LINES: usize = 20;

fn build(cmd: &Path, args: &[OsString], cwd: Option<&Path>, env: EnvVars<'_>) -> Command {
    let mut c = Command::new(cmd);
    c.args(args).stdin(Stdio::null()).kill_on_drop(true);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console flash when run from a GUI parent.
        c.creation_flags(0x0800_0000);
    }
    c
}

fn display_name(cmd: &Path) -> String {
    cmd.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cmd.display().to_string())
}

fn spawn_error(cmd: &Path, e: std::io::Error) -> RustlerError {
    if e.kind() == std::io::ErrorKind::NotFound {
        RustlerError::new(
            ErrorCode::DependencyMissing,
            "run",
            format!("cannot start `{}`: not found ({e})", cmd.display()),
        )
        .hint("run `rustler setup`")
    } else {
        RustlerError::new(
            ErrorCode::Media,
            "run",
            format!("cannot start `{}`: {e}", cmd.display()),
        )
    }
}

fn cancelled() -> RustlerError {
    RustlerError::new(ErrorCode::Cancelled, "run", "cancelled")
}

/// Last `n` non-empty lines of `bytes`, lossily decoded.
pub(crate) fn tail_lines(bytes: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Runs `cmd` to completion, capturing output. Media/DependencyMissing errors carry the
/// last 20 stderr lines in the message; cancellation kills the child (`Cancelled`).
pub async fn run(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
) -> rustler_core::Result<Output> {
    run_with_env(cmd, args, cwd, &[], cancel).await
}

/// [`run`] with extra environment variables.
pub async fn run_with_env(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    env: EnvVars<'_>,
    cancel: &CancellationToken,
) -> rustler_core::Result<Output> {
    let mut c = build(cmd, args, cwd, env);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| spawn_error(cmd, e))?;
    let out = tokio::select! {
        // Dropping the wait future drops the child, which kills it (kill_on_drop).
        _ = cancel.cancelled() => return Err(cancelled()),
        r = child.wait_with_output() => r.map_err(|e| {
            RustlerError::new(ErrorCode::Media, "run", format!("`{}` failed: {e}", display_name(cmd)))
        })?,
    };
    if out.status.success() {
        return Ok(out);
    }
    let tail = if out.stderr.is_empty() {
        tail_lines(&out.stdout, TAIL_LINES)
    } else {
        tail_lines(&out.stderr, TAIL_LINES)
    };
    Err(RustlerError::new(
        ErrorCode::Media,
        "run",
        format!("`{}` exited with {}: {tail}", display_name(cmd), out.status),
    ))
}

/// Like [`run`] but streams stderr line by line to `on_stderr_line` (lines are split on
/// `\n` and `\r`, so ffmpeg's in-place progress updates arrive as separate lines).
/// stdout is discarded. A non-zero exit is returned as `Ok(status)`; the caller decides.
pub async fn run_streaming(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
    on_stderr_line: impl FnMut(&str),
) -> rustler_core::Result<ExitStatus> {
    run_streaming_with_env(cmd, args, cwd, &[], cancel, on_stderr_line).await
}

/// [`run_streaming`] with extra environment variables.
pub async fn run_streaming_with_env(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    env: EnvVars<'_>,
    cancel: &CancellationToken,
    mut on_stderr_line: impl FnMut(&str),
) -> rustler_core::Result<ExitStatus> {
    let mut c = build(cmd, args, cwd, env);
    c.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| spawn_error(cmd, e))?;
    let mut stderr = child.stderr.take().expect("stderr piped");

    let mut line: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(cancelled());
            }
            r = stderr.read(&mut chunk) => r.unwrap_or(0),
        };
        if n == 0 {
            break;
        }
        for &b in &chunk[..n] {
            if b == b'\n' || b == b'\r' {
                if !line.is_empty() {
                    on_stderr_line(&String::from_utf8_lossy(&line));
                    line.clear();
                }
            } else {
                line.push(b);
            }
        }
    }
    if !line.is_empty() {
        on_stderr_line(&String::from_utf8_lossy(&line));
    }
    tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(cancelled())
        }
        r = child.wait() => r.map_err(|e| {
            RustlerError::new(ErrorCode::Media, "run", format!("`{}` failed: {e}", display_name(cmd)))
        }),
    }
}
