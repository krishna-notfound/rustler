//! yt-dlp release binary install, verified against SHA2-256SUMS.

use crate::doctor::{VersionKind, tool_version};
use crate::download::{download, fetch_text, find_checksum};
use crate::paths::ToolPaths;
use crate::setup::{Component, Platform};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::ProgressSink;

const BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

pub(crate) fn asset_name(p: Platform) -> &'static str {
    match p {
        Platform::Windows => "yt-dlp.exe",
        Platform::Linux => "yt-dlp_linux",
    }
}

pub(crate) async fn install(
    paths: &ToolPaths,
    platform: Platform,
    progress: &ProgressSink,
) -> rustler_core::Result<Component> {
    let asset = asset_name(platform);
    let sums = fetch_text(&format!("{BASE}/SHA2-256SUMS")).await?;
    let expected = find_checksum(&sums, asset).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Provider,
            "setup.ytdlp",
            format!("{asset} not listed in the release SHA2-256SUMS"),
        )
        .retryable()
    })?;
    std::fs::create_dir_all(&paths.bin).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, "setup.ytdlp", format!("create bin dir: {e}"))
    })?;
    let sha = download(
        &format!("{BASE}/{asset}"),
        &paths.ytdlp,
        Some(&expected),
        progress,
        "setup.ytdlp",
    )
    .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&paths.ytdlp, std::fs::Permissions::from_mode(0o755));
    }
    let version = tool_version(VersionKind::Ytdlp, &paths.ytdlp)
        .await
        .ok_or_else(|| {
            RustlerError::new(
                ErrorCode::Media,
                "setup.ytdlp",
                "installed yt-dlp does not run (`yt-dlp --version` failed)",
            )
        })?;
    Ok(Component {
        name: "yt-dlp".into(),
        version,
        path: paths.ytdlp.clone(),
        sha256: Some(sha),
    })
}
