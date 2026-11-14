//! Commands: see.frame, see.tiles, see.preview (Task 10b).
//!
//! `see.frame` also looks at a clip's rendered/preview frame (`clip_id` + `out_t`), and
//! `see.tiles` can center on a transcript unit (`unit`) or a phrase (`around`).

use crate::clip_cmds::{fmt_ts, load_transcript};
use rustler_core::units::build_units;
use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError, Source, SourceId, Word};
use rustler_media::{TileLabel, TileSpec};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested below)
// ---------------------------------------------------------------------------------------------

/// Lowercase alphanumeric tokens (Devanagari letters and signs kept), punctuation dropped.
fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || ('\u{0900}'..='\u{097F}').contains(&c) && c != '।' && c != '॥' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Source-time span of the first place `phrase` is spoken (exact token run, else the best fuzzy
/// window with `normalized_levenshtein >= 0.8`).
pub fn find_phrase(words: &[Word], phrase: &str) -> Option<(f64, f64)> {
    let q = tokens(phrase);
    if q.is_empty() || words.is_empty() {
        return None;
    }
    // One entry per spoken token, remembering which word it came from.
    let mut toks: Vec<(String, usize)> = Vec::new();
    for (wi, w) in words.iter().enumerate() {
        for t in tokens(&w.text) {
            toks.push((t, wi));
        }
    }
    let n = q.len();
    if toks.len() < n {
        return None;
    }
    let span = |i: usize| (words[toks[i].1].start, words[toks[i + n - 1].1].end);
    if let Some(i) = (0..=toks.len() - n).find(|&i| (0..n).all(|k| toks[i + k].0 == q[k])) {
        return Some(span(i));
    }
    let want = q.join(" ");
    let mut best: Option<(f64, usize)> = None;
    for i in 0..=toks.len() - n {
        let got = toks[i..i + n].iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(" ");
        let score = strsim::normalized_levenshtein(&got, &want);
        if score >= 0.8 && best.is_none_or(|(b, _)| score > b) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| span(i))
}

/// `start..end` for a tile sheet: explicit `start`/`end` win, else `center +- window/2` (shifted to
/// stay inside the source), else the whole source.
pub fn tile_range(duration: f64, start: Option<f64>, end: Option<f64>, center: Option<f64>, window: f64) -> Result<(f64, f64)> {
    if duration <= 0.0 || !duration.is_finite() {
        return Err(validation("see.tiles", "source has no duration"));
    }
    if start.is_none() && end.is_none() {
        if let Some(c) = center {
            let w = window.clamp(1.0, duration);
            let mut s = c - w / 2.0;
            let mut e = c + w / 2.0;
            if s < 0.0 {
                e -= s;
                s = 0.0;
            }
            if e > duration {
                s = (s - (e - duration)).max(0.0);
                e = duration;
            }
            return Ok((s, e));
        }
    }
    let s = start.unwrap_or(0.0).max(0.0);
    let e = end.unwrap_or(duration).min(duration);
    if !(s.is_finite() && e.is_finite()) || e <= s {
        return Err(validation("see.tiles", format!("empty range {s:.2}..{e:.2} (source is {duration:.2}s long)")));
    }
    Ok((s, e))
}

/// Words spoken within `+-1 s` of `t`, for the tile label strip.
pub fn words_around(words: &[Word], t: f64, max_words: usize) -> String {
    words
        .iter()
        .filter(|w| w.end >= t - 1.0 && w.start <= t + 1.0)
        .take(max_words)
        .map(|w| w.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

/// All `.ttf`/`.otf` files below `dir`, sorted.
pub(crate) fn find_fonts(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(&p, out, depth + 1);
            } else if p.is_file() {
                let ext = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
                if matches!(ext.as_deref(), Some("ttf" | "otf")) {
                    out.push(p);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out, 0);
    out.sort();
    out
}

/// A Latin-capable font for tile labels: `<home>/fonts/**` first, then common system fonts.
fn label_font(paths: &ToolPaths) -> Result<PathBuf> {
    let mut found = find_fonts(&paths.fonts);
    found.retain(|p| !p.to_string_lossy().to_lowercase().contains("devanagari"));
    found.sort_by_key(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        // Prefer plain text faces over display faces.
        if n.contains("montserrat") { 0 } else if n.contains("anton") { 2 } else { 1 }
    });
    if let Some(p) = found.into_iter().next() {
        return Ok(p);
    }
    for p in [
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ] {
        if Path::new(p).is_file() {
            return Ok(PathBuf::from(p));
        }
    }
    Err(RustlerError::new(ErrorCode::DependencyMissing, "see.tiles", "no font available for tile labels")
        .hint("run `rustler setup` to install fonts"))
}

fn analysis_see_dir(store: &ProjectStore, id: &SourceId) -> Result<PathBuf> {
    let dir = store.analysis_dir(id).join("see");
    std::fs::create_dir_all(&dir)
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "see", format!("create {}: {e}", dir.display())))?;
    Ok(dir)
}

/// The video to look at: the proxy when it exists (fast), else the original.
fn viewing_media(store: &ProjectStore, s: &Source, prefer_proxy: bool) -> PathBuf {
    if prefer_proxy {
        if let Some(p) = s.proxy.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()) {
            return p;
        }
    }
    store.resolve(&s.media_path)
}

fn require_video(s: &Source, stage: &str) -> Result<()> {
    if s.probe.has_video {
        Ok(())
    } else {
        Err(validation(stage, format!("source {} has no video", s.id)).hint("audio-only sources have nothing to look at"))
    }
}

// ---------------------------------------------------------------------------------------------
// see.frame
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct FrameIn {
    /// Source to look at (with `t`).
    pub source_id: Option<SourceId>,
    /// Source time in seconds.
    pub t: Option<f64>,
    /// Pixel width of the image (default 640).
    pub width: Option<u32>,
    /// Look at a clip instead (with `out_t`): the rendered preview frame of the clip.
    pub clip_id: Option<ClipId>,
    /// Output time in seconds of the clip.
    pub out_t: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct FrameOut {
    /// Absolute path of the jpg.
    pub image: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<ClipId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_t: Option<f64>,
}

async fn see_frame(ctx: Ctx, i: FrameIn) -> Result<FrameOut> {
    if let (Some(clip_id), Some(out_t)) = (&i.clip_id, i.out_t) {
        let image = crate::render_cmds::preview_frame_image(&ctx, clip_id, out_t).await?;
        return Ok(FrameOut { image, source_id: None, t: None, clip_id: Some(clip_id.clone()), out_t: Some(out_t) });
    }
    if i.clip_id.is_some() || i.out_t.is_some() {
        return Err(validation("see.frame", "clip frames need both --clip-id and --out-t"));
    }
    let (Some(source_id), Some(t)) = (i.source_id, i.t) else {
        return Err(validation("see.frame", "give --source-id and --t (source seconds), or --clip-id and --out-t")
            .hint("e.g. `rustler see frame src_x --t 47.2`"));
    };
    let store = ctx.store()?;
    let source = store.load_source(&source_id)?;
    require_video(&source, "see.frame")?;
    if t < 0.0 || t > source.probe.duration + 0.05 {
        return Err(validation(
            "see.frame",
            format!("t={t:.2}s is outside the source (0..{:.2}s)", source.probe.duration),
        ));
    }
    let width = i.width.unwrap_or(640).clamp(16, 3840);
    let out = analysis_see_dir(&store, &source_id)?.join(format!("frame_{}ms_w{width}.jpg", (t * 1000.0).round() as u64));
    let paths = ToolPaths::from_home(&ctx.home);
    rustler_media::frame(&paths, &viewing_media(&store, &source, false), t, width, &out).await?;
    Ok(FrameOut { image: out, source_id: Some(source_id), t: Some(t), clip_id: None, out_t: None })
}

// ---------------------------------------------------------------------------------------------
// see.tiles
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct TilesIn {
    pub source_id: SourceId,
    /// Source seconds; default 0.
    pub start: Option<f64>,
    /// Source seconds; default end of source.
    pub end: Option<f64>,
    /// Center the sheet on the first place this phrase is spoken.
    pub around: Option<String>,
    /// Center the sheet on this transcript unit.
    pub unit: Option<u32>,
    /// Seconds covered when centering with `around`/`unit` (default 20).
    pub window: Option<f64>,
    /// Number of tiles (default 12).
    pub count: Option<u32>,
    /// Columns (default 4).
    pub cols: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
pub struct TilesOut {
    /// Absolute path of the contact sheet (jpg). LOOK at it.
    pub image: PathBuf,
    pub start: f64,
    pub end: f64,
    pub tiles: Vec<TileLabel>,
    /// What the sheet was centered on, if anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centered_on: Option<String>,
}

async fn see_tiles(ctx: Ctx, i: TilesIn) -> Result<TilesOut> {
    let store = ctx.store()?;
    let source = store.load_source(&i.source_id)?;
    require_video(&source, "see.tiles")?;
    if i.unit.is_some() && i.around.is_some() {
        return Err(validation("see.tiles", "use either --unit or --around, not both"));
    }
    let transcript = load_transcript(&store, &source.id)?;
    let words: Vec<Word> = transcript.map(|t| t.words).unwrap_or_default();

    let mut center = None;
    let mut centered_on = None;
    if let Some(uid) = i.unit {
        let units = build_units(&words);
        let u = units.iter().find(|u| u.id == uid).ok_or_else(|| {
            validation("see.tiles", format!("unit {uid} does not exist ({} units)", units.len()))
                .hint("list units with `rustler transcript get <source_id>`")
        })?;
        center = Some((u.start + u.end) / 2.0);
        centered_on = Some(format!("unit {uid}: {}", u.text));
    } else if let Some(phrase) = i.around.as_deref() {
        let (s, e) = find_phrase(&words, phrase).ok_or_else(|| {
            RustlerError::new(ErrorCode::NotFound, "see.tiles", format!("phrase '{phrase}' not found in the transcript"))
                .hint("check the spelling with `rustler transcript search <source_id> --query ...`, or pass --start/--end")
        })?;
        center = Some((s + e) / 2.0);
        centered_on = Some(format!("'{phrase}' at {}", fmt_ts(s)));
    }
    let (start, end) = tile_range(source.probe.duration, i.start, i.end, center, i.window.unwrap_or(20.0))?;

    let paths = ToolPaths::from_home(&ctx.home);
    let font = label_font(&paths)?;
    let spec = TileSpec { start, end, count: i.count.unwrap_or(12), cols: i.cols.unwrap_or(4), ..Default::default() };
    let out = analysis_see_dir(&store, &source.id)?
        .join(format!("tiles_{}_{}_{}.jpg", (start * 1000.0).round() as u64, (end * 1000.0).round() as u64, spec.count));
    let labels = move |t: f64| words_around(&words, t, 8);
    let tiles = rustler_media::tiles(&paths, &viewing_media(&store, &source, true), &spec, &labels, &font, &out).await?;
    Ok(TilesOut { image: out, start, end, tiles, centered_on })
}

// ---------------------------------------------------------------------------------------------
// see.preview
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct PreviewIn {
    pub source_id: SourceId,
    pub start: f64,
    pub end: f64,
    /// Pixel height (default 360).
    pub height: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
pub struct PreviewOut {
    /// Absolute path of the low-res mp4.
    pub video: PathBuf,
    pub start: f64,
    pub end: f64,
}

async fn see_preview(ctx: Ctx, i: PreviewIn) -> Result<PreviewOut> {
    let store = ctx.store()?;
    let source = store.load_source(&i.source_id)?;
    require_video(&source, "see.preview")?;
    if i.start < 0.0 || i.end <= i.start || i.end > source.probe.duration + 0.05 {
        return Err(validation(
            "see.preview",
            format!("need 0 <= start < end <= {:.2}s (got {}..{})", source.probe.duration, i.start, i.end),
        ));
    }
    let out = analysis_see_dir(&store, &source.id)?
        .join(format!("preview_{}_{}.mp4", (i.start * 1000.0).round() as u64, (i.end * 1000.0).round() as u64));
    let paths = ToolPaths::from_home(&ctx.home);
    rustler_media::preview(&paths, &viewing_media(&store, &source, true), i.start, i.end, i.height.unwrap_or(360), &out).await?;
    Ok(PreviewOut { video: out, start: i.start, end: i.end })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

const READ: Annotations = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false };

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "see.frame",
        "One frame as a jpg: a source frame (source_id + t) or a clip's preview frame (clip_id + out_t)",
        &["source_id"],
        READ,
        vec![
            Example { description: "look at the source at 47.2 s", cli: "rustler see frame src_x --t 47.2" },
            Example { description: "look at the clip at output second 3", cli: "rustler see frame --clip-id clp_x --out-t 3" },
        ],
        see_frame,
    ));
    reg.register(typed(
        "see.tiles",
        "Contact sheet of evenly spaced frames labelled with time and words; center it on a unit or a phrase",
        &["source_id"],
        READ,
        vec![
            Example { description: "20 s around a phrase", cli: "rustler see tiles src_x --around \"fellow Americans\" --window 20" },
            Example { description: "around transcript unit 12", cli: "rustler see tiles src_x --unit 12" },
        ],
        see_tiles,
    ));
    reg.register(typed(
        "see.preview",
        "Low-res mp4 of a source range (max 60 s)",
        &["source_id"],
        READ,
        vec![Example { description: "30 s preview", cli: "rustler see preview src_x --start 120 --end 150" }],
        see_preview,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
        Word { i, text: text.into(), start, end, conf: None, speaker: None }
    }

    fn words() -> Vec<Word> {
        vec![
            w(0, "Ask", 10.0, 10.3),
            w(1, "not", 10.4, 10.7),
            w(2, "what", 10.8, 11.0),
            w(3, "your", 11.1, 11.3),
            w(4, "country,", 11.4, 11.9),
            w(5, "can", 12.0, 12.2),
            w(6, "do.", 12.3, 12.6),
        ]
    }

    #[test]
    fn phrase_exact_and_fuzzy() {
        let ws = words();
        assert_eq!(find_phrase(&ws, "what your COUNTRY"), Some((10.8, 11.9)));
        // transcription typo ("contry") still found
        let (s, e) = find_phrase(&ws, "your contry can").unwrap();
        assert!((s - 11.1).abs() < 1e-9 && (e - 12.2).abs() < 1e-9, "{s} {e}");
        assert_eq!(find_phrase(&ws, "completely different words"), None);
        assert_eq!(find_phrase(&ws, "   "), None);
        assert_eq!(find_phrase(&[], "ask"), None);
    }

    #[test]
    fn tile_range_rules() {
        // whole source by default
        assert_eq!(tile_range(100.0, None, None, None, 20.0).unwrap(), (0.0, 100.0));
        // centered window
        assert_eq!(tile_range(100.0, None, None, Some(50.0), 20.0).unwrap(), (40.0, 60.0));
        // shifted at the edges, width preserved
        assert_eq!(tile_range(100.0, None, None, Some(2.0), 20.0).unwrap(), (0.0, 20.0));
        let (s, e) = tile_range(100.0, None, None, Some(99.0), 20.0).unwrap();
        assert!((s - 80.0).abs() < 1e-9 && (e - 100.0).abs() < 1e-9);
        // explicit start/end win over center
        assert_eq!(tile_range(100.0, Some(5.0), Some(15.0), Some(50.0), 20.0).unwrap(), (5.0, 15.0));
        // window larger than the source
        assert_eq!(tile_range(10.0, None, None, Some(5.0), 60.0).unwrap(), (0.0, 10.0));
        assert!(tile_range(100.0, Some(50.0), Some(40.0), None, 20.0).is_err());
        assert!(tile_range(0.0, None, None, None, 20.0).is_err());
    }

    #[test]
    fn labels_use_words_near_t() {
        let ws = words();
        assert_eq!(words_around(&ws, 11.0, 8), "Ask not what your country, can");
        assert_eq!(words_around(&ws, 50.0, 8), "");
        assert_eq!(words_around(&ws, 11.0, 2), "Ask not");
    }

    #[test]
    fn fonts_found_recursively() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("anton")).unwrap();
        std::fs::write(d.path().join("anton").join("Anton-Regular.ttf"), b"x").unwrap();
        std::fs::write(d.path().join("readme.txt"), b"x").unwrap();
        let f = find_fonts(d.path());
        assert_eq!(f.len(), 1);
        assert!(f[0].ends_with("Anton-Regular.ttf"));
    }
}
