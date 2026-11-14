//! HTTP downloads (reqwest, rustls): stream to `*.part`, verify sha256, rename.

use futures_util::StreamExt;
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// sha256 of a byte slice, lowercase hex.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

/// sha256 of a file (blocking), lowercase hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Finds the sha256 for `filename` in a `sha256sum`-style listing (`<hash>  <name>` or
/// `<hash> *<name>`; names may carry a path prefix). A file holding just one bare hash
/// (the `.sha256` sidecar style) matches any `filename`.
pub fn find_checksum(text: &str, filename: &str) -> Option<String> {
    let is_hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() == 1 {
        let mut parts = lines[0].split_whitespace();
        if let (Some(h), None) = (parts.next(), parts.next()) {
            if is_hash(h) {
                return Some(h.to_ascii_lowercase());
            }
        }
    }
    for l in lines {
        let mut parts = l.splitn(2, char::is_whitespace);
        let (Some(h), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let name = name.trim().trim_start_matches('*');
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        if is_hash(h) && (name == filename || base == filename) {
            return Some(h.to_ascii_lowercase());
        }
    }
    None
}

fn client() -> rustler_core::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("rustler/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "download", format!("http client: {e}"))
        })
}

fn net_err(url: &str, e: impl std::fmt::Display) -> RustlerError {
    RustlerError::new(
        ErrorCode::Provider,
        "download",
        format!("download of {url} failed: {e}"),
    )
    .hint("check your internet connection and re-run `rustler setup`")
    .retryable()
}

fn io_err(what: &str, e: impl std::fmt::Display) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, "download", format!("{what}: {e}"))
}

/// GETs `url` as text (checksum files and the like).
pub async fn fetch_text(url: &str) -> rustler_core::Result<String> {
    let resp = client()?
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| net_err(url, e))?;
    resp.text().await.map_err(|e| net_err(url, e))
}

/// Streams `url` to `<dest>.part`, optionally verifies `expected_sha256`, then renames to
/// `dest`. If `dest` already exists and matches `expected_sha256`, nothing is downloaded.
/// Returns the sha256 of the final file. Progress is emitted under `stage`.
pub async fn download(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    progress: &ProgressSink,
    stage: &str,
) -> rustler_core::Result<String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| io_err("create download dir", e))?;
    }
    if let Some(want) = expected_sha256 {
        if dest.is_file() {
            let d = dest.to_path_buf();
            let have = tokio::task::spawn_blocking(move || sha256_file(&d))
                .await
                .ok()
                .and_then(|r| r.ok());
            if have.as_deref() == Some(want.to_ascii_lowercase().as_str()) {
                return Ok(want.to_ascii_lowercase());
            }
        }
    }

    let resp = client()?
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| net_err(url, e))?;
    let total = resp.content_length();

    let part: PathBuf = {
        let mut s = dest.as_os_str().to_owned();
        s.push(".part");
        PathBuf::from(s)
    };
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| io_err("create .part file", e))?;
    let mut hasher = Sha256::new();
    let mut got: u64 = 0;
    let mut last_pct: f32 = -1.0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| net_err(url, e))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| io_err("write download", e))?;
        got += chunk.len() as u64;
        if let Some(t) = total.filter(|t| *t > 0) {
            let pct = got as f32 / t as f32;
            if pct - last_pct >= 0.01 || pct >= 1.0 {
                last_pct = pct;
                progress.emit(ProgressEvent {
                    stage: stage.to_string(),
                    pct: Some(pct.min(1.0)),
                    message: Some(format!("downloading {}", file_label(url))),
                });
            }
        }
    }
    file.flush().await.map_err(|e| io_err("flush download", e))?;
    drop(file);
    let actual = hex(&hasher.finalize());
    if let Some(want) = expected_sha256 {
        if !actual.eq_ignore_ascii_case(want) {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(RustlerError::new(
                ErrorCode::Provider,
                "download",
                format!("checksum mismatch for {url}: expected {want}, got {actual}"),
            )
            .hint("re-run `rustler setup`; if it persists the upstream release may be mid-update")
            .retryable());
        }
    }
    tokio::fs::rename(&part, dest)
        .await
        .map_err(|e| io_err("finalize download", e))?;
    Ok(actual)
}

fn file_label(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_formats() {
        let h1 = "a".repeat(64);
        let h2 = "b".repeat(64);
        let text = format!("{h1}  ffmpeg-x.zip\n{h2} *other/yt-dlp.exe\n");
        assert_eq!(find_checksum(&text, "ffmpeg-x.zip"), Some(h1.clone()));
        assert_eq!(find_checksum(&text, "yt-dlp.exe"), Some(h2));
        assert_eq!(find_checksum(&text, "nope"), None);
        assert_eq!(find_checksum(&format!("{h1}\n"), "anything"), Some(h1.clone()));
        assert_eq!(find_checksum(&format!("{h1}  uv.zip\n"), "uv.zip"), Some(h1));
    }

    #[test]
    fn sha_of_known_bytes() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
