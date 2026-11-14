//! Single frames and short previews for `see.*`. (Task 6)

use crate::proc::{self, argv};
use rustler_core::{ErrorCode, RustlerError};
use rustler_tools::ToolPaths;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn ensure_parent(out: &Path) -> rustler_core::Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "media", format!("cannot create {}: {e}", dir.display()))
        })?;
    }
    Ok(())
}

/// `-ss` before `-i`, 1 frame, jpg q 3, scaled to `width`.
pub async fn frame(
    paths: &ToolPaths,
    video: &Path,
    t: f64,
    width: u32,
    out: &Path,
) -> rustler_core::Result<()> {
    if !t.is_finite() || t < 0.0 || width == 0 {
        return Err(RustlerError::new(ErrorCode::Validation, "frame", "t must be >= 0 and width > 0"));
    }
    ensure_parent(out)?;
    let _ = std::fs::remove_file(out); // so a missing frame is detectable below
    let mut args = argv!["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-ss", format!("{t:.3}"), "-i"];
    args.push(video.as_os_str().to_owned());
    args.extend(argv![
        "-frames:v", "1",
        "-vf", format!("scale={width}:-2"),
        "-q:v", "3",
    ]);
    args.push(out.as_os_str().to_owned());
    proc::run(&paths.ffmpeg, &args, None, &CancellationToken::new())
        .await
        .map_err(|mut e| {
            e.stage = "frame".into();
            e
        })?;
    if !out.exists() {
        return Err(RustlerError::new(
            ErrorCode::Media,
            "frame",
            format!("no frame at t={t:.3}s (past end of media?)"),
        ));
    }
    Ok(())
}

/// Low-res mp4 preview of `[start, end]` at `height`; max 60 s, longer -> `Validation`.
pub async fn preview(
    paths: &ToolPaths,
    video: &Path,
    start: f64,
    end: f64,
    height: u32,
    out: &Path,
) -> rustler_core::Result<()> {
    let dur = end - start;
    if !(start.is_finite() && end.is_finite()) || start < 0.0 || dur <= 0.0 {
        return Err(RustlerError::new(ErrorCode::Validation, "preview", "need 0 <= start < end"));
    }
    if dur > 60.0 {
        return Err(RustlerError::new(
            ErrorCode::Validation,
            "preview",
            format!("preview range is {dur:.1}s; the maximum is 60s"),
        )
        .hint("pass a shorter --start/--end range"));
    }
    if height < 16 {
        return Err(RustlerError::new(ErrorCode::Validation, "preview", "height must be >= 16"));
    }
    ensure_parent(out)?;
    let mut args = argv!["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-ss", format!("{start:.3}"), "-i"];
    args.push(video.as_os_str().to_owned());
    args.extend(argv![
        "-t", format!("{dur:.3}"),
        "-vf", format!("scale=-2:{}", height & !1),
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "30", "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-b:a", "64k",
        "-movflags", "+faststart",
    ]);
    args.push(out.as_os_str().to_owned());
    proc::run(&paths.ffmpeg, &args, None, &CancellationToken::new())
        .await
        .map_err(|mut e| {
            e.stage = "preview".into();
            e
        })?;
    Ok(())
}
