//! `rustler doctor`: report which tools/encoders/fonts are available.

use crate::paths::ToolPaths;
use crate::run::run;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
pub struct ToolInfo {
    pub path: PathBuf,
    pub version: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
pub struct EncoderStatus {
    /// e.g. "h264_nvenc", "libx264".
    pub name: String,
    /// Passed the real 1-frame encode probe.
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct DoctorReport {
    pub os: String,
    pub arch: String,
    pub ffmpeg: Option<ToolInfo>,
    pub ffprobe: Option<ToolInfo>,
    pub ytdlp: Option<ToolInfo>,
    pub uv: Option<ToolInfo>,
    pub libass: bool,
    pub encoders: Vec<EncoderStatus>,
    pub nvidia_gpu: Option<String>,
    /// Result of engine `engine.capabilities`, when the engine is installed.
    pub engine: Option<serde_json::Value>,
    pub fonts: Vec<String>,
    pub ready: bool,
    pub missing: Vec<String>,
}

/// Encoders probed with a real 1-frame encode, best hardware first.
pub const PROBE_ENCODERS: &[&str] = &[
    "h264_nvenc",
    "hevc_nvenc",
    "h264_qsv",
    "h264_amf",
    "h264_videotoolbox",
    "libx264",
];

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

fn args(a: &[&str]) -> Vec<OsString> {
    a.iter().map(OsString::from).collect()
}

async fn run_text(cmd: &Path, a: &[&str]) -> Option<(String, String)> {
    let cancel = CancellationToken::new();
    let out = tokio::time::timeout(PROBE_TIMEOUT, run(cmd, &args(a), None, &cancel))
        .await
        .ok()?
        .ok()?;
    Some((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

#[derive(Clone, Copy)]
pub(crate) enum VersionKind {
    /// `ffmpeg -version` / `ffprobe -version`: "ffmpeg version <v> ..."
    Ffmpeg,
    /// `yt-dlp --version`: "<v>"
    Ytdlp,
    /// `uv --version`: "uv <v> (...)"
    Uv,
}

/// Version of an installed tool; `None` when it cannot be run.
pub(crate) async fn tool_version(kind: VersionKind, path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let (arg, token) = match kind {
        VersionKind::Ffmpeg => ("-version", 2),
        VersionKind::Ytdlp => ("--version", 0),
        VersionKind::Uv => ("--version", 1),
    };
    let (stdout, _) = run_text(path, &[arg]).await?;
    stdout
        .lines()
        .next()?
        .split_whitespace()
        .nth(token)
        .map(str::to_string)
}

async fn tool_info(kind: VersionKind, path: &Path) -> Option<ToolInfo> {
    tool_version(kind, path).await.map(|version| ToolInfo {
        path: path.to_path_buf(),
        version,
    })
}

async fn probe_encoder(ffmpeg: &Path, enc: &str) -> EncoderStatus {
    let a = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x256:d=0.1",
        "-frames:v",
        "1",
        "-c:v",
        enc,
        "-f",
        "null",
        "-",
    ];
    let cancel = CancellationToken::new();
    let res = tokio::time::timeout(PROBE_TIMEOUT, run(ffmpeg, &args(&a), None, &cancel)).await;
    match res {
        Ok(Ok(_)) => EncoderStatus {
            name: enc.to_string(),
            ok: true,
            error: None,
        },
        Ok(Err(e)) => EncoderStatus {
            name: enc.to_string(),
            ok: false,
            error: Some(
                e.message
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("encoder probe failed")
                    .trim()
                    .to_string(),
            ),
        },
        Err(_) => EncoderStatus {
            name: enc.to_string(),
            ok: false,
            error: Some("probe timed out".into()),
        },
    }
}

/// GPU name from `nvidia-smi`, when an NVIDIA driver is present.
pub async fn detect_nvidia() -> Option<String> {
    let (stdout, _) = run_text(
        Path::new("nvidia-smi"),
        &["--query-gpu=name", "--format=csv,noheader"],
    )
    .await?;
    stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

pub async fn doctor(paths: &ToolPaths) -> DoctorReport {
    use crate::doctor::VersionKind::{Ffmpeg, Uv, Ytdlp};
    let (ffmpeg, ffprobe, ytdlp, uv, nvidia_gpu) = tokio::join!(
        tool_info(Ffmpeg, &paths.ffmpeg),
        tool_info(Ffmpeg, &paths.ffprobe),
        tool_info(Ytdlp, &paths.ytdlp),
        tool_info(Uv, &paths.uv),
        detect_nvidia(),
    );

    let mut libass = false;
    let mut encoders = Vec::new();
    if ffmpeg.is_some() {
        if let Some((stdout, _)) = run_text(&paths.ffmpeg, &["-hide_banner", "-filters"]).await {
            libass = stdout.contains(" ass ");
        }
        encoders = futures_util::future::join_all(
            PROBE_ENCODERS
                .iter()
                .map(|e| probe_encoder(&paths.ffmpeg, e)),
        )
        .await;
    }
    let fonts = crate::fonts::installed(paths);

    let mut missing = Vec::new();
    if ffmpeg.is_none() {
        missing.push("ffmpeg".to_string());
    }
    if ffprobe.is_none() {
        missing.push("ffprobe".to_string());
    }
    if ytdlp.is_none() {
        missing.push("yt-dlp".to_string());
    }
    if uv.is_none() {
        missing.push("uv".to_string());
    }
    if ffmpeg.is_some() {
        if !libass {
            missing.push("libass".to_string());
        }
        if !encoders
            .iter()
            .any(|e| e.ok && (e.name == "libx264" || e.name.starts_with("h264_")))
        {
            missing.push("h264-encoder".to_string());
        }
    }
    if !paths.venv_python.is_file() {
        missing.push("engine".to_string());
    }
    if !crate::fonts::all_installed(paths) {
        missing.push("fonts".to_string());
    }

    DoctorReport {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        ffmpeg,
        ffprobe,
        ytdlp,
        uv,
        libass,
        encoders,
        nvidia_gpu,
        engine: None,
        fonts,
        ready: missing.is_empty(),
        missing,
    }
}
