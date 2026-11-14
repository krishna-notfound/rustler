//! uv standalone install (astral-sh/uv release archive + .sha256).

use crate::archive::extract_binaries;
use crate::doctor::{VersionKind, tool_version};
use crate::download::{download, fetch_text, find_checksum};
use crate::paths::{ToolPaths, exe_name};
use crate::setup::{Component, Platform};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::ProgressSink;

const BASE: &str = "https://github.com/astral-sh/uv/releases/latest/download";

pub(crate) fn asset_name(p: Platform) -> &'static str {
    match p {
        Platform::Windows => "uv-x86_64-pc-windows-msvc.zip",
        Platform::Linux => "uv-x86_64-unknown-linux-gnu.tar.gz",
    }
}

pub(crate) async fn install(
    paths: &ToolPaths,
    platform: Platform,
    progress: &ProgressSink,
) -> rustler_core::Result<Component> {
    let asset = asset_name(platform);
    let sum_text = fetch_text(&format!("{BASE}/{asset}.sha256")).await?;
    let expected = find_checksum(&sum_text, asset).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Provider,
            "setup.uv",
            format!("could not read the sha256 for {asset}"),
        )
        .retryable()
    })?;
    let archive = paths.home.join("dl").join(asset);
    let sha = download(
        &format!("{BASE}/{asset}"),
        &archive,
        Some(&expected),
        progress,
        "setup.uv",
    )
    .await?;
    let (a, bin) = (archive.clone(), paths.bin.clone());
    let wanted = exe_name("uv");
    tokio::task::spawn_blocking(move || extract_binaries(&a, &[wanted.as_str()], &bin))
        .await
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "setup.uv", format!("join: {e}")))??;
    let _ = std::fs::remove_file(&archive);
    let version = tool_version(VersionKind::Uv, &paths.uv)
        .await
        .ok_or_else(|| {
            RustlerError::new(
                ErrorCode::Media,
                "setup.uv",
                "installed uv does not run (`uv --version` failed)",
            )
        })?;
    Ok(Component {
        name: "uv".into(),
        version,
        path: paths.uv.clone(),
        sha256: Some(sha),
    })
}
