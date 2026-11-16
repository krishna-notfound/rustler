//! Command: clips.auto (f37, v2 design §5.7) — an optional convenience pipeline over existing
//! commands. The agent stays free to do any step itself; every decision in the report comes with
//! the exact command that changes it.
//!
//! Stages (each emits NDJSON progress `clips.auto.<stage>`):
//! 1. resolve — a YouTube URL: one cached `source.info` lookup + `source.fetch_transcript`
//!    (captions, no download); or an existing source (`analyze.quick` when it has no transcript).
//! 2. rank — `highlights.candidates` re-ranked with a first-2-seconds hook score, non-overlapping,
//!    never overlapping a clip that already exists on the same moment; `count` picks.
//!    (`ranges` skips ranking: the agent chose the moments.)
//! 3. create — URL: `source.import --sections` per pick (padded), `analyze.quick`; then
//!    `clip.create` (smart defaults + template), `clip.meta_set` (campaign), `clip.retention`
//!    (edit timeline) and only the cheap retention fixes that exist as commands
//!    (trim head/tail silence, remove pauses). Zooms are never added: retention punch-ins stay in
//!    `fixes_suggested`; the template's own punch-ins are re-checked against the new cuts after the
//!    silence fixes (re-planned with the template's pattern when one is < 0.5 s from a cut). Trims never cut into the first/last word
//!    and, like pause cuts, are skipped on approximate (captions-based) word timings; a fix that
//!    changes nothing is reported `no_change`; a silence fix after which the hook or ending scores
//!    worse is restored (`rolled_back`). Text pops and cutaways are creative: they are listed in
//!    `fixes_suggested`, never applied.
//!
//! Clip and export names are unique (`auto-<rank>-<time>-<source slug>`, `-2` when taken), so a
//! run never overwrites another clip's export.
//! 4. render — `render.batch` (progress with `eta_s`), `clip.retention` on the render, `qc.run`,
//!    `clip.cover --auto` (best effort), `export.clip` (writes `<name>.post.json`: title/description/hashtags stay empty for the
//!    agent to write with `clip.meta_set`; Rustler never writes copy and never publishes).
//! 5. report — per clip: range, why, retention, QC, export paths, commands to change each decision.
//!    `fixes_suggested` also lists (f39, never applied) bad takes (`bad_take`, from
//!    `clip.retention` take_fixes), 2-3 hook lines (`hook_title`, `overlay add-text ... --style
//!    hook-big`) and a cold open (`cold_open`) when the clip opens on a question.
//!
//! A failure in one clip is recorded on that clip (`failed_stage`, `error`) and the others go on.

use crate::campaign_cmds::load_campaign;
use crate::clip_cmds::log_event;
use crate::highlights_cmds::{Data, features, load_data, tokens_of, weak_opener_len};
use crate::hook_suggest::SWord;
use crate::project_cmds::mmss;
use crate::source_cmds::{load_source_checked, pending_id};
use crate::transcript_cmds::analysis_rel;
use rustler_core::{ClipId, ErrorCode, ProjectStore, RenderQuality, RustlerError, SourceId, SourceOrigin};
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, ProgressSink, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

type Result<T> = rustler_core::Result<T>;

const STAGE: &str = "clips.auto";
const MAX_COUNT: usize = 20;
/// Points the first-2-seconds hook score (0..1) adds to a candidate's 0-100 score.
const HOOK_WEIGHT: f64 = 20.0;
/// Seconds of padding around each section downloaded for a URL.
const DEFAULT_PAD: f64 = 3.0;
/// URL sections are downloaded at this height unless `download_quality` says otherwise (the
/// 1080x1920 short is cut from a crop of the 16:9 frame; 720p is plenty and much smaller).
pub const DEFAULT_DOWNLOAD_QUALITY: &str = "720";
/// URL sections downloaded at the same time (yt-dlp is network-bound).
pub const PARALLEL_DOWNLOADS: usize = 2;
/// Retention fix actions applied automatically (deterministic, non-creative, framing unchanged).
/// Zooms (`punch_in`) are never applied: the agent decides; they stay in `fixes_suggested`.
const SILENCE_FIXES: &[&str] = &["trim_head", "trim_tail", "remove_pauses"];
/// Action name of the template punch-in check after the silence fixes.
pub const TEMPLATE_PUNCH_CHECK: &str = "template_punch_ins";

pub const RANK_FORMULA: &str = "rank_score = highlights.candidates score (0-100) + 20 x hook_2s (0..1). \
hook_2s over the first 2 s of the window: +0.15 speech starts within 0.3 s, +0.30 emphasis word, +0.25 question, \
+0.15 number, +0.15 direct address (you/your/aap/tum), +0.10 loudness peak >= 3 LU over the source median, \
+0.10 speech rate >= 1.15 x the source median, -0.20 weak opener (so/and/um/...); clamped to 0..1. \
Picks: highest rank_score first, no overlap with each other or with clips that already cover the same moment. \
Moments inside the source's intro trailer (an opening montage of lines said again later) are not ranked; their originals are (include_intro: true ranks the trailer too).";

fn validation(msg: impl Into<String>, hint: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg).hint(hint)
}

// ---------------------------------------------------------------------------------------------
// Input / output
// ---------------------------------------------------------------------------------------------

/// A moment the agent chose (seconds; for a URL, seconds of the original video).
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct RangeIn {
    pub start: f64,
    pub end: f64,
    /// Why you chose it (copied to the report).
    pub note: Option<String>,
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct AutoIn {
    /// Video URL (YouTube etc.; needs network). Give this or `source_id`.
    pub url: Option<String>,
    /// An imported source of this project.
    pub source_id: Option<SourceId>,
    /// Clips to make (default 5, at most 20).
    pub count: Option<usize>,
    /// Shortest clip in seconds (default 25, or the campaign's minimum).
    pub min_len: Option<f64>,
    /// Longest clip in seconds (default 60, or the campaign's maximum).
    pub max_len: Option<f64>,
    /// Template (`name` or `name@version`) for `clip.create --template` (default: config `template.default`).
    pub template: Option<String>,
    /// Render quality: draft | final (default final).
    pub quality: Option<RenderQuality>,
    /// URL only: download height 480 | 720 | 1080 | best (default 720). Sections download 2 at a time.
    pub download_quality: Option<String>,
    /// Attach every clip to this campaign (`clip.meta_set --campaign-id`); its duration and platform are
    /// used when min_len/max_len/platform are not given.
    pub campaign: Option<String>,
    /// shorts | reels | tiktok (candidate length scoring; default shorts or the campaign's platform).
    pub platform: Option<String>,
    /// URL only: caption language for `source.fetch_transcript` (default auto).
    pub lang: Option<String>,
    /// URL only: seconds of padding around each downloaded section (default 3).
    pub pad: Option<f64>,
    /// Your own moments instead of ranking (seconds). min_len/max_len are not enforced on them.
    pub ranges: Option<Vec<RangeIn>>,
    /// Skip the smart defaults at clip.create (default false).
    pub raw: Option<bool>,
    /// Apply the cheap retention fixes (trim silence, remove pauses; never zooms) (default true).
    pub fixes: Option<bool>,
    /// Pick a cover frame with `clip.cover --auto` before export, best effort (default true).
    pub cover: Option<bool>,
    /// Also rank moments inside the source's intro trailer (default false: a montage of lines said
    /// again later is skipped, and the originals are ranked instead; see `ranking.intro_trailer`).
    pub include_intro: Option<bool>,
    /// Plan only: rank and return the picks with the commands that would run; nothing is imported,
    /// created or rendered (a URL's captions are still fetched to rank).
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct Why {
    /// Chosen by `ranking` or by the agent (`ranges`).
    pub chosen_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_score: Option<f64>,
    /// First-2-seconds hook score 0..1 (see `ranking.formula`).
    pub hook_2s: f64,
    pub hook_notes: Vec<String>,
    /// What is said in the first 2 s.
    pub first_2s_text: String,
    pub rank_score: f64,
    /// Candidate reasons from `highlights.candidates` (or the agent's note).
    pub reasons: Vec<String>,
    pub opening_text: String,
    pub closing_text: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct AppliedFix {
    pub action: String,
    /// The command as run (trims may be shortened to keep whole words; see `note`).
    pub command: String,
    /// applied | no_change (ran, the clip is the same) | skipped (not run, see `note`) |
    /// rolled_back (ran, made the hook or ending worse, restored) | failed (see `error`)
    pub status: String,
    /// False only when something went wrong (`status: failed`); read `status` and `changed`.
    pub ok: bool,
    /// True when the clip is different now because of this fix.
    pub changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why it was skipped, shortened, rolled back or changed nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Command that takes this fix back out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
}

impl AppliedFix {
    fn new(action: &str, command: &str, status: &str) -> Self {
        AppliedFix {
            action: action.into(),
            command: command.into(),
            status: status.into(),
            ok: status != "failed",
            changed: status == "applied",
            error: None,
            note: None,
            undo: None,
        }
    }
    fn note(mut self, n: impl Into<String>) -> Self {
        self.note = Some(n.into());
        self
    }
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SuggestedFix {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub why: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct RetentionSummary {
    /// Edit-timeline score before any fix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_fixes: Option<u32>,
    /// Latest score (measured on the render when there is one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<u32>,
    /// render | timeline
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_on: Option<String>,
    /// Dead zones left in the latest measurement.
    pub dead_zones_left: usize,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct QcSummary {
    pub passed: bool,
    pub failed: Vec<String>,
    pub warned: Vec<String>,
    pub verdict: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct ExportSummary {
    pub video: PathBuf,
    /// `<name>.post.json`: title/description/hashtags are empty until you write them (`clip.meta_set`).
    pub post: PathBuf,
    pub report: PathBuf,
    /// True when exported although QC failed only on `meta_present` (the copy you still write).
    pub needs_copy: bool,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Change {
    pub decision: String,
    pub command: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct AutoClip {
    pub rank: usize,
    /// planned (dry run) | done | failed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RustlerError>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<ClipId>,
    /// Source the clip was cut from (for a URL: the imported section).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Range in the ranked timeline (source seconds; for a URL, seconds of the original video).
    pub start: f64,
    pub end: f64,
    pub mmss: String,
    pub duration: f64,
    /// URL only: the padded section downloaded for this clip (original-video seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<(f64, f64)>,
    pub why: Why,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<RetentionSummary>,
    /// Every cheap fix tried, with its `status` (applied, no_change, skipped, rolled_back, failed).
    pub fixes_applied: Vec<AppliedFix>,
    /// What the latest retention measurement (the render when there is one) still suggests: text
    /// pops, cutaways and punch-ins (never applied: they need your judgement), and trims that
    /// were skipped or rolled back (their `why` says so). Always present; empty means no dead zones.
    pub fixes_suggested: Vec<SuggestedFix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qc: Option<QcSummary>,
    /// Cover frame chosen by `clip.cover --auto` (exported as `<name>.cover.jpg`, named in the post JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportSummary>,
    /// Dry run: the commands that would run for this clip.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<String>,
    /// The exact commands that change each decision.
    pub change: Vec<Change>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SourceSummary {
    /// url | source
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The id the moments were ranked on (`src_...`, or `url_...` for fetched captions).
    pub ranked_on: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub duration: f64,
    /// Where the ranking transcript came from: `captions` (URL captions, no download), `whisper`
    /// (URL without captions: the audio only was downloaded and transcribed with whisper base),
    /// `source` (the source's transcript, `analyze quick` when it had none) or `none` (dry run
    /// of a URL without captions).
    pub transcript_via: String,
    /// Seconds spent getting the ranking transcript (fetch, or download + whisper).
    pub transcript_s: f64,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Settings {
    pub count: usize,
    pub min_len: f64,
    pub max_len: f64,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    pub quality: RenderQuality,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign: Option<String>,
    pub fixes: bool,
    pub cover: bool,
    pub raw: bool,
    /// URL only: the height the sections are downloaded at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_quality: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Excluded {
    pub start: f64,
    pub end: f64,
    pub reason: String,
}

#[derive(Serialize, JsonSchema, Debug, Default)]
pub struct Ranking {
    pub formula: String,
    pub candidates_considered: usize,
    pub excluded: Vec<Excluded>,
    /// The source's intro trailer as `highlights.candidates` found it (range, the repeated lines
    /// with where their originals play, windows skipped), when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro_trailer: Option<Value>,
}

#[derive(Serialize, JsonSchema, Debug, Default)]
pub struct Summary {
    pub requested: usize,
    pub picked: usize,
    pub created: usize,
    pub rendered: usize,
    pub qc_passed: usize,
    pub exported: usize,
    pub failed: usize,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct AutoOut {
    pub dry_run: bool,
    pub source: SourceSummary,
    pub settings: Settings,
    pub ranking: Ranking,
    pub clips: Vec<AutoClip>,
    pub summary: Summary,
    pub elapsed_s: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Ranking helpers (pure)
// ---------------------------------------------------------------------------------------------

/// First-2-seconds hook score of a window starting at `start`.
pub fn hook_2s(d: &Data, start: f64) -> (f64, Vec<String>, String) {
    let end = start + 2.0;
    let words: Vec<_> = d
        .words
        .iter()
        .filter(|w| {
            let m = (w.start + w.end) / 2.0;
            m >= start - 0.05 && m < end
        })
        .collect();
    let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    let mut notes = Vec::new();
    let Some(first) = words.first() else {
        return (0.0, vec!["no speech in the first 2 s".into()], text);
    };
    let toks = tokens_of(&text);
    let f = features(d, start, end);
    let mut s: f64 = 0.0;
    if first.start - start <= 0.3 {
        s += 0.15;
        notes.push("speech starts at once".into());
    }
    if f.emphasis_count > 0 {
        s += 0.30;
        notes.push(format!("emphasis word in the first 2 s ({})", f.emphasis_words.join(", ")));
    }
    if text.contains('?') {
        s += 0.25;
        notes.push("question in the first 2 s".into());
    }
    const NUMBER_WORDS: &[&str] = &["one", "two", "three", "five", "ten", "hundred", "thousand", "million", "billion", "ek", "do", "teen", "das"];
    if toks.iter().any(|t| t.chars().any(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&t.as_str())) {
        s += 0.15;
        notes.push("number in the first 2 s".into());
    }
    const YOU: &[&str] = &["you", "your", "youre", "yourself", "aap", "aapko", "aapka", "tum", "tumhe", "tumhara"];
    if toks.iter().any(|t| YOU.contains(&t.as_str())) {
        s += 0.15;
        notes.push("speaks to the viewer (you)".into());
    }
    if f.loudness_peak_lu.is_some_and(|lu| lu >= 3.0) {
        s += 0.10;
        notes.push(format!("loud opening (+{:.1} LU over the median)", f.loudness_peak_lu.unwrap_or(0.0)));
    }
    if f.wpm_vs_median.is_some_and(|r| r >= 1.15) {
        s += 0.10;
        notes.push("fast opening (speech rate above the source median)".into());
    }
    if weak_opener_len(&toks) > 0 {
        s -= 0.20;
        notes.push("starts with a weak opener".into());
    }
    ((s.clamp(0.0, 1.0) * 100.0).round() / 100.0, notes, text)
}

fn overlaps(a: (f64, f64), b: (f64, f64)) -> bool {
    a.0 < b.1 - 1e-6 && b.0 < a.1 - 1e-6
}

/// One ranked moment.
#[derive(Clone, Debug)]
pub struct Pick {
    pub start: f64,
    pub end: f64,
    pub start_unit: Option<u32>,
    pub end_unit: Option<u32>,
    pub why: Why,
}

/// Highest `rank_score` first; drops picks overlapping a better pick or an `existing` range.
/// Returns (picks in time order, excluded).
pub fn select(mut pool: Vec<Pick>, count: usize, existing: &[(f64, f64, String)]) -> (Vec<Pick>, Vec<Excluded>) {
    pool.sort_by(|a, b| b.why.rank_score.total_cmp(&a.why.rank_score));
    let mut chosen: Vec<Pick> = Vec::new();
    let mut excluded = Vec::new();
    for p in pool {
        if chosen.len() >= count {
            break;
        }
        if let Some((_, _, by)) = existing.iter().find(|(s, e, _)| overlaps((p.start, p.end), (*s, *e))) {
            excluded.push(Excluded { start: p.start, end: p.end, reason: format!("overlaps existing clip {by}") });
            continue;
        }
        if chosen.iter().any(|c| overlaps((p.start, p.end), (c.start, c.end))) {
            excluded.push(Excluded { start: p.start, end: p.end, reason: "overlaps a better pick".into() });
            continue;
        }
        chosen.push(p);
    }
    chosen.sort_by(|a, b| a.start.total_cmp(&b.start));
    (chosen, excluded)
}

// ---------------------------------------------------------------------------------------------
// Running retention fix commands (the CLI strings `clip.retention` prints)
// ---------------------------------------------------------------------------------------------

fn split_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

fn scalar(s: &str) -> Value {
    if let Ok(n) = s.parse::<f64>() {
        if n.is_finite() {
            return json!(n);
        }
    }
    match s {
        "true" => json!(true),
        "false" => json!(false),
        _ => json!(s),
    }
}

/// `rustler <group> <action> <positional...> --flag value ...` -> (command name, input JSON),
/// positionals mapped through the command's spec. `None` when the command does not exist here.
pub fn parse_cli(reg: &Registry, cmd: &str) -> Option<(String, Value)> {
    let w = split_words(cmd);
    let rest = if w.first().is_some_and(|x| x == "rustler") { &w[1..] } else { &w[..] };
    if rest.len() < 2 {
        return None;
    }
    let name = format!("{}.{}", rest[0], rest[1].replace('-', "_"));
    let spec = reg.describe(&name)?;
    let mut obj = serde_json::Map::new();
    let mut pos = 0usize;
    let mut k = 2;
    while k < rest.len() {
        let t = &rest[k];
        if let Some(flag) = t.strip_prefix("--") {
            let key = flag.replace('-', "_");
            match rest.get(k + 1) {
                Some(v) if !v.starts_with("--") => {
                    obj.insert(key, scalar(v));
                    k += 2;
                }
                _ => {
                    obj.insert(key, json!(true));
                    k += 1;
                }
            }
        } else {
            let slot = spec.positional.get(pos)?;
            // Ids and names stay strings even when they look numeric.
            let v = if slot.ends_with("_id") || *slot == "kind" || *slot == "name" { json!(t) } else { scalar(t) };
            obj.insert((*slot).to_string(), v);
            pos += 1;
            k += 1;
        }
    }
    Some((name, Value::Object(obj)))
}

// ---------------------------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------------------------

/// Every command (the pipeline goes through the same registry an agent calls).
fn inner() -> Registry {
    let mut r = Registry::new();
    crate::register_all(&mut r);
    r
}

/// The context nested commands run with: no progress of their own, never a dry run (the
/// pipeline only calls mutating commands when it is not a dry run).
fn quiet(ctx: &Ctx) -> Ctx {
    let mut c = ctx.clone();
    c.progress = ProgressSink::noop();
    c.dry_run = false;
    c
}

fn emit(ctx: &Ctx, stage: &str, pct: Option<f32>, msg: impl Into<String>) {
    ctx.progress.emit(ProgressEvent { stage: format!("{STAGE}.{stage}"), pct, message: Some(msg.into()), eta_s: None });
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn has_transcript(store: &ProjectStore, id: &str) -> bool {
    analysis_rel(id).is_ok_and(|d| store.resolve(&d.join("transcript.json")).is_file())
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// `02m15s` (file-name safe).
fn stamp(t: f64) -> String {
    let t = t.max(0.0).floor() as u64;
    if t >= 3600 { format!("{}h{:02}m{:02}s", t / 3600, (t % 3600) / 60, t % 60) } else { format!("{:02}m{:02}s", t / 60, t % 60) }
}

/// A short file-name-safe slug of a source title: its first words (at most 3 words / 24
/// characters), lower case ASCII letters and digits; a trailing `[11:54-12:40]` section label is
/// dropped. Empty when nothing ASCII is left.
pub fn title_slug(title: &str) -> String {
    let t = match title.rfind(" [") {
        Some(k) if title.trim_end().ends_with(']') => &title[..k],
        _ => title,
    };
    let mut out = String::new();
    for w in t.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(3) {
        let w = w.to_ascii_lowercase();
        if !out.is_empty() && out.len() + 1 + w.len() > 24 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w[..w.len().min(24)]);
    }
    out
}

/// `base`, or `base-2`, `base-3`, ... : the first not in `taken` (lower-cased: export file names
/// compare case-insensitively on Windows).
pub fn unique_name(base: &str, taken: &std::collections::HashSet<String>) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while taken.contains(&name.to_lowercase()) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Names in use (lower case): clip names of the project and the stems of files in `exports/`.
fn taken_names(store: &ProjectStore) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    if let Ok(p) = store.load_project() {
        for id in &p.clips {
            if let Ok(c) = store.load_clip(id) {
                out.insert(c.name.to_lowercase());
            }
        }
    }
    for e in std::fs::read_dir(store.exports_dir()).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        for ext in [".report.json", ".post.json", ".cover.jpg", ".mp4", ".srt"] {
            if let Some(stem) = n.strip_suffix(ext) {
                out.insert(stem.to_string());
                break;
            }
        }
    }
    out
}

/// Existing clip ranges on the ranked timeline: (start, end, clip id).
fn existing_ranges(store: &ProjectStore, source: Option<&SourceId>, url: Option<&str>) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    let Ok(project) = store.load_project() else { return out };
    for id in &project.clips {
        let Ok(clip) = store.load_clip(id) else { continue };
        for seg in &clip.segments {
            if source == Some(&seg.source_id) {
                out.push((seg.start, seg.end, clip.id.to_string()));
            } else if let Some(u) = url {
                if let Ok(src) = store.load_source(&seg.source_id) {
                    if let SourceOrigin::Url { url: su, original_offset, .. } = &src.origin {
                        if su.trim() == u.trim() {
                            out.push((seg.start + original_offset, seg.end + original_offset, clip.id.to_string()));
                        }
                    }
                }
            }
        }
    }
    out
}

fn retention_fixes(v: &Value) -> Vec<SuggestedFix> {
    let mut out = Vec::new();
    let zone_fixes = v.get("dead_zones").and_then(Value::as_array).into_iter().flatten().flat_map(|z| z.get("fixes").and_then(Value::as_array).into_iter().flatten());
    // Whole-clip fixes (bad takes) come after the dead-zone fixes; never applied here.
    let take_fixes = v.get("take_fixes").and_then(Value::as_array).into_iter().flatten();
    for fx in zone_fixes.chain(take_fixes) {
        out.push(SuggestedFix {
            action: s(fx, "action"),
            command: fx.get("command").and_then(Value::as_str).map(String::from),
            why: s(fx, "why"),
        });
    }
    out
}

fn to_fix(x: crate::hook_suggest::Suggestion) -> SuggestedFix {
    SuggestedFix { action: x.action, command: Some(x.command), why: x.why }
}

/// Hook-title and cold-open suggestions for a created clip (f39), from the words it plays.
/// Never applied: the agent writes the hook and decides the order.
pub fn content_suggestions(store: &ProjectStore, cid: &ClipId) -> Vec<SuggestedFix> {
    let Ok(clip) = store.load_clip(cid) else { return Vec::new() };
    let Some(src) = clip.segments.first().map(|s| s.source_id.clone()) else { return Vec::new() };
    let Ok(Some(t)) = crate::clip_cmds::load_transcript(store, &src) else { return Vec::new() };
    let approx = t.word_timing == rustler_core::WordTiming::Approximate;
    let by = std::collections::HashMap::from([(src, t.words)]);
    let ws: Vec<SWord> = crate::takes_cmds::clip_words(&clip, &by).into_iter().map(|w| SWord { text: w.text, start: w.start, end: w.end }).collect();
    crate::hook_suggest::suggest(&ws, cid.as_str(), 0.0, "")
        .into_iter()
        .map(|mut x| {
            if approx && x.action == "cold_open" {
                x.why.push_str(" (word timings are approximate: check the range with `rustler transcript read-window` first)");
            }
            to_fix(x)
        })
        .collect()
}

/// The same suggestions for a planned pick (dry run), from the ranking transcript; cold-open
/// seconds are made relative to the section a URL run would download (`offset`).
fn planned_suggestions(d: &Data, start: f64, end: f64, offset: f64) -> Vec<SuggestedFix> {
    let ws: Vec<SWord> = d
        .words
        .iter()
        .filter(|w| {
            let m = (w.start + w.end) / 2.0;
            m >= start && m < end && !w.text.trim().is_empty()
        })
        .map(|w| SWord { text: w.text.trim().to_string(), start: w.start, end: w.end })
        .collect();
    crate::hook_suggest::suggest(&ws, "<clip_id>", offset, "").into_iter().map(to_fix).collect()
}

fn dead_zones(v: &Value) -> usize {
    v.get("dead_zones").and_then(Value::as_array).map_or(0, Vec::len)
}

/// What the latest retention measurement `v` still suggests, with a note on the actions
/// `clips.auto` skipped or rolled back (so the agent knows why they are still listed).
fn suggestions_left(v: &Value, applied: &[AppliedFix]) -> Vec<SuggestedFix> {
    let mut out = retention_fixes(v);
    for f in &mut out {
        if f.action == "punch_in" {
            f.why = format!("{} (clips auto never adds zooms: run the command yourself if you want them)", f.why);
        }
        if f.action == "bad_take" {
            f.why = format!("{} (clips auto never cuts takes: read it, then run the command if you agree)", f.why);
        }
        if let Some(a) = applied.iter().find(|a| a.action == f.action && matches!(a.status.as_str(), "skipped" | "rolled_back")) {
            f.why = format!("{} (clips auto {} it: {})", f.why, a.status.replace('_', " "), a.note.clone().unwrap_or_default());
        }
    }
    out
}

/// Seconds kept between the last (first) word and a trimmed tail (head).
pub const WORD_PAD: f64 = 0.2;

/// The trim deltas to use on `seg` so a trim never cuts into a word: the tail stops at least
/// [`WORD_PAD`] after the last word that starts inside the segment, the head at least `WORD_PAD`
/// before the first word. `words` are the source's words; `approximate` is the transcript's
/// `word_timing == approximate` (captions-based: word times are spread over the cue, so a trim
/// could cut a word, and it is skipped). `Ok((start_delta, end_delta, note))` with the deltas to
/// run (a note when one was shortened); `Err(reason)` when the trim is skipped.
pub fn guard_trim(
    seg: &rustler_core::Segment,
    words: &[rustler_core::Word],
    approximate: bool,
    start_delta: Option<f64>,
    end_delta: Option<f64>,
) -> std::result::Result<(Option<f64>, Option<f64>, Option<String>), String> {
    if approximate {
        return Err(format!(
            "word timings are approximate (captions-based transcript), so the trim could cut a word; check the frames and trim yourself, or run `rustler analyze transcribe {}` for exact timings",
            seg.source_id
        ));
    }
    let inside: Vec<&rustler_core::Word> = words.iter().filter(|w| w.end > seg.start + 1e-6 && w.start < seg.end - 1e-6).collect();
    let mut notes = Vec::new();
    // Down to 2 decimals (room is positive): a rounded trim never reaches past the pad.
    let toward_zero = |d: f64| (d * 100.0 + 1e-6).floor() / 100.0;
    let mut sd = start_delta.filter(|d| d.abs() > 1e-9);
    let mut ed = end_delta.filter(|d| d.abs() > 1e-9);
    if let Some(d) = sd.filter(|d| *d > 0.0) {
        if let Some(first) = inside.iter().min_by(|a, b| a.start.total_cmp(&b.start)) {
            if first.start <= seg.start + 0.02 {
                return Err(format!("the segment starts inside or right at the first word \"{}\" ({:.2} s); a head trim would cut it", first.text, first.start));
            }
            let room = toward_zero(first.start - WORD_PAD - seg.start);
            if room < 0.05 {
                return Err(format!("nothing to trim before the first word \"{}\" (starts {:.2} s, segment starts {:.2} s, {WORD_PAD} s kept)", first.text, first.start, seg.start));
            }
            if d > room {
                notes.push(format!("head trim shortened from {d:.2} to {room:.2} s to keep \"{}\" whole", first.text));
                sd = Some(room);
            }
        }
    }
    if let Some(d) = ed.filter(|d| *d < 0.0) {
        if let Some(last) = inside.iter().max_by(|a, b| a.end.total_cmp(&b.end)) {
            if last.end >= seg.end - 0.02 {
                return Err(format!("the segment ends inside or right at the last word \"{}\" ({:.2} s); a tail trim would cut it", last.text, last.end));
            }
            let room = toward_zero(seg.end - (last.end + WORD_PAD));
            if room < 0.05 {
                return Err(format!("nothing to trim after the last word \"{}\" (ends {:.2} s, segment ends {:.2} s, {WORD_PAD} s kept)", last.text, last.end, seg.end));
            }
            if -d > room {
                notes.push(format!("tail trim shortened from {:.2} to {room:.2} s to keep \"{}\" whole", -d, last.text));
                ed = Some(-room);
            }
        }
    }
    Ok((sd, ed, (!notes.is_empty()).then(|| notes.join("; "))))
}

fn factor_points(v: &Value, name: &str) -> Option<f64> {
    v.get("breakdown")?.as_array()?.iter().find(|f| f.get("name").and_then(Value::as_str) == Some(name))?.get("points")?.as_f64()
}

/// Why the retention measurement `after` a fix is worse than `before`: fewer hook or ending
/// points (a trim that cut a word turns a full sentence into a hard stop), or a lower score.
pub fn got_worse(before: &Value, after: &Value) -> Option<String> {
    let mut why = Vec::new();
    for name in ["hook", "ending"] {
        if let (Some(b), Some(a)) = (factor_points(before, name), factor_points(after, name)) {
            if a + 1e-6 < b {
                let kinds = if name == "ending" {
                    format!(" ({} -> {})", s(before.get("ending").unwrap_or(&Value::Null), "kind"), s(after.get("ending").unwrap_or(&Value::Null), "kind"))
                } else {
                    String::new()
                };
                why.push(format!("{name} {b} -> {a} points{kinds}"));
            }
        }
    }
    let score = |v: &Value| v.get("retention_score").and_then(Value::as_u64);
    if let (Some(b), Some(a)) = (score(before), score(after)) {
        if a < b {
            why.push(format!("retention {b} -> {a}"));
        }
    }
    (!why.is_empty()).then(|| why.join(", "))
}

/// True when `source.fetch_transcript` failed because the video has no captions (or none in the
/// asked language): the cue for the whisper fallback. Network / tool errors are not this.
pub fn no_captions(e: &RustlerError) -> bool {
    let m = e.message.to_ascii_lowercase();
    (e.code == ErrorCode::Validation && m.contains("no captions")) || (e.code == ErrorCode::Provider && m.contains("wrote no json3 captions"))
}

/// The whisper fallback for a URL without captions: `source.import <input> --audio-only true`
/// (the whole audio, no video) and `analyze.quick --transcribe always --model base` on it. Returns
/// the audio source to rank on and warnings. `input` is the URL in clips auto (a local file in
/// tests: `source.import` imports it as it is).
pub async fn transcribe_for_ranking(reg: &Registry, ctx: &Ctx, input: &str, name: &str) -> Result<(String, Vec<String>)> {
    let q = quiet(ctx);
    let mut warnings = Vec::new();
    let out = reg.dispatch("source.import", json!({"input": input, "audio_only": true, "name": name}), q.clone()).await?;
    let Some(sid) = out.pointer("/sources/0/id").and_then(Value::as_str).map(String::from) else {
        return Err(RustlerError::new(ErrorCode::Internal, STAGE, "source.import returned no source"));
    };
    let a = reg.dispatch("analyze.quick", json!({"source_id": sid, "transcribe": "always", "model": "base"}), q).await?;
    if a.get("transcript_source").and_then(Value::as_str) != Some("whisper") || a.get("words").and_then(Value::as_u64).unwrap_or(0) == 0 {
        let why: Vec<String> = a.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)).collect();
        return Err(RustlerError::new(ErrorCode::DependencyMissing, STAGE, format!("whisper gave no transcript for {sid}: {}", why.join("; ")))
            .hint("`rustler doctor` (engine ready?), then `rustler analyze transcribe <src>`"));
    }
    if a.get("complete").and_then(Value::as_bool) == Some(false) {
        warnings.push(format!("the whisper transcript of {sid} is incomplete; ranking used what is there (`rustler analyze quick {sid}` continues it)"));
    }
    Ok((sid, warnings))
}

/// Words of `source` and whether their timing is approximate (`None`: no transcript).
fn source_words(store: &ProjectStore, source: &SourceId) -> Option<(Vec<rustler_core::Word>, bool)> {
    let t = crate::clip_cmds::load_transcript(store, source).ok().flatten()?;
    Some((t.words, t.word_timing == rustler_core::WordTiming::Approximate))
}

/// Runs the silence fixes (`trim_head`, `trim_tail`, `remove_pauses`) one by one: a trim never
/// cuts into a word ([`guard_trim`]), trims and pause cuts are skipped on approximate word
/// timings, a fix that changes nothing says so, and a fix after which the hook or ending scores
/// worse ([`got_worse`]) is restored with `history.restore`. Returns what ran and the latest
/// edit-timeline retention measurement.
async fn apply_silence_fixes(x: &Ctx2<'_>, store: &ProjectStore, cid: &ClipId, fixes: &[SuggestedFix], before: Value, ret_in: &Value) -> (Vec<AppliedFix>, Value) {
    let mut current = before;
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for fx in fixes.iter().filter(|f| SILENCE_FIXES.contains(&f.action.as_str())) {
        let Some(cmd) = &fx.command else { continue };
        for part in cmd.split("&&").map(str::trim).filter(|p| !p.is_empty()) {
            if seen.iter().any(|x| x == part) {
                continue;
            }
            seen.push(part.to_string());
            let Some((name, mut input)) = parse_cli(x.reg, part) else {
                let mut a = AppliedFix::new(&fx.action, part, "failed");
                a.error = Some("no such command in this build".into());
                out.push(a);
                continue;
            };
            let clip = match store.load_clip(cid) {
                Ok(c) => c,
                Err(e) => {
                    let mut a = AppliedFix::new(&fx.action, part, "failed");
                    a.error = Some(e.message);
                    out.push(a);
                    continue;
                }
            };
            let rev_before = clip.revision;
            let mut note = None;
            let mut command = part.to_string();
            // Guards that need the words of the clip's sources.
            match name.as_str() {
                "edit.trim" => {
                    // A pause cut before this re-cuts the segments: the tail is the last one now, the head the first.
                    let seg = match (fx.action.as_str(), input.get("segment_id").and_then(Value::as_str)) {
                        ("trim_tail", _) => clip.segments.last(),
                        ("trim_head", _) | (_, None) => clip.segments.first(),
                        (_, Some(id)) => clip.segments.iter().find(|s| s.id.as_str() == id),
                    };
                    if let Some(sg) = seg {
                        if input.get("segment_id").is_some_and(|v| v.as_str() != Some(sg.id.as_str())) {
                            input["segment_id"] = json!(sg.id);
                            command = format!("rustler edit trim {cid} --segment-id {} --end-delta {:.2}", sg.id, f(&input, "end_delta").unwrap_or(0.0));
                        }
                    }
                    let Some(seg) = seg else {
                        out.push(AppliedFix::new(&fx.action, part, "skipped").note("the segment it names is no longer in the clip"));
                        continue;
                    };
                    let Some((words, approx)) = source_words(store, &seg.source_id) else {
                        out.push(AppliedFix::new(&fx.action, part, "skipped").note(format!("source {} has no transcript, so the last/first word is unknown", seg.source_id)));
                        continue;
                    };
                    match guard_trim(seg, &words, approx, f(&input, "start_delta"), f(&input, "end_delta")) {
                        Err(why) => {
                            out.push(AppliedFix::new(&fx.action, part, "skipped").note(why));
                            continue;
                        }
                        Ok((sd, ed, n)) => {
                            if let Some(n) = n {
                                for (k, v) in [("start_delta", sd), ("end_delta", ed)] {
                                    if let Some(v) = v {
                                        input[k] = json!(v);
                                    }
                                }
                                command = match (sd, ed) {
                                    (Some(d), _) if d > 0.0 => format!("rustler edit trim {cid} --start-delta {d:.2}"),
                                    (_, Some(d)) => format!("rustler edit trim {cid} --segment-id {} --end-delta {d:.2}", seg.id),
                                    _ => command,
                                };
                                note = Some(n);
                            }
                        }
                    }
                }
                "edit.remove_pauses" => {
                    if let Some(seg) = clip.segments.iter().find(|s| source_words(store, &s.source_id).is_some_and(|(_, approx)| approx)) {
                        out.push(AppliedFix::new(&fx.action, part, "skipped").note(format!(
                            "word timings of {} are approximate (captions-based transcript): the gaps between words are not real pauses, so nothing reliable can be cut; run `rustler analyze transcribe {}` for exact timings",
                            seg.source_id, seg.source_id
                        )));
                        continue;
                    }
                }
                _ => {}
            }
            let r = x.call(&name, input).await;
            let v = match r {
                Ok(v) => v,
                Err(e) => {
                    let mut a = AppliedFix::new(&fx.action, &command, "failed");
                    a.error = Some(e.message);
                    a.note = note;
                    out.push(a);
                    continue;
                }
            };
            let rev_after = store.load_clip(cid).map(|c| c.revision).unwrap_or(rev_before);
            if rev_after == rev_before {
                let warn: Vec<String> = v.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)).collect();
                let why = if warn.is_empty() { "the command changed nothing".to_string() } else { warn.join("; ") };
                out.push(AppliedFix::new(&fx.action, &command, "no_change").note(why));
                continue;
            }
            // Measure again: roll the fix back when the hook or the ending got worse.
            let after = match x.call("clip.retention", ret_in.clone()).await {
                Ok(a) => a,
                Err(e) => {
                    let mut a = AppliedFix::new(&fx.action, &command, "applied");
                    a.note = Some(format!("{}not re-measured: {}", note.map(|n| format!("{n}; ")).unwrap_or_default(), e.message));
                    a.undo = Some(format!("rustler history restore {cid} {rev_before}   # the clip before this fix (also undoes later edits)"));
                    out.push(a);
                    continue;
                }
            };
            if let Some(worse) = got_worse(&current, &after) {
                match x.call("history.restore", json!({"clip_id": cid, "revision": rev_before})).await {
                    Ok(_) => {
                        let mut a = AppliedFix::new(&fx.action, &command, "rolled_back");
                        a.note = Some(format!("{worse} after the fix; restored revision {rev_before}{}", note.map(|n| format!(" ({n})")).unwrap_or_default()));
                        out.push(a);
                        if let Ok(m) = x.call("clip.retention", ret_in.clone()).await {
                            current = m;
                        }
                    }
                    Err(e) => {
                        let mut a = AppliedFix::new(&fx.action, &command, "applied");
                        a.note = Some(format!("{worse} after the fix, and restoring revision {rev_before} failed: {}", e.message));
                        a.undo = Some(format!("rustler history restore {cid} {rev_before}"));
                        out.push(a);
                        current = after;
                    }
                }
                continue;
            }
            let mut a = AppliedFix::new(&fx.action, &command, "applied");
            a.note = note;
            a.undo = Some(format!("rustler history restore {cid} {rev_before}   # the clip before this fix (also undoes later edits)"));
            out.push(a);
            current = after;
        }
    }
    (out, current)
}

/// Template punch-ins (`(id, start, end)`, edit-timeline seconds) that break the punch-in
/// planner's rules on the timeline described by `input`: closer than `gap` to a cut (or across
/// one), or inside a blocked span (graphic / blur-fit / cutaway segment, transition overlap).
/// Returns `(id, reason)` per broken punch-in. Pure.
pub fn punch_violations(punches: &[(String, f64, f64)], input: &rustler_media::punchin::PunchInput, gap: f64) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (id, s, e) in punches {
        if let Some(c) = input.cuts.iter().copied().find(|&c| c > s - gap && c < e + gap) {
            let why = if c > *s && c < *e {
                format!("{id} ({s:.2}-{e:.2} s) runs across the cut at {c:.2} s")
            } else {
                format!("{id} ({s:.2}-{e:.2} s) is {:.2} s from the cut at {c:.2} s (needs {gap:.1} s)", (c - s).abs().min((c - e).abs()))
            };
            out.push((id.clone(), why));
            continue;
        }
        if let Some((a, b)) = input.blocked.iter().copied().find(|&(a, b)| a < *e && b > *s) {
            out.push((id.clone(), format!("{id} ({s:.2}-{e:.2} s) overlaps a span where no punch-in may be ({a:.2}-{b:.2} s: graphic, blur-fit, cutaway or transition)")));
        }
    }
    out
}

/// [`recheck_template_punch_ins`] for one clip of the open project (the step clips auto runs
/// after the silence fixes; public for tests).
pub async fn recheck_template_punch_ins_for(ctx: &Ctx, clip_id: &ClipId) -> Result<Option<AppliedFix>> {
    let store = ctx.store()?;
    let reg = inner();
    let x = Ctx2 { reg: &reg, ctx };
    Ok(recheck_template_punch_ins(&x, &store, clip_id).await)
}

/// After the silence fixes re-cut the clip, the template's own punch-ins (tagged `by_template`,
/// placed by the template's planner pattern before the cuts existed) are checked against the
/// planner's rules on the new timeline. When one breaks them, all template punch-ins are planned
/// again with the template's pattern (what applying the template to the cut clip would give) and
/// saved through `timeline.apply` (one revision; undo with `history restore`). Nothing is added
/// when the clip has no template punch-in pattern: clips auto never adds zooms itself.
async fn recheck_template_punch_ins(x: &Ctx2<'_>, store: &ProjectStore, cid: &ClipId) -> Option<AppliedFix> {
    use rustler_core::timeline::{EffectItem, EffectKind};
    use crate::template_cmds::{BY_TEMPLATE, PlaceAt};
    let (old, _) = crate::timeline_doc::assemble(store, cid).ok()?;
    let is_tpl_punch = |e: &EffectItem| e.kind == EffectKind::PunchIn && e.extra.contains_key(BY_TEMPLATE);
    let punches: Vec<(String, f64, f64)> = old.tracks.effects.iter().filter(|e| is_tpl_punch(e)).map(|e| (e.id.clone(), e.start, e.end)).collect();
    if punches.is_empty() {
        return None;
    }
    let tref = old.template_ref.as_ref()?;
    let spec = match tref.version {
        Some(v) => format!("{}@{v}", tref.id),
        None => tref.id.clone(),
    };
    let cmd = format!("check the {} template punch-in(s) of {spec} against the cuts", punches.len());
    let tpl = match crate::template_cmds::prepare(x.ctx, STAGE, &spec, None) {
        Ok(t) => t,
        Err(e) => {
            return Some(AppliedFix::new(TEMPLATE_PUNCH_CHECK, &cmd, "skipped").note(format!("template {spec} could not be loaded ({}); its punch-ins were not re-checked", e.message)));
        }
    };
    let patterns: Vec<_> = tpl.effects.iter().flatten().filter(|e| e.kind == EffectKind::PunchIn && matches!(e.at, PlaceAt::Emphasis | PlaceAt::Every)).collect();
    // The timeline without the template's punch-ins: cuts, blocked spans, other changes.
    let mut new = old.clone();
    let removed: Vec<EffectItem> = new.tracks.effects.iter().filter(|e| is_tpl_punch(e)).cloned().collect();
    new.tracks.effects.retain(|e| !is_tpl_punch(e));
    let (input, _) = crate::timeline_doc::punch_input(store, &new);
    let gap = patterns.first().map(|p| crate::template_cmds::punch_config(p).min_cut_gap_s).unwrap_or(rustler_media::punchin::PunchConfig::default().min_cut_gap_s);
    let bad = punch_violations(&punches, &input, gap);
    if bad.is_empty() {
        return Some(AppliedFix::new(TEMPLATE_PUNCH_CHECK, &cmd, "no_change").note(format!(
            "{} template punch-in(s) checked on the cut clip: all at least {gap:.1} s from every cut, none across a cut or on a graphic",
            punches.len()
        )));
    }
    // Re-plan every template punch-in with the template's pattern on the cut timeline.
    let mut tag: serde_json::Map<String, Value> = removed.first().map(|e| e.extra.clone()).unwrap_or_default();
    tag.retain(|k, _| k == BY_TEMPLATE || k == "template_role");
    tag.entry(BY_TEMPLATE.to_string()).or_insert_with(|| json!(format!("{}@{}", tpl.name, tpl.version)));
    tag.entry("template_role".to_string()).or_insert_with(|| json!("effect"));
    for p in &patterns {
        let (input, _) = crate::timeline_doc::punch_input(store, &new);
        for pl in rustler_media::punchin::plan(&input, &crate::template_cmds::punch_config(p)) {
            let id = crate::timeline_doc::next_item_id("fx", new.tracks.effects.iter().map(|x| x.id.as_str()));
            let mut extra = tag.clone();
            if let (Some(fx), Some(fy)) = (pl.focus_x, pl.focus_y) {
                extra.insert("focus_x".into(), json!(fx));
                extra.insert("focus_y".into(), json!(fy));
            }
            new.tracks.effects.push(EffectItem { id, kind: EffectKind::PunchIn, start: pl.start, end: pl.end, amount: Some(pl.amount), extra });
        }
    }
    let after: Vec<(String, f64, f64)> = new.tracks.effects.iter().filter(|e| is_tpl_punch(e)).map(|e| (e.id.clone(), e.start, e.end)).collect();
    let (Ok(a), Ok(b)) = (serde_json::to_value(&old), serde_json::to_value(&new)) else { return None };
    let ops: Vec<Value> = serde_json::to_value(json_patch::diff(&a, &b)).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let reasons: Vec<String> = bad.iter().map(|(_, r)| r.clone()).collect();
    let cmd = format!("re-plan the template punch-ins of {spec} on the cut clip ({} -> {})", punches.len(), after.len());
    match x.call("timeline.apply", json!({"clip_id": cid, "base_revision": old.revision, "patch": ops})).await {
        Ok(_) => {
            let mut fx = AppliedFix::new(TEMPLATE_PUNCH_CHECK, &cmd, "applied").note(format!(
                "{} of {} template punch-in(s) broke the planner rules after the cuts ({}); all re-planned with the template's pattern: {} punch-in(s) now, each at least {gap:.1} s from every cut",
                bad.len(),
                punches.len(),
                reasons.join("; "),
                after.len()
            ));
            fx.undo = Some(format!("rustler history restore {cid} {}   # the clip before the re-plan", old.revision));
            Some(fx)
        }
        Err(e) => {
            let mut fx = AppliedFix::new(TEMPLATE_PUNCH_CHECK, &cmd, "failed").note(reasons.join("; "));
            fx.error = Some(e.message);
            Some(fx)
        }
    }
}

fn qc_summary(v: &Value) -> QcSummary {
    let mut failed = Vec::new();
    let mut warned = Vec::new();
    for c in v.get("checks").and_then(Value::as_array).into_iter().flatten() {
        match c.get("status").and_then(Value::as_str) {
            Some("fail") => failed.push(s(c, "name")),
            Some("warn") => warned.push(s(c, "name")),
            _ => {}
        }
    }
    QcSummary { passed: v.get("passed").and_then(Value::as_bool).unwrap_or(false), failed, warned, verdict: s(v, "verdict") }
}

struct Ctx2<'a> {
    reg: &'a Registry,
    ctx: &'a Ctx,
}

impl Ctx2<'_> {
    async fn call(&self, name: &str, input: Value) -> Result<Value> {
        self.reg.dispatch(name, input, quiet(self.ctx)).await
    }
}

fn fail(c: &mut AutoClip, stage: &str, e: RustlerError) {
    c.status = "failed".into();
    c.failed_stage = Some(stage.into());
    c.error = Some(e);
}

// ---------------------------------------------------------------------------------------------
// clips.auto
// ---------------------------------------------------------------------------------------------

async fn clips_auto(ctx: Ctx, i: AutoIn) -> Result<AutoOut> {
    let t0 = Instant::now();
    let dry = i.dry_run || ctx.dry_run;
    let store = ctx.store()?;
    let url = i.url.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(String::from);
    if url.is_some() == i.source_id.is_some() {
        return Err(validation("give exactly one of url or source_id", "`rustler clips auto --url <url>` or `rustler clips auto --source-id <src_id>`"));
    }
    let count = i.count.unwrap_or(5);
    if count == 0 || count > MAX_COUNT {
        return Err(validation(format!("count {count} is out of range"), format!("use --count 1..{MAX_COUNT} (default 5)")));
    }
    let campaign = match i.campaign.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(id) => Some(load_campaign(&store, id)?),
        None => None,
    };
    let c_dur = campaign.as_ref().and_then(|c| c.duration);
    let min_len = i.min_len.or(c_dur.as_ref().and_then(|d| d.min)).unwrap_or(25.0);
    let max_len = i.max_len.or(c_dur.as_ref().and_then(|d| d.max)).unwrap_or(60.0);
    if !(min_len.is_finite() && max_len.is_finite()) || min_len <= 0.0 || max_len <= min_len {
        return Err(validation(format!("need 0 < min_len < max_len, got {min_len} and {max_len}"), "defaults are --min-len 25 --max-len 60"));
    }
    let platform = i
        .platform
        .clone()
        .or_else(|| campaign.as_ref().and_then(|c| crate::post_cmds::platform_key(c.platform)).map(String::from))
        .unwrap_or_else(|| "shorts".into())
        .to_ascii_lowercase();
    let pad = i.pad.unwrap_or(DEFAULT_PAD).clamp(0.0, 30.0);
    let quality = i.quality.unwrap_or(RenderQuality::Final);
    let fixes_on = i.fixes.unwrap_or(true);
    let cover_on = i.cover.unwrap_or(true);
    let raw = i.raw.unwrap_or(false);
    // An explicit template must resolve before anything is written.
    let template = i.template.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    if let Some(t) = &template {
        crate::template_cmds::prepare(&ctx, STAGE, t, None)?;
    }
    let ranges = i.ranges.clone().unwrap_or_default();
    for r in &ranges {
        if !(r.start.is_finite() && r.end.is_finite()) || r.start < 0.0 || r.end <= r.start {
            return Err(validation(format!("bad range {}..{}", r.start, r.end), "each range needs 0 <= start < end (seconds)"));
        }
    }
    let mut sorted = ranges.clone();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    if sorted.windows(2).any(|w| overlaps((w[0].start, w[0].end), (w[1].start, w[1].end))) {
        return Err(validation("ranges overlap", "give non-overlapping ranges"));
    }

    let reg = inner();
    let x = Ctx2 { reg: &reg, ctx: &ctx };
    let mut warnings: Vec<String> = Vec::new();

    // ---- 1. resolve ------------------------------------------------------------------------
    emit(&ctx, "resolve", Some(0.0), url.clone().unwrap_or_else(|| i.source_id.as_ref().map(|s| s.to_string()).unwrap_or_default()));
    let (summary_src, rank_id, local_source): (SourceSummary, String, Option<SourceId>) = if let Some(u) = &url {
        // One cached yt-dlp lookup (source.info) shared with fetch_transcript and the imports.
        let info = x.call("source.info", json!({"url": u})).await?;
        let mut fetch_in = json!({"url": u});
        if let Some(l) = &i.lang {
            fetch_in["lang"] = json!(l);
        }
        emit(&ctx, "resolve", Some(0.5), "fetching captions (no download)");
        let tt = Instant::now();
        let manual_hint = |e: RustlerError| {
            let hint = format!(
                "{} — or import the video yourself (`rustler source import {u} --sections MM:SS-MM:SS`), run `rustler analyze quick <src>` and then `rustler clips auto --source-id <src>`",
                e.hint.clone().unwrap_or_default()
            );
            e.hint(hint)
        };
        let (pid, via) = match x.call("source.fetch_transcript", fetch_in).await {
            Ok(fetched) => {
                warnings.extend(fetched.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)));
                (fetched.get("pending_id").and_then(Value::as_str).map(String::from).unwrap_or_else(|| pending_id(u)), "captions")
            }
            Err(e) if no_captions(&e) && dry => {
                warnings.push(format!(
                    "{}: a real run downloads the audio only (`rustler source import {u} --audio-only true`) and transcribes it with whisper base (`rustler analyze quick <src> --transcribe always --model base`) to rank; nothing is planned in a dry run",
                    e.message
                ));
                (pending_id(u), "none")
            }
            Err(e) if no_captions(&e) => {
                // No captions: rank on a cheap whisper transcript of the audio only, then download
                // only the chosen sections (as with captions).
                let name = format!("{} (audio for ranking)", s(&info, "title"));
                emit(&ctx, "resolve", Some(0.5), format!("{}: downloading the audio only to transcribe with whisper base", e.message));
                let (sid, w) = transcribe_for_ranking(&reg, &ctx, u, &name).await.map_err(manual_hint)?;
                warnings.push(format!("{}: ranked on a whisper (base) transcript of the audio ({sid}); the chosen sections are downloaded as video", e.message));
                warnings.extend(w);
                (sid, "whisper")
            }
            Err(e) => return Err(manual_hint(e)),
        };
        let transcript_s = round2(tt.elapsed().as_secs_f64());
        emit(&ctx, "resolve", Some(1.0), format!("ranking transcript via {via} in {transcript_s:.1} s"));
        // Save the info next to the fetched captions so candidates see the "most replayed" heatmap.
        if let Ok(dir) = analysis_rel(&pid) {
            let rel = dir.join("source_info.json");
            if !store.resolve(&rel).is_file() {
                if let Err(e) = store.write_json(&rel, &info) {
                    warnings.push(format!("could not save source_info.json for the heatmap: {}", e.message));
                }
            }
        }
        (
            SourceSummary {
                kind: "url".into(),
                url: Some(u.clone()),
                ranked_on: pid.clone(),
                title: Some(s(&info, "title")),
                duration: f(&info, "duration").unwrap_or(0.0),
                transcript_via: via.into(),
                transcript_s,
            },
            pid,
            None,
        )
    } else {
        let sid = i.source_id.clone().unwrap_or_else(SourceId::new);
        let src = load_source_checked(&store, &sid)?;
        let tt = Instant::now();
        if !has_transcript(&store, sid.as_str()) && ranges.is_empty() {
            if dry {
                warnings.push(format!(
                    "source {sid} has no transcript yet: a real run starts with `rustler analyze quick {sid}`; run that (or clips auto without --dry-run) to see the picks"
                ));
            } else {
                emit(&ctx, "analyze", None, format!("analyze quick {sid} (transcript, loudness, scenes)"));
                let q = x.call("analyze.quick", json!({"source_id": sid})).await?;
                warnings.extend(q.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(|w| format!("analyze quick: {w}"))));
            }
        }
        (
            SourceSummary {
                kind: "source".into(),
                url: None,
                ranked_on: sid.to_string(),
                title: Some(src.name.clone()),
                duration: src.probe.duration,
                transcript_via: "source".into(),
                transcript_s: round2(tt.elapsed().as_secs_f64()),
            },
            sid.to_string(),
            Some(sid),
        )
    };

    // ---- 2. rank ---------------------------------------------------------------------------
    emit(&ctx, "rank", Some(0.1), format!("ranking moments on {rank_id}"));
    let mut ranking = Ranking { formula: RANK_FORMULA.into(), ..Ranking::default() };
    let mut data_warn = Vec::new();
    let data = if has_transcript(&store, &rank_id) { load_data(&store, &rank_id, 30.0, &mut data_warn, true).ok() } else { None };
    let hook_of = |start: f64| data.as_ref().map(|d| hook_2s(d, start)).unwrap_or((0.0, vec!["no transcript: hook not measured".into()], String::new()));
    let mut pool: Vec<Pick> = Vec::new();
    let mut chosen_by = "ranking";
    if !ranges.is_empty() {
        chosen_by = "agent";
        for r in &sorted {
            let (h, notes, text) = hook_of(r.start);
            pool.push(Pick {
                start: r.start,
                end: r.end,
                start_unit: None,
                end_unit: None,
                why: Why {
                    chosen_by: "agent".into(),
                    candidate_score: None,
                    hook_2s: h,
                    hook_notes: notes,
                    first_2s_text: text,
                    rank_score: round2(HOOK_WEIGHT * h),
                    reasons: r.note.clone().into_iter().collect(),
                    ..Why::default()
                },
            });
        }
        ranking.candidates_considered = pool.len();
        // Your moments are made as given; say when one sits in the source's intro trailer.
        if let Some(tr) = data.as_ref().and_then(|d| crate::intro_trailer::detect(&d.words, d.duration, d.scene_starts.as_deref())) {
            for r in sorted.iter().filter(|r| overlaps((r.start, r.end), (tr.start, tr.end))) {
                let orig = tr.repeats.iter().find(|x| overlaps((r.start, r.end), (x.early_start, x.early_end)));
                warnings.push(format!(
                    "range {}-{} is inside the source's intro trailer ({}-{}){}",
                    mmss(r.start),
                    mmss(r.end),
                    mmss(tr.start),
                    mmss(tr.end),
                    orig.map(|x| format!("; the original of \"{}\" plays at {}", x.text, mmss(x.later_start))).unwrap_or_default()
                ));
            }
            ranking.intro_trailer = serde_json::to_value(&tr).ok();
        }
        if ranges.len() > count {
            warnings.push(format!("{} ranges given; count {count} ignored (every range is made)", ranges.len()));
        }
    } else if data.is_some() {
        let want = (count * 3).max(count + 5).min(100);
        let cands = x
            .call(
                "highlights.candidates",
                json!({"source_id": rank_id, "count": want, "min_len": min_len, "max_len": max_len, "platform": platform, "include_intro": i.include_intro.unwrap_or(false)}),
            )
            .await?;
        warnings.extend(cands.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(|w| format!("candidates: {w}"))));
        ranking.intro_trailer = cands.get("intro_trailer").filter(|v| !v.is_null()).cloned();
        for c in cands.get("candidates").and_then(Value::as_array).into_iter().flatten() {
            let (Some(st), Some(en)) = (f(c, "start"), f(c, "end")) else { continue };
            let score = f(c, "score").unwrap_or(0.0);
            let (h, notes, text) = hook_of(st);
            pool.push(Pick {
                start: st,
                end: en,
                start_unit: c.get("start_unit").and_then(Value::as_u64).map(|u| u as u32),
                end_unit: c.get("end_unit").and_then(Value::as_u64).map(|u| u as u32),
                why: Why {
                    chosen_by: "ranking".into(),
                    candidate_score: Some(score),
                    hook_2s: h,
                    hook_notes: notes,
                    first_2s_text: text,
                    rank_score: round2(score + HOOK_WEIGHT * h),
                    reasons: c.get("reasons").and_then(Value::as_array).into_iter().flatten().filter_map(|r| r.as_str().map(String::from)).collect(),
                    opening_text: s(c, "opening_text"),
                    closing_text: s(c, "closing_text"),
                },
            });
        }
        ranking.candidates_considered = pool.len();
    }
    let existing = if chosen_by == "ranking" { existing_ranges(&store, local_source.as_ref(), url.as_deref()) } else { Vec::new() };
    let limit = if chosen_by == "agent" { pool.len() } else { count };
    let (picks, excluded) = select(pool, limit, &existing);
    ranking.excluded = excluded;
    if chosen_by == "ranking" && data.is_some() && picks.len() < count {
        warnings.push(format!(
            "only {} of {count} clips found between {min_len} and {max_len} s; widen --min-len/--max-len or read the transcript yourself (`rustler transcript get {rank_id}`)",
            picks.len()
        ));
    }
    emit(&ctx, "rank", Some(0.15), format!("{} moments picked", picks.len()));

    let dl_quality = i.download_quality.clone().filter(|q| !q.trim().is_empty()).unwrap_or_else(|| DEFAULT_DOWNLOAD_QUALITY.to_string());
    let settings = Settings {
        count,
        min_len,
        max_len,
        platform: platform.clone(),
        template: template.clone(),
        quality,
        campaign: campaign.as_ref().map(|c| c.id.clone()),
        fixes: fixes_on,
        cover: cover_on,
        raw,
        download_quality: url.as_ref().map(|_| dl_quality.clone()),
    };

    // Per-clip skeletons. Names are unique in the project and in exports/ (an export of another
    // clip is never overwritten): auto-<rank>-<time in the original video>-<source title slug>,
    // with -2, -3 ... when that is taken.
    let slug = title_slug(summary_src.title.as_deref().unwrap_or_default());
    // A source imported as a section of a URL: name by the time in the original video.
    let stamp_offset = local_source
        .as_ref()
        .and_then(|sid| store.load_source(sid).ok())
        .map(|src| match src.origin {
            SourceOrigin::Url { original_offset, .. } => original_offset,
            _ => 0.0,
        })
        .unwrap_or(0.0);
    let mut taken = taken_names(&store);
    let names: Vec<String> = picks
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let mut base = format!("auto-{}-{}", k + 1, stamp(p.start + stamp_offset));
            if !slug.is_empty() {
                base = format!("{base}-{slug}");
            }
            let n = unique_name(&base, &taken);
            taken.insert(n.to_lowercase());
            n
        })
        .collect();
    let mut clips: Vec<AutoClip> = picks
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let section = url.as_ref().map(|_| (round2((p.start - pad).max(0.0)), round2(if summary_src.duration > 0.0 { (p.end + pad).min(summary_src.duration) } else { p.end + pad })));
            AutoClip {
                rank: k + 1,
                status: if dry { "planned".into() } else { "pending".into() },
                failed_stage: None,
                error: None,
                name: names[k].clone(),
                clip_id: None,
                source_id: local_source.as_ref().map(|s| s.to_string()),
                start: round2(p.start),
                end: round2(p.end),
                mmss: format!("{}-{}", mmss(p.start), mmss(p.end)),
                duration: round2(p.end - p.start),
                section,
                why: p.why.clone(),
                retention: None,
                fixes_applied: Vec::new(),
                fixes_suggested: Vec::new(),
                render: None,
                qc: None,
                cover: None,
                export: None,
                plan: Vec::new(),
                change: Vec::new(),
                warnings: Vec::new(),
            }
        })
        .collect();

    let tpl_flag = template.as_ref().map(|t| format!(" --template {t}")).unwrap_or_default();
    let raw_flag = if raw { " --raw true" } else { "" };
    let create_cmd = |c: &AutoClip, p: &Pick, src: &str, offset: f64| -> String {
        match (p.start_unit, p.end_unit, url.is_some()) {
            (Some(a), Some(b), false) => format!("rustler clip create --name {} --source-id {src} --start-unit {a} --end-unit {b}{tpl_flag}{raw_flag}", c.name),
            _ => format!("rustler clip create --name {} --source-id {src} --start {:.2} --end {:.2}{tpl_flag}{raw_flag}", c.name, p.start - offset, p.end - offset),
        }
    };

    if dry {
        for (c, p) in clips.iter_mut().zip(&picks) {
            if let (Some(u), Some((a, b))) = (&url, c.section) {
                c.plan.push(format!("rustler source import {u} --sections {a:.1}-{b:.1} --quality {dl_quality}   # {PARALLEL_DOWNLOADS} sections at a time"));
                c.plan.push("rustler analyze quick <new src_id> --transcribe always   # whisper on the section (captions if whisper is missing)".into());
                c.plan.push(create_cmd(c, p, "<new src_id>", a));
            } else {
                c.plan.push(create_cmd(c, p, &rank_id, 0.0));
            }
            if let Some(cp) = &campaign {
                c.plan.push(format!("rustler clip meta-set <clip_id> --campaign-id {}", cp.id));
            }
            c.plan.push("rustler clip retention <clip_id> --timeline-only --per-second false".into());
            if fixes_on {
                c.plan.push("apply the retention fixes trim_head / trim_tail / remove_pauses when suggested (trims keep whole words; skipped on approximate word timings; rolled back when the hook or ending gets worse); re-check the template's punch-ins against the new cuts; never add zooms (punch-ins stay in fixes_suggested)".into());
            }
            c.plan.push(format!("rustler render batch --clip-ids <clip_ids> --quality {}", quality_str(quality)));
            c.plan.push("rustler clip retention <clip_id> --per-second false".into());
            c.plan.push("rustler qc run <clip_id> --allow-fail true".into());
            if cover_on {
                c.plan.push("rustler clip cover <clip_id> --auto true   # best effort".into());
            }
            c.plan.push("rustler export clip <clip_id>   # writes <name>.post.json with empty copy for you to write".into());
            // Hook lines and a cold open from the planned words (the real run re-reads the clip).
            if let Some(d) = &data {
                c.fixes_suggested = planned_suggestions(d, p.start, p.end, c.section.map_or(0.0, |s| s.0));
            }
            c.change = vec![
                Change { decision: "this moment".into(), command: format!("rustler clips auto {} --dry-run --ranges <json array of {{start,end}}>   # your own moments", src_arg(&url, &rank_id)) },
                Change { decision: "more candidates".into(), command: format!("rustler highlights candidates {rank_id} --count 20 --min-len {min_len} --max-len {max_len}") },
            ];
        }
        emit(&ctx, "done", Some(1.0), format!("plan: {} clips", clips.len()));
        let next = if clips.is_empty() {
            vec![format!("nothing to plan yet: see warnings; `rustler highlights candidates {rank_id}` lists windows once the source is analysed")]
        } else {
            vec![
                format!("read each pick before committing: rustler transcript read-window {rank_id} --start <start> --end <end>"),
                format!("run it: rustler clips auto {} --count {}", src_arg(&url, &rank_id), clips.len()),
            ]
        };
        let n = clips.len();
        return Ok(AutoOut {
            dry_run: true,
            source: summary_src,
            settings,
            ranking,
            clips,
            summary: Summary { requested: count, picked: n, ..Summary::default() },
            elapsed_s: round2(t0.elapsed().as_secs_f64()),
            warnings,
            next,
        });
    }

    // ---- 3a. URL: download the padded sections, PARALLEL_DOWNLOADS at a time --------------
    // (each import writes only its own source folder and appends one event line).
    let mut imported: Vec<Option<Result<Value>>> = (0..clips.len()).map(|_| None).collect();
    if let Some(u) = &url {
        let jobs: Vec<(usize, Value)> = clips
            .iter()
            .enumerate()
            .filter_map(|(k, c)| c.section.map(|(a, b)| (k, json!({"input": u, "sections": [format!("{a:.2}-{b:.2}")], "name": c.name, "quality": dl_quality}))))
            .collect();
        let n = jobs.len();
        let started = Instant::now();
        emit(&ctx, "import", Some(0.15), format!("downloading {n} sections at {dl_quality}p, {PARALLEL_DOWNLOADS} at a time"));
        let mut queue = jobs.into_iter();
        let mut set = tokio::task::JoinSet::new();
        let mut done = 0usize;
        loop {
            while set.len() < PARALLEL_DOWNLOADS && !ctx.cancel.is_cancelled() {
                let Some((k, input)) = queue.next() else { break };
                let cctx = quiet(&ctx);
                set.spawn(async move {
                    let reg = inner();
                    let r = reg.dispatch("source.import", input, cctx).await;
                    (k, r)
                });
            }
            let Some(res) = set.join_next().await else { break };
            done += 1;
            let name = match res {
                Ok((k, r)) => {
                    let ok = if r.is_ok() { "downloaded" } else { "failed" };
                    imported[k] = Some(r);
                    format!("{} {ok}", clips[k].name)
                }
                Err(e) => format!("a download task stopped: {e}"),
            };
            // Completions so far set the pace (they already overlap PARALLEL_DOWNLOADS at a time).
            let el = started.elapsed().as_secs_f32();
            let eta = el / done as f32 * (n - done) as f32;
            ctx.progress.emit(ProgressEvent {
                stage: format!("{STAGE}.import"),
                pct: Some(0.15 + 0.15 * done as f32 / n.max(1) as f32),
                message: Some(format!("{done}/{n} sections: {name}")),
                eta_s: Some(eta),
            });
        }
    }

    // ---- 3b. create + retention fixes (sequential: each writes project.json) ---------------
    let total = clips.len().max(1);
    for (k, (c, p)) in clips.iter_mut().zip(&picks).enumerate() {
        if ctx.cancel.is_cancelled() {
            fail(c, "create", RustlerError::new(ErrorCode::Cancelled, STAGE, "cancelled"));
            continue;
        }
        let base = if url.is_some() { 0.3 + 0.3 * (k as f32 / total as f32) } else { 0.15 + 0.45 * (k as f32 / total as f32) };
        emit(&ctx, "create", Some(base), format!("{} ({}/{}) {}", c.name, k + 1, total, c.mmss));
        // URL: the downloaded section, analysed.
        let (src_id, offset) = if let (Some(_), Some((a, _))) = (&url, c.section) {
            let out = match imported[k].take() {
                Some(Ok(v)) => v,
                Some(Err(e)) => {
                    fail(c, "import", e);
                    continue;
                }
                None => {
                    let why = if ctx.cancel.is_cancelled() { "cancelled" } else { "the download task stopped before it finished" };
                    fail(c, "import", RustlerError::new(ErrorCode::Internal, STAGE, why));
                    continue;
                }
            };
            let Some(sid) = out.pointer("/sources/0/id").and_then(Value::as_str).map(String::from) else {
                fail(c, "import", RustlerError::new(ErrorCode::Internal, STAGE, "source.import returned no source"));
                continue;
            };
            c.warnings.extend(out.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)));
            // Whisper on the short section: exact word timings (the silence fixes need them) and no
            // caption markup ("[ Laughter ]", "-" speaker dashes) in the burned captions. The
            // fetched captions are the fallback when whisper is not available.
            emit(&ctx, "analyze", Some(base), format!("{}: analyze quick {sid} --transcribe always", c.name));
            match x.call("analyze.quick", json!({"source_id": sid, "transcribe": "always"})).await {
                Ok(v) if v.get("transcript_source").and_then(Value::as_str) == Some("whisper") => {}
                Ok(v) => {
                    let why: Vec<String> = v.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)).collect();
                    if let Err(e) = x.call("analyze.quick", json!({"source_id": sid})).await {
                        fail(c, "analyze", e);
                        c.source_id = Some(sid);
                        continue;
                    }
                    c.warnings.push(format!(
                        "whisper did not transcribe {sid} ({}); the fetched captions are used (approximate timings: trims and pause cuts are skipped; captions may show markup like [ Laughter ])",
                        why.join("; ")
                    ));
                }
                Err(e) => {
                    fail(c, "analyze", e);
                    c.source_id = Some(sid);
                    continue;
                }
            }
            (sid, a)
        } else {
            (rank_id.clone(), 0.0)
        };
        c.source_id = Some(src_id.clone());

        let mut input = json!({"name": c.name, "source_id": src_id});
        match (p.start_unit, p.end_unit, url.is_some()) {
            (Some(a), Some(b), false) => {
                input["start_unit"] = json!(a);
                input["end_unit"] = json!(b);
            }
            _ => {
                input["start"] = json!(round2((p.start - offset).max(0.0)));
                input["end"] = json!(round2(p.end - offset));
            }
        }
        if let Some(t) = &template {
            input["template"] = json!(t);
        }
        if raw {
            input["raw"] = json!(true);
        }
        let created = match x.call("clip.create", input).await {
            Ok(v) => v,
            Err(e) => {
                fail(c, "create", e);
                continue;
            }
        };
        let Some(cid) = created.get("id").and_then(|v| serde_json::from_value::<ClipId>(v.clone()).ok()) else {
            fail(c, "create", RustlerError::new(ErrorCode::Internal, STAGE, "clip.create returned no id"));
            continue;
        };
        c.clip_id = Some(cid.clone());
        c.warnings.extend(created.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)));
        let undo_smart = created.pointer("/smart/undo_all").and_then(Value::as_str).map(String::from);
        c.change = changes(c, p, &cid, &rank_id, &src_id, url.is_some(), template.as_deref(), undo_smart, min_len, max_len);

        if let Some(cp) = &campaign {
            if let Err(e) = x.call("clip.meta_set", json!({"clip_id": cid, "campaign_id": cp.id})).await {
                c.warnings.push(format!("could not attach campaign {}: {}", cp.id, e.message));
            }
        }

        // Retention on the edit timeline, then the cheap fixes it suggests.
        emit(&ctx, "fix", Some(base + 0.2 / total as f32), format!("{}: retention", c.name));
        let ret_in = json!({"clip_id": cid, "timeline_only": true, "per_second": false});
        match x.call("clip.retention", ret_in.clone()).await {
            Ok(r0) => {
                let mut summary = RetentionSummary {
                    before_fixes: r0.get("retention_score").and_then(Value::as_u64).map(|v| v as u32),
                    score: r0.get("retention_score").and_then(Value::as_u64).map(|v| v as u32),
                    measured_on: Some(s(&r0, "measured_on")),
                    dead_zones_left: dead_zones(&r0),
                };
                let mut latest = r0.clone();
                if fixes_on {
                    // Silence fixes one by one (word-safe trims, honest no-ops, roll back what hurts the hook or ending).
                    let (mut applied, r1) = apply_silence_fixes(&x, &store, &cid, &retention_fixes(&r0), r0, &ret_in).await;
                    latest = r1;
                    // The cuts may have moved: the template's own punch-ins are checked against them
                    // (and re-planned with the template's pattern when one broke the rules). No zoom
                    // is ever added by clips auto itself; retention punch-ins stay suggestions.
                    if let Some(chk) = recheck_template_punch_ins(&x, &store, &cid).await {
                        if chk.changed {
                            if let Ok(r2) = x.call("clip.retention", ret_in.clone()).await {
                                latest = r2;
                            }
                        }
                        applied.push(chk);
                    }
                    summary.score = latest.get("retention_score").and_then(Value::as_u64).map(|v| v as u32);
                    summary.dead_zones_left = dead_zones(&latest);
                    for a in applied.iter().filter(|a| a.changed) {
                        if let Some(u) = &a.undo {
                            c.change.push(Change { decision: format!("fix {} ({})", a.action, a.command), command: u.clone() });
                        }
                    }
                    c.fixes_applied = applied;
                }
                c.fixes_suggested = suggestions_left(&latest, &c.fixes_applied);
                c.fixes_suggested.extend(content_suggestions(&store, &cid));
                c.retention = Some(summary);
            }
            Err(e) => {
                c.warnings.push(format!("retention not measured: {}", e.message));
                c.fixes_suggested.extend(content_suggestions(&store, &cid));
            }
        }
        c.status = "created".into();
    }

    // ---- 4. render (batch, progress with ETA) ----------------------------------------------
    let to_render: Vec<ClipId> = clips.iter().filter(|c| c.status == "created").filter_map(|c| c.clip_id.clone()).collect();
    if !to_render.is_empty() {
        emit(&ctx, "render", Some(0.6), format!("rendering {} clips ({})", to_render.len(), quality_str(quality)));
        let outer = ctx.progress.clone();
        let started = Instant::now();
        let mut rctx = quiet(&ctx);
        rctx.progress = ProgressSink::new(move |e: ProgressEvent| {
            if e.stage != "render.batch" {
                return;
            }
            let eta = e.pct.filter(|p| *p > 0.0 && *p < 1.0).map(|p| started.elapsed().as_secs_f32() * (1.0 - p) / p);
            outer.emit(ProgressEvent {
                stage: format!("{STAGE}.render"),
                pct: e.pct.map(|p| 0.6 + 0.3 * p),
                message: e.message,
                eta_s: eta.or(e.pct.filter(|p| *p >= 1.0).map(|_| 0.0)),
            });
        });
        // The GPU encoder when one passes the probe (`rustler doctor` lists them); libx264 otherwise.
        match reg.dispatch("render.batch", json!({"clip_ids": to_render, "quality": quality, "prefer_gpu": true}), rctx).await {
            Ok(b) => {
                for r in b.get("results").and_then(Value::as_array).into_iter().flatten() {
                    let id = s(r, "clip_id");
                    let Some(c) = clips.iter_mut().find(|c| c.clip_id.as_ref().is_some_and(|x| x.as_str() == id)) else { continue };
                    c.render = Some(json!({
                        "status": r.get("status"), "path": r.get("path"), "encoder": r.get("encoder"),
                        "duration": r.get("duration"), "render_id": r.get("render_id"),
                    }));
                    if s(r, "status") != "rendered" {
                        let err = r
                            .get("error")
                            .and_then(|e| serde_json::from_value::<RustlerError>(e.clone()).ok())
                            .unwrap_or_else(|| RustlerError::new(ErrorCode::Internal, "render.batch", format!("render {}", s(r, "status"))));
                        fail(c, "render", err);
                    }
                }
            }
            Err(e) => {
                for c in clips.iter_mut().filter(|c| c.status == "created") {
                    fail(c, "render", e.clone());
                }
            }
        }
    }

    // ---- 5. retention on the render, QC, export ---------------------------------------------
    let n_render = clips.iter().filter(|c| c.status == "created").count().max(1);
    let mut k = 0usize;
    for c in clips.iter_mut().filter(|c| c.status == "created") {
        k += 1;
        let Some(cid) = c.clip_id.clone() else { continue };
        emit(&ctx, "qc", Some(0.9 + 0.1 * (k as f32 / n_render as f32)), format!("{}: retention + qc + export", c.name));
        match x.call("clip.retention", json!({"clip_id": cid, "per_second": false})).await {
            Ok(r) => {
                let sum = c.retention.get_or_insert_with(RetentionSummary::default);
                sum.score = r.get("retention_score").and_then(Value::as_u64).map(|v| v as u32);
                sum.measured_on = Some(s(&r, "measured_on"));
                sum.dead_zones_left = dead_zones(&r);
                // The render shows what is still dead (frame changes, real loudness): list those fixes.
                c.fixes_suggested = suggestions_left(&r, &c.fixes_applied);
                c.fixes_suggested.extend(content_suggestions(&store, &cid));
            }
            Err(e) => c.warnings.push(format!("retention on the render not measured: {}", e.message)),
        }
        let qc = match x.call("qc.run", json!({"clip_id": cid, "allow_fail": true})).await {
            Ok(v) => qc_summary(&v),
            Err(e) => {
                fail(c, "qc", e);
                continue;
            }
        };
        // Export when QC passed, or when the only failure is the copy the agent still writes.
        let copy_only = !qc.passed && qc.failed.iter().all(|n| n == "meta_present");
        let qc_ok = qc.passed || copy_only;
        let failed_checks = qc.failed.clone();
        c.qc = Some(qc);
        if !qc_ok {
            fail(
                c,
                "qc",
                RustlerError::new(ErrorCode::QcFailed, "qc.run", format!("QC failed: {}", failed_checks.join(", ")))
                    .hint(format!("rustler fix suggest {cid}   then re-render; export with `rustler export clip {cid}`")),
            );
            continue;
        }
        if cover_on {
            match x.call("clip.cover", json!({"clip_id": cid, "auto": true})).await {
                Ok(v) => {
                    c.cover = v.get("image").and_then(Value::as_str).map(PathBuf::from);
                    c.change.push(Change { decision: format!("cover frame at {:.2} s", f(&v, "out_t").unwrap_or(0.0)), command: format!("rustler clip cover {cid} --out-t <seconds>   then   rustler export clip {cid}") });
                }
                Err(e) => c.warnings.push(format!("no cover picked (export goes on without one): {}", e.message)),
            }
        }
        match x.call("export.clip", json!({"clip_id": cid, "allow_fail": copy_only})).await {
            Ok(v) => {
                c.export = Some(ExportSummary {
                    video: PathBuf::from(s(&v, "video")),
                    post: PathBuf::from(s(&v, "post")),
                    report: PathBuf::from(s(&v, "report")),
                    needs_copy: copy_only || v.get("warnings").and_then(Value::as_array).is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("no posting copy")))),
                });
                c.status = "done".into();
            }
            Err(e) => fail(c, "export", e),
        }
    }

    // ---- report ------------------------------------------------------------------------------
    let created = clips.iter().filter(|c| c.clip_id.is_some()).count();
    let rendered = clips.iter().filter(|c| c.render.as_ref().is_some_and(|r| r.get("status").and_then(Value::as_str) == Some("rendered"))).count();
    let summary = Summary {
        requested: count,
        picked: clips.len(),
        created,
        rendered,
        qc_passed: clips.iter().filter(|c| c.qc.as_ref().is_some_and(|q| q.passed)).count(),
        exported: clips.iter().filter(|c| c.export.is_some()).count(),
        failed: clips.iter().filter(|c| c.status == "failed").count(),
    };
    log_event(&ctx, &store, STAGE, json!({"ranked_on": rank_id, "picked": summary.picked, "exported": summary.exported, "failed": summary.failed}))?;
    let mut next = Vec::new();
    if summary.exported > 0 {
        next.push("look at each export (open the video or `rustler see preview <clip_id>`) before calling it done".into());
        next.push("write the posting copy per clip: rustler clip meta-context <clip_id>, then rustler clip meta-set <clip_id> --input-json @meta.json, then rustler export clip <clip_id> to refresh <name>.post.json".into());
        next.push("record your own judgement: rustler clip score-set <clip_id> --hook H --flow F --value V --trend T --notes \"why\"".into());
    }
    if summary.failed > 0 {
        next.push("failed clips: see failed_stage, error and change[]; fix and continue with the single commands (render run / qc run / export clip)".into());
    }
    next.push("publishing is manual: Rustler never uploads".into());
    emit(&ctx, "done", Some(1.0), format!("{} exported, {} failed", summary.exported, summary.failed));
    Ok(AutoOut {
        dry_run: false,
        source: summary_src,
        settings,
        ranking,
        clips,
        summary,
        elapsed_s: round2(t0.elapsed().as_secs_f64()),
        warnings,
        next,
    })
}

fn quality_str(q: RenderQuality) -> String {
    serde_json::to_value(q).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "final".into())
}

fn src_arg(url: &Option<String>, rank_id: &str) -> String {
    match url {
        Some(u) => format!("--url {u}"),
        None => format!("--source-id {rank_id}"),
    }
}

/// The commands that change each decision `clips.auto` made for one clip.
#[allow(clippy::too_many_arguments)]
fn changes(
    c: &AutoClip,
    p: &Pick,
    cid: &ClipId,
    rank_id: &str,
    src_id: &str,
    from_url: bool,
    template: Option<&str>,
    undo_smart: Option<String>,
    min_len: f64,
    max_len: f64,
) -> Vec<Change> {
    let mut v = Vec::new();
    let around = (p.start - 15.0).max(0.0);
    v.push(Change {
        decision: format!("range {} ({})", c.mmss, c.why.chosen_by),
        command: match (p.start_unit, p.end_unit, from_url) {
            (Some(a), Some(b), false) => format!("rustler edit set-range {cid} --start-unit {a} --end-unit {b}   # change the unit ids (rustler transcript read-window {src_id} --start {around:.1} --end {:.1})", p.end + 15.0),
            _ => format!("rustler transcript get {src_id}   then   rustler edit set-range {cid} --start-unit <id> --end-unit <id>"),
        },
    });
    v.push(Change { decision: "a different moment".into(), command: format!("rustler highlights candidates {rank_id} --count 20 --min-len {min_len} --max-len {max_len}   then its create_cmd") });
    v.push(Change {
        decision: "smart defaults (layout, framing, filler and pause trim)".into(),
        command: undo_smart.unwrap_or_else(|| format!("rustler history restore {cid} 1   # the raw clip")),
    });
    v.push(Change {
        decision: match template {
            Some(t) => format!("template {t}"),
            None => "look (no template given)".into(),
        },
        command: format!("rustler template apply <name> --clip-ids {cid}   # another look; see rustler template list"),
    });
    v.push(Change { decision: "your score".into(), command: format!("rustler clip score-set {cid} --hook H --flow F --value V --trend T --notes \"why\"") });
    v.push(Change { decision: "posting copy (empty)".into(), command: format!("rustler clip meta-context {cid}   then   rustler clip meta-set {cid} --input-json @meta.json") });
    v.push(Change { decision: "render / qc / export".into(), command: format!("rustler render run {cid} && rustler qc run {cid} && rustler export clip {cid}") });
    v
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "clips.auto",
        "Optional one-call pipeline over existing commands: resolve a URL (cached info + captions, sections only) or a source, rank moments (highlights.candidates + first-2-s hook), create clips (smart defaults + template), apply cheap retention fixes (silence trims and pause cuts only; never zooms: punch-ins are suggested), render.batch with ETA, qc.run, export.clip (post.json skeleton, copy left to you). Per-clip report with why, retention, QC, export paths and the commands to change each decision; one clip failing never stops the others. --dry-run plans only",
        &[],
        Annotations { long_running: true, network: true, ..Annotations::default() },
        vec![
            Example { description: "plan 5 clips from an imported source", cli: "rustler clips auto --source-id src_X --dry-run" },
            Example { description: "3 clips from a YouTube video with a template", cli: "rustler clips auto --url https://youtu.be/ID --count 3 --template podcast-viral" },
            Example { description: "your own moments, the rest automated", cli: "rustler clips auto --source-id src_X --input-json @ranges.json" },
        ],
        clips_auto,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(start: f64, end: f64, score: f64) -> Pick {
        Pick { start, end, start_unit: None, end_unit: None, why: Why { rank_score: score, ..Why::default() } }
    }

    #[test]
    fn select_respects_count_overlap_and_existing() {
        let pool = vec![pick(0.0, 30.0, 50.0), pick(20.0, 50.0, 90.0), pick(60.0, 90.0, 40.0), pick(100.0, 130.0, 70.0), pick(140.0, 170.0, 60.0)];
        let existing = vec![(95.0, 105.0, "clp_old".to_string())];
        let (chosen, excluded) = select(pool, 2, &existing);
        assert_eq!(chosen.len(), 2);
        // 20-50 (90) first, 100-130 (70) overlaps existing, 140-170 (60) next. Time order out.
        assert_eq!((chosen[0].start, chosen[1].start), (20.0, 140.0));
        assert!(excluded.iter().any(|e| e.reason.contains("clp_old")));
    }

    #[test]
    fn parse_cli_maps_positionals_and_flags() {
        let reg = inner();
        let (name, v) = parse_cli(&reg, "rustler effect add clp_x punch-in --start 1.50 --end 2.70").unwrap();
        assert_eq!(name, "effect.add");
        assert_eq!(v, json!({"clip_id": "clp_x", "kind": "punch-in", "start": 1.5, "end": 2.7}));
        let (name, v) = parse_cli(&reg, "rustler edit trim clp_x --segment-id seg_1 --end-delta -0.40").unwrap();
        assert_eq!(name, "edit.trim");
        assert_eq!(v, json!({"clip_id": "clp_x", "segment_id": "seg_1", "end_delta": -0.4}));
        let (name, _) = parse_cli(&reg, "rustler edit remove-pauses clp_x --min-gap 0.8").unwrap();
        assert_eq!(name, "edit.remove_pauses");
        assert!(parse_cli(&reg, "rustler nothing here clp_x").is_none());
        assert_eq!(split_words("a \"b c\" d"), vec!["a", "b c", "d"]);
    }

    #[test]
    fn stamp_is_file_safe() {
        assert_eq!(stamp(75.4), "01m15s");
        assert_eq!(stamp(3725.0), "1h02m05s");
    }

    #[test]
    fn slug_and_unique_names() {
        assert_eq!(title_slug("Why Your Communication Makes People Ignore You | Vinh Giang [11:54-12:40]"), "why-your-communication");
        assert_eq!(title_slug("talk.mp4"), "talk-mp4");
        assert_eq!(title_slug("मेरा क्लिप"), "");
        let taken: std::collections::HashSet<String> = ["auto-1-00m03s".to_string(), "auto-1-00m03s-2".to_string()].into_iter().collect();
        assert_eq!(unique_name("auto-1-00m03s", &taken), "auto-1-00m03s-3");
        assert_eq!(unique_name("Auto-1-00m03s", &taken), "Auto-1-00m03s-3", "case-insensitive");
        assert_eq!(unique_name("auto-2-00m03s", &taken), "auto-2-00m03s");
    }

    fn seg(start: f64, end: f64) -> rustler_core::Segment {
        serde_json::from_value(json!({"id": "seg_1", "source_id": "src_1", "start": start, "end": end, "layout": {"preset": "center-crop", "regions": [], "background": "black"}})).unwrap()
    }

    fn w(i: u32, text: &str, start: f64, end: f64) -> rustler_core::Word {
        rustler_core::Word { i, text: text.into(), start, end, conf: None, speaker: None }
    }

    #[test]
    fn trims_keep_whole_words_and_skip_approximate_timings() {
        let words = vec![w(0, "You're", 2.5, 2.9), w(1, "actually", 25.0, 25.6), w(2, "have.", 25.7, 26.1)];
        let s = seg(2.14, 26.96);
        // The trial-2 case: -1.11 would end at 25.85, inside "have." -> shortened to end 0.2 s after it.
        let (sd, ed, note) = guard_trim(&s, &words, false, None, Some(-1.11)).unwrap();
        assert_eq!(sd, None);
        assert!((ed.unwrap() - -0.66).abs() < 1e-9, "{ed:?}");
        assert!(26.96 + ed.unwrap() >= 26.1 + WORD_PAD - 1e-9);
        assert!(note.unwrap().contains("have."));
        // A trim that fits is kept as it is.
        assert_eq!(guard_trim(&s, &words, false, None, Some(-0.4)).unwrap(), (None, Some(-0.4), None));
        // Approximate timings: skipped.
        assert!(guard_trim(&s, &words, true, None, Some(-0.4)).unwrap_err().contains("approximate"));
        // The segment already ends inside the last word.
        assert!(guard_trim(&seg(2.14, 25.9), &words, false, None, Some(-0.3)).unwrap_err().contains("have."));
        // Head: never past the first word minus the pad; nothing left -> skipped.
        let (sd, _, _) = guard_trim(&s, &words, false, Some(1.0), None).unwrap();
        assert!((sd.unwrap() - 0.16).abs() < 1e-9, "{sd:?}");
        assert!(guard_trim(&seg(2.4, 26.96), &words, false, Some(0.3), None).is_err());
    }

    #[test]
    fn worse_hook_or_ending_is_detected() {
        let m = |hook: f64, ending: f64, kind: &str, score: u64| {
            json!({"retention_score": score, "ending": {"kind": kind}, "breakdown": [{"name": "hook", "points": hook}, {"name": "ending", "points": ending}, {"name": "silence", "points": 2.0}]})
        };
        let why = got_worse(&m(10.0, 6.0, "sentence_final", 53), &m(10.0, 0.0, "hard_stop", 55)).unwrap();
        assert!(why.contains("ending 6 -> 0") && why.contains("hard_stop"), "{why}");
        assert!(got_worse(&m(10.0, 6.0, "sentence_final", 53), &m(10.0, 6.0, "sentence_final", 56)).is_none());
        assert!(got_worse(&m(10.0, 6.0, "sentence_final", 53), &m(10.0, 6.0, "sentence_final", 50)).unwrap().contains("retention 53 -> 50"));
    }
}
