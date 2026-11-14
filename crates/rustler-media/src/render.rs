//! Run a compiled render. (Task 9)

use crate::compile::CompiledRender;
use rustler_registry::ProgressSink;
use rustler_tools::ToolPaths;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// Runs ffmpeg with cwd = workdir and `-progress pipe:2`; pct = out_time / expected_duration;
/// deletes partial output on failure.
pub async fn render(
    paths: &ToolPaths,
    compiled: &CompiledRender,
    expected_duration: f64,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<()> {
    let _ = (paths, compiled, expected_duration, cancel, progress);
    todo!("Task 9: render")
}

/// Same graph, `-ss out_t` on the output side, `-frames:v 1`.
pub async fn preview_frame(
    paths: &ToolPaths,
    compiled: &CompiledRender,
    out_t: f64,
    out_jpg: &Path,
) -> rustler_core::Result<()> {
    let _ = (paths, compiled, out_t, out_jpg);
    todo!("Task 9: preview_frame")
}
