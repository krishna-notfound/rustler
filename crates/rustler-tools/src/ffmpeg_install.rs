//! FFmpeg/ffprobe install from the BtbN GitHub release, verified against checksums.sha256.

use crate::archive::extract_binaries;
use crate::doctor::{VersionKind, tool_version};
use crate::download::{download, fetch_text, find_checksum};
use crate::paths::{ToolPaths, exe_name};
use crate::setup::{Component, Platform};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::ProgressSink;

const BASE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";

pub(crate) fn asset_name(p: Platform) -> &'static str {
    match p {
        Platform::Windows => "ffmpeg-master-latest-win64-gpl.zip",
        Platform::Linux => "ffmpeg-master-latest-linux64-gpl.tar.xz",
    }
}

pub(crate) async fn install(
    paths: &ToolPaths,
    platform: Platform,
    progress: &ProgressSink,
) -> rustler_core::Result<Component> {
    let asset = asset_name(platform);
    let sums = fetch_text(&format!("{BASE}/checksums.sha256")).await?;
    let expected = find_checksum(&sums, asset).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Provider,
            "setup.ffmpeg",
            format!("{asset} not listed in the release checksums.sha256"),
        )
        .retryable()
    })?;

    let archive = paths.home.join("dl").join(asset);
    let sha = download(
        &format!("{BASE}/{asset}"),
        &archive,
        Some(&expected),
        progress,
        "setup.ffmpeg",
    )
    .await?;

    let (a, bin) = (archive.clone(), paths.bin.clone());
    let wanted = [exe_name("ffmpeg"), exe_name("ffprobe")];
    tokio::task::spawn_blocking(move || {
        let w: Vec<&str> = wanted.iter().map(String::as_str).collect();
        extract_binaries(&a, &w, &bin)
    })
    .await
    .map_err(|e| RustlerError::new(ErrorCode::Internal, "setup.ffmpeg", format!("join: {e}")))??;
    let _ = std::fs::remove_file(&archive);

    let version = tool_version(VersionKind::Ffmpeg, &paths.ffmpeg)
        .await
        .ok_or_else(|| {
            RustlerError::new(
                ErrorCode::Media,
                "setup.ffmpeg",
                "installed ffmpeg does not run (`ffmpeg -version` failed)",
            )
        })?;
    Ok(Component {
        name: "ffmpeg".into(),
        version,
        path: paths.ffmpeg.clone(),
        sha256: Some(sha),
    })
}
