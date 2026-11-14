//! Proxy video and analysis audio extraction. (Task 6)

use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// 540p max height, libx264 veryfast crf 30, aac 96k, keyint 30; skipped if `out` is newer than `src`.
pub async fn make_proxy(
    paths: &ToolPaths,
    src: &Path,
    out: &Path,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<()> {
    let _ = (paths, src, out, cancel, progress);
    todo!("Task 6: make_proxy")
}

/// 16 kHz mono s16 wav.
pub async fn extract_audio_wav(
    paths: &ToolPaths,
    src: &Path,
    out: &Path,
    cancel: &CancellationToken,
) -> rustler_core::Result<()> {
    let _ = (paths, src, out, cancel);
    todo!("Task 6: extract_audio_wav")
}
