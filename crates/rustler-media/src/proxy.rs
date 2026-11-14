//! Proxy video and analysis audio extraction. (Task 6)

use crate::probe::probe;
use crate::proc::{self, argv};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use rustler_tools::ToolPaths;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

fn base_args() -> Vec<OsString> {
    argv!["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]
}

fn part_path(out: &Path) -> PathBuf {
    let mut name = out.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    out.with_file_name(name)
}

fn ensure_parent(out: &Path) -> rustler_core::Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "media", format!("cannot create {}: {e}", dir.display()))
        })?;
    }
    Ok(())
}

fn finish(part: &Path, out: &Path) -> rustler_core::Result<()> {
    let _ = std::fs::remove_file(out);
    std::fs::rename(part, out).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, "media", format!("cannot move output into place: {e}"))
    })
}

fn out_newer_than_src(src: &Path, out: &Path) -> bool {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    matches!((m(src), m(out)), (Some(s), Some(o)) if o >= s)
}

/// 540p max height, libx264 veryfast crf 30, aac 96k, keyint 30; skipped if `out` is newer than `src`.
pub async fn make_proxy(
    paths: &ToolPaths,
    src: &Path,
    out: &Path,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<()> {
    if out_newer_than_src(src, out) {
        return Ok(());
    }
    let info = probe(paths, src).await?;
    if !info.has_video {
        return Err(RustlerError::new(ErrorCode::Validation, "proxy", "source has no video")
            .hint("audio-only sources have no proxy; use the audio wav instead"));
    }
    ensure_parent(out)?;
    let part = part_path(out);
    let mut args = base_args();
    args.extend(argv!["-nostats", "-progress", "pipe:2", "-i"]);
    args.push(src.as_os_str().to_owned());
    args.extend(argv![
        "-map", "0:v:0", "-map", "0:a:0?",
        "-vf", "scale=-2:min(540\\,ih)",
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "30", "-g", "30", "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-b:a", "96k",
        "-movflags", "+faststart",
        "-f", "mp4",
    ]);
    args.push(part.as_os_str().to_owned());

    let total = info.duration;
    let res = proc::run_streaming(&paths.ffmpeg, &args, None, cancel, |line| {
        if let Some(pct) = progress_pct(line, total) {
            progress.emit(ProgressEvent {
                stage: "proxy".into(),
                pct: Some(pct),
                message: None,
            });
        }
    })
    .await;
    if let Err(mut e) = res {
        let _ = std::fs::remove_file(&part);
        e.stage = "proxy".into();
        return Err(e);
    }
    finish(&part, out)?;
    progress.emit(ProgressEvent { stage: "proxy".into(), pct: Some(1.0), message: None });
    Ok(())
}

/// Parses `out_time_us=123456` / `out_time_ms=123456` (both microseconds) from `-progress` output.
pub(crate) fn progress_pct(line: &str, total_secs: f64) -> Option<f32> {
    let v = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    let us: f64 = v.trim().parse().ok()?;
    (total_secs > 0.0).then(|| (us / 1e6 / total_secs).clamp(0.0, 1.0) as f32)
}

/// 16 kHz mono s16 wav.
pub async fn extract_audio_wav(
    paths: &ToolPaths,
    src: &Path,
    out: &Path,
    cancel: &CancellationToken,
) -> rustler_core::Result<()> {
    ensure_parent(out)?;
    let part = part_path(out);
    let mut args = base_args();
    args.push("-i".into());
    args.push(src.as_os_str().to_owned());
    args.extend(argv!["-map", "0:a:0", "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-f", "wav"]);
    args.push(part.as_os_str().to_owned());
    if let Err(mut e) = proc::run(&paths.ffmpeg, &args, None, cancel).await {
        let _ = std::fs::remove_file(&part);
        e.stage = "extract_audio".into();
        if e.code == ErrorCode::Media && e.message.contains("matches no streams") {
            return Err(RustlerError::new(ErrorCode::Validation, "extract_audio", "source has no audio track"));
        }
        return Err(e);
    }
    finish(&part, out)
}
