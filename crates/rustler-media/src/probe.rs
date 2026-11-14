//! ffprobe wrapper. (Task 6)

use rustler_core::MediaProbe;
use rustler_tools::ToolPaths;
use std::path::Path;

/// `ffprobe -v error -print_format json -show_streams -show_format`; rotation from
/// side_data/tags; display dims swapped on +/-90.
pub async fn probe(paths: &ToolPaths, media: &Path) -> rustler_core::Result<MediaProbe> {
    let _ = (paths, media);
    todo!("Task 6: probe")
}
