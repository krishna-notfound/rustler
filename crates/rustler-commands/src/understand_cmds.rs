//! Commands: analyze.ocr, analyze.events, analyze.faces, analyze.deep (understanding pack v2).
//!
//! The agent understands a video beyond speech: what text is on screen, what the audio does
//! (laughter, applause, music, speech) and when faces are visible. All three run locally in the
//! Python engine (RapidOCR, YAMNet, YuNet); nothing calls an AI API. Results live in
//! `analysis/<source_id>/`: `ocr.json`, `events.json`, `faces.json` and (from `analyze.deep`)
//! `understanding.json`.

use crate::analyze_cmds::spawn_engine;
use crate::project_cmds::{Outcome, log_event, mmss, validation};
use crate::source_cmds::load_source_checked;
use crate::transcript_cmds::{load_units, unit_at};
use rustler_core::{ProjectStore, RustlerError, Source, SourceId, Unit};
use rustler_engine_bridge::EngineClient;
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Upper bound of OCR samples per run; `every` is raised automatically beyond this.
const MAX_OCR_SAMPLES: f64 = 1500.0;
/// Upper bound of face samples per run (engine limit is 20000).
const MAX_FACE_SAMPLES: f64 = 18000.0;

fn rel(source: &Source, file: &str) -> PathBuf {
    Path::new("analysis").join(source.id.as_str()).join(file)
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn clip_text(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
}

fn err_note(e: &RustlerError) -> String {
    match &e.hint {
        Some(h) if !h.is_empty() => format!("{} ({h})", e.message),
        _ => e.message.clone(),
    }
}

fn video_for_ocr(store: &ProjectStore, source: &Source) -> PathBuf {
    // OCR reads the original (the proxy is only 540p and thin text gets lost).
    store.resolve(&source.media_path)
}

fn video_for_faces(store: &ProjectStore, source: &Source) -> PathBuf {
    source.proxy.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()).unwrap_or_else(|| store.resolve(&source.media_path))
}

fn require_video(stage: &str, source: &Source) -> rustler_core::Result<()> {
    if source.probe.has_video {
        return Ok(());
    }
    Err(validation(stage, format!("source {} has no video stream", source.id), "this needs video; for audio-only sources use `rustler analyze events` and `analyze transcribe`"))
}

// ---------------------------------------------------------------------------------------
// analyze.ocr
// ---------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct OcrBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    /// Normalized 0..1 box of the frame (`x`,`y` = top-left).
    #[serde(rename = "box")]
    pub bbox: OcrBox,
    pub conf: f64,
}

/// A run of samples that showed the same text: first seen at `t`, last seen at `t_end`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct OcrFrame {
    pub t: f64,
    #[serde(default)]
    pub t_end: Option<f64>,
    pub lines: Vec<OcrLine>,
}

/// `analysis/<id>/ocr.json`
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct OcrResult {
    pub frames: Vec<OcrFrame>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub samples: usize,
    #[serde(default)]
    pub every: f64,
    #[serde(default)]
    pub engine: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct OcrIn {
    pub source_id: SourceId,
    /// Seconds between sampled frames (default 2.0; raised automatically on very long videos).
    pub every: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct OcrText {
    pub text: String,
    pub first_seen: f64,
    pub last_seen: f64,
    pub first_mmss: String,
    pub last_mmss: String,
    /// Separate appearances (a text that disappears and returns counts twice).
    pub appearances: usize,
    /// Approximate total seconds on screen.
    pub seconds_visible: f64,
    /// Mean vertical centre, 0 (top) .. 1 (bottom).
    pub y: f64,
    pub conf: f64,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct OcrRegion {
    /// `burned-in-captions` | `persistent-overlay` | `timecode-counter` | `on-screen-text`.
    pub kind: String,
    /// Mean vertical centre of the band, 0 (top) .. 1 (bottom).
    pub y: f64,
    pub description: String,
    pub distinct_texts: usize,
    /// Share of the video during which text is visible in this band (0..100).
    pub coverage_pct: f64,
    pub examples: Vec<String>,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct OcrOut {
    pub source_id: SourceId,
    pub samples: usize,
    pub every: f64,
    pub engine: String,
    /// Texts that were seen at least once (identical text collapsed).
    pub unique_texts: usize,
    /// First 40 unique texts by first appearance.
    pub texts: Vec<OcrText>,
    pub regions: Vec<OcrRegion>,
    /// True when a band of changing subtitles is burned into the picture.
    pub burned_in_captions: bool,
    pub caption_band_y: Option<f64>,
    /// What to do about it (avoiding double captions).
    pub advice: Option<String>,
    pub path: PathBuf,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

struct TextAgg {
    text: String,
    first: f64,
    run_start: f64,
    run_end: f64,
    seen: f64,
    runs: usize,
    y_sum: f64,
    y_n: f64,
    conf_max: f64,
}

fn norm_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Folds the OCR frames into unique texts (first/last seen) and screen regions.
pub fn summarize_ocr(frames: &[OcrFrame], duration: f64, every: f64) -> (Vec<OcrText>, Vec<OcrRegion>) {
    let every = if every > 0.0 { every } else { 2.0 };
    let mut order: Vec<String> = Vec::new();
    let mut agg: BTreeMap<String, TextAgg> = BTreeMap::new();
    for f in frames {
        let t_end = f.t_end.unwrap_or(f.t).max(f.t);
        let mut seen_here: Vec<String> = Vec::new();
        for l in &f.lines {
            let key = norm_text(&l.text);
            if key.is_empty() || seen_here.contains(&key) {
                continue;
            }
            seen_here.push(key.clone());
            let yc = l.bbox.y + l.bbox.h / 2.0;
            match agg.get_mut(&key) {
                Some(a) => {
                    if f.t <= a.run_end + every * 1.01 {
                        a.run_end = a.run_end.max(t_end);
                    } else {
                        a.seen += a.run_end - a.run_start + every;
                        a.runs += 1;
                        a.run_start = f.t;
                        a.run_end = t_end;
                    }
                    a.y_sum += yc;
                    a.y_n += 1.0;
                    a.conf_max = a.conf_max.max(l.conf);
                }
                None => {
                    order.push(key.clone());
                    agg.insert(
                        key,
                        TextAgg { text: l.text.trim().to_string(), first: f.t, run_start: f.t, run_end: t_end, seen: 0.0, runs: 1, y_sum: yc, y_n: 1.0, conf_max: l.conf },
                    );
                }
            }
        }
    }
    let mut texts: Vec<OcrText> = order
        .iter()
        .map(|k| {
            let a = &agg[k];
            let seen = a.seen + (a.run_end - a.run_start + every);
            OcrText {
                text: a.text.clone(),
                first_seen: r1(a.first),
                last_seen: r1(a.run_end),
                first_mmss: mmss(a.first),
                last_mmss: mmss(a.run_end),
                appearances: a.runs,
                seconds_visible: r1(seen),
                y: r2(a.y_sum / a.y_n),
                conf: r2(a.conf_max),
            }
        })
        .collect();
    texts.sort_by(|a, b| a.first_seen.partial_cmp(&b.first_seen).unwrap_or(std::cmp::Ordering::Equal));
    (texts.clone(), regions_from_texts(&texts, duration))
}

/// Groups texts into horizontal bands and names what each band is.
fn regions_from_texts(texts: &[OcrText], duration: f64) -> Vec<OcrRegion> {
    let mut by_y: Vec<&OcrText> = texts.iter().collect();
    by_y.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
    let mut clusters: Vec<Vec<&OcrText>> = Vec::new();
    for t in by_y {
        match clusters.last_mut() {
            Some(c) if (t.y - c.last().map(|l| l.y).unwrap_or(t.y)).abs() <= 0.06 => c.push(t),
            _ => clusters.push(vec![t]),
        }
    }
    let dur = duration.max(1.0);
    let mut regions = Vec::new();
    for c in clusters {
        let y = r2(c.iter().map(|t| t.y).sum::<f64>() / c.len() as f64);
        let d = c.len();
        let covered: f64 = c.iter().map(|t| t.seconds_visible).sum::<f64>().min(dur);
        let coverage = r1(covered / dur * 100.0);
        let mut durations: Vec<f64> = c.iter().map(|t| t.seconds_visible).collect();
        let med = median(&mut durations);
        let longest = c.iter().map(|t| t.seconds_visible).fold(0.0, f64::max);
        let placement = if y < 0.33 { "top" } else if y > 0.66 { "bottom" } else { "middle" };
        let examples: Vec<String> = c.iter().take(3).map(|t| clip_text(&t.text, 50)).collect();
        let wordy = c.iter().filter(|t| t.text.chars().filter(|ch| ch.is_alphabetic()).count() >= 3).count();
        let (kind, description) = if wordy == 0 && d >= 3 {
            ("timecode-counter", format!("changing numbers (timecode, frame counter or score) at y≈{y:.2} ({placement}; {d} values, e.g. \"{}\"): not captions", clip_text(&c[0].text, 30)))
        } else if wordy >= 4 && med <= 8.0 && coverage >= 20.0 {
            ("burned-in-captions", format!("burned-in captions band at y≈{y:.2} ({placement}; {d} distinct lines, text visible {coverage:.0}% of the time)"))
        } else if longest >= 0.5 * dur && dur >= 6.0 {
            ("persistent-overlay", format!("persistent overlay at y≈{y:.2} ({placement}; e.g. watermark, channel name or title: \"{}\")", clip_text(&c[0].text, 40)))
        } else {
            ("on-screen-text", format!("on-screen text at y≈{y:.2} ({placement}; {d} text{}, e.g. \"{}\")", if d == 1 { "" } else { "s" }, clip_text(&c[0].text, 40)))
        };
        regions.push(OcrRegion { kind: kind.to_string(), y, description, distinct_texts: d, coverage_pct: coverage, examples });
    }
    regions
}

/// Runs OCR over `every`-second samples with an already running engine and saves `ocr.json`.
async fn ocr_with(ctx: &Ctx, store: &ProjectStore, engine: &EngineClient, source: &Source, every: Option<f64>) -> rustler_core::Result<OcrOut> {
    require_video("analyze.ocr", source)?;
    let mut every = every.unwrap_or(2.0);
    if !(every > 0.0) {
        return Err(validation("analyze.ocr", "every must be greater than 0", "use seconds between sampled frames, e.g. --every 2"));
    }
    let mut warnings = Vec::new();
    let dur = source.probe.duration;
    if dur > 0.0 && dur / every > MAX_OCR_SAMPLES {
        every = (dur / MAX_OCR_SAMPLES * 10.0).ceil() / 10.0;
        warnings.push(format!("long video: sampling every {every} s instead to keep OCR under {MAX_OCR_SAMPLES:.0} frames; pass --start/--end style windows via `see` for finer text checks"));
    }
    let video = video_for_ocr(store, source);
    let res: OcrResult = engine.call("ocr.run", json!({"video_path": video, "every": every, "end": if dur > 0.0 { json!(dur) } else { json!(null) }}), &ctx.progress, &ctx.cancel).await?;
    let path = rel(source, "ocr.json");
    store.write_json(&path, &res)?;
    let (texts, regions) = summarize_ocr(&res.frames, dur, every);
    let captions = regions.iter().find(|r| r.kind == "burned-in-captions");
    let caption_band_y = captions.map(|r| r.y);
    let advice = captions.map(|r| {
        let hint = if r.y > 0.5 {
            "add Rustler captions near the top instead (`rustler clip set-captions <clip> --position-y 0.2`), or only if the source subtitles will be cropped out by the layout"
        } else {
            "keep Rustler captions in the lower half (`rustler clip set-captions <clip> --position-y 0.75`) or turn them off"
        };
        format!("the source already has burned-in captions at y≈{:.2}: do not stack a second caption on that band; {hint}", r.y)
    });
    if res.frames.is_empty() {
        warnings.push("no on-screen text found (or only text the model cannot read: the OCR model reads Latin and Chinese scripts, not Devanagari)".to_string());
    }
    log_event(ctx, store, "analyze.ocr", json!({"source_id": source.id, "unique_texts": texts.len(), "burned_in_captions": caption_band_y.is_some()}))?;
    Ok(OcrOut {
        source_id: source.id.clone(),
        samples: res.samples,
        every,
        engine: res.engine,
        unique_texts: texts.len(),
        texts: texts.into_iter().take(40).collect(),
        regions,
        burned_in_captions: caption_band_y.is_some(),
        caption_band_y,
        advice,
        path,
        warnings,
        next: vec![format!("rustler analyze events {}", source.id), format!("rustler analyze faces {}", source.id)],
    })
}

pub async fn ocr(ctx: Ctx, input: OcrIn) -> rustler_core::Result<Outcome<OcrOut>> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    require_video("analyze.ocr", &source)?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"ocr": source.id, "every": input.every.unwrap_or(2.0)})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let out = ocr_with(&ctx, &store, &engine, &source, input.every).await;
    engine.shutdown().await;
    Ok(Outcome::Done(out?))
}

// ---------------------------------------------------------------------------------------
// analyze.events
// ---------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct AudioEvent {
    pub start: f64,
    pub end: f64,
    /// `laughter` | `applause` | `cheer` | `music` | `speech` | `silence`
    pub label: String,
    pub score: f64,
}

/// `analysis/<id>/events.json`
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct EventsResult {
    pub events: Vec<AudioEvent>,
    /// `yamnet` or `heuristic`.
    pub model: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub hop: f64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct EventsIn {
    pub source_id: SourceId,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct LabelCount {
    pub events: usize,
    pub seconds: f64,
    /// Share of the audio covered by this label (0..100).
    pub pct: f64,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct EventMoment {
    pub label: String,
    pub start: f64,
    pub end: f64,
    pub mmss: String,
    pub score: f64,
    /// Transcript unit being spoken at (or next to) the moment.
    pub unit_id: Option<u32>,
    pub text: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct EventsOut {
    pub source_id: SourceId,
    pub model: String,
    pub duration: f64,
    pub counts: BTreeMap<String, LabelCount>,
    /// Strongest laughter / applause / cheering moments joined with what was said (clip candidates).
    pub top_moments: Vec<EventMoment>,
    pub path: PathBuf,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

const REACTIONS: [&str; 3] = ["laughter", "applause", "cheer"];

/// Counts per label plus the strongest reaction moments, joined with transcript units when known.
pub fn summarize_events(res: &EventsResult, units: &[Unit], top: usize) -> (BTreeMap<String, LabelCount>, Vec<EventMoment>) {
    let total = if res.duration > 0.0 { res.duration } else { res.events.iter().map(|e| e.end).fold(0.0, f64::max) };
    let mut counts: BTreeMap<String, LabelCount> = BTreeMap::new();
    for e in &res.events {
        let c = counts.entry(e.label.clone()).or_insert(LabelCount { events: 0, seconds: 0.0, pct: 0.0 });
        c.events += 1;
        c.seconds += (e.end - e.start).max(0.0);
    }
    for c in counts.values_mut() {
        c.seconds = r1(c.seconds);
        c.pct = if total > 0.0 { r1(c.seconds / total * 100.0) } else { 0.0 };
    }
    let mut reactions: Vec<&AudioEvent> = res.events.iter().filter(|e| REACTIONS.contains(&e.label.as_str())).collect();
    reactions.sort_by(|a, b| {
        let da = (a.end - a.start).min(5.0);
        let db = (b.end - b.start).min(5.0);
        (b.score * (0.5 + da / 10.0)).partial_cmp(&(a.score * (0.5 + db / 10.0))).unwrap_or(std::cmp::Ordering::Equal)
    });
    let moments = reactions
        .into_iter()
        .take(top)
        .map(|e| {
            // For a reaction the line that triggered it is just before it, so look slightly back.
            let u = unit_at(units, (e.start - 3.0).max(0.0), e.end, 2.0);
            EventMoment {
                label: e.label.clone(),
                start: e.start,
                end: e.end,
                mmss: mmss(e.start),
                score: e.score,
                unit_id: u.map(|u| u.id),
                text: u.map(|u| clip_text(&u.text, 90)),
            }
        })
        .collect();
    (counts, moments)
}

async fn events_with(ctx: &Ctx, store: &ProjectStore, engine: &EngineClient, source: &Source) -> rustler_core::Result<EventsOut> {
    let wav = source.audio_wav.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()).ok_or_else(|| {
        validation("analyze.events", format!("source {} has no audio track (or its 16 kHz wav is missing)", source.id), "silent video has no audio events; re-import if the wav was deleted")
    })?;
    let res: EventsResult = engine.call("audio.events", json!({"audio_path": wav, "hop": 0.5}), &ctx.progress, &ctx.cancel).await?;
    let path = rel(source, "events.json");
    store.write_json(&path, &res)?;
    let units = load_units(store, source.id.as_str()).unwrap_or_default();
    let (counts, top_moments) = summarize_events(&res, &units, 8);
    let mut warnings = res.warnings.clone();
    if res.model == "heuristic" {
        warnings.push("heuristic classifier (no model available): only silence, speech, music and applause are reported; laughter and cheering cannot be detected".to_string());
    }
    log_event(ctx, store, "analyze.events", json!({"source_id": source.id, "model": res.model, "events": res.events.len()}))?;
    let mut next = vec![format!("rustler analyze faces {}", source.id)];
    if let Some(m) = top_moments.first() {
        next.insert(0, format!("rustler see tiles {} --start {} --end {}   (look at the strongest {})", source.id, (m.start - 5.0).max(0.0).floor(), m.end.ceil() + 2.0, m.label));
    }
    Ok(EventsOut { source_id: source.id.clone(), model: res.model, duration: res.duration, counts, top_moments, path, warnings, next })
}

pub async fn events(ctx: Ctx, input: EventsIn) -> rustler_core::Result<Outcome<EventsOut>> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if !source.probe.has_audio {
        return Err(validation("analyze.events", format!("source {} has no audio track", source.id), "nothing to classify; use `analyze ocr` and `see tiles` for silent video"));
    }
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"events": source.id})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let out = events_with(&ctx, &store, &engine, &source).await;
    engine.shutdown().await;
    Ok(Outcome::Done(out?))
}

// ---------------------------------------------------------------------------------------
// analyze.faces
// ---------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FaceBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub score: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FaceSample {
    pub t: f64,
    pub faces: Vec<FaceBox>,
}

/// `analysis/<id>/faces.json`
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FacesResult {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub fps: f64,
    pub samples: Vec<FaceSample>,
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct FacesIn {
    pub source_id: SourceId,
    /// Samples per second (default 1; lowered automatically on very long videos).
    pub fps: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct LayoutSuggestion {
    /// A preset from `layout list-presets`: `center-crop` | `blur-fit` | `split-vertical`.
    pub layout: String,
    /// Also run `crop track-face` so the crop follows the face.
    pub track_face: bool,
    /// Horizontal focus (0..1) for center-crop when the face stays put.
    pub focus_x: Option<f64>,
    pub reason: String,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct FaceSpan {
    pub start: f64,
    pub end: f64,
    pub mmss: String,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct FacesOut {
    pub source_id: SourceId,
    pub samples: usize,
    pub fps: f64,
    /// Share of samples with at least one face (0..100).
    pub face_time_pct: f64,
    /// Median face count over samples that have a face.
    pub typical_faces: u32,
    pub max_faces: u32,
    /// Median box of the largest face (normalized), when any face was found.
    pub dominant_box: Option<OcrBox>,
    /// Typical area of the largest face as a share of the frame (0..100).
    pub face_area_pct: f64,
    /// Spread (10th..90th percentile) of the largest face's horizontal centre; small = static speaker.
    pub center_x_range: Option<[f64; 2]>,
    /// First 20 stretches with a face (gaps up to 2 s bridged).
    pub spans: Vec<FaceSpan>,
    pub suggestion: LayoutSuggestion,
    pub path: PathBuf,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn area(b: &FaceBox) -> f64 {
    b.w * b.h
}

/// Face presence statistics and a layout suggestion from engine samples.
pub fn summarize_faces(res: &FacesResult, fps: f64) -> FacesOut {
    let n = res.samples.len();
    let with: Vec<&FaceSample> = res.samples.iter().filter(|s| !s.faces.is_empty()).collect();
    let pct = if n == 0 { 0.0 } else { r1(with.len() as f64 / n as f64 * 100.0) };
    let mut counts: Vec<f64> = with.iter().map(|s| s.faces.len() as f64).collect();
    let typical = median(&mut counts).round() as u32;
    let max_faces = with.iter().map(|s| s.faces.len() as u32).max().unwrap_or(0);

    let biggest: Vec<&FaceBox> = with.iter().filter_map(|s| s.faces.iter().max_by(|a, b| area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal))).collect();
    let dominant_box = if biggest.is_empty() {
        None
    } else {
        let m = |f: fn(&FaceBox) -> f64| median(&mut biggest.iter().map(|b| f(b)).collect::<Vec<_>>());
        Some(OcrBox { x: r2(m(|b| b.x)), y: r2(m(|b| b.y)), w: r2(m(|b| b.w)), h: r2(m(|b| b.h)) })
    };
    let face_area_pct = r1(median(&mut biggest.iter().map(|b| area(b)).collect::<Vec<_>>()) * 100.0);
    let mut cx: Vec<f64> = biggest.iter().map(|b| b.x + b.w / 2.0).collect();
    cx.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let center_x_range = if cx.is_empty() { None } else { Some([r2(percentile(&cx, 0.1)), r2(percentile(&cx, 0.9))]) };

    // stretches with a face, bridging gaps up to 2 s
    let mut spans: Vec<FaceSpan> = Vec::new();
    let step = if fps > 0.0 { 1.0 / fps } else { 1.0 };
    let mut cur: Option<(f64, f64)> = None;
    for s in &with {
        match cur {
            Some((a, b)) if s.t - b <= 2.0 + 1e-6 => cur = Some((a, s.t)),
            Some((a, b)) => {
                spans.push(FaceSpan { start: r1(a), end: r1(b + step), mmss: mmss(a) });
                cur = Some((s.t, s.t));
            }
            None => cur = Some((s.t, s.t)),
        }
    }
    if let Some((a, b)) = cur {
        spans.push(FaceSpan { start: r1(a), end: r1(b + step), mmss: mmss(a) });
    }
    spans.truncate(20);

    let suggestion = suggest_layout(res, &with, pct, typical, face_area_pct, dominant_box.as_ref(), center_x_range);
    FacesOut {
        source_id: SourceId::default(),
        samples: n,
        fps,
        face_time_pct: pct,
        typical_faces: typical,
        max_faces,
        dominant_box,
        face_area_pct,
        center_x_range,
        spans,
        suggestion,
        path: PathBuf::new(),
        warnings: Vec::new(),
        next: Vec::new(),
    }
}

fn suggest_layout(res: &FacesResult, with: &[&FaceSample], pct: f64, typical: u32, area_pct: f64, dom: Option<&OcrBox>, cxr: Option<[f64; 2]>) -> LayoutSuggestion {
    let landscape = res.width as f64 >= res.height as f64 * 1.2;
    let sug = |layout: &str, track: bool, focus: Option<f64>, reason: String| LayoutSuggestion { layout: layout.to_string(), track_face: track, focus_x: focus, reason };
    let Some(dom) = dom else {
        return sug("blur-fit", false, None, "no faces found: keep the whole frame with blur-fit, or center-crop if the action sits in the middle (look with `see tiles`)".to_string());
    };
    if pct < 20.0 {
        return sug("blur-fit", false, None, format!("faces appear in only {pct:.0}% of samples: face tracking would mostly hold the centre; blur-fit keeps the whole frame"));
    }
    if !landscape {
        return sug("center-crop", false, Some(0.5), "the source is already vertical or square: center-crop needs no reframing".to_string());
    }
    // two people side by side: split the frame into two panes
    if typical >= 2 {
        let spread: Vec<f64> = with
            .iter()
            .filter(|s| s.faces.len() >= 2)
            .map(|s| {
                let mut f: Vec<&FaceBox> = s.faces.iter().collect();
                f.sort_by(|a, b| area(b).partial_cmp(&area(a)).unwrap_or(std::cmp::Ordering::Equal));
                ((f[0].x + f[0].w / 2.0) - (f[1].x + f[1].w / 2.0)).abs()
            })
            .collect();
        let mut sp = spread;
        if median(&mut sp) >= 0.25 {
            return sug("split-vertical", false, None, format!("typically {typical} faces, well apart horizontally: split-vertical with one pane per face (`--top`/`--bottom` rects around each)"));
        }
    }
    // small face in a corner = streamer webcam over other content
    let (cx, cy) = (dom.x + dom.w / 2.0, dom.y + dom.h / 2.0);
    let corner = (cx < 0.3 || cx > 0.7) && (cy < 0.3 || cy > 0.6);
    if typical <= 1 && area_pct < 4.0 && corner {
        return sug("split-vertical", false, None, format!("one small face (about {area_pct:.1}% of the frame) in a corner at x≈{cx:.2}, y≈{cy:.2}: looks like a webcam overlay; split-vertical with the webcam pane on top and the main content below"));
    }
    let range = cxr.map(|r| r[1] - r[0]).unwrap_or(1.0);
    if range <= 0.2 {
        let near_center = (0.35..=0.65).contains(&cx);
        let reason = if near_center {
            format!("one speaker who stays put near the centre (x≈{cx:.2}, within {range:.2} of travel): plain center-crop is enough")
        } else {
            format!("one speaker who stays put at x≈{cx:.2}: center-crop with that horizontal focus")
        };
        return sug("center-crop", false, Some(r2(cx)), reason);
    }
    sug("center-crop", true, Some(r2(cx)), format!("a speaker who moves ({range:.2} of the frame width): center-crop plus `crop track-face` so the crop follows the face"))
}

async fn faces_with(ctx: &Ctx, store: &ProjectStore, engine: &EngineClient, source: &Source, fps: Option<f64>) -> rustler_core::Result<FacesOut> {
    require_video("analyze.faces", source)?;
    let mut fps = fps.unwrap_or(1.0);
    if !(fps > 0.0 && fps <= 10.0) {
        return Err(validation("analyze.faces", "fps must be in (0, 10]", "1 sample per second is plenty for presence statistics"));
    }
    let mut warnings = Vec::new();
    let dur = source.probe.duration;
    if dur > 0.0 && dur * fps > MAX_FACE_SAMPLES {
        fps = (MAX_FACE_SAMPLES / dur * 1000.0).floor() / 1000.0;
        warnings.push(format!("long video: sampling at {fps} samples/s to stay under {MAX_FACE_SAMPLES:.0} samples"));
    }
    let video = video_for_faces(store, source);
    let mut params = json!({"video_path": video, "fps": fps});
    if dur > 0.0 {
        params["end"] = json!(dur);
    }
    let mut res: FacesResult = engine.call("faces.track", params, &ctx.progress, &ctx.cancel).await?;
    res.fps = fps;
    let path = rel(source, "faces.json");
    store.write_json(&path, &res)?;
    let mut out = summarize_faces(&res, fps);
    out.source_id = source.id.clone();
    out.path = path;
    if res.samples.is_empty() {
        warnings.push("no frames could be read for face detection".to_string());
    }
    out.next = vec![format!("rustler analyze deep {}", source.id), format!("rustler see tiles {}", source.id)];
    out.warnings = warnings;
    log_event(ctx, store, "analyze.faces", json!({"source_id": source.id, "face_time_pct": out.face_time_pct, "layout": out.suggestion.layout}))?;
    Ok(out)
}

pub async fn faces(ctx: Ctx, input: FacesIn) -> rustler_core::Result<Outcome<FacesOut>> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    require_video("analyze.faces", &source)?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"faces": source.id, "fps": input.fps.unwrap_or(1.0)})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let out = faces_with(&ctx, &store, &engine, &source, input.fps).await;
    engine.shutdown().await;
    Ok(Outcome::Done(out?))
}

// ---------------------------------------------------------------------------------------
// analyze.deep
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct DeepIn {
    pub source_id: SourceId,
    /// OCR sampling interval in seconds (default 2.0).
    pub every: Option<f64>,
    /// Face samples per second (default 1).
    pub fps: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct DeepOut {
    pub source_id: SourceId,
    pub duration: f64,
    /// Plain-language findings in one place: read these first.
    pub understanding: Vec<String>,
    pub ocr: Option<OcrOut>,
    pub events: Option<EventsOut>,
    pub faces: Option<FacesOut>,
    pub path: PathBuf,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn understanding_lines(ocr: &Option<OcrOut>, events: &Option<EventsOut>, faces: &Option<FacesOut>) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(o) = ocr {
        if o.unique_texts == 0 {
            v.push("On-screen text: none found.".to_string());
        } else {
            let mut s = format!("On-screen text: {} unique text{}", o.unique_texts, if o.unique_texts == 1 { "" } else { "s" });
            if let Some(t) = o.texts.first() {
                s.push_str(&format!("; first at {} \"{}\"", t.first_mmss, clip_text(&t.text, 40)));
            }
            s.push('.');
            v.push(s);
            for r in o.regions.iter().filter(|r| r.kind != "on-screen-text").take(3) {
                v.push(format!("  - {}", r.description));
            }
            if let Some(a) = &o.advice {
                v.push(format!("  - {a}"));
            }
        }
    }
    if let Some(e) = events {
        let mut parts: Vec<String> = ["speech", "music", "silence"].iter().filter_map(|k| e.counts.get(*k).map(|c| format!("{:.0}% {k}", c.pct))).collect();
        for k in REACTIONS {
            if let Some(c) = e.counts.get(k) {
                parts.push(format!("{} {k} moment{}", c.events, if c.events == 1 { "" } else { "s" }));
            }
        }
        v.push(format!("Audio ({}): {}.", e.model, if parts.is_empty() { "no clear events".to_string() } else { parts.join(", ") }));
        for m in e.top_moments.iter().take(3) {
            let said = m.text.as_ref().map(|t| format!(" after \"{t}\"")).unwrap_or_default();
            v.push(format!("  - {} at {} (score {:.2}){said}", m.label, m.mmss, m.score));
        }
    }
    if let Some(f) = faces {
        let mut s = format!("Faces: visible {:.0}% of the time", f.face_time_pct);
        if f.face_time_pct > 0.0 {
            s.push_str(&format!(", typically {}", f.typical_faces));
            if let Some(b) = &f.dominant_box {
                s.push_str(&format!(", main face about {:.0}% of the frame at x≈{:.2}", f.face_area_pct, b.x + b.w / 2.0));
            }
        }
        s.push_str(&format!(". Suggested layout: {}{} ({}).", f.suggestion.layout, if f.suggestion.track_face { " + crop track-face" } else { "" }, f.suggestion.reason));
        v.push(s);
    }
    v
}

pub async fn deep(ctx: Ctx, input: DeepIn) -> rustler_core::Result<Outcome<DeepOut>> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"deep": source.id, "steps": ["ocr", "events", "faces"], "has_video": source.probe.has_video, "has_audio": source.probe.has_audio})));
    }
    let mut warnings = Vec::new();
    if !source.probe.has_video {
        warnings.push("no video stream: skipped on-screen text and faces".to_string());
    }
    if !source.probe.has_audio {
        warnings.push("no audio track: skipped audio events".to_string());
    }
    if !source.probe.has_video && !source.probe.has_audio {
        return Err(validation("analyze.deep", format!("source {} has neither video nor audio", source.id), "re-import the file"));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let (mut ocr_out, mut events_out, mut faces_out) = (None, None, None);
    if source.probe.has_video {
        match ocr_with(&ctx, &store, &engine, &source, input.every).await {
            Ok(o) => ocr_out = Some(o),
            Err(e) => warnings.push(format!("on-screen text skipped: {}", err_note(&e))),
        }
    }
    if source.probe.has_audio {
        match events_with(&ctx, &store, &engine, &source).await {
            Ok(o) => events_out = Some(o),
            Err(e) => warnings.push(format!("audio events skipped: {}", err_note(&e))),
        }
    }
    if source.probe.has_video {
        match faces_with(&ctx, &store, &engine, &source, input.fps).await {
            Ok(o) => faces_out = Some(o),
            Err(e) => warnings.push(format!("faces skipped: {}", err_note(&e))),
        }
    }
    engine.shutdown().await;
    for w in ocr_out.iter().flat_map(|o| &o.warnings).chain(events_out.iter().flat_map(|o| &o.warnings)).chain(faces_out.iter().flat_map(|o| &o.warnings)) {
        warnings.push(w.clone());
    }
    let understanding = understanding_lines(&ocr_out, &events_out, &faces_out);
    let id = source.id.to_string();
    let mut next = Vec::new();
    if let Some(m) = events_out.as_ref().and_then(|e| e.top_moments.first()) {
        next.push(format!("rustler see tiles {id} --start {} --end {}   (look at the strongest {})", (m.start - 5.0).max(0.0).floor(), m.end.ceil() + 2.0, m.label));
    }
    next.push(format!("rustler analyze quick {id}   (transcript + units, if not done yet)"));
    next.push(format!("rustler transcript get {id}"));
    let path = rel(&source, "understanding.json");
    let out = DeepOut { source_id: source.id.clone(), duration: source.probe.duration, understanding, ocr: ocr_out, events: events_out, faces: faces_out, path: path.clone(), warnings, next };
    store.write_json(&path, &out)?;
    log_event(&ctx, &store, "analyze.deep", json!({"source_id": id, "ocr": out.ocr.is_some(), "events": out.events.is_some(), "faces": out.faces.is_some()}))?;
    Ok(Outcome::Done(out))
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let long = Annotations { long_running: true, ..Default::default() };
    reg.register(typed(
        "analyze.ocr",
        "Read on-screen text (local RapidOCR): unique texts with first/last seen, plus burned-in caption bands (writes ocr.json)",
        &["source_id"],
        long,
        vec![Example { description: "Check for burned-in subtitles before adding captions", cli: "rustler analyze ocr src_01k... --every 2" }],
        ocr,
    ));
    reg.register(typed(
        "analyze.events",
        "Classify audio into laughter / applause / cheer / music / speech / silence (local YAMNet); top reaction moments joined with the transcript (writes events.json)",
        &["source_id"],
        long,
        vec![Example { description: "Find reaction moments", cli: "rustler analyze events src_01k..." }],
        events,
    ));
    reg.register(typed(
        "analyze.faces",
        "Face-presence timeline (local YuNet): % of time with a face, typical face count, dominant face box and a layout suggestion (writes faces.json)",
        &["source_id"],
        long,
        vec![Example { description: "Decide center-crop vs blur-fit vs split", cli: "rustler analyze faces src_01k... --fps 1" }],
        faces,
    ));
    reg.register(typed(
        "analyze.deep",
        "Understanding pack v2 in one call: on-screen text + audio events + face timeline, with a plain-language summary (writes understanding.json)",
        &["source_id"],
        long,
        vec![Example { description: "Understand a video beyond speech", cli: "rustler analyze deep src_01k..." }],
        deep,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_core::UnitKind;

    fn line(text: &str, y: f64) -> OcrLine {
        OcrLine { text: text.to_string(), bbox: OcrBox { x: 0.1, y: y - 0.02, w: 0.6, h: 0.04 }, conf: 0.95 }
    }

    fn frame(t: f64, t_end: f64, lines: Vec<OcrLine>) -> OcrFrame {
        OcrFrame { t, t_end: Some(t_end), lines }
    }

    #[test]
    fn burned_in_captions_band_is_detected() {
        let frames: Vec<OcrFrame> = (0..8).map(|i| frame(i as f64 * 4.0, i as f64 * 4.0 + 2.0, vec![line(&format!("caption number {i}"), 0.82)])).collect();
        let (texts, regions) = summarize_ocr(&frames, 32.0, 2.0);
        assert_eq!(texts.len(), 8);
        let r = regions.iter().find(|r| r.kind == "burned-in-captions").expect("captions band");
        assert!((r.y - 0.82).abs() < 0.02);
        assert!(r.description.contains("burned-in captions band at y≈0.82"), "{}", r.description);
        assert_eq!(r.distinct_texts, 8);
    }

    #[test]
    fn persistent_text_is_an_overlay_and_returns_count_separately() {
        let frames = vec![
            frame(0.0, 20.0, vec![line("@myhandle", 0.1)]),
            frame(24.0, 30.0, vec![line("@MyHandle", 0.1), line("Big title", 0.5)]),
        ];
        let (texts, regions) = summarize_ocr(&frames, 30.0, 2.0);
        let handle = texts.iter().find(|t| t.text.to_lowercase() == "@myhandle").unwrap();
        assert_eq!(handle.first_seen, 0.0);
        assert_eq!(handle.last_seen, 30.0);
        assert_eq!(handle.appearances, 2);
        assert!(regions.iter().any(|r| r.kind == "persistent-overlay" && r.y < 0.2));
        assert!(regions.iter().any(|r| r.kind == "on-screen-text" && r.description.contains("Big title")));
        assert!(!regions.iter().any(|r| r.kind == "burned-in-captions"));
    }

    #[test]
    fn ticking_timecode_is_not_mistaken_for_captions() {
        let frames: Vec<OcrFrame> = (0..10).map(|i| frame(i as f64 * 2.0, i as f64 * 2.0, vec![line(&format!("00:00:{:02}.000", i * 2), 0.02), line(&format!("{}", 240 + i * 50), 0.04)])).collect();
        let (_, regions) = summarize_ocr(&frames, 20.0, 2.0);
        assert!(regions.iter().all(|r| r.kind != "burned-in-captions"), "{regions:?}");
        assert!(regions.iter().any(|r| r.kind == "timecode-counter" && r.description.contains("not captions")), "{regions:?}");
    }

    #[test]
    fn empty_ocr_has_nothing() {
        let (t, r) = summarize_ocr(&[], 10.0, 2.0);
        assert!(t.is_empty() && r.is_empty());
    }

    fn unit(id: u32, start: f64, end: f64, text: &str) -> Unit {
        Unit { id, kind: UnitKind::Sentence, word_start: 0, word_end: 0, start, end, text: text.into(), boundary_strength: 1.0 }
    }

    #[test]
    fn event_counts_and_top_moments_join_the_transcript() {
        let ev = |s: f64, e: f64, l: &str, sc: f64| AudioEvent { start: s, end: e, label: l.into(), score: sc };
        let res = EventsResult {
            events: vec![ev(0.0, 20.0, "speech", 0.9), ev(20.0, 24.0, "laughter", 0.8), ev(30.0, 31.0, "applause", 0.4), ev(40.0, 50.0, "music", 0.7)],
            model: "yamnet".into(),
            duration: 50.0,
            hop: 0.5,
            warnings: vec![],
        };
        let units = vec![unit(0, 0.0, 19.0, "so I told him the price"), unit(1, 19.0, 21.0, "and he said that is a joke")];
        let (counts, top) = summarize_events(&res, &units, 5);
        assert_eq!(counts["speech"].events, 1);
        assert_eq!(counts["speech"].pct, 40.0);
        assert_eq!(counts["laughter"].seconds, 4.0);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].label, "laughter");
        assert_eq!(top[0].unit_id, Some(1));
        assert_eq!(top[0].mmss, "00:20");
        assert_eq!(top[1].label, "applause");
    }

    fn fs(t: f64, boxes: &[(f64, f64, f64, f64)]) -> FaceSample {
        FaceSample { t, faces: boxes.iter().map(|&(x, y, w, h)| FaceBox { x, y, w, h, score: 0.9 }).collect() }
    }

    fn res(samples: Vec<FaceSample>) -> FacesResult {
        FacesResult { width: 1920, height: 1080, fps: 1.0, samples }
    }

    #[test]
    fn centered_speaker_gets_center_crop() {
        let samples: Vec<_> = (0..10).map(|i| fs(i as f64, &[(0.42, 0.2, 0.16, 0.28)])).collect();
        let o = summarize_faces(&res(samples), 1.0);
        assert_eq!(o.face_time_pct, 100.0);
        assert_eq!(o.typical_faces, 1);
        assert_eq!(o.suggestion.layout, "center-crop");
        assert!(!o.suggestion.track_face);
        assert_eq!(o.spans.len(), 1);
        assert_eq!(o.dominant_box.as_ref().unwrap().x, 0.42);
    }

    #[test]
    fn moving_speaker_gets_tracking() {
        let samples: Vec<_> = (0..10).map(|i| fs(i as f64, &[(0.1 + 0.07 * i as f64, 0.2, 0.15, 0.27)])).collect();
        let o = summarize_faces(&res(samples), 1.0);
        assert_eq!(o.suggestion.layout, "center-crop");
        assert!(o.suggestion.track_face, "{}", o.suggestion.reason);
    }

    #[test]
    fn few_faces_get_blur_fit() {
        let mut samples: Vec<_> = (0..10).map(|i| fs(i as f64, &[])).collect();
        samples[3] = fs(3.0, &[(0.4, 0.3, 0.1, 0.18)]);
        let o = summarize_faces(&res(samples), 1.0);
        assert_eq!(o.face_time_pct, 10.0);
        assert_eq!(o.suggestion.layout, "blur-fit");
        let none = summarize_faces(&res(vec![fs(0.0, &[]), fs(1.0, &[])]), 1.0);
        assert_eq!(none.suggestion.layout, "blur-fit");
        assert!(none.dominant_box.is_none());
    }

    #[test]
    fn two_people_apart_get_split_and_corner_webcam_gets_split() {
        let two: Vec<_> = (0..6).map(|i| fs(i as f64, &[(0.15, 0.2, 0.14, 0.25), (0.7, 0.22, 0.13, 0.24)])).collect();
        let o = summarize_faces(&res(two), 1.0);
        assert_eq!(o.typical_faces, 2);
        assert_eq!(o.suggestion.layout, "split-vertical");
        let cam: Vec<_> = (0..6).map(|i| fs(i as f64, &[(0.84, 0.72, 0.1, 0.17)])).collect();
        let o = summarize_faces(&res(cam), 1.0);
        assert_eq!(o.suggestion.layout, "split-vertical", "{}", o.suggestion.reason);
        assert!(o.suggestion.reason.contains("webcam"));
    }

    #[test]
    fn face_spans_bridge_short_gaps() {
        let mut samples: Vec<_> = (0..12).map(|i| fs(i as f64, &[])).collect();
        for i in [0usize, 1, 2, 4, 5, 10, 11] {
            samples[i] = fs(i as f64, &[(0.4, 0.2, 0.15, 0.27)]);
        }
        let o = summarize_faces(&res(samples), 1.0);
        let spans: Vec<(f64, f64)> = o.spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, vec![(0.0, 6.0), (10.0, 12.0)]);
    }

    #[test]
    fn vertical_sources_need_no_reframing() {
        let mut r = res((0..4).map(|i| fs(i as f64, &[(0.4, 0.2, 0.2, 0.15)])).collect());
        r.width = 1080;
        r.height = 1920;
        assert_eq!(summarize_faces(&r, 1.0).suggestion.layout, "center-crop");
    }
}
