//! Commands: reference.import, reference.list, reference.show, reference.analyze (Task 1.6, style references).
//!
//! A *reference set* is a folder of example videos the agent wants to learn a clip style from. It lives
//! under `<project>/references/<name>/` (`reference.json`, `proxies/`, `audio/`, `sheets/`, `profile.json`).
//! `reference.analyze` measures what eyes cannot count (cuts per minute, caption band, zoom rate, loudness,
//! music, speech rate) with light tools only (ffmpeg plus OpenCV in the engine; the YuNet face model when
//! available) and renders a contact sheet and a first-3-seconds filmstrip per video for the agent to LOOK at.
//! Nothing here calls an AI API; the agent combines the numbers with what it sees.

use crate::analyze_cmds::spawn_engine;
use rustler_core::{ErrorCode, MediaProbe, ProjectStore, RustlerError};
use rustler_media::{TileSpec, tiles};
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

const VIDEO_EXTS: [&str; 11] = ["mp4", "mov", "mkv", "webm", "m4v", "avi", "flv", "wmv", "mpg", "mpeg", "ts"];
const MAX_FILES: usize = 60;
const DEFAULT_MAX_SECONDS: f64 = 180.0;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn internal(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Stored model
// ---------------------------------------------------------------------------------------------

/// One video of a reference set.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RefVideo {
    /// `v01`, `v02`, ... (stable within the set).
    pub id: String,
    /// Original file name.
    pub file: String,
    /// Absolute path of the original (never modified or copied).
    pub path: PathBuf,
    pub size_bytes: u64,
    pub probe: MediaProbe,
    /// Project-relative low-res proxy (<= 540 px short side, 15 fps, no audio).
    pub proxy: PathBuf,
    /// Project-relative 16 kHz mono wav (absent when the file has no audio).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<PathBuf>,
}

/// `<project>/references/<name>/reference.json`
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RefSet {
    pub name: String,
    pub created: String,
    /// Only the first this-many seconds of each video are proxied and analysed.
    pub max_seconds: f64,
    pub videos: Vec<RefVideo>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !name.starts_with('.')
}

fn set_rel(name: &str) -> PathBuf {
    Path::new("references").join(name)
}

/// `references/<name>/profile.json` (written by `reference.analyze`); a name that is not a valid
/// set name maps to a path that never exists.
pub(crate) fn profile_rel(name: &str) -> PathBuf {
    if valid_name(name) { set_rel(name).join("profile.json") } else { Path::new("references").join(".invalid").join("profile.json") }
}

fn load_set(store: &ProjectStore, name: &str, stage: &str) -> Result<RefSet> {
    if !valid_name(name) {
        return Err(validation(stage, format!("bad reference set name '{name}'")).hint("use letters, digits, - _ . (max 64)"));
    }
    let rel = set_rel(name).join("reference.json");
    if !store.resolve(&rel).is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, stage, format!("no reference set named '{name}'"))
            .hint("list sets with `rustler reference list`; create one with `rustler reference import <name> --folder <dir>`"));
    }
    store.read_json(&rel)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn ffmpeg_args(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}

// ---------------------------------------------------------------------------------------------
// reference.import
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ImportIn {
    /// Name of the reference set (letters, digits, - _ .).
    pub name: String,
    /// A folder of example videos.
    pub folder: Option<PathBuf>,
    /// Or explicit video files (can be combined with `folder`).
    pub files: Option<Vec<PathBuf>>,
    /// Also look into sub-folders of `folder` (default false).
    pub recursive: Option<bool>,
    /// Replace an existing set of the same name (default false).
    pub replace: Option<bool>,
    /// Only the first N seconds of each video are kept for analysis (default 180).
    pub max_seconds: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ImportOut {
    pub name: String,
    pub dir: PathBuf,
    pub videos: Vec<RefVideo>,
    pub skipped: Vec<SkippedFile>,
    pub next: String,
}

fn is_video_ext(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn walk_videos(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if recursive {
                walk_videos(&p, true, out);
            }
        } else if p.is_file() && is_video_ext(&p) {
            out.push(p);
        }
    }
}

/// Files named by `folder` / `files`, sorted and de-duplicated.
pub fn collect_video_files(folder: Option<&Path>, files: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if let Some(f) = folder {
        if !f.is_dir() {
            return Err(RustlerError::new(ErrorCode::NotFound, "reference.import", format!("folder not found: {}", f.display())));
        }
        walk_videos(f, recursive, &mut out);
    }
    for f in files {
        if !f.is_file() {
            return Err(RustlerError::new(ErrorCode::NotFound, "reference.import", format!("file not found: {}", f.display())));
        }
        out.push(f.clone());
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err(validation("reference.import", "no video files found").hint(format!("looked for {}", VIDEO_EXTS.join(", "))));
    }
    if out.len() > MAX_FILES {
        return Err(validation("reference.import", format!("{} videos is too many for one set (max {MAX_FILES})", out.len()))
            .hint("a style needs a handful of examples; pass a smaller folder or --files"));
    }
    Ok(out)
}

async fn ffmpeg_run(paths: &ToolPaths, ctx: &Ctx, args: Vec<OsString>, what: &str) -> Result<()> {
    rustler_tools::run(&paths.ffmpeg, &args, None, &ctx.cancel).await.map(|_| ()).map_err(|mut e| {
        e.stage = what.to_string();
        e
    })
}

async fn reference_import(ctx: Ctx, i: ImportIn) -> Result<ImportOut> {
    let stage = "reference.import";
    if !valid_name(&i.name) {
        return Err(validation(stage, format!("bad reference set name '{}'", i.name)).hint("use letters, digits, - _ . (max 64)"));
    }
    let store = ctx.store()?;
    let max_seconds = i.max_seconds.unwrap_or(DEFAULT_MAX_SECONDS);
    if !(max_seconds.is_finite() && max_seconds >= 3.0) {
        return Err(validation(stage, "max_seconds must be at least 3"));
    }
    let dir = store.root.join(set_rel(&i.name));
    if dir.join("reference.json").is_file() && !i.replace.unwrap_or(false) {
        return Err(validation(stage, format!("reference set '{}' already exists", i.name))
            .hint("pass --replace to rebuild it, or pick another name"));
    }
    let files = collect_video_files(i.folder.as_deref(), i.files.as_deref().unwrap_or(&[]), i.recursive.unwrap_or(false))?;
    let paths = ToolPaths::from_home(&ctx.home);
    if !paths.ffmpeg.is_file() {
        return Err(RustlerError::new(ErrorCode::DependencyMissing, stage, "ffmpeg is not installed").hint("run `rustler setup`"));
    }
    if dir.is_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    for sub in ["proxies", "audio", "sheets"] {
        std::fs::create_dir_all(dir.join(sub)).map_err(|e| internal(stage, format!("create {}: {e}", dir.join(sub).display())))?;
    }

    let mut videos = Vec::new();
    let mut skipped = Vec::new();
    let total = files.len();
    for (n, f) in files.iter().enumerate() {
        ctx.progress.emit(ProgressEvent {
            stage: stage.into(),
            pct: Some(n as f32 / total as f32),
            message: Some(format!("{} ({}/{})", f.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), n + 1, total)),
            eta_s: None,
        });
        let probe = match rustler_media::probe(&paths, f).await {
            Ok(p) => p,
            Err(e) => {
                skipped.push(SkippedFile { path: f.clone(), reason: format!("cannot probe: {}", e.message) });
                continue;
            }
        };
        if !probe.has_video || probe.duration <= 0.5 {
            skipped.push(SkippedFile { path: f.clone(), reason: "no video stream (or shorter than 0.5 s)".into() });
            continue;
        }
        let id = format!("v{:02}", videos.len() + 1);
        let proxy_rel = set_rel(&i.name).join("proxies").join(format!("{id}.mp4"));
        let t = format!("{max_seconds}");
        let mut a = ffmpeg_args(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
        a.push(f.as_os_str().to_owned());
        a.extend(ffmpeg_args(&[
            "-map", "0:v:0", "-an", "-t", &t,
            "-vf", "fps=15,scale='if(lt(iw,ih),min(540,iw),-2)':'if(lt(iw,ih),-2,min(540,ih))'",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "28", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
        ]));
        a.push(store.resolve(&proxy_rel).into_os_string());
        ffmpeg_run(&paths, &ctx, a, "reference.proxy").await?;

        let mut audio = None;
        if probe.has_audio {
            let wav_rel = set_rel(&i.name).join("audio").join(format!("{id}.wav"));
            let mut a = ffmpeg_args(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
            a.push(f.as_os_str().to_owned());
            a.extend(ffmpeg_args(&["-map", "0:a:0", "-vn", "-t", &t, "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]));
            a.push(store.resolve(&wav_rel).into_os_string());
            match ffmpeg_run(&paths, &ctx, a, "reference.audio").await {
                Ok(()) => audio = Some(wav_rel),
                Err(e) if e.code == ErrorCode::Cancelled => return Err(e),
                Err(_) => {} // unreadable audio track: the video is still useful
            }
        }
        videos.push(RefVideo {
            id,
            file: f.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            path: std::fs::canonicalize(f).unwrap_or_else(|_| f.clone()),
            size_bytes: std::fs::metadata(f).map(|m| m.len()).unwrap_or(0),
            probe,
            proxy: proxy_rel,
            audio,
        });
    }
    if videos.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(validation(stage, "none of the files has a video stream").hint("only video files can be style references"));
    }
    let set = RefSet { name: i.name.clone(), created: now_iso(), max_seconds, videos: videos.clone() };
    store.write_json(&set_rel(&i.name).join("reference.json"), &set)?;
    ctx.progress.emit(ProgressEvent { stage: stage.into(), pct: Some(1.0), message: None, eta_s: None });
    Ok(ImportOut {
        name: i.name.clone(),
        dir,
        videos,
        skipped,
        next: format!("run `rustler reference analyze {}`, then LOOK at each contact sheet and filmstrip it returns", i.name),
    })
}

// ---------------------------------------------------------------------------------------------
// reference.list / reference.show
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ListIn {}

#[derive(Serialize, JsonSchema)]
pub struct SetSummary {
    pub name: String,
    pub created: String,
    pub videos: usize,
    pub total_duration_s: f64,
    pub analyzed: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOut {
    pub sets: Vec<SetSummary>,
}

async fn reference_list(ctx: Ctx, _i: ListIn) -> Result<ListOut> {
    let store = ctx.store()?;
    let root = store.root.join("references");
    let mut sets = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Ok(s) = load_set(&store, &name, "reference.list") {
                sets.push(SetSummary {
                    analyzed: root.join(&name).join("profile.json").is_file(),
                    name,
                    created: s.created,
                    videos: s.videos.len(),
                    total_duration_s: s.videos.iter().map(|v| v.probe.duration.min(s.max_seconds)).sum::<f64>(),
                });
            }
        }
    }
    sets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ListOut { sets })
}

#[derive(Deserialize, JsonSchema)]
pub struct ShowIn {
    pub name: String,
    /// Only this video (`v01`...): its sheets are returned as images.
    pub video: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ShowOut {
    pub name: String,
    pub set: RefSet,
    /// The saved profile (`profile.json`), when `reference.analyze` has run.
    pub profile: Option<Value>,
    /// Contact sheets and filmstrips to LOOK at (all videos, or the one asked for).
    pub image: Vec<PathBuf>,
}

fn sheet_paths(store: &ProjectStore, name: &str, id: &str) -> (PathBuf, PathBuf) {
    let d = store.root.join(set_rel(name)).join("sheets");
    (d.join(format!("{id}_contact.jpg")), d.join(format!("{id}_first3s.jpg")))
}

async fn reference_show(ctx: Ctx, i: ShowIn) -> Result<ShowOut> {
    let store = ctx.store()?;
    let set = load_set(&store, &i.name, "reference.show")?;
    if let Some(v) = &i.video {
        if !set.videos.iter().any(|x| &x.id == v) {
            return Err(validation("reference.show", format!("no video '{v}' in set '{}'", i.name))
                .hint(format!("ids: {}", set.videos.iter().map(|v| v.id.as_str()).collect::<Vec<_>>().join(", "))));
        }
    }
    let profile = store.read_json::<Value>(&set_rel(&i.name).join("profile.json")).ok();
    let mut contact = Vec::new();
    let mut strips = Vec::new();
    for v in set.videos.iter().filter(|v| i.video.as_ref().is_none_or(|w| w == &v.id)) {
        let (c, s) = sheet_paths(&store, &i.name, &v.id);
        if c.is_file() {
            contact.push(c);
        }
        if s.is_file() {
            strips.push(s);
        }
    }
    contact.extend(strips);
    Ok(ShowOut { name: i.name, set, profile, image: contact })
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested below)
// ---------------------------------------------------------------------------------------------

/// `{count, mean, median, min, max}` of a list of numbers (`null` fields when empty).
pub fn num_stats(values: &[f64]) -> Value {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return json!({"count": 0});
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let median = if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 };
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    json!({"count": n, "mean": r(v.iter().sum::<f64>() / n as f64), "median": r(median), "min": r(v[0]), "max": r(v[n - 1])})
}

fn median_of(values: &[f64]) -> Option<f64> {
    num_stats(values).get("median").and_then(Value::as_f64)
}

fn path_f64(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_f64()
}

/// Cues of an SRT or VTT file as `(start, end, text)`.
pub fn parse_subtitles(src: &str) -> Vec<(f64, f64, String)> {
    fn ts(s: &str) -> Option<f64> {
        let s = s.trim().replace(',', ".");
        let parts: Vec<&str> = s.split(':').collect();
        let (h, m, sec) = match parts.as_slice() {
            [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
            [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
            _ => return None,
        };
        Some(h * 3600.0 + m * 60.0 + sec)
    }
    let mut out = Vec::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some((a, b)) = line.split_once("-->") {
            let (Some(s), Some(e)) = (ts(a), ts(b.split_whitespace().next().unwrap_or(""))) else { continue };
            let mut text = Vec::new();
            while let Some(l) = lines.peek() {
                if l.trim().is_empty() {
                    break;
                }
                text.push(lines.next().unwrap().trim().to_string());
            }
            out.push((s, e, text.join(" ")));
        }
    }
    out
}

/// Speech measures from cues, limited to the first `limit` seconds.
pub fn speech_from_cues(cues: &[(f64, f64, String)], video_duration: f64, limit: f64) -> Option<Value> {
    let cues: Vec<&(f64, f64, String)> = cues.iter().filter(|c| c.0 < limit).collect();
    if cues.is_empty() {
        return None;
    }
    let words: usize = cues.iter().map(|c| c.2.split_whitespace().count()).sum();
    // Time spent speaking: cue spans merged across gaps of up to 0.5 s (word-level cues leave gaps between words).
    let mut spans: Vec<(f64, f64)> = cues.iter().map(|c| (c.0, c.1.min(limit))).filter(|s| s.1 > s.0).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for s in spans {
        match merged.last_mut() {
            Some(l) if s.0 - l.1 <= 0.5 => l.1 = l.1.max(s.1),
            _ => merged.push(s),
        }
    }
    let spoken: f64 = merged.iter().map(|s| s.1 - s.0).sum();
    if words == 0 || spoken <= 0.0 {
        return None;
    }
    let dur = video_duration.min(limit).max(1.0);
    Some(json!({
        "source": "subtitle sidecar (.srt/.vtt)",
        "cues": cues.len(),
        "words": words,
        "words_per_cue": (words as f64 / cues.len() as f64 * 100.0).round() / 100.0,
        "wpm_while_speaking": (words as f64 / (spoken / 60.0)).round(),
        "wpm_over_video": (words as f64 / (dur / 60.0)).round(),
        "coverage": ((spoken / dur) * 100.0).round() / 100.0,
    }))
}

fn sidecar_cues(video: &Path) -> Option<Vec<(f64, f64, String)>> {
    for ext in ["srt", "vtt"] {
        let p = video.with_extension(ext);
        if let Ok(s) = std::fs::read_to_string(&p) {
            let c = parse_subtitles(s.trim_start_matches('\u{feff}'));
            if !c.is_empty() {
                return Some(c);
            }
        }
    }
    None
}

/// `(integrated LUFS, LRA, true peak dBFS)` from ffmpeg `ebur128` stderr (the Summary block).
pub fn parse_ebur128(stderr: &str) -> Option<(f64, Option<f64>, Option<f64>)> {
    let tail = &stderr[stderr.rfind("Summary:")?..];
    let grab = |key: &str| -> Option<f64> {
        let line = tail.lines().find(|l| l.trim_start().starts_with(key))?;
        line.split(key).nth(1)?.split_whitespace().next()?.parse().ok()
    };
    let i = grab("I:")?;
    if i < -100.0 {
        return None;
    }
    Some((i, grab("LRA:"), grab("Peak:")))
}

fn hex_rgb(h: &str) -> Option<[f64; 3]> {
    let h = h.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(f64::from);
    Some([p(0)?, p(2)?, p(4)?])
}

/// Merges `(hex, weight)` colours across videos: colours within RGB distance 60 join; top `n` by weight.
pub fn merge_colors(colors: &[(String, f64)], n: usize) -> Vec<Value> {
    let mut groups: Vec<([f64; 3], f64, String)> = Vec::new();
    let mut sorted: Vec<&(String, f64)> = colors.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (hex, w) in sorted {
        let Some(rgb) = hex_rgb(hex) else { continue };
        let d = |g: &([f64; 3], f64, String)| ((g.0[0] - rgb[0]).powi(2) + (g.0[1] - rgb[1]).powi(2) + (g.0[2] - rgb[2]).powi(2)).sqrt();
        if let Some(g) = groups.iter_mut().find(|g| d(g) < 60.0) {
            g.1 += w;
        } else {
            groups.push((rgb, *w, hex.clone()));
        }
    }
    let total: f64 = groups.iter().map(|g| g.1).sum::<f64>().max(1e-9);
    groups.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    groups.into_iter().take(n).map(|g| json!({"hex": g.2, "share": (g.1 / total * 100.0).round() / 100.0})).collect()
}

/// Cross-video aggregate of the per-video profiles.
pub fn aggregate(videos: &[Value]) -> Value {
    let col = |f: &dyn Fn(&Value) -> Option<f64>| -> Vec<f64> { videos.iter().filter_map(f).collect() };
    let durations = col(&|v| path_f64(v, &["duration_s"]));
    let cpm = col(&|v| path_f64(v, &["cuts", "per_minute"]));
    let zpm = col(&|v| path_f64(v, &["zoom", "per_minute"]));
    let zcov = col(&|v| path_f64(v, &["zoom", "coverage"]));
    let lufs = col(&|v| path_f64(v, &["loudness", "integrated_lufs"]));
    let wpm = col(&|v| path_f64(v, &["speech", "wpm_while_speaking"]));
    let wpc = col(&|v| path_f64(v, &["speech", "words_per_cue"]));

    // pooled shot lengths
    let mut shots = Vec::new();
    let mut layouts: std::collections::BTreeMap<String, u64> = Default::default();
    for v in videos {
        for s in v.get("shots").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(a), Some(b)) = (s.get("start").and_then(Value::as_f64), s.get("end").and_then(Value::as_f64)) {
                shots.push(b - a);
            }
        }
        for (k, c) in v.get("layout_distribution").and_then(Value::as_object).into_iter().flatten() {
            *layouts.entry(k.clone()).or_default() += c.as_u64().unwrap_or(0);
        }
    }
    let nshots: u64 = layouts.values().sum();
    let layout_share: serde_json::Map<String, Value> =
        layouts.iter().map(|(k, c)| (k.clone(), json!((*c as f64 / nshots.max(1) as f64 * 100.0).round() / 100.0))).collect();

    // captions
    let capt: Vec<&Value> = videos.iter().filter_map(|v| v.pointer("/text/caption")).filter(|c| c.get("present").and_then(Value::as_bool) == Some(true)).collect();
    let cap_f = |k: &str| -> Vec<f64> { capt.iter().filter_map(|c| c.get(k).and_then(Value::as_f64)).collect() };
    let wpl: Vec<f64> = capt.iter().filter_map(|c| c.pointer("/words_per_line/median").and_then(Value::as_f64)).collect();
    let mut fills = Vec::new();
    let mut outlines = Vec::new();
    for c in &capt {
        for f in c.get("fill_colors").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(h), Some(s)) = (f.get("hex").and_then(Value::as_str), f.get("share").and_then(Value::as_f64)) {
                fills.push((h.to_string(), s));
            }
        }
        if let Some(h) = c.get("outline_or_box_color").and_then(Value::as_str) {
            outlines.push((h.to_string(), 1.0));
        }
    }
    // hooks
    let hooks: Vec<&Value> = videos.iter().filter_map(|v| v.pointer("/text/hook")).filter(|c| c.get("present").and_then(Value::as_bool) == Some(true)).collect();
    let hook_f = |k: &str| -> Vec<f64> { hooks.iter().filter_map(|c| c.get(k).and_then(Value::as_f64)).collect() };
    let music_n = videos.iter().filter(|v| v.pointer("/music/music_present").and_then(Value::as_bool) == Some(true)).count();
    let music_known = videos.iter().filter(|v| v.pointer("/music/available").and_then(Value::as_bool) == Some(true)).count();

    let n = videos.len();
    let mut summary = Vec::new();
    if let Some(m) = median_of(&durations) {
        summary.push(format!("{n} videos, median length {m:.0} s"));
    }
    if let (Some(c), Some(s)) = (median_of(&cpm), median_of(&shots)) {
        summary.push(format!("pacing: median {c:.1} cuts per minute, median shot {s:.1} s"));
    }
    if !capt.is_empty() {
        let y = median_of(&cap_f("y_center")).unwrap_or(0.0);
        let h = median_of(&cap_f("height")).unwrap_or(0.0);
        summary.push(format!(
            "captions in {}/{n} videos: band centred at y={y:.2} of the frame height ({h:.3} tall), about {} words per line",
            capt.len(),
            median_of(&wpl).map_or("?".to_string(), |w| format!("{w:.0}"))
        ));
    } else {
        summary.push("no burned-in caption band detected".into());
    }
    if !hooks.is_empty() {
        summary.push(format!(
            "hook text on screen at the start in {}/{n} videos, gone after about {:.1} s",
            hooks.len(),
            median_of(&hook_f("visible_to")).unwrap_or(0.0)
        ));
    }
    if let Some(z) = median_of(&zpm) {
        summary.push(format!("zoom/punch events: median {z:.1} per minute (measurable in {:.0}% of frame pairs)", median_of(&zcov).unwrap_or(0.0) * 100.0));
    }
    if let Some(l) = median_of(&lufs) {
        summary.push(format!("loudness: median {l:.1} LUFS integrated"));
    }
    if music_known > 0 {
        summary.push(format!("music heuristic fired in {music_n}/{music_known} videos"));
    }
    if let Some(w) = median_of(&wpm) {
        summary.push(format!("speech: median {w:.0} words per minute while speaking"));
    }
    if nshots > 0 {
        let top = layouts.iter().max_by_key(|(_, c)| **c).map(|(k, _)| k.clone()).unwrap_or_default();
        summary.push(format!("most common shot layout: {top} ({} of {nshots} shots)", layouts.get(&top).copied().unwrap_or(0)));
    }
    json!({
        "videos": n,
        "duration_s": num_stats(&durations),
        "cuts_per_minute": num_stats(&cpm),
        "shot_length_s": num_stats(&shots),
        "caption": {
            "present_in": capt.len(),
            "y_center": num_stats(&cap_f("y_center")),
            "height": num_stats(&cap_f("height")),
            "words_per_line": num_stats(&wpl),
            "fill_colors": merge_colors(&fills, 4),
            "outline_or_box_colors": merge_colors(&outlines, 2),
            "presence_over_time": num_stats(&cap_f("presence")),
        },
        "hook": {
            "present_in": hooks.len(),
            "y_top": num_stats(&hook_f("y0")),
            "y_bottom": num_stats(&hook_f("y1")),
            "lines": num_stats(&hook_f("lines")),
            "visible_to_s": num_stats(&hook_f("visible_to")),
        },
        "zoom_per_minute": num_stats(&zpm),
        "zoom_measurable_fraction": num_stats(&zcov),
        "layout_share": Value::Object(layout_share),
        "loudness_lufs": num_stats(&lufs),
        "music_present_in": {"videos": music_n, "of": music_known},
        "speech_wpm": num_stats(&wpm),
        "words_per_caption_cue": num_stats(&wpc),
        "summary": summary,
    })
}

// ---------------------------------------------------------------------------------------------
// reference.analyze
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AnalyzeIn {
    pub name: String,
    /// Analyse only the first N videos of the set (default all).
    pub max_videos: Option<usize>,
    /// Skip the YuNet face detector (layout classes then rely on frame structure only).
    pub no_faces: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct AnalyzeOut {
    pub name: String,
    /// Contact sheets first, then first-3-second filmstrips (one per video). LOOK at them.
    pub image: Vec<PathBuf>,
    /// Absolute path of the saved `profile.json`.
    pub profile_path: PathBuf,
    pub aggregate: Value,
    pub videos: Vec<Value>,
    pub notes: Vec<String>,
}

async fn loudness(paths: &ToolPaths, ctx: &Ctx, media: &Path, limit: f64) -> Option<Value> {
    let t = format!("{limit}");
    let mut a = ffmpeg_args(&["-hide_banner", "-nostdin", "-nostats", "-t", &t, "-i"]);
    a.push(media.as_os_str().to_owned());
    a.extend(ffmpeg_args(&["-map", "0:a:0", "-vn", "-af", "ebur128=peak=true", "-f", "null", "-"]));
    let out = rustler_tools::run(&paths.ffmpeg, &a, None, &ctx.cancel).await.ok()?;
    let (i, lra, peak) = parse_ebur128(&String::from_utf8_lossy(&out.stderr))?;
    Some(json!({"integrated_lufs": i, "lra_lu": lra, "true_peak_dbfs": peak, "method": "ffmpeg ebur128 on the original audio"}))
}

async fn make_sheets(paths: &ToolPaths, store: &ProjectStore, set: &RefSet, v: &RefVideo, cues: Option<&Vec<(f64, f64, String)>>) -> Result<(PathBuf, PathBuf)> {
    let font = rustler_media::qc::default_font(paths)
        .ok_or_else(|| RustlerError::new(ErrorCode::DependencyMissing, "reference.analyze", "no font available for sheet labels").hint("run `rustler setup`"))?;
    let proxy = store.resolve(&v.proxy);
    let dur = v.probe.duration.min(set.max_seconds) - 0.1;
    let (contact, strip) = sheet_paths(store, &set.name, &v.id);
    let cue_text: Vec<(f64, f64, String)> = cues.cloned().unwrap_or_default();
    let labels = move |t: f64| -> String {
        cue_text.iter().find(|c| c.0 <= t && t <= c.1).map(|c| c.2.chars().take(28).collect()).unwrap_or_default()
    };
    let count = if dur > 40.0 { 15 } else { 12 };
    tiles(paths, &proxy, &TileSpec { start: 0.0, end: dur.max(1.0), count, cols: 4, tile_width: 240 }, &labels, &font, &contact).await?;
    let nolabel = |_: f64| String::new();
    tiles(paths, &proxy, &TileSpec { start: 0.0, end: dur.clamp(1.0, 3.0), count: 8, cols: 4, tile_width: 300 }, &nolabel, &font, &strip).await?;
    Ok((contact, strip))
}

async fn reference_analyze(ctx: Ctx, i: AnalyzeIn) -> Result<AnalyzeOut> {
    let stage = "reference.analyze";
    let store = ctx.store()?;
    let set = load_set(&store, &i.name, stage)?;
    let paths = ToolPaths::from_home(&ctx.home);
    let take = i.max_videos.unwrap_or(usize::MAX).max(1);
    let chosen: Vec<&RefVideo> = set.videos.iter().take(take).collect();
    let engine = spawn_engine(&paths).await?;
    let mut notes = Vec::new();
    let mut profiles: Vec<Value> = Vec::new();
    let total = chosen.len();
    let mut failure: Option<RustlerError> = None;
    for (n, v) in chosen.iter().enumerate() {
        ctx.progress.emit(ProgressEvent { stage: stage.into(), pct: Some(n as f32 / total as f32), message: Some(format!("{} ({}/{})", v.file, n + 1, total)), eta_s: None });
        let mut params = json!({"video_path": store.resolve(&v.proxy), "max_seconds": set.max_seconds, "faces": !i.no_faces.unwrap_or(false)});
        if let Some(a) = &v.audio {
            params["audio_path"] = json!(store.resolve(a));
        }
        let measures: Value = match engine.call::<_, Value>("reference.measure", params, &ctx.progress, &ctx.cancel).await {
            Ok(m) => m,
            Err(e) if e.code == ErrorCode::Cancelled => {
                engine.shutdown().await;
                return Err(e);
            }
            Err(e) => {
                notes.push(format!("{}: measuring failed: {}", v.file, e.message));
                failure = Some(e);
                continue;
            }
        };
        let cues = sidecar_cues(&v.path);
        let speech = cues.as_ref().and_then(|c| speech_from_cues(c, v.probe.duration, set.max_seconds));
        if speech.is_none() {
            notes.push(format!("{}: no .srt/.vtt next to the video, speech rate skipped", v.file));
        }
        let loud = if v.probe.has_audio { loudness(&paths, &ctx, &v.path, set.max_seconds).await } else { None };
        if loud.is_none() {
            notes.push(format!("{}: no measurable audio, loudness skipped", v.file));
        }
        let (contact, strip) = make_sheets(&paths, &store, &set, v, cues.as_ref()).await?;
        if let Some(n) = measures.pointer("/faces/note").and_then(Value::as_str) {
            notes.push(format!("{}: {n}", v.file));
        }
        if measures.get("truncated").and_then(Value::as_bool) == Some(true) {
            notes.push(format!("{}: only the first {:.0} s were analysed", v.file, set.max_seconds));
        }
        let mut p = serde_json::Map::new();
        p.insert("id".into(), json!(v.id));
        p.insert("file".into(), json!(v.file));
        p.insert("duration_s".into(), json!(measures["duration"]));
        p.insert("size".into(), json!({"w": v.probe.width, "h": v.probe.height, "fps": v.probe.fps}));
        for k in ["cuts", "shots", "layout_distribution", "zoom", "text", "faces", "music"] {
            if let Some(x) = measures.get(k) {
                p.insert(k.into(), x.clone());
            }
        }
        p.insert("loudness".into(), loud.unwrap_or(Value::Null));
        p.insert("speech".into(), speech.unwrap_or(Value::Null));
        p.insert("contact_sheet".into(), json!(contact));
        p.insert("first_3s_filmstrip".into(), json!(strip));
        profiles.push(Value::Object(p));
    }
    engine.shutdown().await;
    if profiles.is_empty() {
        return Err(failure.unwrap_or_else(|| internal(stage, "nothing was analysed")));
    }
    if set.videos.len() > chosen.len() {
        notes.push(format!("analysed {} of {} videos (max_videos)", chosen.len(), set.videos.len()));
    }
    notes.push("Numbers are measurements from a low-res proxy with light heuristics; confirm them against the contact sheets and filmstrips before writing a template".into());
    let agg = aggregate(&profiles);
    let mut image: Vec<PathBuf> = profiles.iter().filter_map(|p| p["contact_sheet"].as_str().map(PathBuf::from)).collect();
    image.extend(profiles.iter().filter_map(|p| p["first_3s_filmstrip"].as_str().map(PathBuf::from)));
    let full = json!({"name": set.name, "analyzed": now_iso(), "aggregate": agg, "videos": profiles, "notes": notes});
    let rel = set_rel(&set.name).join("profile.json");
    store.write_json(&rel, &full)?;
    Ok(AnalyzeOut {
        name: set.name,
        image,
        profile_path: store.resolve(&rel),
        aggregate: full["aggregate"].clone(),
        videos: full["videos"].as_array().cloned().unwrap_or_default(),
        notes: full["notes"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let ro = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false };
    let write = Annotations { read_only: false, ..ro };
    let long = Annotations { long_running: true, ..write };
    reg.register(typed(
        "reference.import",
        "Create a style-reference set from a folder (or files) of example videos: probes, low-res proxies and audio, stored under <project>/references/<name>/. Originals are never touched",
        &["name"],
        long,
        vec![
            Example { description: "all videos of a folder", cli: "rustler reference import shorts-i-like --folder C:\\refs\\shorts" },
            Example { description: "explicit files", cli: "rustler reference import hormozi --input-json @refs.json" },
        ],
        reference_import,
    ));
    reg.register(typed("reference.list", "List the project's style-reference sets", &[], ro, vec![Example { description: "all sets", cli: "rustler reference list" }], reference_list));
    reg.register(typed(
        "reference.show",
        "One reference set: its videos, the saved profile (if analysed) and the contact sheets/filmstrips as images",
        &["name"],
        ro,
        vec![Example { description: "look at one video again", cli: "rustler reference show shorts-i-like --video v02" }],
        reference_show,
    ));
    reg.register(typed(
        "reference.analyze",
        "Measure a reference set (cuts/min, shot lengths, caption band, hook text, zoom rate, face layout, loudness, music, speech rate) and render a contact sheet + first-3-seconds filmstrip per video to LOOK at; saves profile.json",
        &["name"],
        long,
        vec![Example { description: "profile a set", cli: "rustler reference analyze shorts-i-like" }],
        reference_analyze,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_stats() {
        assert!(valid_name("my-set_1.v2") && !valid_name("") && !valid_name("../x") && !valid_name(".hid") && !valid_name("a b"));
        let s = num_stats(&[3.0, 1.0, 2.0, 10.0]);
        assert_eq!(s["median"], 2.5);
        assert_eq!(s["min"], 1.0);
        assert_eq!(num_stats(&[]), json!({"count": 0}));
    }

    #[test]
    fn subtitles_and_speech() {
        let srt = "1\n00:00:00,150 --> 00:00:00,390\nNow for\n\n2\n00:00:01,000 --> 00:00:02,000\nexample in the\nVietnamese\n";
        let c = parse_subtitles(srt);
        assert_eq!(c.len(), 2);
        assert!((c[0].0 - 0.15).abs() < 1e-9 && (c[1].1 - 2.0).abs() < 1e-9);
        assert_eq!(c[1].2, "example in the Vietnamese");
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.500 align:start\nhello there friend\n";
        assert_eq!(parse_subtitles(vtt)[0].2, "hello there friend");
        let sp = speech_from_cues(&c, 10.0, 180.0).unwrap();
        assert_eq!(sp["words"], 6);
        // 6 words over 0.24 + 1.0 s of speech (gap of 0.61 s is not bridged) = 290 wpm while speaking, 36 wpm over 10 s
        assert_eq!(sp["wpm_while_speaking"], 290.0);
        assert_eq!(sp["wpm_over_video"], 36.0);
        assert!(speech_from_cues(&[], 10.0, 10.0).is_none());
    }

    #[test]
    fn ebur128_summary() {
        let s = "[Parsed_ebur128_0 @ 0x1] t: 3 M: -20 I: -30.0 LUFS\n[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -14.2 LUFS\n    Threshold: -24.2 LUFS\n\n  Loudness range:\n    LRA:         5.1 LU\n\n  True peak:\n    Peak:       -0.9 dBFS\n";
        let (i, lra, peak) = parse_ebur128(s).unwrap();
        assert_eq!((i, lra, peak), (-14.2, Some(5.1), Some(-0.9)));
        assert!(parse_ebur128("no summary here").is_none());
    }

    #[test]
    fn colors_merge_across_videos() {
        let c = vec![("#ffffff".into(), 0.5), ("#f5f5f0".into(), 0.4), ("#4df873".into(), 0.45), ("#48eb6c".into(), 0.4)];
        let m = merge_colors(&c, 4);
        assert_eq!(m.len(), 2, "{m:?}");
        assert_eq!(m[0]["hex"], "#ffffff");
        assert_eq!(m[1]["hex"], "#4df873");
    }

    #[test]
    fn aggregate_medians_and_summary() {
        let v = |cpm: f64, y: f64| {
            json!({"duration_s": 30.0, "cuts": {"per_minute": cpm}, "shots": [{"start": 0.0, "end": 5.0}, {"start": 5.0, "end": 8.0}],
                   "layout_distribution": {"face_closeup": 2}, "zoom": {"per_minute": 1.0, "coverage": 0.5},
                   "text": {"caption": {"present": true, "y_center": y, "height": 0.04, "words_per_line": {"median": 2.0}, "fill_colors": [{"hex": "#ffffff", "share": 1.0}]},
                            "hook": {"present": false}}})
        };
        let a = aggregate(&[v(6.0, 0.70), v(10.0, 0.72), v(8.0, 0.71)]);
        assert_eq!(a["cuts_per_minute"]["median"], 8.0);
        assert_eq!(a["caption"]["present_in"], 3);
        assert_eq!(a["layout_share"]["face_closeup"], 1.0);
        assert!(a["summary"].as_array().unwrap().iter().any(|s| s.as_str().unwrap().contains("captions in 3/3")));
    }
}
