//! Deno release install: the JavaScript runtime yt-dlp needs to solve YouTube's challenges.
//! Downloaded from the official GitHub release into `<home>/bin` (where `run_ytdlp` puts `PATH`),
//! verified against the release's `.sha256sum` file.

use crate::archive::extract_binaries;
use crate::doctor::{VersionKind, tool_version};
use crate::download::{download, fetch_text};
use crate::paths::{ToolPaths, exe_name};
use crate::setup::{Component, Platform};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::ProgressSink;
use std::path::PathBuf;

const BASE: &str = "https://github.com/denoland/deno/releases/latest/download";

/// Release asset for an OS name (`std::env::consts::OS`); `None` where Rustler has no prebuilt tools.
pub fn asset_name_for(os: &str) -> Option<&'static str> {
    match os {
        "windows" => Some("deno-x86_64-pc-windows-msvc.zip"),
        "linux" => Some("deno-x86_64-unknown-linux-gnu.zip"),
        _ => None,
    }
}

pub(crate) fn asset_name(p: Platform) -> &'static str {
    match p {
        Platform::Windows => "deno-x86_64-pc-windows-msvc.zip",
        Platform::Linux => "deno-x86_64-unknown-linux-gnu.zip",
    }
}

/// Where `setup` puts Deno: `<home>/bin/deno[.exe]`.
pub fn deno_path(paths: &ToolPaths) -> PathBuf {
    paths.bin.join(exe_name("deno"))
}

/// The sha256 in a Deno `.sha256sum` file. Linux releases use `<hash>  <file>`; Windows releases
/// carry a PowerShell `Get-FileHash` table (`Hash      : <HASH>`). Lowercased.
pub fn parse_checksum(text: &str) -> Option<String> {
    let is_hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Hash") {
            if let Some(h) = rest.trim_start().strip_prefix(':').map(str::trim) {
                if is_hash(h) {
                    return Some(h.to_ascii_lowercase());
                }
            }
        }
        if let Some(h) = line.split_whitespace().next() {
            if is_hash(h) {
                return Some(h.to_ascii_lowercase());
            }
        }
    }
    None
}

pub(crate) async fn install(
    paths: &ToolPaths,
    platform: Platform,
    progress: &ProgressSink,
) -> rustler_core::Result<Component> {
    let asset = asset_name(platform);
    let sum_text = fetch_text(&format!("{BASE}/{asset}.sha256sum")).await?;
    let expected = parse_checksum(&sum_text).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Provider,
            "setup.deno",
            format!("could not read the sha256 for {asset}"),
        )
        .retryable()
    })?;
    let archive = paths.home.join("dl").join(asset);
    let sha = download(&format!("{BASE}/{asset}"), &archive, Some(&expected), progress, "setup.deno").await?;
    let (a, bin) = (archive.clone(), paths.bin.clone());
    let wanted = exe_name("deno");
    tokio::task::spawn_blocking(move || extract_binaries(&a, &[wanted.as_str()], &bin))
        .await
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "setup.deno", format!("join: {e}")))??;
    let _ = std::fs::remove_file(&archive);
    let path = deno_path(paths);
    let version = tool_version(VersionKind::Deno, &path).await.ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Media,
            "setup.deno",
            "installed deno does not run (`deno --version` failed)",
        )
    })?;
    Ok(Component { name: "deno".into(), version, path, sha256: Some(sha) })
}

/// [`install`] for the current platform (used by the live test and `setup`).
pub async fn install_current(paths: &ToolPaths, progress: &ProgressSink) -> rustler_core::Result<Component> {
    install(paths, Platform::current()?, progress).await
}
