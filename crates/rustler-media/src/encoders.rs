//! Hardware/software H.264 encoder selection. (Task 9)

use rustler_core::RenderQuality;
use rustler_tools::ToolPaths;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct EncoderChoice {
    /// "h264_nvenc" | "h264_qsv" | "h264_amf" | "h264_videotoolbox" | "libx264"
    pub name: String,
    pub args: Vec<String>,
}

/// Order: h264_nvenc, h264_qsv, h264_amf, h264_videotoolbox, libx264; each must pass the
/// 1-frame probe (cached in `home/encoders.json`).
pub async fn pick_encoder(
    paths: &ToolPaths,
    quality: RenderQuality,
    prefer_gpu: bool,
) -> EncoderChoice {
    let _ = (paths, quality, prefer_gpu);
    todo!("Task 9: pick_encoder")
}
