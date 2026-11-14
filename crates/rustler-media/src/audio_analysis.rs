//! Loudness curve, peaks and silences in one ffmpeg pass. (Task 6)

use rustler_core::AudioEvents;
use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// ebur128 momentary loudness (10 Hz) -> 1 s buckets (max M); peaks = buckets >= median + 6 LU
/// that are local maxima within +/-5 s; silences via `silencedetect=n=-35dB:d=0.6` in the same graph.
pub async fn analyze_audio(
    paths: &ToolPaths,
    media: &Path,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<AudioEvents> {
    let _ = (paths, media, cancel, progress);
    todo!("Task 6: analyze_audio")
}
