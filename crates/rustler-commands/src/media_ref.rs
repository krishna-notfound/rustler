//! Media references: one argument that can name a local file, an `asset:<id>`, an `http(s)` URL, or
//! the audio of an imported source (`source:<source_id>[@start-end]`).
//!
//! `audio.add_music`, `audio.add_sfx` and `asset.add` resolve their file argument through
//! [`resolve`]. Downloads and extractions land in the **assets cache** (`<project>/assets/_cache/`,
//! or `<home>/assets/_cache/` without a project) so the same URL or source range is fetched once.
//! Fetching a URL needs the project's `network` + `downloads` permissions (like `source.import`).

use crate::asset_cmds::resolve_asset_path_in;
use crate::project_cmds::{require_network, validation};
use rustler_core::{ErrorCode, RustlerError, SourceId};
use rustler_registry::Ctx;
use rustler_tools::assets::{assets_root, project_assets_root};
use rustler_tools::{Tool, ToolPaths};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// Video containers whose audio track is used when an audio file is wanted.
pub const VIDEO_EXTS: [&str; 6] = ["mp4", "mov", "mkv", "webm", "m4v", "avi"];
const KNOWN_EXTS: [&str; 19] = [
    "mp3", "wav", "ogg", "m4a", "aac", "flac", "opus", "png", "jpg", "jpeg", "webp", "gif", "ttf", "otf", "mp4", "mov",
    "mkv", "webm", "m4v",
];

/// What the caller will do with the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Want {
    /// Use the file as it is.
    Any,
    /// An audio file is needed: a video container is reduced to its audio track (m4a).
    Audio,
}

/// A resolved media reference.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub path: PathBuf,
    /// Dry run: the file would be downloaded/extracted to `path` but does not exist yet.
    pub pending: bool,
    /// `path` | `asset` | `url` | `source`
    pub kind: &'static str,
    /// Human note when something was fetched or extracted.
    pub note: Option<String>,
}

pub fn is_url(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn sha8(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// `<project>/assets/_cache` (or the global one without a project).
pub fn cache_dir(ctx: &Ctx) -> PathBuf {
    match &ctx.project_dir {
        Some(p) => project_assets_root(p).join("_cache"),
        None => assets_root(&ctx.home).join("_cache"),
    }
}

/// Extension implied by the first bytes of a file (audio, image, font and video containers).
pub fn sniff_ext(b: &[u8]) -> Option<&'static str> {
    if b.len() >= 12 && &b[0..4] == b"RIFF" {
        return match &b[8..12] {
            b"WAVE" => Some("wav"),
            b"WEBP" => Some("webp"),
            _ => None,
        };
    }
    if b.len() >= 8 && &b[4..8] == b"ftyp" {
        return Some(if b.len() >= 12 && &b[8..12] == b"M4A " { "m4a" } else { "mp4" });
    }
    if b.starts_with(b"ID3") || (b.len() >= 2 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0) {
        return Some("mp3");
    }
    if b.starts_with(b"OggS") {
        return Some("ogg");
    }
    if b.starts_with(b"fLaC") {
        return Some("flac");
    }
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("png");
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if b.starts_with(b"GIF8") {
        return Some("gif");
    }
    if b.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
        return Some("ttf");
    }
    if b.starts_with(b"OTTO") {
        return Some("otf");
    }
    if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("webm");
    }
    None
}

/// Safe file name taken from a URL's path (query and fragment dropped).
pub fn file_name_from_url(url: &str) -> String {
    let no_frag = url.split('#').next().unwrap_or(url);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let after_scheme = no_query.split_once("://").map_or(no_query, |(_, r)| r);
    let path = after_scheme.split_once('/').map_or("", |(_, p)| p);
    let last = path.rsplit('/').next().unwrap_or("");
    let clean: String = last
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    let clean = clean.trim_matches(['.', '-']).to_string();
    let clean: String = clean.chars().take(80).collect();
    if clean.is_empty() { "download".to_string() } else { clean }
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}

fn tools(stage: &str, ctx: &Ctx) -> rustler_core::Result<ToolPaths> {
    let paths = ToolPaths::from_home(&ctx.home);
    paths.require(Tool::Ffmpeg).map_err(|mut e| {
        e.stage = stage.to_string();
        e
    })?;
    Ok(paths)
}

async fn ffmpeg_extract(
    stage: &str,
    ctx: &Ctx,
    input: &Path,
    range: Option<(f64, f64)>,
    out: &Path,
) -> rustler_core::Result<()> {
    let paths = tools(stage, ctx)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| RustlerError::new(ErrorCode::Internal, stage, format!("create {}: {e}", dir.display())))?;
    }
    let mut a: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-y"].iter().map(OsString::from).collect();
    if let Some((s, e)) = range {
        a.extend(["-ss".into(), format!("{s:.3}").into(), "-to".into(), format!("{e:.3}").into()]);
    }
    a.extend(["-i".into(), input.as_os_str().to_owned(), "-vn".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into(), out.as_os_str().to_owned()]);
    let cancel: CancellationToken = ctx.cancel.clone();
    rustler_tools::run(&paths.ffmpeg, &a, None, &cancel).await.map_err(|e| {
        RustlerError::new(ErrorCode::Media, stage, format!("cannot extract audio from {}: {}", input.display(), e.message))
            .hint("the file may have no audio stream; check it with `rustler doctor` and `ffprobe`")
    })?;
    if !out.is_file() {
        return Err(RustlerError::new(ErrorCode::Media, stage, format!("ffmpeg wrote no audio for {}", input.display())));
    }
    Ok(())
}

async fn fetch_url(ctx: &Ctx, stage: &str, url: &str) -> rustler_core::Result<Resolved> {
    if let Ok(store) = ctx.store() {
        let project = store.load_project()?;
        require_network(&project, true, "downloading a media file")?;
    }
    let dir = cache_dir(ctx).join(format!("url-{}", sha8(url.trim())));
    // reuse an earlier download of the same URL
    if let Some(existing) = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file() && ext_of(p) != "part")
    {
        return Ok(Resolved { path: existing, pending: false, kind: "url", note: Some("reused the earlier download of this URL".into()) });
    }
    let name = file_name_from_url(url);
    let mut dest = dir.join(&name);
    if ctx.dry_run {
        return Ok(Resolved { path: dest, pending: true, kind: "url", note: Some(format!("dry run: would download {url}")) });
    }
    rustler_tools::download::download(url.trim(), &dest, None, &ctx.progress, "asset.download")
        .await
        .map_err(|mut e| {
            e.stage = stage.to_string();
            e
        })?;
    // No usable extension in the URL: name the file after what it actually is.
    if !KNOWN_EXTS.contains(&ext_of(&dest).as_str()) {
        let mut head = [0u8; 16];
        let n = std::fs::File::open(&dest)
            .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
            .unwrap_or(0);
        match sniff_ext(&head[..n]) {
            Some(ext) => {
                let renamed = dir.join(format!("{}.{ext}", dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name.clone())));
                std::fs::rename(&dest, &renamed)
                    .map_err(|e| RustlerError::new(ErrorCode::Internal, stage, format!("rename download: {e}")))?;
                dest = renamed;
            }
            None => {
                let _ = std::fs::remove_file(&dest);
                return Err(validation(
                    stage,
                    format!("{url} is not a recognised audio, image, font or video file"),
                    "pass a direct link to the media file (mp3/wav/m4a/ogg/flac, png/jpg/webp/gif, ttf/otf, mp4/mov/webm); page URLs are not downloaded",
                ));
            }
        }
    }
    Ok(Resolved { path: dest, pending: false, kind: "url", note: Some(format!("downloaded {url}")) })
}

/// `source:<id>[@start-end]` -> (id, range). Times are seconds or `MM:SS` / `H:MM:SS`.
pub fn parse_source_ref(stage: &str, s: &str) -> rustler_core::Result<(SourceId, Option<(f64, f64)>)> {
    let rest = s.trim().strip_prefix("source:").unwrap_or(s.trim());
    let (id, range) = match rest.split_once('@') {
        Some((i, r)) => (i, Some(r)),
        None => (rest, None),
    };
    let id: SourceId = serde_json::from_value(serde_json::json!(id.trim())).map_err(|_| {
        validation(stage, format!("`{id}` is not a source id"), "use source:<source_id>[@start-end], e.g. source:src_01k...@12-45 (see `rustler source list`)")
    })?;
    let range = match range {
        Some(r) => {
            let t = crate::source_cmds::parse_section(r).map_err(|e| {
                validation(stage, format!("bad range `{r}` in `{s}`: {}", e.message), "use source:<id>@start-end with seconds or MM:SS, e.g. source:src_01k...@12-45")
            })?;
            Some((t.start, t.end))
        }
        None => None,
    };
    Ok((id, range))
}

async fn extract_source_audio(ctx: &Ctx, stage: &str, s: &str) -> rustler_core::Result<Resolved> {
    let store = ctx.store()?;
    let (id, range) = parse_source_ref(stage, s)?;
    let source = crate::source_cmds::load_source_checked(&store, &id)?;
    if !source.probe.has_audio {
        return Err(validation(stage, format!("source {id} has no audio track"), "pick another source (`rustler source list`)"));
    }
    if let Some((start, _)) = range {
        if source.probe.duration > 0.0 && start >= source.probe.duration {
            return Err(validation(
                stage,
                format!("range starts at {start:.1}s but source {id} is only {:.1}s long", source.probe.duration),
                "choose a range inside the source",
            ));
        }
    }
    let range = range.map(|(a, b)| (a, if source.probe.duration > 0.0 { b.min(source.probe.duration) } else { b }));
    let tag = match range {
        Some((a, b)) => format!("{}-{}", (a * 1000.0).round() as u64, (b * 1000.0).round() as u64),
        None => "all".to_string(),
    };
    let out = cache_dir(ctx).join(format!("source-{id}-{tag}.m4a"));
    let label = match range {
        Some((a, b)) => format!("audio of {id} {a:.1}s-{b:.1}s"),
        None => format!("audio of {id}"),
    };
    if out.is_file() {
        return Ok(Resolved { path: out, pending: false, kind: "source", note: Some(format!("reused the earlier extraction ({label})")) });
    }
    if ctx.dry_run {
        return Ok(Resolved { path: out, pending: true, kind: "source", note: Some(format!("dry run: would extract the {label}")) });
    }
    ffmpeg_extract(stage, ctx, &store.resolve(&source.media_path), range, &out).await?;
    Ok(Resolved { path: out, pending: false, kind: "source", note: Some(format!("extracted the {label}")) })
}

/// Resolves a media reference to a local file (see the module docs). Plain paths are returned as given
/// (their existence is the caller's check).
pub async fn resolve(ctx: &Ctx, stage: &str, s: &str, want: Want) -> rustler_core::Result<Resolved> {
    let s = s.trim();
    let mut r = if is_url(s) {
        fetch_url(ctx, stage, s).await?
    } else if s.starts_with("source:") {
        extract_source_audio(ctx, stage, s).await?
    } else if s.starts_with("asset:") {
        Resolved { path: resolve_asset_path_in(ctx.project_dir.as_deref(), &ctx.home, s)?, pending: false, kind: "asset", note: None }
    } else {
        Resolved { path: PathBuf::from(s), pending: false, kind: "path", note: None }
    };
    // A video container where audio is wanted: use its audio track.
    if want == Want::Audio && !r.pending && r.kind != "source" && VIDEO_EXTS.contains(&ext_of(&r.path).as_str()) && r.path.is_file() {
        let stem = r.path.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
        let tag = sha8(&r.path.to_string_lossy());
        let out = cache_dir(ctx).join(format!("audio-{stem}-{tag}.m4a"));
        if !out.is_file() {
            ffmpeg_extract(stage, ctx, &r.path, None, &out).await?;
        }
        r.note = Some(format!("used the audio track of {}", r.path.display()));
        r.path = out;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_names() {
        assert!(is_url(" HTTPS://x.org/a.mp3"));
        assert!(!is_url("C:/a/b.mp3") && !is_url("asset:x") && !is_url("source:src_x"));
        assert_eq!(file_name_from_url("https://cdn.example.com/music/Chill%20Beat.mp3?token=abc#t=3"), "Chill-20Beat.mp3");
        assert_eq!(file_name_from_url("https://example.com/"), "download");
        assert_eq!(file_name_from_url("https://example.com"), "download");
    }

    #[test]
    fn sniffing_magic_bytes() {
        assert_eq!(sniff_ext(b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(sniff_ext(b"ID3\x04\0\0\0\0\0\0\0\0"), Some("mp3"));
        assert_eq!(sniff_ext(&[0xFF, 0xFB, 0x90, 0x00]), Some("mp3"));
        assert_eq!(sniff_ext(b"OggS\0\x02"), Some("ogg"));
        assert_eq!(sniff_ext(b"\0\0\0\x20ftypM4A \0"), Some("m4a"));
        assert_eq!(sniff_ext(b"\0\0\0\x20ftypisom\0"), Some("mp4"));
        assert_eq!(sniff_ext(&[0x89, b'P', b'N', b'G', 0, 0]), Some("png"));
        assert_eq!(sniff_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff_ext(b"<html>"), None);
    }

    #[test]
    fn source_refs() {
        let id = SourceId::new().to_string();
        let (got, r) = parse_source_ref("t", &format!("source:{id}@12-45.5")).unwrap();
        assert_eq!(got.to_string(), id);
        assert_eq!(r, Some((12.0, 45.5)));
        let (_, r) = parse_source_ref("t", &format!("source:{id}@1:00-1:30")).unwrap();
        assert_eq!(r, Some((60.0, 90.0)));
        assert_eq!(parse_source_ref("t", &format!("source:{id}")).unwrap().1, None);
        assert_eq!(parse_source_ref("t", &format!("source:{id}@9-3")).unwrap_err().code, ErrorCode::Validation);
    }
}
