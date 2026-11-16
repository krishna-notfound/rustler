//! Commands: brief.set, brief.show, brief.clear, brief.check, brief.description (f22).
//!
//! A *brief* is the job's rules (campaign payout terms, length range, required credit, banned
//! words, ...). It lives in `<project>/brief.json`. `brief.check` verifies one clip against it and
//! saves the result to `clips/<id>/brief_check.json`; `export.clip` embeds that file in its report
//! (see [`brief_status`]) and `export.batch` refuses clips that fail it.
//!
//! Matching of required/banned text is case-, accent- and punctuation-insensitive (the same
//! `normalize` the transcript search uses) over whole words, in three places: the spoken words of
//! the clip's segments (project caption script plus the original-script transcript when one was
//! saved), the caption text (the same words, when captions are on) and the text overlays.

use crate::clip_cmds::{caption_words_for_clip, load_transcript, log_event};
use crate::extras;
use crate::overlay_cmds::load_overlays;
use crate::transcript_cmds::normalize;
use chrono::{DateTime, Utc};
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, QcStatus, RustlerError, TimeMap, Transcript};
use rustler_media::qc::QcView;
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// Project-relative path of the brief.
pub const BRIEF_FILE: &str = "brief.json";
/// Tolerance when comparing durations to the brief's range (seconds).
const DURATION_EPS: f64 = 0.05;
/// Words of the hook line at most.
const HOOK_MAX_WORDS: usize = 18;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// The brief
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Tiktok,
    Reels,
    Shorts,
    All,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Tiktok => "TikTok",
            Platform::Reels => "Instagram Reels",
            Platform::Shorts => "YouTube Shorts",
            Platform::All => "all platforms",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PayoutModel {
    PerView,
    PerClip,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Payout {
    pub model: PayoutModel,
    /// Amount per 1000 views (per_view) or per accepted clip (per_clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

/// `<project>/brief.json` (schema `rustler.brief@1`). Every field is optional.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Brief {
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<f64>,
    /// `W:H`, e.g. `9:16`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption_style: Option<String>,
    /// Layout preset name every segment must use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Phrases that must appear in the captions, overlays or speech.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required_text: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required_hashtags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required_mentions: Vec<String>,
    /// Words or phrases that must not appear in speech, captions or overlays.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub banned_words: Vec<String>,
    /// On-screen credit that must be in an overlay (e.g. "Clip from @creator").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_text: Option<String>,
    /// On-screen watermark text that must be in an overlay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payout: Option<Payout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// The brief of the project, `None` when none was set.
pub fn load_brief(store: &ProjectStore) -> Result<Option<Brief>> {
    let rel = Path::new(BRIEF_FILE);
    if !store.resolve(rel).is_file() {
        return Ok(None);
    }
    store.read_json::<Brief>(rel).map(Some)
}

fn require_brief(store: &ProjectStore, stage: &str) -> Result<Brief> {
    load_brief(store)?.ok_or_else(|| {
        RustlerError::new(ErrorCode::NotFound, stage, "this project has no brief yet")
            .hint("set one: rustler brief set --min-duration 20 --max-duration 60 --credit-text \"Clip from @creator\"")
    })
}

fn clean_list(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in items {
        let s = s.trim().to_string();
        if !s.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    }
    out
}

fn tag_list(stage: &str, items: &[String], sigil: char) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for s in clean_list(items) {
        let s = s.trim_start_matches(sigil).to_string();
        if s.is_empty() || s.chars().any(char::is_whitespace) {
            return Err(validation(stage, format!("'{sigil}{s}' is not a valid {} (no spaces)", if sigil == '#' { "hashtag" } else { "mention" })));
        }
        let tagged = format!("{sigil}{s}");
        if !out.contains(&tagged) {
            out.push(tagged);
        }
    }
    Ok(out)
}

/// `W:H` -> width / height.
pub fn parse_aspect(s: &str) -> Option<f64> {
    let (w, h) = s.trim().split_once([':', 'x', '/'])?;
    let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
    (w > 0.0 && h > 0.0).then_some(w / h)
}

// ---------------------------------------------------------------------------------------------
// brief.set / brief.show / brief.clear
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Default)]
pub struct BriefSetIn {
    pub creator: Option<String>,
    pub campaign: Option<String>,
    /// tiktok | reels | shorts | all
    pub platform: Option<Platform>,
    pub min_duration: Option<f64>,
    pub max_duration: Option<f64>,
    /// `W:H`; Rustler renders 9:16.
    pub aspect: Option<String>,
    /// Caption style preset the clip must use (see caption.list_styles).
    pub caption_style: Option<String>,
    /// Layout preset every segment must use (see layout.presets).
    pub layout: Option<String>,
    /// Phrases that must appear in captions, overlays or speech. A given list replaces the old one.
    pub required_text: Option<Vec<String>>,
    /// Hashtags for the post description (a leading # is added when missing).
    pub required_hashtags: Option<Vec<String>>,
    /// Mentions for the post description (a leading @ is added when missing).
    pub required_mentions: Option<Vec<String>>,
    /// Words/phrases that must not appear in speech, captions or overlays.
    pub banned_words: Option<Vec<String>>,
    /// On-screen credit (an overlay must contain it).
    pub credit_text: Option<String>,
    /// On-screen watermark text (an overlay must contain it).
    pub watermark_text: Option<String>,
    pub payout: Option<Payout>,
    pub notes: Option<String>,
    /// Overwrite the whole brief with this input instead of merging into it.
    #[serde(default)]
    pub replace: bool,
}

fn opt_text(v: &Option<String>) -> Option<Option<String>> {
    v.as_ref().map(|s| {
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    })
}

/// Applies `i` to `b` (merge: only given fields change; an empty string/list clears a field).
pub fn apply_set(b: &mut Brief, i: &BriefSetIn) -> Result<Vec<String>> {
    const STAGE: &str = "brief.set";
    let mut warnings = Vec::new();
    for (name, v) in [("min_duration", i.min_duration), ("max_duration", i.max_duration)] {
        if let Some(v) = v {
            if !v.is_finite() || v <= 0.0 {
                return Err(validation(STAGE, format!("{name} must be a positive number of seconds")));
            }
        }
    }
    if let Some(a) = &i.aspect {
        if parse_aspect(a).is_none() {
            return Err(validation(STAGE, format!("aspect '{a}' is not W:H")).hint("e.g. --aspect 9:16"));
        }
    }
    if let Some(p) = &i.payout {
        if p.rate.is_some_and(|r| !r.is_finite() || r < 0.0) {
            return Err(validation(STAGE, "payout.rate must be >= 0"));
        }
    }
    if let Some(v) = &i.creator {
        b.creator = opt_text(&Some(v.clone())).flatten();
    }
    if let Some(v) = &i.campaign {
        b.campaign = opt_text(&Some(v.clone())).flatten();
    }
    if i.platform.is_some() {
        b.platform = i.platform;
    }
    if i.min_duration.is_some() {
        b.min_duration = i.min_duration;
    }
    if i.max_duration.is_some() {
        b.max_duration = i.max_duration;
    }
    if let (Some(lo), Some(hi)) = (b.min_duration, b.max_duration) {
        if lo > hi {
            return Err(validation(STAGE, format!("min_duration {lo} is longer than max_duration {hi}")));
        }
    }
    if let Some(v) = opt_text(&i.aspect) {
        b.aspect = v;
    }
    if let Some(v) = opt_text(&i.caption_style) {
        if let Some(s) = &v {
            if !rustler_captions::PRESETS.contains(&s.as_str()) {
                warnings.push(format!(
                    "caption style '{s}' is not a built-in preset ({}); custom styles are fine if you saved them",
                    rustler_captions::PRESETS.join(", ")
                ));
            }
        }
        b.caption_style = v;
    }
    if let Some(v) = opt_text(&i.layout) {
        if let Some(s) = &v {
            if !rustler_core::layout::PRESETS.contains(&s.as_str()) {
                warnings.push(format!("layout '{s}' is not a built-in preset ({}); fine for a saved custom preset", rustler_core::layout::PRESETS.join(", ")));
            }
        }
        b.layout = v;
    }
    if let Some(v) = &i.required_text {
        b.required_text = clean_list(v);
    }
    if let Some(v) = &i.banned_words {
        b.banned_words = clean_list(v);
    }
    if let Some(v) = &i.required_hashtags {
        b.required_hashtags = tag_list(STAGE, v, '#')?;
    }
    if let Some(v) = &i.required_mentions {
        b.required_mentions = tag_list(STAGE, v, '@')?;
    }
    if let Some(v) = opt_text(&i.credit_text) {
        b.credit_text = v;
    }
    if let Some(v) = opt_text(&i.watermark_text) {
        b.watermark_text = v;
    }
    if let Some(p) = &i.payout {
        b.payout = Some(p.clone());
    }
    if let Some(v) = opt_text(&i.notes) {
        b.notes = v;
    }
    b.schema = "rustler.brief@1".to_string();
    b.updated_at = Some(Utc::now());
    Ok(warnings)
}

#[derive(Serialize, JsonSchema)]
pub struct BriefOut {
    pub brief: Brief,
    pub path: PathBuf,
    /// True when an existing brief was overwritten with `replace`.
    pub replaced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn brief_next() -> Vec<String> {
    vec![
        "rustler brief check <clip_id>".to_string(),
        "rustler brief description <clip_id>".to_string(),
    ]
}

async fn brief_set(ctx: Ctx, i: BriefSetIn) -> Result<BriefOut> {
    let store = ctx.store()?;
    let existing = load_brief(&store)?;
    let replaced = i.replace && existing.is_some();
    let mut b = if i.replace { Brief::default() } else { existing.unwrap_or_default() };
    let warnings = apply_set(&mut b, &i)?;
    let path = store.resolve(Path::new(BRIEF_FILE));
    if !ctx.dry_run {
        store.write_json(Path::new(BRIEF_FILE), &b)?;
        log_event(&ctx, &store, "brief.set", json!({"replace": i.replace}))?;
    }
    Ok(BriefOut { brief: b, path, replaced, dry_run: ctx.dry_run.then_some(true), warnings, next: brief_next() })
}

#[derive(Deserialize, JsonSchema)]
pub struct EmptyIn {}

#[derive(Serialize, JsonSchema)]
pub struct BriefShowOut {
    pub exists: bool,
    pub path: PathBuf,
    pub brief: Option<Brief>,
    pub next: Vec<String>,
}

async fn brief_show(ctx: Ctx, _i: EmptyIn) -> Result<BriefShowOut> {
    let store = ctx.store()?;
    let brief = load_brief(&store)?;
    let next = if brief.is_some() {
        brief_next()
    } else {
        vec!["rustler brief set --min-duration 20 --max-duration 60 --credit-text \"Clip from @creator\"".to_string()]
    };
    Ok(BriefShowOut { exists: brief.is_some(), path: store.resolve(Path::new(BRIEF_FILE)), brief, next })
}

#[derive(Serialize, JsonSchema)]
pub struct BriefClearOut {
    pub cleared: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn brief_clear(ctx: Ctx, _i: EmptyIn) -> Result<BriefClearOut> {
    let store = ctx.store()?;
    let path = store.resolve(Path::new(BRIEF_FILE));
    let exists = path.is_file();
    if exists && !ctx.dry_run {
        std::fs::remove_file(&path)
            .map_err(|e| RustlerError::new(ErrorCode::Internal, "brief.clear", format!("cannot delete {}: {e}", path.display())))?;
        log_event(&ctx, &store, "brief.clear", json!({}))?;
    }
    Ok(BriefClearOut { cleared: exists && !ctx.dry_run, dry_run: ctx.dry_run.then_some(true) })
}

// ---------------------------------------------------------------------------------------------
// Text gathering and matching (pure parts are unit-tested below)
// ---------------------------------------------------------------------------------------------

/// A normalized word with the output time it is spoken / shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Tok {
    pub norm: String,
    pub out_t: f64,
}

/// Tokens of `text` (normalized words) all at `out_t`.
pub fn tokenize(text: &str, out_t: f64) -> Vec<Tok> {
    normalize(text).split(' ').filter(|s| !s.is_empty()).map(|s| Tok { norm: s.to_string(), out_t }).collect()
}

/// Output times of every place `phrase` occurs in `toks` as consecutive whole words.
pub fn find_phrase(toks: &[Tok], phrase: &str) -> Vec<f64> {
    let want: Vec<String> = normalize(phrase).split(' ').filter(|s| !s.is_empty()).map(String::from).collect();
    if want.is_empty() || toks.len() < want.len() {
        return Vec::new();
    }
    toks.windows(want.len())
        .filter(|w| w.iter().zip(&want).all(|(t, p)| &t.norm == p))
        .map(|w| w[0].out_t)
        .collect()
}

/// Everything the clip says or shows, as tokens.
pub struct ClipText {
    /// Word streams in output order: the transcript, the original-script transcript (when one was
    /// saved) and the caption words. Kept apart so a phrase never matches across two copies.
    pub streams: Vec<Vec<Tok>>,
    /// Text overlays: `(overlay id, tokens)`.
    pub overlays: Vec<(String, Vec<Tok>)>,
    pub warnings: Vec<String>,
}

fn raw_transcript(store: &ProjectStore, id: &rustler_core::SourceId) -> Option<Transcript> {
    let rel = Path::new("analysis").join(id.as_str()).join("transcript.raw.json");
    if !store.resolve(&rel).is_file() {
        return None;
    }
    store.read_json(&rel).ok()
}

pub fn gather_text(store: &ProjectStore, clip: &Clip) -> Result<ClipText> {
    use std::collections::HashMap;
    let tm = TimeMap::from_segments(&clip.segments);
    let mut warnings = Vec::new();
    let mut cache: HashMap<String, (Option<Transcript>, Option<Transcript>)> = HashMap::new();
    let (mut main, mut raw): (Vec<Tok>, Vec<Tok>) = (Vec::new(), Vec::new());
    for m in tm.segments() {
        let key = m.source_id.to_string();
        if !cache.contains_key(&key) {
            let t = load_transcript(store, &m.source_id)?;
            if t.is_none() {
                warnings.push(format!(
                    "source {} has no transcript, so spoken words cannot be checked (run `rustler analyze transcribe {}`)",
                    m.source_id, m.source_id
                ));
            }
            cache.insert(key.clone(), (t, raw_transcript(store, &m.source_id)));
        }
        let (t, r) = &cache[&key];
        for (tr, out) in [(t, &mut main), (r, &mut raw)] {
            let Some(tr) = tr else { continue };
            for w in &tr.words {
                let mid = (w.start + w.end) / 2.0;
                if mid >= m.src_start && mid < m.src_end {
                    out.extend(tokenize(&w.text, m.out_start + (w.start.max(m.src_start) - m.src_start)));
                }
            }
        }
    }
    // The caption words (script-converted by the render path) cover romanized Hinglish even when
    // only the raw Devanagari transcript was saved.
    let mut captions: Vec<Tok> = Vec::new();
    if clip.captions.enabled {
        if let Ok((words, _)) = caption_words_for_clip(store, clip) {
            for w in words {
                captions.extend(tokenize(&w.text, w.out_start));
            }
        }
    }
    let duration = tm.duration();
    let overlays = load_overlays(store, &clip.id)?
        .into_iter()
        .filter(|o| o.start < duration)
        .map(|o| {
            let toks = tokenize(&o.text, o.start);
            (o.id, toks)
        })
        .collect();
    Ok(ClipText { streams: vec![main, raw, captions], overlays, warnings })
}

impl ClipText {
    /// Where `phrase` occurs: `(place, out_t)` pairs; place is `captions`, `speech` or `overlay <id>`.
    pub fn find(&self, phrase: &str, captions_on: bool) -> Vec<(String, f64)> {
        let place = if captions_on { "captions" } else { "speech" };
        let mut hits: Vec<(String, f64)> = Vec::new();
        for stream in &self.streams {
            for t in find_phrase(stream, phrase) {
                // The same words appear in several streams; keep one hit per ~0.1 s.
                if !hits.iter().any(|(p, h)| p == place && (h - t).abs() < 0.1) {
                    hits.push((place.to_string(), t));
                }
            }
        }
        for (id, toks) in &self.overlays {
            if !find_phrase(toks, phrase).is_empty() {
                hits.push((format!("overlay {id}"), toks.first().map_or(0.0, |t| t.out_t)));
            }
        }
        hits
    }
}

// ---------------------------------------------------------------------------------------------
// brief.check
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum RuleStatus {
    Pass,
    /// Not blocking, but look at it.
    Warn,
    Fail,
    /// Something to do at upload (hashtags, mentions); never blocks.
    Info,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RuleResult {
    pub rule: String,
    pub status: RuleStatus,
    pub message: String,
    /// Exact commands that fix a failing rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix: Vec<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

fn rule(rule: &str, status: RuleStatus, message: impl Into<String>) -> RuleResult {
    RuleResult { rule: rule.to_string(), status, message: message.into(), fix: Vec::new(), detail: Value::Null }
}

impl RuleResult {
    fn fix(mut self, cmds: impl IntoIterator<Item = String>) -> Self {
        self.fix.extend(cmds);
        self
    }
    fn detail(mut self, v: Value) -> Self {
        self.detail = v;
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BriefCheck {
    pub schema: String,
    pub clip_id: ClipId,
    pub clip_name: String,
    pub clip_revision: u64,
    pub checked_at: DateTime<Utc>,
    /// fail when any rule fails, warn when any warns, else pass.
    pub status: RuleStatus,
    pub rules: Vec<RuleResult>,
    /// Things to add in the post description / at upload (hashtags, mentions).
    #[serde(default)]
    pub post_items: Vec<String>,
    /// Ready-to-edit post description.
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn check_rel(clip: &ClipId) -> PathBuf {
    Path::new("clips").join(clip.as_str()).join("brief_check.json")
}

/// Summed transition overlap of the clip (seconds): `transitions_total` in extras.json, else the
/// sum of `duration` over `transitions` that are not plain cuts.
pub fn transitions_total(map: &serde_json::Map<String, Value>) -> f64 {
    if let Some(t) = map.get("transitions_total").and_then(Value::as_f64) {
        return t.max(0.0);
    }
    match map.get("transitions") {
        Some(Value::Array(a)) => a
            .iter()
            .filter(|t| t.get("kind").and_then(Value::as_str).is_none_or(|k| k != "cut"))
            .filter_map(|t| t.get("duration").and_then(Value::as_f64))
            .sum(),
        _ => 0.0,
    }
}

fn latest_qc(store: &ProjectStore, clip: &Clip) -> Option<QcView> {
    clip.renders.last().and_then(|r| r.qc.as_ref()).and_then(|p| crate::qc_cmds::read_qc(store, p))
}

/// Seconds without a trailing `.0`.
fn secs(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// Runs every rule of `brief` against `clip`. Pure over the project files; nothing is saved.
pub fn check_clip(store: &ProjectStore, clip: &Clip, brief: &Brief) -> Result<BriefCheck> {
    let id = clip.id.as_str();
    let map = extras::load(store, &clip.id)?;
    let text = gather_text(store, clip)?;
    let mut rules: Vec<RuleResult> = Vec::new();
    let mut post_items = Vec::new();
    let captions_on = clip.captions.enabled;

    // Duration.
    let tm_dur = TimeMap::from_segments(&clip.segments).duration();
    let trans = transitions_total(&map);
    let dur = (tm_dur - trans).max(0.0);
    if brief.min_duration.is_some() || brief.max_duration.is_some() {
        let (lo, hi) = (brief.min_duration, brief.max_duration);
        let range = match (lo, hi) {
            (Some(a), Some(b)) => format!("{}-{}s", secs(a), secs(b)),
            (Some(a), None) => format!("at least {}s", secs(a)),
            (None, Some(b)) => format!("at most {}s", secs(b)),
            (None, None) => unreachable!(),
        };
        let note = if trans > 0.0 { format!(" (after {trans:.2}s of transitions)") } else { String::new() };
        if let Some(a) = lo.filter(|a| dur < a - DURATION_EPS) {
            rules.push(
                rule("duration", RuleStatus::Fail, format!("clip is {dur:.1}s{note}, shorter than the brief's {range}"))
                    .fix([
                        format!("rustler edit set-range {id} --start-unit <earlier unit> --end-unit <later unit>   # add about {:.1}s", a - dur + 0.2),
                        format!("rustler clip add-segment {id} --source-id <src> --start-unit <n> --end-unit <m>"),
                    ])
                    .detail(json!({"duration": dur, "min": lo, "max": hi})),
            );
        } else if let Some(b) = hi.filter(|b| dur > b + DURATION_EPS) {
            rules.push(
                rule("duration", RuleStatus::Fail, format!("clip is {dur:.1}s{note}, longer than the brief's {range}"))
                    .fix([
                        format!("rustler edit trim {id} --input-json '{{\"end_delta\":-{:.1}}}'", dur - b + 0.1),
                        format!("rustler edit remove-pauses {id}"),
                    ])
                    .detail(json!({"duration": dur, "min": lo, "max": hi})),
            );
        } else {
            rules.push(
                rule("duration", RuleStatus::Pass, format!("clip is {dur:.1}s{note}, inside {range}"))
                    .detail(json!({"duration": dur, "min": lo, "max": hi})),
            );
        }
    }

    // Aspect ratio of the canvas.
    if let Some(a) = &brief.aspect {
        let want = parse_aspect(a).unwrap_or(9.0 / 16.0);
        let have = clip.canvas.width as f64 / clip.canvas.height.max(1) as f64;
        if (want - have).abs() < 0.01 {
            rules.push(rule("aspect", RuleStatus::Pass, format!("canvas {}x{} is {a}", clip.canvas.width, clip.canvas.height)));
        } else {
            rules.push(rule(
                "aspect",
                RuleStatus::Fail,
                format!("canvas is {}x{} but the brief wants {a}", clip.canvas.width, clip.canvas.height),
            ));
        }
    }

    // Caption style.
    if let Some(style) = &brief.caption_style {
        if !captions_on {
            rules.push(
                rule("caption_style", RuleStatus::Fail, format!("captions are off; the brief wants '{style}'"))
                    .fix([format!("rustler caption set {id} --enabled true --style {style}")]),
            );
        } else if &clip.captions.preset == style {
            rules.push(rule("caption_style", RuleStatus::Pass, format!("caption style is '{style}'")));
        } else {
            rules.push(
                rule("caption_style", RuleStatus::Fail, format!("caption style is '{}' but the brief wants '{style}'", clip.captions.preset))
                    .fix([format!("rustler caption set {id} --style {style}")]),
            );
        }
    }

    // Layout preset on every segment.
    if let Some(want) = &brief.layout {
        let bad: Vec<String> = clip
            .segments
            .iter()
            .filter(|s| s.layout.preset.as_deref() != Some(want.as_str()))
            .map(|s| format!("{} ({})", s.id, s.layout.preset.as_deref().unwrap_or("custom regions")))
            .collect();
        if bad.is_empty() {
            rules.push(rule("layout", RuleStatus::Pass, format!("every segment uses layout '{want}'")));
        } else {
            rules.push(
                rule("layout", RuleStatus::Fail, format!("layout differs from the brief's '{want}': {}", bad.join(", ")))
                    .fix([format!("rustler layout apply-preset {id} {want}")]),
            );
        }
    }

    // Required text.
    for phrase in &brief.required_text {
        let hits = text.find(phrase, captions_on);
        if hits.is_empty() {
            rules.push(
                rule("required_text", RuleStatus::Fail, format!("required text \"{phrase}\" does not appear in the captions, overlays or speech"))
                    .fix([format!("rustler overlay add-text {id} \"{phrase}\" --style lower-third")])
                    .detail(json!({"phrase": phrase})),
            );
        } else {
            let places: Vec<&str> = hits.iter().map(|(p, _)| p.as_str()).collect();
            rules.push(
                rule("required_text", RuleStatus::Pass, format!("\"{phrase}\" found in {}", places.join(", ")))
                    .detail(json!({"phrase": phrase, "at": hits.iter().map(|(p, t)| json!({"in": p, "out_t": t})).collect::<Vec<_>>()})),
            );
        }
    }

    // Banned words.
    for word in &brief.banned_words {
        let hits = text.find(word, captions_on);
        if hits.is_empty() {
            rules.push(rule("banned_words", RuleStatus::Pass, format!("\"{word}\" does not appear")));
            continue;
        }
        let mut fixes = Vec::new();
        for (place, _) in &hits {
            if let Some(oid) = place.strip_prefix("overlay ") {
                fixes.push(format!("rustler overlay remove {id} {oid}"));
            } else {
                let cmd = format!("rustler edit remove-words {id} --words '[\"{}\"]'", word.replace('\'', ""));
                if !fixes.contains(&cmd) {
                    fixes.push(cmd);
                }
            }
        }
        let first = hits.iter().map(|(_, t)| *t).fold(f64::INFINITY, f64::min);
        let where_ = hits.iter().map(|(p, t)| format!("{p} @{t:.1}s")).collect::<Vec<_>>().join(", ");
        rules.push(
            rule("banned_words", RuleStatus::Fail, format!("banned word \"{word}\" appears: {where_}"))
                .fix(fixes.into_iter().chain([format!("rustler edit trim {id}   # or cut the clip before @{first:.1}s")]))
                .detail(json!({"word": word, "at": hits.iter().map(|(p, t)| json!({"in": p, "out_t": t})).collect::<Vec<_>>()})),
        );
    }

    // Credit and watermark overlays.
    for (name, value, style) in [
        ("credit_text", &brief.credit_text, "--style lower-third"),
        ("watermark_text", &brief.watermark_text, "--style plain --position-y 0.9"),
    ] {
        let Some(v) = value else { continue };
        let in_overlay: Vec<&str> = text
            .overlays
            .iter()
            .filter(|(_, toks)| !find_phrase(toks, v).is_empty())
            .map(|(id, _)| id.as_str())
            .collect();
        if in_overlay.is_empty() {
            rules.push(
                rule(name, RuleStatus::Fail, format!("the overlay \"{v}\" required by the brief is not on screen"))
                    .fix([format!("rustler overlay add-text {id} \"{v}\" {style} --start 0 --end {tm_dur:.1}")]),
            );
        } else {
            rules.push(rule(name, RuleStatus::Pass, format!("\"{v}\" is on screen ({})", in_overlay.join(", "))));
        }
    }

    // Hashtags and mentions are added in the post, not in the video.
    for h in &brief.required_hashtags {
        post_items.push(format!("add hashtag {h}"));
    }
    for m in &brief.required_mentions {
        post_items.push(format!("mention {m}"));
    }
    if !post_items.is_empty() {
        rules.push(
            rule("post_description", RuleStatus::Info, format!("add in the post description: {}", post_items.join(", ")))
                .fix([format!("rustler brief description {id}")]),
        );
    }

    // QC of the latest render.
    match clip.renders.last() {
        None => rules.push(
            rule("qc", RuleStatus::Warn, "not rendered yet; the brief is checked on the clip definition only")
                .fix([format!("rustler render run {id}"), format!("rustler qc run {id}")]),
        ),
        Some(r) => match latest_qc(store, clip) {
            None => rules.push(rule("qc", RuleStatus::Warn, format!("render {} has no QC report yet", r.id)).fix([format!("rustler qc run {id}")])),
            Some(q) if !q.report.passed => {
                let failed: Vec<String> = q.report.checks.iter().filter(|c| c.status == QcStatus::Fail).map(|c| c.name.clone()).collect();
                rules.push(rule("qc", RuleStatus::Fail, format!("QC failed: {}", failed.join(", "))).fix([format!("rustler fix suggest {id}")]));
            }
            Some(q) => rules.push(rule("qc", RuleStatus::Pass, q.verdict)),
        },
    }

    let status = if rules.iter().any(|r| r.status == RuleStatus::Fail) {
        RuleStatus::Fail
    } else if rules.iter().any(|r| r.status == RuleStatus::Warn) {
        RuleStatus::Warn
    } else {
        RuleStatus::Pass
    };
    let description = draft_description(store, clip, brief)?.text;
    Ok(BriefCheck {
        schema: "rustler.brief_check@1".to_string(),
        clip_id: clip.id.clone(),
        clip_name: clip.name.clone(),
        clip_revision: clip.revision,
        checked_at: Utc::now(),
        status,
        rules,
        post_items,
        description,
        warnings: text.warnings,
    })
}

/// The saved result of the last `brief.check` of the clip (`clips/<id>/brief_check.json`), for
/// reports such as `export.clip`. `None` when the clip was never checked or the file is unreadable.
pub fn brief_status(store: &ProjectStore, clip_id: &ClipId) -> Option<Value> {
    let rel = check_rel(clip_id);
    if !store.resolve(&rel).is_file() {
        return None;
    }
    store.read_json::<Value>(&rel).ok()
}

/// Runs the brief check and saves it. `None` when the project has no brief.
pub fn run_and_save(store: &ProjectStore, clip_id: &ClipId, persist: bool) -> Result<Option<BriefCheck>> {
    let Some(brief) = load_brief(store)? else { return Ok(None) };
    let clip = store.load_clip(clip_id)?;
    let c = check_clip(store, &clip, &brief)?;
    if persist {
        store.write_json(&check_rel(clip_id), &c)?;
    }
    Ok(Some(c))
}

#[derive(Deserialize, JsonSchema)]
pub struct ClipIdIn {
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema)]
pub struct BriefCheckOut {
    #[serde(flatten)]
    pub check: BriefCheck,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

async fn brief_check(ctx: Ctx, i: ClipIdIn) -> Result<BriefCheckOut> {
    let store = ctx.store()?;
    require_brief(&store, "brief.check")?;
    let check = run_and_save(&store, &i.clip_id, !ctx.dry_run)?.expect("brief exists");
    let mut next: Vec<String> = check.rules.iter().filter(|r| r.status == RuleStatus::Fail).flat_map(|r| r.fix.iter().cloned()).collect();
    if next.is_empty() {
        next.push(format!("rustler export clip {}", i.clip_id));
    }
    if !ctx.dry_run {
        log_event(&ctx, &store, "brief.check", json!({"clip_id": i.clip_id, "status": check.status}))?;
    }
    Ok(BriefCheckOut {
        path: store.resolve(&check_rel(&i.clip_id)),
        dry_run: ctx.dry_run.then_some(true),
        next,
        check,
    })
}

// ---------------------------------------------------------------------------------------------
// brief.description
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Description {
    pub clip_id: ClipId,
    /// Platform the brief targets (all when unset).
    pub platform: String,
    /// First sentence of what the clip says.
    pub hook: String,
    /// Short title (hook cut to 70 characters) for platforms with titles.
    pub title: String,
    /// The full post text: hook, credit, mentions, hashtags.
    pub text: String,
    pub hashtags: Vec<String>,
    pub mentions: Vec<String>,
    pub credit: Option<String>,
    pub chars: usize,
}

/// First sentence of the clip's spoken words (<= 18 words, ends at . ! ? when there is one).
pub fn hook_line(words: &[String]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for w in words {
        out.push(w);
        let last = w.trim_end_matches(['"', '\'', ')', '”', '’']);
        if last.ends_with(['.', '!', '?', '।']) || out.len() >= HOOK_MAX_WORDS {
            break;
        }
    }
    let mut line = out.join(" ");
    if !line.ends_with(['.', '!', '?', '।']) && words.len() > out.len() {
        line = format!("{}…", line.trim_end_matches([',', ';', ':']));
    }
    line
}

fn cut_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    match cut.rfind(' ') {
        Some(i) if i > max / 2 => format!("{}…", cut[..i].trim_end_matches([',', '.', ';'])),
        _ => format!("{cut}…"),
    }
}

/// Deterministic post text for `clip` from the brief (the agent can rewrite it).
pub fn draft_description(store: &ProjectStore, clip: &Clip, brief: &Brief) -> Result<Description> {
    let (words, _) = caption_words_for_clip(store, clip).unwrap_or_default();
    let spoken: Vec<String> = words.into_iter().map(|w| w.text).collect();
    let hook = if spoken.is_empty() { clip.name.clone() } else { hook_line(&spoken) };
    let mut blocks = vec![hook.clone()];
    if let Some(c) = &brief.credit_text {
        blocks.push(c.clone());
    }
    if !brief.required_mentions.is_empty() {
        blocks.push(brief.required_mentions.join(" "));
    }
    if !brief.required_hashtags.is_empty() {
        blocks.push(brief.required_hashtags.join(" "));
    }
    let text = blocks.join("\n\n");
    Ok(Description {
        clip_id: clip.id.clone(),
        platform: brief.platform.unwrap_or(Platform::All).label().to_string(),
        title: cut_chars(&hook, 70),
        chars: text.chars().count(),
        text,
        hook,
        hashtags: brief.required_hashtags.clone(),
        mentions: brief.required_mentions.clone(),
        credit: brief.credit_text.clone(),
    })
}

#[derive(Serialize, JsonSchema)]
pub struct DescriptionOut {
    #[serde(flatten)]
    pub description: Description,
    pub next: Vec<String>,
}

async fn brief_description(ctx: Ctx, i: ClipIdIn) -> Result<DescriptionOut> {
    let store = ctx.store()?;
    let brief = load_brief(&store)?.unwrap_or_default();
    let clip = store.load_clip(&i.clip_id)?;
    let description = draft_description(&store, &clip, &brief)?;
    Ok(DescriptionOut { description, next: vec![format!("rustler brief check {}", i.clip_id)] })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let read = Annotations { read_only: true, ..Annotations::default() };
    reg.register(typed(
        "brief.set",
        "Set or merge the job brief (length range, credit, required/banned text, hashtags, payout) into <project>/brief.json; replace=true overwrites",
        &[],
        Annotations::default(),
        vec![
            Example {
                description: "campaign rules",
                cli: "rustler brief set --campaign \"Spring push\" --min-duration 20 --max-duration 60 --credit-text \"Clip from @creator\" --banned-words '[\"sponsor\"]' --required-hashtags '[\"#ad\"]'",
            },
            Example { description: "start over", cli: "rustler brief set --replace true --max-duration 45" },
        ],
        brief_set,
    ));
    reg.register(typed(
        "brief.show",
        "Show the project's brief",
        &[],
        read,
        vec![Example { description: "show", cli: "rustler brief show" }],
        brief_show,
    ));
    reg.register(typed(
        "brief.clear",
        "Delete the project's brief",
        &[],
        Annotations { destructive: true, ..Annotations::default() },
        vec![Example { description: "remove the brief", cli: "rustler brief clear" }],
        brief_clear,
    ));
    reg.register(typed(
        "brief.check",
        "Check a clip against the brief: duration, aspect, caption style, layout, required text, banned words, credit/watermark overlays, QC; pass/warn/fail per rule with fix commands. Saved to clips/<id>/brief_check.json",
        &["clip_id"],
        Annotations::default(),
        vec![Example { description: "check a clip", cli: "rustler brief check clp_X" }],
        brief_check,
    ));
    reg.register(typed(
        "brief.description",
        "Draft the post text for a clip: hook line from the transcript, credit, mentions, required hashtags (deterministic; rewrite freely)",
        &["clip_id"],
        read,
        vec![Example { description: "post text", cli: "rustler brief description clp_X" }],
        brief_description,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_matching_is_whole_word_and_insensitive() {
        let toks = tokenize("Hello THERE, friends! Don't use the CLASS", 1.0);
        assert_eq!(find_phrase(&toks, "there friends"), vec![1.0]);
        assert!(find_phrase(&toks, "ass").is_empty(), "must not match inside 'class'");
        assert_eq!(find_phrase(&toks, "Class"), vec![1.0]);
        assert!(find_phrase(&toks, "!!!").is_empty());
        assert_eq!(find_phrase(&tokenize("Café déjà", 0.0), "cafe deja"), vec![0.0]);
    }

    #[test]
    fn aspect_parsing() {
        assert!((parse_aspect("9:16").unwrap() - 0.5625).abs() < 1e-9);
        assert!(parse_aspect("wide").is_none());
        assert!(parse_aspect("0:16").is_none());
    }

    #[test]
    fn hook_is_first_sentence() {
        let w: Vec<String> = ["Hello", "there", "friends.", "Fellow", "Americans"].iter().map(|s| s.to_string()).collect();
        assert_eq!(hook_line(&w), "Hello there friends.");
        let long: Vec<String> = (0..30).map(|i| format!("w{i}")).collect();
        assert!(hook_line(&long).ends_with('…'));
    }

    #[test]
    fn transitions_sum() {
        let m = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(transitions_total(&m(json!({"transitions_total": 1.5}))), 1.5);
        assert_eq!(
            transitions_total(&m(json!({"transitions": [{"kind": "fade", "duration": 0.5}, {"kind": "cut", "duration": 0.3}, {"kind": "xfade-slide", "duration": 0.25}]}))),
            0.75
        );
        assert_eq!(transitions_total(&m(json!({}))), 0.0);
    }

    #[test]
    fn set_merges_and_validates() {
        let mut b = Brief::default();
        apply_set(&mut b, &BriefSetIn { min_duration: Some(20.0), banned_words: Some(vec![" a ".into(), "A".into(), "b".into()]), ..Default::default() }).unwrap();
        apply_set(&mut b, &BriefSetIn { max_duration: Some(60.0), required_hashtags: Some(vec!["ad".into(), "#ad".into()]), ..Default::default() }).unwrap();
        assert_eq!((b.min_duration, b.max_duration), (Some(20.0), Some(60.0)));
        assert_eq!(b.banned_words, vec!["a", "b"]);
        assert_eq!(b.required_hashtags, vec!["#ad"]);
        assert!(apply_set(&mut b, &BriefSetIn { min_duration: Some(90.0), ..Default::default() }).is_err());
        assert!(apply_set(&mut b, &BriefSetIn { aspect: Some("tall".into()), ..Default::default() }).is_err());
        assert!(apply_set(&mut b, &BriefSetIn { required_mentions: Some(vec!["a b".into()]), ..Default::default() }).is_err());
    }
}
