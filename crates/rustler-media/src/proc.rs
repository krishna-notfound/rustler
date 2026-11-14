//! Private subprocess helper (argv only, never a shell). Mirrors the signatures of
//! `rustler_tools::{run, run_streaming}` so the orchestrator can swap it out later.

use rustler_core::{ErrorCode, RustlerError};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Output, Stdio};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const STDERR_TAIL_LINES: usize = 20;

fn command(cmd: &Path, args: &[OsString], cwd: Option<&Path>) -> Command {
    let mut c = Command::new(cmd);
    c.args(args).stdin(Stdio::null()).kill_on_drop(true);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW
        c.creation_flags(0x0800_0000);
    }
    c
}

fn spawn_error(cmd: &Path, e: std::io::Error) -> RustlerError {
    if e.kind() == std::io::ErrorKind::NotFound {
        RustlerError::new(
            ErrorCode::DependencyMissing,
            "process",
            format!("tool not found: {}", cmd.display()),
        )
        .hint("run `rustler setup`")
    } else {
        RustlerError::new(
            ErrorCode::Media,
            "process",
            format!("failed to run {}: {e}", cmd.display()),
        )
    }
}

fn cancelled() -> RustlerError {
    RustlerError::new(ErrorCode::Cancelled, "process", "cancelled")
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(STDERR_TAIL_LINES);
    lines[from..].join("\n")
}

fn failure(cmd: &Path, status: ExitStatus, stderr_tail: &str) -> RustlerError {
    let name = cmd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cmd.display().to_string());
    RustlerError::new(
        ErrorCode::Media,
        "process",
        format!("{name} failed ({status}):\n{stderr_tail}"),
    )
}

/// Runs `cmd` to completion capturing stdout/stderr. Non-zero exit -> `Media` with the last
/// 20 stderr lines in the message; cancellation kills the child (`Cancelled`).
pub(crate) async fn run(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
) -> rustler_core::Result<Output> {
    let mut c = command(cmd, args, cwd);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| spawn_error(cmd, e))?;
    let out = tokio::select! {
        r = child.wait_with_output() => r.map_err(|e| spawn_error(cmd, e))?,
        _ = cancel.cancelled() => return Err(cancelled()), // dropping the child kills it
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(failure(cmd, out.status, &tail(&err)));
    }
    Ok(out)
}

/// Like [`run`] but streams stderr line by line (lines end at `\n` or `\r`) to `on_line`.
/// Stdout is discarded. Non-zero exit -> `Media` with the last 20 stderr lines.
pub(crate) async fn run_streaming(
    cmd: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> rustler_core::Result<ExitStatus> {
    let mut c = command(cmd, args, cwd);
    c.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| spawn_error(cmd, e))?;
    let stderr = child.stderr.take().expect("piped stderr");
    let mut reader = BufReader::new(stderr);
    let mut tail_buf: VecDeque<String> = VecDeque::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];

    fn emit(
        bytes: &[u8],
        tail_buf: &mut VecDeque<String>,
        on_line: &mut impl FnMut(&str),
    ) {
        let line = String::from_utf8_lossy(bytes);
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        on_line(line);
        if tail_buf.len() == STDERR_TAIL_LINES {
            tail_buf.pop_front();
        }
        tail_buf.push_back(line.to_string());
    }

    loop {
        let n = tokio::select! {
            r = reader.read(&mut chunk) => r.map_err(|e| spawn_error(cmd, e))?,
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(cancelled());
            }
        };
        if n == 0 {
            break;
        }
        for &b in &chunk[..n] {
            if b == b'\n' || b == b'\r' {
                emit(&pending, &mut tail_buf, &mut on_line);
                pending.clear();
            } else {
                pending.push(b);
            }
        }
    }
    emit(&pending, &mut tail_buf, &mut on_line);

    let status = tokio::select! {
        r = child.wait() => r.map_err(|e| spawn_error(cmd, e))?,
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(cancelled());
        }
    };
    if !status.success() {
        let t: Vec<String> = tail_buf.into_iter().collect();
        return Err(failure(cmd, status, &t.join("\n")));
    }
    Ok(status)
}

/// `Vec<OsString>` argv from string-ish items.
macro_rules! argv {
    ($($a:expr),* $(,)?) => {{
        let v: Vec<std::ffi::OsString> = vec![$(std::ffi::OsString::from($a)),*];
        v
    }};
}
pub(crate) use argv;
