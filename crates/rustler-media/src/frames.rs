//! Single frames and short previews for `see.*`. (Task 6)

use rustler_tools::ToolPaths;
use std::path::Path;

/// `-ss` before `-i`, 1 frame, jpg q 3, scaled to `width`.
pub async fn frame(
    paths: &ToolPaths,
    video: &Path,
    t: f64,
    width: u32,
    out: &Path,
) -> rustler_core::Result<()> {
    let _ = (paths, video, t, width, out);
    todo!("Task 6: frame")
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
    let _ = (paths, video, start, end, height, out);
    todo!("Task 6: preview")
}
