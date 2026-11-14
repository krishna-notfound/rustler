//! Run a compiled render. (Task 9)

use crate::compile::{
    CompiledRender, GRAPH_FILE, LoudnessMeasurement, LoudnessPass, PREVIEW_GRAPH_FILE,
    measured_loudnorm, parse_loudnorm_json,
};
use crate::proc_render;
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::Serialize;
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

/// How the loudness of a render was normalized.
#[derive(Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum LoudnessMode {
    /// loudnorm is off for the clip.
    Off,
    /// One loudnorm pass (two-pass disabled, or the measurement failed).
    SinglePass,
    /// Measured first, then normalized with the measured values.
    TwoPass,
}

#[derive(Serialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct RenderReport {
    pub loudness_mode: LoudnessMode,
    /// First-pass values (two-pass only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<LoudnessMeasurement>,
    /// Why two-pass fell back to single pass.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Runs the audio-only measurement pass; `Err(reason)` when it fails or prints no usable JSON.
async fn measure_loudness(
    ffmpeg: &Path,
    compiled: &CompiledRender,
    pass: &LoudnessPass,
    cancel: &CancellationToken,
) -> rustler_core::Result<std::result::Result<LoudnessMeasurement, String>> {
    let mut log = String::new();
    let o = proc_render::run_streaming(ffmpeg, &pass.args, Some(&compiled.workdir), cancel, |l| {
        log.push_str(l);
        log.push('\n');
        true
    })
    .await;
    let o = match o {
        Ok(o) => o,
        Err(e) if e.code == ErrorCode::Cancelled => return Err(e),
        Err(e) => return Ok(Err(e.message)),
    };
    if !o.status.success() {
        let tail: Vec<&str> = log.lines().rev().take(5).collect();
        return Ok(Err(format!(
            "measurement pass failed ({}): {}",
            o.status,
            tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
        )));
    }
    Ok(parse_loudnorm_json(&log)
        .ok_or_else(|| "measurement printed no finite loudness values (silent audio?)".to_string()))
}

/// Runs ffmpeg with cwd = workdir and `-progress pipe:2`; pct = out_time / expected_duration;
/// deletes partial output on failure. With `compiled.loudness` set, a fast audio-only pass
/// measures the loudness first and the final pass uses the measured values (falls back to
/// single-pass loudnorm when the measurement fails).
pub async fn render(
    paths: &ToolPaths,
    compiled: &CompiledRender,
    expected_duration: f64,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<()> {
    render_with_report(paths, compiled, expected_duration, cancel, progress)
        .await
        .map(|_| ())
}

/// [`render`] plus a report of how loudness was normalized.
pub async fn render_with_report(
    paths: &ToolPaths,
    compiled: &CompiledRender,
    expected_duration: f64,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<RenderReport> {
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
    let mut report = RenderReport {
        loudness_mode: if compiled.filter_script.contains("loudnorm=") {
            LoudnessMode::SinglePass
        } else {
            LoudnessMode::Off
        },
        measured: None,
        warnings: Vec::new(),
    };
    let mut graph = compiled.filter_script.clone();
    if let Some(pass) = &compiled.loudness {
        progress.emit(ProgressEvent {
            stage: "render.loudness".into(),
            pct: Some(0.0),
            message: Some("measuring loudness (pass 1 of 2)".into()),
        });
        match measure_loudness(&ffmpeg, compiled, pass, cancel).await? {
            Ok(m) if graph.contains(&pass.filter) => {
                graph = graph.replacen(&pass.filter, &measured_loudnorm(&pass.filter, &m), 1);
                report.loudness_mode = LoudnessMode::TwoPass;
                report.measured = Some(m);
            }
            Ok(_) => report
                .warnings
                .push("two-pass loudnorm skipped: loudnorm filter not found in the graph".into()),
            Err(reason) => {
                tracing::warn!("two-pass loudnorm fell back to single pass: {reason}");
                report
                    .warnings
                    .push(format!("two-pass loudnorm fell back to single pass: {reason}"));
            }
        }
    }
    std::fs::write(compiled.workdir.join(GRAPH_FILE), &graph).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, STAGE, format!("write graph: {e}"))
    })?;
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
    Ok(report)
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
