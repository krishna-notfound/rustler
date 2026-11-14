//! Run a compiled render. (Task 9)

use crate::compile::{CompiledRender, GRAPH_FILE, PREVIEW_GRAPH_FILE};
use crate::proc_render;
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use rustler_tools::ToolPaths;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

const STAGE: &str = "render";

/// `key=value` lines written by `-progress pipe:2`.
fn is_progress_line(l: &str) -> bool {
    match l.split_once('=') {
        Some((k, v)) => {
            !k.is_empty()
                && k.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && !v.contains(' ')
        }
        None => false,
    }
}

/// Parses `out_time_us=` (preferred) or `out_time=HH:MM:SS.micro` into seconds.
fn progress_seconds(l: &str) -> Option<f64> {
    if let Some(v) = l
        .strip_prefix("out_time_us=")
        .or_else(|| l.strip_prefix("out_time_ms="))
    {
        // ffmpeg's out_time_ms is also microseconds (historical misnomer).
        return v.trim().parse::<f64>().ok().map(|us| us / 1e6);
    }
    let v = l.strip_prefix("out_time=")?;
    let mut it = v.trim().split(':');
    let (h, m, s) = (it.next()?, it.next()?, it.next()?);
    Some(h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?)
}

fn out_path(compiled: &CompiledRender) -> rustler_core::Result<PathBuf> {
    compiled.args.last().map(PathBuf::from).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Internal,
            STAGE,
            "compiled render has no arguments",
        )
    })
}

fn failure(what: &str, status: std::process::ExitStatus, tail: &[String]) -> RustlerError {
    let detail = if tail.is_empty() {
        String::new()
    } else {
        format!(":\n{}", tail.join("\n"))
    };
    RustlerError::new(
        ErrorCode::Media,
        STAGE,
        format!("ffmpeg {what} failed ({status}){detail}"),
    )
    .hint("inspect graph.txt in the render workdir; check that source files still exist")
}

/// Runs ffmpeg with cwd = workdir and `-progress pipe:2`; pct = out_time / expected_duration;
/// deletes partial output on failure.
pub async fn render(
    paths: &ToolPaths,
    compiled: &CompiledRender,
    expected_duration: f64,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<()> {
    let ffmpeg = proc_render::resolve_ffmpeg(paths)?;
    let out = out_path(compiled)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            RustlerError::new(
                ErrorCode::Internal,
                STAGE,
                format!("create {}: {e}", parent.display()),
            )
        })?;
    }
    if !compiled.workdir.join(GRAPH_FILE).is_file() {
        std::fs::write(compiled.workdir.join(GRAPH_FILE), &compiled.filter_script).map_err(
            |e| RustlerError::new(ErrorCode::Internal, STAGE, format!("write graph: {e}")),
        )?;
    }
    let mut args: Vec<OsString> = ["-progress", "pipe:2", "-nostats"]
        .iter()
        .map(OsString::from)
        .collect();
    args.extend(compiled.args.iter().cloned());

    progress.emit(ProgressEvent {
        stage: STAGE.into(),
        pct: Some(0.0),
        message: None,
    });
    let expected = if expected_duration > 0.0 {
        expected_duration
    } else {
        f64::INFINITY
    };
    let mut last_pct = 0.0f32;
    let result = proc_render::run_streaming(&ffmpeg, &args, Some(&compiled.workdir), cancel, |l| {
        if !is_progress_line(l) {
            return false;
        }
        if let Some(t) = progress_seconds(l) {
            let pct = (t / expected).clamp(0.0, 1.0) as f32;
            if pct - last_pct >= 0.01 {
                last_pct = pct;
                progress.emit(ProgressEvent {
                    stage: STAGE.into(),
                    pct: Some(pct),
                    message: None,
                });
            }
        }
        true
    })
    .await;

    let err = match result {
        Ok(o) if o.status.success() && out.is_file() => None,
        Ok(o) => Some(failure("render", o.status, &o.tail)),
        Err(e) => Some(e),
    };
    if let Some(e) = err {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    progress.emit(ProgressEvent {
        stage: STAGE.into(),
        pct: Some(1.0),
        message: Some(out.display().to_string()),
    });
    Ok(())
}

/// Same graph, `-ss out_t` on the output side, `-frames:v 1`.
pub async fn preview_frame(
    paths: &ToolPaths,
    compiled: &CompiledRender,
    out_t: f64,
    out_jpg: &Path,
) -> rustler_core::Result<()> {
    if !(out_t.is_finite() && out_t >= 0.0) {
        return Err(RustlerError::new(
            ErrorCode::Validation,
            STAGE,
            format!("out_t must be >= 0 (got {out_t})"),
        ));
    }
    let ffmpeg = proc_render::resolve_ffmpeg(paths)?;
    let Some(fc) = compiled.args.iter().position(|a| a == "-/filter_complex") else {
        return Err(RustlerError::new(
            ErrorCode::Internal,
            STAGE,
            "compiled render has no -/filter_complex",
        ));
    };
    // The full graph also produces [aout]; the preview graph sinks it so nothing dangles.
    std::fs::write(
        compiled.workdir.join(PREVIEW_GRAPH_FILE),
        format!("{};\n[aout]anullsink", compiled.filter_script),
    )
    .map_err(|e| {
        RustlerError::new(
            ErrorCode::Internal,
            STAGE,
            format!("write preview graph: {e}"),
        )
    })?;

    let out_jpg = std::path::absolute(out_jpg).unwrap_or_else(|_| out_jpg.to_path_buf());
    if let Some(parent) = out_jpg.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut args: Vec<OsString> = compiled.args[..fc].to_vec();
    args.extend(
        [
            "-/filter_complex",
            PREVIEW_GRAPH_FILE,
            "-map",
            "[vout]",
            "-ss",
        ]
        .iter()
        .map(OsString::from),
    );
    args.push(format!("{out_t:.3}").into());
    args.extend(
        ["-frames:v", "1", "-q:v", "3", "-update", "1"]
            .iter()
            .map(OsString::from),
    );
    args.push(out_jpg.clone().into_os_string());

    let cancel = CancellationToken::new();
    let o = proc_render::run_streaming(&ffmpeg, &args, Some(&compiled.workdir), &cancel, |_| false)
        .await?;
    if !o.status.success() || !out_jpg.is_file() {
        let _ = std::fs::remove_file(&out_jpg);
        let mut e = failure("preview_frame", o.status, &o.tail);
        if o.status.success() {
            e.message = format!("no frame at out_t={out_t:.3} (past the end of the clip?)");
            e.hint = Some("pick out_t inside the clip duration".into());
        }
        return Err(e);
    }
    Ok(())
}
