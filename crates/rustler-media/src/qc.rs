//! QC v1 checks on a finished render. (Task 11)

use rustler_core::{Clip, QcReport, RenderRecord};
use rustler_tools::ToolPaths;
use std::path::Path;

pub async fn run_qc(
    paths: &ToolPaths,
    clip: &Clip,
    render: &RenderRecord,
    render_path: &Path,
    font: &Path,
    out_dir: &Path,
) -> rustler_core::Result<QcReport> {
    let _ = (paths, clip, render, render_path, font, out_dir);
    todo!("Task 11: run_qc")
}
