//! Hardware/software H.264 encoder selection. (Task 9)

use crate::proc_render;
use rustler_core::RenderQuality;
use rustler_tools::ToolPaths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct EncoderChoice {
    /// "h264_nvenc" | "h264_qsv" | "h264_amf" | "h264_videotoolbox" | "libx264"
    pub name: String,
    pub args: Vec<String>,
}

/// GPU encoders in preference order; `libx264` is the always-available fallback.
pub const GPU_ORDER: &[&str] = &["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"];

/// Encoder argv for `name` at `quality` (always 4:2:0 output).
pub fn encoder_args(name: &str, quality: RenderQuality) -> Vec<String> {
    let final_q = quality == RenderQuality::Final;
    let a: Vec<&str> = match name {
        "h264_nvenc" => {
            if final_q {
                vec![
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p5",
                    "-rc",
                    "vbr",
                    "-cq",
                    "23",
                    "-b:v",
                    "0",
                ]
            } else {
                vec![
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p2",
                    "-rc",
                    "vbr",
                    "-cq",
                    "30",
                    "-b:v",
                    "0",
                ]
            }
        }
        "h264_qsv" => {
            if final_q {
                vec![
                    "-c:v",
                    "h264_qsv",
                    "-preset",
                    "medium",
                    "-global_quality",
                    "23",
                ]
            } else {
                vec![
                    "-c:v",
                    "h264_qsv",
                    "-preset",
                    "veryfast",
                    "-global_quality",
                    "30",
                ]
            }
        }
        "h264_amf" => {
            if final_q {
                vec![
                    "-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "22",
                    "-qp_p", "24",
                ]
            } else {
                vec![
                    "-c:v", "h264_amf", "-quality", "speed", "-rc", "cqp", "-qp_i", "30", "-qp_p",
                    "32",
                ]
            }
        }
        "h264_videotoolbox" => {
            if final_q {
                vec!["-c:v", "h264_videotoolbox", "-q:v", "65"]
            } else {
                vec!["-c:v", "h264_videotoolbox", "-q:v", "50"]
            }
        }
        _ => {
            if final_q {
                vec!["-c:v", "libx264", "-preset", "medium", "-crf", "20"]
            } else {
                vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "28"]
            }
        }
    };
    let mut v: Vec<String> = a.into_iter().map(String::from).collect();
    // QSV wants NV12 frames (still 4:2:0); everyone else gets plain yuv420p.
    let pix = if name == "h264_qsv" {
        "nv12"
    } else {
        "yuv420p"
    };
    v.extend(["-pix_fmt".to_string(), pix.to_string()]);
    v
}

pub fn choice(name: &str, quality: RenderQuality) -> EncoderChoice {
    let name = if GPU_ORDER.contains(&name) {
        name
    } else {
        "libx264"
    };
    EncoderChoice {
        name: name.to_string(),
        args: encoder_args(name, quality),
    }
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct EncoderCache {
    ffmpeg: PathBuf,
    /// ffmpeg binary size + mtime: re-probe after an ffmpeg upgrade.
    ffmpeg_stamp: String,
    results: BTreeMap<String, bool>,
}

fn stamp(p: &Path) -> String {
    std::fs::metadata(p)
        .map(|m| {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{t}", m.len())
        })
        .unwrap_or_default()
}

/// Real 1-frame encode to the null muxer with the encoder's draft args.
pub async fn probe_encoder(ffmpeg: &Path, name: &str) -> bool {
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x256:r=30:d=0.1",
        "-frames:v",
        "1",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.extend(
        encoder_args(name, RenderQuality::Draft)
            .into_iter()
            .map(OsString::from),
    );
    args.extend(["-f", "null", "-"].iter().map(OsString::from));
    let cancel = CancellationToken::new();
    let fut = proc_render::run_streaming(ffmpeg, &args, None, &cancel, |_| false);
    match tokio::time::timeout(std::time::Duration::from_secs(20), fut).await {
        Ok(Ok(o)) => o.status.success(),
        _ => false,
    }
}

/// Order: h264_nvenc, h264_qsv, h264_amf, h264_videotoolbox, libx264; each must pass the
/// 1-frame probe (cached in `home/encoders.json`).
pub async fn pick_encoder(
    paths: &ToolPaths,
    quality: RenderQuality,
    prefer_gpu: bool,
) -> EncoderChoice {
    if !prefer_gpu {
        return choice("libx264", quality);
    }
    let Ok(ffmpeg) = proc_render::resolve_ffmpeg(paths) else {
        return choice("libx264", quality);
    };
    let cache_path = paths.home.join("encoders.json");
    let ffmpeg_stamp = stamp(&ffmpeg);
    let mut cache: EncoderCache = std::fs::read(&cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|c: &EncoderCache| c.ffmpeg == ffmpeg && c.ffmpeg_stamp == ffmpeg_stamp)
        .unwrap_or_else(|| EncoderCache {
            ffmpeg: ffmpeg.clone(),
            ffmpeg_stamp: ffmpeg_stamp.clone(),
            results: BTreeMap::new(),
        });
    let mut dirty = false;
    let mut picked = None;
    for name in GPU_ORDER {
        let ok = match cache.results.get(*name) {
            Some(ok) => *ok,
            None => {
                let ok = probe_encoder(&ffmpeg, name).await;
                cache.results.insert(name.to_string(), ok);
                dirty = true;
                ok
            }
        };
        if ok {
            picked = Some(*name);
            break;
        }
    }
    if dirty && let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        let _ = std::fs::create_dir_all(&paths.home);
        let tmp = cache_path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &cache_path);
        }
    }
    choice(picked.unwrap_or("libx264"), quality)
}
