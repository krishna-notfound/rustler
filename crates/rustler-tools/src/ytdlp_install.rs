//! yt-dlp release binary install, verified against SHA2-256SUMS.

use crate::doctor::{VersionKind, tool_version};
use crate::download::{download, fetch_text, find_checksum};
use crate::paths::ToolPaths;
use crate::setup::{Component, Platform};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::ProgressSink;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

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

// ---------------------------------------------------------------------------------------
// Network self-fixes: JS runtime (Deno) and TLS behind an intercepting proxy
// ---------------------------------------------------------------------------------------

/// `<home>/yt-dlp.conf`, written by `setup run --insecure-tls`.
pub fn conf_path(home: &Path) -> PathBuf {
    home.join("yt-dlp.conf")
}

/// `--config-locations <home>/yt-dlp.conf` when that file exists (else nothing), so every yt-dlp
/// call Rustler makes honors the TLS setting chosen at setup.
pub fn config_args(home: &Path) -> Vec<OsString> {
    let conf = conf_path(home);
    if conf.is_file() {
        vec!["--config-locations".into(), conf.into_os_string()]
    } else {
        Vec::new()
    }
}

/// Writes `<home>/yt-dlp.conf` with `--no-check-certificates` (for networks that re-sign TLS with a
/// private CA, e.g. a corporate or cloud proxy). Returns the path.
pub fn write_insecure_tls_conf(home: &Path) -> rustler_core::Result<PathBuf> {
    let conf = conf_path(home);
    std::fs::create_dir_all(home).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, "setup.ytdlp", format!("create {}: {e}", home.display()))
    })?;
    std::fs::write(
        &conf,
        "# Written by `rustler setup run --insecure-tls`: yt-dlp does not verify TLS certificates.\n\
         # Delete this file to verify certificates again.\n\
         --no-check-certificates\n",
    )
    .map_err(|e| RustlerError::new(ErrorCode::Internal, "setup.ytdlp", format!("write {}: {e}", conf.display())))?;
    Ok(conf)
}

/// The inherited `PATH` with `bin` first, so yt-dlp finds the Deno that `setup` installed.
pub fn path_with_bin(bin: &Path) -> OsString {
    let mut dirs = vec![bin.to_path_buf()];
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| bin.as_os_str().to_owned())
}

/// The value of the `[debug] JS runtimes:` line of `yt-dlp -v` (`deno-2.9.2`, `none`, ...);
/// `None` when the line is absent (older yt-dlp).
pub fn js_runtime_name(verbose: &str) -> Option<String> {
    verbose.lines().find_map(|l| {
        let (_, v) = l.split_once("JS runtimes:")?;
        Some(v.trim().to_string())
    })
}

/// `Some(true)` when yt-dlp reports a JS runtime, `Some(false)` for `none`, `None` when not reported.
pub fn js_runtime_detected(verbose: &str) -> Option<bool> {
    js_runtime_name(verbose).map(|v| !v.is_empty() && !v.eq_ignore_ascii_case("none"))
}

/// What a failed yt-dlp run points at, and the command that fixes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeIssue {
    /// `tls` | `js_runtime`
    pub kind: &'static str,
    pub hint: &'static str,
}

const TLS_FIX: &str = "rustler setup run --insecure-tls";

/// The fix command when yt-dlp's stderr shows a certificate failure (a proxy that re-signs TLS
/// with its own CA), else `None`.
pub fn classify_tls_error_hint(stderr: &str) -> Option<&'static str> {
    let s = stderr.to_ascii_lowercase();
    [
        "certificate_verify_failed",
        "certificate verify failed",
        "self-signed certificate",
        "self signed certificate",
        "unable to get local issuer certificate",
        "sslcertverificationerror",
    ]
    .iter()
    .any(|p| s.contains(p))
    .then_some(TLS_FIX)
}

/// TLS or missing-JS-runtime trouble in yt-dlp output, with the command that fixes it.
pub fn classify_probe_failure(stderr: &str) -> Option<ProbeIssue> {
    if classify_tls_error_hint(stderr).is_some() {
        return Some(ProbeIssue {
            kind: "tls",
            hint: "this network re-signs TLS with its own CA; run `rustler setup run --insecure-tls` (yt-dlp then skips certificate checks)",
        });
    }
    let s = stderr.to_ascii_lowercase();
    ["javascript runtime", "js runtime", "challenge solving failed", "n challenge"]
        .iter()
        .any(|p| s.contains(p))
        .then_some(ProbeIssue {
            kind: "js_runtime",
            hint: "no JS runtime for YouTube's challenge; run `rustler setup run` (installs Deno next to yt-dlp)",
        })
}

/// Output of `yt-dlp -v` (stdout + stderr) with `bin` on `PATH` and the Rustler config applied;
/// `None` when yt-dlp cannot be started. `-v` with no URL exits non-zero after printing its header.
pub(crate) async fn verbose_header(ytdlp: &Path, home: &Path, bin: &Path) -> Option<String> {
    let mut cmd = tokio::process::Command::new(ytdlp);
    cmd.args(config_args(home))
        .arg("-v")
        .env("PATH", path_with_bin(bin))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
        .await
        .ok()?
        .ok()?;
    Some(format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}
