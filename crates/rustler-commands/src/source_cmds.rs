//! Commands: source.import, source.info, source.fetch_transcript, source.search, source.list, source.show (Task 10a).
//!
//! Online sources are scanned before anything is downloaded: `source.info` (chapters, "most
//! replayed" heatmap, caption languages) and `source.fetch_transcript` (YouTube captions as
//! approximate-timing words) use yt-dlp with `--skip-download`; `source.import --sections`
//! then downloads only the chosen windows.

use crate::analyze_cmds::{has_devanagari, romanize, spawn_engine};
use crate::project_cmds::{Outcome, log_event, mmss, one_or_many, path_allowed, require_network, update_project, validation};
use crate::transcript_cmds::{save_transcript, units_preview};
use rustler_core::{
    CaptionScript, ErrorCode, MediaProbe, Project, ProjectStore, RustlerError, Source, SourceId, SourceOrigin,
    TimeRange, Transcript, Word, WordTiming,
};
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use rustler_tools::{Tool, ToolPaths};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------
// pure helpers (unit-tested)
// ---------------------------------------------------------------------------------------

/// `"SS"`, `"MM:SS"` or `"H:MM:SS"` (seconds may be fractional) to seconds.
pub fn parse_clock(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut total = 0.0;
    for (i, p) in parts.iter().enumerate() {
        let v: f64 = p.trim().parse().ok()?;
        if v < 0.0 || !v.is_finite() {
            return None;
        }
        // Only the last field may be fractional / exceed 59.
        if i + 1 < parts.len() && (v.fract() != 0.0 || (i > 0 && v >= 60.0)) {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

/// `"MM:SS-MM:SS"` or `"H:MM:SS-H:MM:SS"` to a range.
pub fn parse_section(s: &str) -> rustler_core::Result<TimeRange> {
    let bad = |why: &str| {
        validation(
            "source.import",
            format!("bad section `{s}`: {why}"),
            "use MM:SS-MM:SS or H:MM:SS-H:MM:SS, e.g. --sections 47:00-49:30",
        )
    };
    let (a, b) = s.split_once('-').ok_or_else(|| bad("missing `-` between start and end"))?;
    let start = parse_clock(a).ok_or_else(|| bad("start is not a valid time"))?;
    let end = parse_clock(b).ok_or_else(|| bad("end is not a valid time"))?;
    if end <= start {
        return Err(bad("end must be after start"));
    }
    Ok(TimeRange { start, end })
}

/// `H:MM:SS` / `H:MM:SS.mmm` for yt-dlp.
fn clock_arg(t: f64) -> String {
    let ms = (t * 1000.0).round() as u64;
    let (s, ms) = (ms / 1000, ms % 1000);
    let base = format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60);
    if ms == 0 { base } else { format!("{base}.{ms:03}") }
}

/// Percentage from a yt-dlp `[download]  45.2% of ...` line, as 0..1.
pub fn parse_ytdlp_progress(line: &str) -> Option<f32> {
    let rest = line.strip_prefix("[download]")?;
    let tok = rest.split_whitespace().find(|t| t.ends_with('%'))?;
    let v: f32 = tok.trim_end_matches('%').parse().ok()?;
    Some((v / 100.0).clamp(0.0, 1.0))
}

/// Parses YouTube `json3` captions into words. A word starts at event `tStartMs` + seg
/// `tOffsetMs` and ends at the next word (or the event end); a seg with several tokens
/// (manual captions) is split in proportion to token length.
pub fn parse_json3(s: &str) -> rustler_core::Result<Vec<Word>> {
    let v: Value = serde_json::from_str(s).map_err(|e| {
        RustlerError::new(ErrorCode::Provider, "source.fetch_transcript", format!("caption file is not valid json3: {e}"))
    })?;
    let events = v["events"].as_array().cloned().unwrap_or_default();
    let mut raw: Vec<(String, f64, f64)> = Vec::new();
    for ev in &events {
        let Some(segs) = ev["segs"].as_array() else { continue };
        let t0 = ev["tStartMs"].as_f64().unwrap_or(0.0);
        let dur = ev["dDurationMs"].as_f64().unwrap_or(0.0);
        let ev_end = (t0 + dur) / 1000.0;
        let items: Vec<(String, f64)> = segs
            .iter()
            .filter_map(|sg| {
                let text = sg["utf8"].as_str()?.trim().to_string();
                (!text.is_empty()).then(|| (text, (t0 + sg["tOffsetMs"].as_f64().unwrap_or(0.0)) / 1000.0))
            })
            .collect();
        for (k, (text, start)) in items.iter().enumerate() {
            let end = items.get(k + 1).map(|n| n.1).unwrap_or(ev_end).max(*start);
            let toks: Vec<&str> = text.split_whitespace().collect();
            let total: usize = toks.iter().map(|t| t.chars().count().max(1)).sum();
            let mut at = *start;
            for t in &toks {
                let share = (end - start) * (t.chars().count().max(1) as f64 / total as f64);
                raw.push((t.to_string(), at, (at + share).min(end.max(at))));
                at += share;
            }
        }
    }
    // Words must not overlap: cut each word at the next word's start.
    for i in 0..raw.len().saturating_sub(1) {
        let next_start = raw[i + 1].1;
        if next_start < raw[i].2 {
            raw[i].2 = next_start.max(raw[i].1);
        }
    }
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, (text, start, end))| Word { i: i as u32, text, start, end: end.max(start), conf: None, speaker: None })
        .collect())
}

/// Picks a caption track. `requested` is `"auto"` or a language code. Returns the yt-dlp
/// language key and whether it is a manual (uploaded) track. Manual beats auto; `en`, then `hi`.
pub fn choose_caption(manual: &[String], auto: &[String], requested: &str) -> Option<(String, bool)> {
    let base = |l: &str| l.trim_end_matches("-orig").split(['-', '_']).next().unwrap_or("").to_string();
    let find = |list: &[String], lang: &str, orig_first: bool| -> Option<String> {
        let exact = list.iter().find(|l| l.as_str() == lang);
        let orig = list.iter().find(|l| l.as_str() == format!("{lang}-orig"));
        let prefix = list.iter().find(|l| base(l) == lang);
        let order = if orig_first { [orig, exact, prefix] } else { [exact, orig, prefix] };
        order.into_iter().flatten().next().cloned()
    };
    if requested != "auto" {
        let r = requested.to_ascii_lowercase();
        return find(manual, &r, false).map(|l| (l, true)).or_else(|| find(auto, &r, true).map(|l| (l, false)));
    }
    for lang in ["en", "hi"] {
        if let Some(l) = find(manual, lang, false) {
            return Some((l, true));
        }
    }
    if let Some(l) = manual.first() {
        return Some((l.clone(), true));
    }
    for lang in ["en", "hi"] {
        if let Some(l) = find(auto, lang, true) {
            return Some((l, false));
        }
    }
    auto.iter().find(|l| l.ends_with("-orig")).or(auto.first()).map(|l| (l.clone(), false))
}

/// Short, actionable hint for a yt-dlp failure.
pub fn classify_ytdlp_error(stderr: &str) -> (&'static str, bool) {
    let s = stderr.to_ascii_lowercase();
    if s.contains("sign in to confirm") || s.contains("not a bot") || s.contains("http error 429") || s.contains("too many requests") {
        ("the site is blocking this machine's IP; import a local file instead, or set your own cookies with `rustler project set-permissions --cookies-file <path>`", true)
    } else if s.contains("private video") || s.contains("members-only") || s.contains("sign in") {
        ("the video needs a login; use a local file or provide your own cookies file via `rustler project set-permissions --cookies-file`", false)
    } else if s.contains("video unavailable") || s.contains("not available") || s.contains("removed") {
        ("the video is unavailable (removed, private or region-locked); check the URL", false)
    } else if s.contains("unable to download") || s.contains("timed out") || s.contains("temporary failure") || s.contains("connection") {
        ("network problem; check connectivity and retry", true)
    } else {
        ("run `rustler doctor`; if yt-dlp is outdated, `rustler setup --force` updates it", false)
    }
}

pub(crate) fn sha8(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// Pending-transcript id for a URL: `url_<sha8>`.
pub fn pending_id(url: &str) -> String {
    format!("url_{}", sha8(url.trim()))
}

fn is_url(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

// ---------------------------------------------------------------------------------------
// yt-dlp runner
// ---------------------------------------------------------------------------------------

struct YtOut {
    stdout: String,
}

fn cookie_args(project: &Project) -> Vec<OsString> {
    match &project.permissions.cookies_file {
        // The path is passed as argv only; it is never logged or echoed in errors.
        Some(p) => vec!["--cookies".into(), p.as_os_str().to_owned()],
        None => Vec::new(),
    }
}

/// Runs yt-dlp (argv, no shell). Stdout lines go to `on_line`; stdout is also collected when
/// `collect` is set. A failing exit becomes a `Provider` error with a hint.
async fn run_ytdlp(
    ytdlp: &Path,
    args: &[OsString],
    cancel: &CancellationToken,
    collect: bool,
    mut on_line: impl FnMut(&str),
) -> rustler_core::Result<YtOut> {
    let mut cmd = tokio::process::Command::new(ytdlp);
    // Persistent cache (player JS / challenge solutions) next to the tools, so repeat calls are cheaper.
    if let Some(home) = ytdlp.parent().and_then(Path::parent) {
        cmd.arg("--cache-dir").arg(home.join("cache").join("yt-dlp"));
    }
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| {
        RustlerError::new(ErrorCode::DependencyMissing, "yt-dlp", format!("cannot start yt-dlp at {}: {e}", ytdlp.display()))
            .hint("run `rustler setup`")
    })?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let err_task = tokio::spawn(async move {
        let mut tail: std::collections::VecDeque<String> = Default::default();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if tail.len() >= 20 {
                tail.pop_front();
            }
            tail.push_back(l);
        }
        tail.into_iter().collect::<Vec<_>>().join("\n")
    });
    let mut collected = String::new();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(RustlerError::new(ErrorCode::Cancelled, "yt-dlp", "cancelled"));
            }
            l = lines.next_line() => match l {
                Ok(Some(l)) => {
                    on_line(&l);
                    if collect { collected.push_str(&l); collected.push('\n'); }
                }
                _ => break,
            }
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "yt-dlp", format!("waiting for yt-dlp: {e}")))?;
    let err_tail = err_task.await.unwrap_or_default();
    if !status.success() {
        let (hint, retryable) = classify_ytdlp_error(&err_tail);
        let mut e = RustlerError::new(ErrorCode::Provider, "yt-dlp", format!("yt-dlp failed: {}", trunc(err_tail.trim(), 800))).hint(hint);
        if retryable {
            e = e.retryable();
        }
        return Err(e);
    }
    Ok(YtOut { stdout: collected })
}

async fn ytdlp_json(ctx: &Ctx, paths: &ToolPaths, project: &Project, args: Vec<OsString>) -> rustler_core::Result<Value> {
    let ytdlp = paths.require(Tool::Ytdlp)?;
    let mut a: Vec<OsString> = vec!["-J".into(), "--skip-download".into(), "--no-playlist".into(), "--no-colors".into()];
    a.extend(cookie_args(project));
    a.extend(args);
    let out = run_ytdlp(&ytdlp, &a, &ctx.cancel, true, |_| {}).await?;
    serde_json::from_str(out.stdout.trim()).map_err(|e| {
        RustlerError::new(ErrorCode::Provider, "yt-dlp", format!("yt-dlp returned unreadable JSON: {e}"))
            .hint("update yt-dlp with `rustler setup --force`")
    })
}

/// How long a saved yt-dlp info file is reused. YouTube's signed media URLs inside it stay valid
/// for roughly 6 hours, so downloads with `--load-info-json` work within this window.
const INFO_CACHE_SECS: u64 = 4 * 3600;

/// `<home>/cache/ytinfo/<hash>.json` — the full `yt-dlp -J` output for a URL.
fn info_cache_path(home: &Path, url: &str) -> PathBuf {
    home.join("cache").join("ytinfo").join(format!("{}.json", sha8(url.trim())))
}

/// Full yt-dlp info for `url`, asked from YouTube at most once per [`INFO_CACHE_SECS`].
/// YouTube makes every extraction solve a JavaScript challenge (minutes on some machines), so
/// `source info`, `fetch-transcript` and `import --sections` all share this one lookup and later
/// yt-dlp runs use `--load-info-json` instead of contacting YouTube again.
async fn cached_info(ctx: &Ctx, paths: &ToolPaths, project: &Project, url: &str, refresh: bool) -> rustler_core::Result<(Value, PathBuf)> {
    let path = info_cache_path(&ctx.home, url);
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < INFO_CACHE_SECS);
    if fresh && !refresh {
        if let Some(v) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            return Ok((v, path));
        }
    }
    ctx.progress.emit(ProgressEvent { stage: "lookup".into(), pct: None, message: Some("asking YouTube for video info (one time; cached)".into()), eta_s: None });
    let v = ytdlp_json(ctx, paths, project, vec![url.trim().into()]).await?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.part");
    if std::fs::write(&tmp, serde_json::to_vec(&v).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    Ok((v, path))
}

// ---------------------------------------------------------------------------------------
// source.info
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct InfoIn {
    /// Video URL to look up (needs network). Give this or `source_id`.
    pub url: Option<String>,
    /// An imported source: reads the saved `analysis/<id>/source_info.json` instead (no network).
    pub source_id: Option<SourceId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Chapter {
    pub start: f64,
    pub end: f64,
    pub title: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HeatWindow {
    pub start: f64,
    pub end: f64,
    pub value: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HeatPeak {
    pub start: f64,
    pub end: f64,
    pub value: f64,
    /// Ready for `--sections`: `MM:SS-MM:SS`.
    pub range: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct Subtitles {
    pub manual: Vec<String>,
    /// Auto-generated tracks (original-language ones first; machine translations are not listed).
    pub auto: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct InfoOut {
    pub url: String,
    pub title: String,
    pub duration: f64,
    pub uploader: Option<String>,
    pub upload_date: Option<String>,
    pub view_count: Option<u64>,
    pub description: Option<String>,
    pub chapters: Vec<Chapter>,
    /// "Most replayed" windows in time order (empty when the video has none).
    pub heatmap: Vec<HeatWindow>,
    /// Top 8 heatmap windows by value, highest first.
    pub heatmap_peaks: Vec<HeatPeak>,
    pub subtitles: Subtitles,
    pub next: Vec<String>,
}

/// Builds [`InfoOut`] from yt-dlp's `-J` JSON.
pub fn parse_info(v: &Value, url: &str) -> InfoOut {
    let f = |x: &Value| x.as_f64().unwrap_or(0.0);
    let chapters: Vec<Chapter> = v["chapters"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| Chapter { start: f(&c["start_time"]), end: f(&c["end_time"]), title: c["title"].as_str().unwrap_or("").to_string() })
                .collect()
        })
        .unwrap_or_default();
    let heatmap: Vec<HeatWindow> = v["heatmap"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|h| HeatWindow {
                    start: round1(f(&h["start_time"])),
                    end: round1(f(&h["end_time"])),
                    value: (f(&h["value"]) * 1000.0).round() / 1000.0,
                })
                .collect()
        })
        .unwrap_or_default();
    let heatmap_peaks = top_peaks(&heatmap);
    let keys = |k: &str| -> Vec<String> {
        v[k].as_object()
            .map(|o| o.keys().filter(|l| *l != "live_chat").cloned().collect())
            .unwrap_or_default()
    };
    let manual = keys("subtitles");
    let all_auto = keys("automatic_captions");
    let mut auto: Vec<String> = all_auto.iter().filter(|l| l.ends_with("-orig")).cloned().collect();
    for l in ["en", "hi"] {
        if all_auto.iter().any(|a| a == l) && !auto.iter().any(|a| a == l) {
            auto.push(l.to_string());
        }
    }
    if auto.is_empty() {
        auto = all_auto.iter().take(10).cloned().collect();
    }
    let mut next = Vec::new();
    if !manual.is_empty() || !auto.is_empty() {
        next.push(format!("rustler source fetch-transcript {url}   (captions, no download)"));
    } else {
        next.push("no captions: download the audio window with `source import --sections`, then `analyze transcribe`".to_string());
    }
    if let Some(p) = heatmap_peaks.first() {
        next.push(format!("rustler source import {url} --sections {}   (top replayed window)", p.range));
    } else if let Some(c) = chapters.first() {
        next.push(format!("rustler source import {url} --sections {}-{}   (first chapter: {})", mmss(c.start), mmss(c.end), c.title));
    } else {
        next.push(format!("rustler source import {url} --sections MM:SS-MM:SS   (pick the window from the transcript)"));
    }
    InfoOut {
        url: v["webpage_url"].as_str().unwrap_or(url).to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
        duration: f(&v["duration"]),
        uploader: v["uploader"].as_str().or(v["channel"].as_str()).map(String::from),
        upload_date: v["upload_date"].as_str().map(String::from),
        view_count: v["view_count"].as_u64(),
        description: v["description"].as_str().map(|d| trunc(d, 400)),
        chapters,
        heatmap,
        heatmap_peaks,
        subtitles: Subtitles { manual, auto },
        next,
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Top 8 heatmap windows by value, highest first.
fn top_peaks(heatmap: &[HeatWindow]) -> Vec<HeatPeak> {
    let mut ranked = heatmap.to_vec();
    ranked.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .take(8)
        .map(|h| HeatPeak { range: format!("{}-{}", mmss(h.start), mmss(h.end)), start: h.start, end: h.end, value: h.value })
        .collect()
}

/// Keeps the heatmap windows that overlap `[from, to]` (clipped to it) and moves them by `-shift`
/// seconds. `shift_heatmap(h, s, e, s)` gives section-local times (0 = the section start);
/// `shift = 0` keeps original-video times but drops everything outside the section.
pub fn shift_heatmap(heat: &[HeatWindow], from: f64, to: f64, shift: f64) -> Vec<HeatWindow> {
    heat.iter()
        .filter(|h| h.end > from && h.start < to)
        .map(|h| HeatWindow { start: round1(h.start.max(from) - shift), end: round1(h.end.min(to) - shift), value: h.value })
        .collect()
}

/// What `analysis/<src>/source_info.json` holds: the [`InfoOut`] of the video (title, duration, chapters,
/// uploader, webpage_url, heatmap, peaks, subtitles) plus section bookkeeping.
///
/// `heatmap` stays in *original-video* time (restricted to the imported section), because
/// `highlights.signals` adds the source's `original_offset` itself when it reads it; `heatmap_local`
/// is the same data shifted by `-original_offset` (0 = the start of the imported media).
pub fn build_source_info(v: &Value, url: &str, section: Option<TimeRange>) -> Value {
    let mut info = parse_info(v, url);
    let offset = section.map_or(0.0, |s| s.start);
    let mut local = info.heatmap.clone();
    if let Some(s) = section {
        info.heatmap = shift_heatmap(&info.heatmap, s.start, s.end, 0.0);
        local = shift_heatmap(&local, s.start, s.end, s.start);
        info.heatmap_peaks = top_peaks(&info.heatmap);
    }
    info.next = vec![
        "rustler highlights signals <source-id>   (uses the saved heatmap)".to_string(),
        "rustler highlights candidates <source-id>".to_string(),
    ];
    let mut out = serde_json::to_value(&info).unwrap_or(Value::Null);
    if let Some(o) = out.as_object_mut() {
        o.insert("original_offset".into(), json!(offset));
        o.insert("section".into(), json!(section));
        o.insert("heatmap_local".into(), json!(local));
    }
    out
}

pub async fn info(ctx: Ctx, input: InfoIn) -> rustler_core::Result<InfoOut> {
    let store = ctx.store()?;
    if let Some(id) = &input.source_id {
        let source = load_source_checked(&store, id)?;
        let rel = Path::new("analysis").join(id.as_str()).join("source_info.json");
        if !store.resolve(&rel).is_file() {
            let hint = match &source.origin {
                SourceOrigin::Url { url, .. } => format!("look it up online with `rustler source info {url}`; sources imported before this feature have no saved info"),
                SourceOrigin::File { .. } => "only sources imported from a URL have saved video info; this one came from a local file".to_string(),
            };
            return Err(RustlerError::new(ErrorCode::NotFound, "source.info", format!("no saved video info for source {id}")).hint(hint));
        }
        return store.read_json(&rel);
    }
    let Some(url) = input.url.as_deref() else {
        return Err(validation("source.info", "pass a url or a source_id", "`rustler source info <url>` or `rustler source info --source-id <id>`"));
    };
    let project = store.load_project()?;
    require_url(url, "source.info")?;
    require_network(&project, false, "looking up an online video")?;
    let paths = ToolPaths::from_home(&ctx.home);
    let (v, _) = cached_info(&ctx, &paths, &project, url, false).await?;
    Ok(parse_info(&v, url.trim()))
}

fn require_url(url: &str, stage: &str) -> rustler_core::Result<()> {
    if is_url(url) {
        Ok(())
    } else {
        Err(validation(stage, format!("`{url}` is not an http(s) URL"), "pass the full video URL, e.g. https://www.youtube.com/watch?v=..."))
    }
}

// ---------------------------------------------------------------------------------------
// source.search
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SearchIn {
    pub query: String,
    /// Default 10.
    pub limit: Option<u32>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub duration: Option<f64>,
    pub channel: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SearchOut {
    pub results: Vec<SearchResult>,
    pub next: Vec<String>,
}

pub async fn search(ctx: Ctx, input: SearchIn) -> rustler_core::Result<SearchOut> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    if input.query.trim().is_empty() {
        return Err(validation("source.search", "query is empty", "pass search words: `rustler source search \"podcast clips\"`"));
    }
    require_network(&project, false, "searching online")?;
    let limit = input.limit.unwrap_or(10).clamp(1, 50);
    let paths = ToolPaths::from_home(&ctx.home);
    let v = ytdlp_json(
        &ctx,
        &paths,
        &project,
        vec!["--flat-playlist".into(), "--no-warnings".into(), format!("ytsearch{limit}:{}", input.query.trim()).into()],
    )
    .await?;
    let results: Vec<SearchResult> = v["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let url = e["url"]
                        .as_str()
                        .filter(|u| is_url(u))
                        .map(String::from)
                        .or_else(|| e["webpage_url"].as_str().map(String::from))
                        .or_else(|| e["id"].as_str().map(|id| format!("https://www.youtube.com/watch?v={id}")))?;
                    Some(SearchResult {
                        url,
                        title: e["title"].as_str().unwrap_or("").to_string(),
                        duration: e["duration"].as_f64(),
                        channel: e["channel"].as_str().or(e["uploader"].as_str()).map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let next = match results.first() {
        Some(r) => vec![format!("rustler source info {}", r.url)],
        None => vec!["no results: try other words".to_string()],
    };
    Ok(SearchOut { results, next })
}

// ---------------------------------------------------------------------------------------
// source.fetch_transcript
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct FetchIn {
    pub url: String,
    /// `auto` (manual captions first, `en` then `hi`, then auto-generated) or a language code.
    pub lang: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct FetchOut {
    /// Use as `source_id` in `transcript get|search` until the video is imported.
    pub pending_id: String,
    pub transcript_path: PathBuf,
    pub units_path: PathBuf,
    pub language: String,
    /// `manual` (uploaded) or `auto` (YouTube speech recognition).
    pub caption_kind: String,
    /// Always `approximate` for captions: whisper re-transcribes imported sections for exact timing.
    pub word_timing: String,
    pub word_count: usize,
    pub unit_count: usize,
    pub text_preview: String,
    /// First 30 units as `#id [mm:ss] text`.
    pub units_preview: Vec<String>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

pub async fn fetch_transcript(ctx: Ctx, input: FetchIn) -> rustler_core::Result<Outcome<FetchOut>> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let url = input.url.trim().to_string();
    require_url(&url, "source.fetch_transcript")?;
    require_network(&project, false, "fetching captions")?;
    let requested = input.lang.clone().unwrap_or_else(|| "auto".to_string());
    let id = pending_id(&url);
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"fetch_captions_for": url, "lang": requested, "pending_id": id})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    ctx.progress.emit(ProgressEvent { stage: "captions".into(), pct: Some(0.0), message: Some("listing caption tracks".into()), eta_s: None });
    let (v, info_path) = cached_info(&ctx, &paths, &project, &url, false).await?;
    let meta = parse_info(&v, &url);
    let Some((key, manual)) = choose_caption(&meta.subtitles.manual, &meta.subtitles.auto, &requested) else {
        let avail = [meta.subtitles.manual.clone(), meta.subtitles.auto.clone()].concat();
        return Err(validation(
            "source.fetch_transcript",
            if avail.is_empty() {
                "this video has no captions".to_string()
            } else {
                format!("no captions for language `{requested}`; available: {}", avail.join(", "))
            },
            format!("retry with --lang <one of the available>, or `rustler source import {url} --sections MM:SS-MM:SS` then `rustler analyze transcribe` (whisper)"),
        ));
    };

    let dir_rel = Path::new("analysis").join(&id);
    let dir = store.resolve(&dir_rel);
    std::fs::create_dir_all(&dir)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "source.fetch_transcript", format!("cannot create {}: {e}", dir.display())))?;
    for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if f.file_name().to_string_lossy().starts_with("captions.") {
            let _ = std::fs::remove_file(f.path());
        }
    }
    let ytdlp = paths.require(Tool::Ytdlp)?;
    let mut a: Vec<OsString> = ["--skip-download", "--write-subs", "--write-auto-subs", "--sub-format", "json3", "--no-playlist", "--no-colors", "--newline", "--sub-langs"]
        .into_iter()
        .map(OsString::from)
        .collect();
    a.push(key.clone().into());
    a.push("-o".into());
    a.push(dir.join("captions.%(ext)s").into_os_string());
    a.extend(cookie_args(&project));
    // Reuse the saved info: no second YouTube extraction (and no second JS challenge).
    let mut cached = a.clone();
    cached.push("--load-info-json".into());
    cached.push(info_path.clone().into_os_string());
    if run_ytdlp(&ytdlp, &cached, &ctx.cancel, false, |_| {}).await.is_err() {
        a.push(url.clone().into());
        run_ytdlp(&ytdlp, &a, &ctx.cancel, false, |_| {}).await?;
    }

    let file = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "json3") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("captions.")))
        .ok_or_else(|| {
            RustlerError::new(ErrorCode::Provider, "source.fetch_transcript", format!("yt-dlp wrote no json3 captions for `{key}`"))
                .hint("try --lang with another track, or transcribe with whisper after `source import`")
        })?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "source.fetch_transcript", format!("cannot read {}: {e}", file.display())))?;
    let mut words = parse_json3(&text)?;
    let mut warnings = Vec::new();
    let language = key.trim_end_matches("-orig").to_string();
    let mut script = CaptionScript::Latin;
    if has_devanagari(words.iter().map(|w| w.text.as_str())) {
        if project.settings.caption_script == CaptionScript::Latin {
            let engine = spawn_engine(&paths).await?;
            let texts: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
            let rom = romanize(&engine, &ctx, texts).await;
            engine.shutdown().await;
            let rom = rom?;
            for (w, t) in words.iter_mut().zip(rom) {
                w.text = t;
            }
        } else {
            script = CaptionScript::Devanagari;
        }
    }
    if words.is_empty() {
        warnings.push("the caption track contains no words".to_string());
    }
    let kind = if manual { "manual" } else { "auto" };
    let transcript = Transcript {
        schema: "rustler.transcript@1".to_string(),
        source_id: serde_json::from_value(json!(id)).map_err(|e| RustlerError::new(ErrorCode::Internal, "source.fetch_transcript", e.to_string()))?,
        language: language.clone(),
        engine: format!("youtube:{kind}"),
        script,
        words,
        word_timing: WordTiming::Approximate,
    };
    let units = save_transcript(&store, &id, &transcript)?;
    log_event(&ctx, &store, "source.transcript_fetched", json!({"url": url, "pending_id": id, "lang": key, "kind": kind}))?;
    let preview = transcript.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    Ok(Outcome::Done(FetchOut {
        transcript_path: dir_rel.join("transcript.json"),
        units_path: dir_rel.join("units.json"),
        word_count: transcript.words.len(),
        unit_count: units.len(),
        text_preview: trunc(&preview, 400),
        units_preview: units_preview(&units, 30),
        next: vec![
            format!("rustler transcript get {id}   (read units) / rustler transcript search {id} \"phrase\""),
            format!("then download only what you need: rustler source import {url} --sections MM:SS-MM:SS"),
            "caption timing is approximate; whisper re-times imported sections (`rustler analyze transcribe`)".to_string(),
        ],
        pending_id: id,
        language,
        caption_kind: kind.to_string(),
        word_timing: "approximate".to_string(),
        warnings,
    }))
}

// ---------------------------------------------------------------------------------------
// source.import
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct ImportIn {
    /// Local file path or http(s) URL.
    pub input: String,
    /// URL only: download only these windows (`MM:SS-MM:SS` or `H:MM:SS-H:MM:SS`); one source per window.
    #[serde(default, deserialize_with = "one_or_many")]
    #[schemars(with = "Option<Vec<String>>")]
    pub sections: Option<Vec<String>>,
    /// Source name. Default: file stem / video title.
    pub name: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ImportOut {
    pub sources: Vec<Source>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

pub async fn import(ctx: Ctx, input: ImportIn) -> rustler_core::Result<Outcome<ImportOut>> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let raw = input.input.trim().to_string();
    if raw.is_empty() {
        return Err(validation("source.import", "input is empty", "pass a file path or URL: `rustler source import <file-or-url>`"));
    }
    let sections: Vec<TimeRange> = input.sections.iter().flatten().map(|s| parse_section(s)).collect::<Result<_, _>>()?;
    let paths = ToolPaths::from_home(&ctx.home);
    let mut warnings = Vec::new();
    let mut sources = Vec::new();

    if is_url(&raw) {
        require_network(&project, true, "downloading a video")?;
        if ctx.dry_run {
            return Ok(Outcome::dry_run(json!({"download": raw, "sections": input.sections, "sources": sections.len().max(1)})));
        }
        let ranges: Vec<Option<TimeRange>> = if sections.is_empty() { vec![None] } else { sections.iter().copied().map(Some).collect() };
        let total = ranges.len();
        for (n, range) in ranges.into_iter().enumerate() {
            let s = import_url(&ctx, &store, &project, &paths, &raw, range, input.name.as_deref(), n, total, &mut warnings).await?;
            sources.push(s);
        }
    } else {
        if !sections.is_empty() {
            return Err(validation(
                "source.import",
                "--sections only applies to URLs",
                "local files are imported whole; choose the range later with `clip create --start/--end`",
            ));
        }
        let path = PathBuf::from(&raw);
        if !path.is_file() {
            return Err(RustlerError::new(ErrorCode::NotFound, "source.import", format!("file not found: {raw}"))
                .hint("check the path (quote it if it contains spaces); URLs must start with http:// or https://"));
        }
        let path = std::path::absolute(&path).unwrap_or(path);
        if !path_allowed(&project.permissions.fs_roots, &path) {
            return Err(validation(
                "source.import",
                format!("{} is outside this project's allowed folders", path.display()),
                "allow it with `rustler project set-permissions --fs-roots <dir>` or copy the file into an allowed folder",
            ));
        }
        if ctx.dry_run {
            return Ok(Outcome::dry_run(json!({"import_file": path, "sources": 1})));
        }
        let name = input
            .name
            .clone()
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "source".to_string());
        let id = SourceId::new();
        let origin = SourceOrigin::File { path: path.clone() };
        sources.push(finish_import(&ctx, &store, &paths, id, name, origin, path, &mut warnings, 0.0, 1).await?);
    }

    for s in &sources {
        let id = s.id.clone();
        update_project(&store, |p| {
            if !p.sources.contains(&id) {
                p.sources.push(id.clone());
            }
            Ok(())
        })?;
        log_event(&ctx, &store, "source.imported", json!({"source_id": s.id, "name": s.name, "duration": s.probe.duration}))?;
    }
    let first = &sources[0];
    let mut next = Vec::new();
    if first.probe.has_audio {
        next.push(format!("rustler analyze quick {}   (loudness + scenes + transcript, one call)", first.id));
    } else {
        next.push(format!("rustler analyze scenes {}   (no audio track: nothing to transcribe)", first.id));
    }
    next.push(format!("rustler see tiles {}   (look at it)", first.id));
    Ok(Outcome::Done(ImportOut { sources, warnings, next }))
}

#[allow(clippy::too_many_arguments)]
async fn import_url(
    ctx: &Ctx,
    store: &ProjectStore,
    project: &Project,
    paths: &ToolPaths,
    url: &str,
    range: Option<TimeRange>,
    name: Option<&str>,
    n: usize,
    total: usize,
    warnings: &mut Vec<String>,
) -> rustler_core::Result<Source> {
    let ytdlp = paths.require(Tool::Ytdlp)?;
    let id = SourceId::new();
    let dir = store.source_dir(&id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "source.import", format!("cannot create {}: {e}", dir.display())))?;
    let result = async {
        let mut a: Vec<OsString> = vec![
            "-f".into(),
            "bv*[height<=1080]+ba/b[height<=1080]".into(),
            "--merge-output-format".into(),
            "mp4".into(),
            "--no-playlist".into(),
            "--newline".into(),
            "--no-colors".into(),
            "--write-info-json".into(),
            "--ffmpeg-location".into(),
            paths.bin.clone().into_os_string(),
            "-o".into(),
            dir.join("media.%(ext)s").into_os_string(),
        ];
        if let Some(r) = range {
            a.push("--download-sections".into());
            a.push(format!("*{}-{}", clock_arg(r.start), clock_arg(r.end)).into());
        }
        a.extend(cookie_args(project));
        let progress = ctx.progress.clone();
        let label = range.map(|r| format!("{}-{}", mmss(r.start), mmss(r.end)));
        let mut on_line = |line: &str| {
            if let Some(p) = parse_ytdlp_progress(line) {
                progress.emit(ProgressEvent {
                    stage: "download".into(),
                    pct: Some((n as f32 + p) / total as f32 * 0.7),
                    message: label.clone().map(|l| format!("section {l}")),
                    eta_s: None,
                });
            }
        };
        // One YouTube lookup shared by every section (and by an earlier `source info` /
        // `fetch-transcript`); yt-dlp then downloads straight from the saved, signed URLs.
        let (_, info_path) = cached_info(ctx, paths, project, url, false).await?;
        let mut cached = a.clone();
        cached.push("--load-info-json".into());
        cached.push(info_path.into_os_string());
        if let Err(e) = run_ytdlp(&ytdlp, &cached, &ctx.cancel, false, &mut on_line).await {
            if e.code == ErrorCode::Cancelled {
                return Err(e);
            }
            // Saved URLs expired or were rejected: ask YouTube again (slow path).
            let (_, fresh) = cached_info(ctx, paths, project, url, true).await?;
            let mut retry = a.clone();
            retry.push("--load-info-json".into());
            retry.push(fresh.into_os_string());
            if run_ytdlp(&ytdlp, &retry, &ctx.cancel, false, &mut on_line).await.is_err() {
                a.push(url.into());
                run_ytdlp(&ytdlp, &a, &ctx.cancel, false, &mut on_line).await?;
            }
        }
        let media = find_media(&dir)?;
        let info_json = std::fs::read_to_string(dir.join("media.info.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
        let title = info_json.as_ref().and_then(|v| v["title"].as_str().map(String::from));
        let _ = std::fs::remove_file(dir.join("media.info.json"));
        let base = name.map(String::from).or(title).unwrap_or_else(|| "video".to_string());
        let display = match range {
            Some(r) => format!("{base} [{}-{}]", mmss(r.start), mmss(r.end)),
            None => base,
        };
        let origin = SourceOrigin::Url { url: url.to_string(), section: range, original_offset: range.map_or(0.0, |r| r.start) };
        let progress_base = 0.7 + 0.3 * (n as f32 / total as f32);
        let s = finish_import(ctx, store, paths, id.clone(), display, origin, media, warnings, progress_base, total).await?;
        // Save the video's yt-dlp info (title, chapters, heatmap, ...) for highlights.signals / source.info.
        match &info_json {
            Some(v) => {
                let rel = Path::new("analysis").join(s.id.as_str()).join("source_info.json");
                if let Err(e) = store.write_json(&rel, &build_source_info(v, url, range)) {
                    warnings.push(format!("could not save source_info.json: {}", e.message));
                }
            }
            None => warnings.push("yt-dlp wrote no info json: source_info.json (heatmap, chapters) was not saved".to_string()),
        }
        if let Some(r) = range {
            let want = r.end - r.start;
            if (s.probe.duration - want).abs() > 1.5 {
                warnings.push(format!(
                    "section {}-{} came out {:.1}s long (asked {:.1}s): the cut landed on keyframes, so timestamps may be off by up to {:.1}s; whisper on the imported file gives exact timing",
                    mmss(r.start), mmss(r.end), s.probe.duration, want, (s.probe.duration - want).abs()
                ));
            }
        }
        Ok::<Source, RustlerError>(s)
    }
    .await;
    if result.is_err() && !store.source_dir(&id).join("source.json").exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

fn find_media(dir: &Path) -> rustler_core::Result<PathBuf> {
    const EXT: &[&str] = &["mp4", "mkv", "webm", "mov", "m4a", "mp3", "opus", "ogg", "aac", "flac", "wav"];
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_stem().is_some_and(|s| s == "media")
                && p.extension().is_some_and(|e| EXT.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
        })
        .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .ok_or_else(|| {
            RustlerError::new(ErrorCode::Provider, "source.import", "yt-dlp finished but produced no media file")
                .hint("the section may be outside the video's duration; check `rustler source info <url>`")
        })
}

/// Probe, proxy and wav for a downloaded/located media file; writes `source.json`.
#[allow(clippy::too_many_arguments)]
async fn finish_import(
    ctx: &Ctx,
    store: &ProjectStore,
    paths: &ToolPaths,
    id: SourceId,
    name: String,
    origin: SourceOrigin,
    media_abs: PathBuf,
    warnings: &mut Vec<String>,
    progress_base: f32,
    total: usize,
) -> rustler_core::Result<Source> {
    let emit = |stage: &str, frac: f32, msg: &str| {
        let pct = progress_base + (1.0 - progress_base) * frac;
        ctx.progress.emit(ProgressEvent { stage: stage.into(), pct: Some(pct.min(1.0)), message: Some(msg.to_string()), eta_s: None });
    };
    let _ = total;
    emit("probe", 0.0, "reading media info");
    let probe: MediaProbe = rustler_media::probe(paths, &media_abs).await?;
    if !probe.has_video && !probe.has_audio {
        return Err(validation(
            "source.import",
            format!("{} has neither a video nor an audio stream", media_abs.display()),
            "check that the file is a playable media file (try `ffprobe` on it)",
        ));
    }
    let dir = store.source_dir(&id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "source.import", format!("cannot create {}: {e}", dir.display())))?;
    let made = async {
        let mut proxy = None;
        if probe.has_video {
            emit("proxy", 0.2, "making 540p proxy");
            let p = dir.join("proxy.mp4");
            rustler_media::make_proxy(paths, &media_abs, &p, &ctx.cancel, &ctx.progress).await?;
            proxy = Some(store.relativize(&p));
        } else {
            warnings.push("audio-only source: accepted for transcription, but `clip create` needs a video stream".to_string());
        }
        let mut wav = None;
        if probe.has_audio {
            emit("audio_wav", 0.8, "extracting 16 kHz audio for transcription");
            let w = dir.join("audio.wav");
            rustler_media::extract_audio_wav(paths, &media_abs, &w, &ctx.cancel).await?;
            wav = Some(store.relativize(&w));
        } else {
            warnings.push("video has no audio track: nothing to transcribe; renders will get a silent audio track".to_string());
        }
        Ok::<_, RustlerError>((proxy, wav))
    }
    .await;
    let (proxy, audio_wav) = match made {
        Ok(v) => v,
        Err(e) => {
            if matches!(origin, SourceOrigin::File { .. }) {
                let _ = std::fs::remove_dir_all(&dir);
            }
            return Err(e);
        }
    };
    let source = Source {
        id,
        name,
        origin,
        media_path: store.relativize(&media_abs),
        probe,
        proxy,
        audio_wav,
        imported_at: chrono::Utc::now(),
    };
    store.save_source(&source)?;
    emit("done", 1.0, "imported");
    Ok(source)
}

// ---------------------------------------------------------------------------------------
// source.list / source.show
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug, Default)]
pub struct ListIn {}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ListItem {
    pub id: SourceId,
    pub name: String,
    pub duration: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub origin: String,
    pub transcribed: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ListOut {
    pub sources: Vec<ListItem>,
    pub next: Vec<String>,
}

pub async fn list(ctx: Ctx, _input: ListIn) -> rustler_core::Result<ListOut> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let mut sources = Vec::new();
    for id in &project.sources {
        let Ok(s) = store.load_source(id) else { continue };
        sources.push(ListItem {
            origin: match &s.origin {
                SourceOrigin::File { path } => path.display().to_string(),
                SourceOrigin::Url { url, .. } => url.clone(),
            },
            transcribed: store.analysis_dir(&s.id).join("transcript.json").is_file(),
            id: s.id,
            name: s.name,
            duration: s.probe.duration,
            has_video: s.probe.has_video,
            has_audio: s.probe.has_audio,
        });
    }
    let next = if sources.is_empty() { vec!["rustler source import <file-or-url>".to_string()] } else { vec!["rustler source show <id>".to_string()] };
    Ok(ListOut { sources, next })
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct ShowIn {
    pub source_id: SourceId,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct AnalysisStatus {
    pub transcript: bool,
    pub units: bool,
    pub scenes: bool,
    pub audio_events: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ShowOut {
    pub source: Source,
    pub analysis: AnalysisStatus,
    pub next: Vec<String>,
}

pub async fn show(ctx: Ctx, input: ShowIn) -> rustler_core::Result<ShowOut> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    let a = store.analysis_dir(&source.id);
    let analysis = AnalysisStatus {
        transcript: a.join("transcript.json").is_file(),
        units: a.join("units.json").is_file(),
        scenes: a.join("scenes.json").is_file(),
        audio_events: a.join("audio_events.json").is_file(),
    };
    let mut next = Vec::new();
    if !analysis.transcript && source.probe.has_audio {
        next.push(format!("rustler analyze transcribe {}", source.id));
    }
    if analysis.transcript {
        next.push(format!("rustler transcript get {}", source.id));
    }
    if !analysis.audio_events {
        next.push(format!("rustler analyze audio {}", source.id));
    }
    Ok(ShowOut { source, analysis, next })
}

/// Loads a source or a NotFound error that names `source list`.
pub fn load_source_checked(store: &ProjectStore, id: &SourceId) -> rustler_core::Result<Source> {
    store.load_source(id).map_err(|e| {
        if e.code == ErrorCode::NotFound {
            RustlerError::new(ErrorCode::NotFound, "source", format!("no source {id} in this project")).hint("run `rustler source list` to see source ids")
        } else {
            e
        }
    })
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let net = Annotations { network: true, ..Default::default() };
    reg.register(typed(
        "source.import",
        "Import a local file or URL (URL + --sections downloads only those windows); makes proxy + 16 kHz wav",
        &["input"],
        Annotations { network: true, long_running: true, ..Default::default() },
        vec![
            Example { description: "Local file", cli: "rustler source import \"C:\\videos\\podcast.mp4\"" },
            Example { description: "Only two windows of a URL", cli: "rustler source import https://youtu.be/ID --sections 47:00-49:30 --sections 1:02:10-1:03:00" },
        ],
        import,
    ));
    reg.register(typed(
        "source.info",
        "Scan an online video without downloading: title, chapters, most-replayed heatmap peaks, caption languages. With --source-id: the info saved when that URL source was imported (no network)",
        &["url"],
        Annotations { read_only: true, ..net },
        vec![
            Example { description: "Scan first", cli: "rustler source info https://youtu.be/ID" },
            Example { description: "Saved info of an imported source", cli: "rustler source info --source-id src_X" },
        ],
        info,
    ));
    reg.register(typed(
        "source.fetch_transcript",
        "Download existing captions (no video) as a pending transcript with units; approximate word timing",
        &["url"],
        net,
        vec![Example { description: "Captions in English or Hindi", cli: "rustler source fetch-transcript https://youtu.be/ID" }],
        fetch_transcript,
    ));
    reg.register(typed(
        "source.search",
        "Search YouTube (public results) via yt-dlp",
        &["query"],
        Annotations { read_only: true, ..net },
        vec![Example { description: "Find videos", cli: "rustler source search \"nasa apollo 11\" --limit 5" }],
        search,
    ));
    reg.register(typed(
        "source.list",
        "List the project's sources",
        &[],
        Annotations { read_only: true, ..Default::default() },
        vec![],
        list,
    ));
    reg.register(typed(
        "source.show",
        "Show one source with its probe and which analyses exist",
        &["source_id"],
        Annotations { read_only: true, ..Default::default() },
        vec![],
        show,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_and_sections() {
        assert_eq!(parse_clock("47:05"), Some(2825.0));
        assert_eq!(parse_clock("1:02:05.5"), Some(3725.5));
        assert_eq!(parse_clock("90"), Some(90.0));
        assert_eq!(parse_clock("1:75:00"), None);
        let r = parse_section("00:47:00-00:49:30").unwrap();
        assert_eq!((r.start, r.end), (2820.0, 2970.0));
        let r = parse_section("47:00-49:30").unwrap();
        assert_eq!((r.start, r.end), (2820.0, 2970.0));
        assert!(parse_section("49:30-47:00").is_err());
        assert!(parse_section("nonsense").is_err());
        assert_eq!(parse_section("1:00-").unwrap_err().code, ErrorCode::Validation);
    }

    #[test]
    fn progress_lines() {
        assert!((parse_ytdlp_progress("[download]  45.2% of  123.45MiB at  2.0MiB/s ETA 00:30").unwrap() - 0.452).abs() < 1e-5);
        assert_eq!(parse_ytdlp_progress("[download] 100% of 5.00MiB in 00:02"), Some(1.0));
        assert_eq!(parse_ytdlp_progress("[info] something"), None);
    }

    #[test]
    fn caption_choice() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(choose_caption(&s(&["hi", "en"]), &s(&["de-orig"]), "auto"), Some(("en".into(), true)));
        assert_eq!(choose_caption(&[], &s(&["hi-orig", "en"]), "auto"), Some(("en".into(), false)));
        assert_eq!(choose_caption(&[], &s(&["hi-orig", "fr-orig"]), "auto"), Some(("hi-orig".into(), false)));
        assert_eq!(choose_caption(&[], &s(&["fr-orig"]), "auto"), Some(("fr-orig".into(), false)));
        assert_eq!(choose_caption(&s(&["en-GB"]), &[], "en"), Some(("en-GB".into(), true)));
        assert_eq!(choose_caption(&s(&["en"]), &[], "ja"), None);
    }

    #[test]
    fn info_heatmap_peaks_and_langs() {
        let heat: Vec<Value> = (0..20)
            .map(|i| json!({"start_time": i as f64 * 30.0, "end_time": (i + 1) as f64 * 30.0, "value": i as f64 / 20.0}))
            .collect();
        let v = json!({
            "title": "T", "duration": 600.0, "heatmap": heat,
            "chapters": [{"start_time": 0.0, "end_time": 60.0, "title": "Intro"}],
            "subtitles": {"en": [], "live_chat": []},
            "automatic_captions": {"en": [], "en-orig": [], "fr": [], "de": []}
        });
        let i = parse_info(&v, "https://x");
        assert_eq!(i.heatmap_peaks.len(), 8);
        assert_eq!(i.heatmap_peaks[0].start, 570.0);
        assert_eq!(i.heatmap_peaks[0].range, "09:30-10:00");
        assert_eq!(i.chapters[0].title, "Intro");
        assert_eq!(i.subtitles.manual, vec!["en"]);
        assert!(i.subtitles.auto.contains(&"en-orig".to_string()) && !i.subtitles.auto.contains(&"fr".to_string()));
    }

    #[test]
    fn error_hints() {
        assert!(classify_ytdlp_error("ERROR: Sign in to confirm you're not a bot").1);
        assert!(!classify_ytdlp_error("Video unavailable").1);
    }

    #[test]
    fn pending_id_is_stable() {
        assert_eq!(pending_id("https://a/b"), pending_id(" https://a/b "));
        assert!(pending_id("x").starts_with("url_") && pending_id("x").len() == 12);
    }
}
