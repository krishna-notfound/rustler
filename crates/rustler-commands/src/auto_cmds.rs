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
//!    (trim head/tail silence, remove pauses, punch-ins). Text pops and cutaways are creative:
//!    they are listed, never applied.
//! 4. render — `render.batch` (progress with `eta_s`), `clip.retention` on the render, `qc.run`,
//!    `clip.cover --auto` (best effort), `export.clip` (writes `<name>.post.json`: title/description/hashtags stay empty for the
//!    agent to write with `clip.meta_set`; Rustler never writes copy and never publishes).
//! 5. report — per clip: range, why, retention, QC, export paths, commands to change each decision.
//!
//! A failure in one clip is recorded on that clip (`failed_stage`, `error`) and the others go on.

use crate::campaign_cmds::load_campaign;
use crate::clip_cmds::log_event;
use crate::highlights_cmds::{Data, features, load_data, tokens_of, weak_opener_len};
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
/// Retention fix actions applied automatically (deterministic, non-creative).
const SILENCE_FIXES: &[&str] = &["trim_head", "trim_tail", "remove_pauses"];
const VISUAL_FIXES: &[&str] = &["punch_in"];

pub const RANK_FORMULA: &str = "rank_score = highlights.candidates score (0-100) + 20 x hook_2s (0..1). \
hook_2s over the first 2 s of the window: +0.15 speech starts within 0.3 s, +0.30 emphasis word, +0.25 question, \
+0.15 number, +0.15 direct address (you/your/aap/tum), +0.10 loudness peak >= 3 LU over the source median, \
+0.10 speech rate >= 1.15 x the source median, -0.20 weak opener (so/and/um/...); clamped to 0..1. \
Picks: highest rank_score first, no overlap with each other or with clips that already cover the same moment.";

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
    /// URL only: download height 480 | 720 | 1080 | best (default: `source.import` default).
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
    /// Apply the cheap retention fixes (trim silence, remove pauses, punch-ins) (default true).
    pub fixes: Option<bool>,
    /// Pick a cover frame with `clip.cover --auto` before export, best effort (default true).
    pub cover: Option<bool>,
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
    pub command: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Command that takes this fix back out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fixes_applied: Vec<AppliedFix>,
    /// Retention fixes that need your judgement (text pops, cutaways) or were turned off.
    #[serde(skip_serializing_if = "Vec::is_empty")]
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
    for z in v.get("dead_zones").and_then(Value::as_array).into_iter().flatten() {
        for fx in z.get("fixes").and_then(Value::as_array).into_iter().flatten() {
            out.push(SuggestedFix {
                action: s(fx, "action"),
                command: fx.get("command").and_then(Value::as_str).map(String::from),
                why: s(fx, "why"),
            });
        }
    }
    out
}

fn dead_zones(v: &Value) -> usize {
    v.get("dead_zones").and_then(Value::as_array).map_or(0, Vec::len)
}

/// Applies the fixes whose action is in `actions` (each distinct command once). Returns what ran.
async fn apply_fixes(reg: &Registry, ctx: &Ctx, fixes: &[SuggestedFix], actions: &[&str]) -> Vec<AppliedFix> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for fx in fixes.iter().filter(|f| actions.contains(&f.action.as_str())) {
        let Some(cmd) = &fx.command else { continue };
        for part in cmd.split("&&").map(str::trim).filter(|p| !p.is_empty()) {
            if seen.iter().any(|x| x == part) {
                continue;
            }
            seen.push(part.to_string());
            let Some((name, input)) = parse_cli(reg, part) else {
                out.push(AppliedFix { action: fx.action.clone(), command: part.into(), ok: false, error: Some("no such command in this build".into()), undo: None });
                continue;
            };
            let clip = s(&input, "clip_id");
            let r = reg.dispatch(&name, input, quiet(ctx)).await;
            // Effects live outside the clip revisions: undo by id. Edits are revisions: history undo.
            let undo = match (&r, name.as_str()) {
                (Ok(v), "effect.add") => {
                    let ids: Vec<String> = v.get("effect_ids").and_then(Value::as_array).into_iter().flatten().filter_map(|x| x.as_str().map(|id| format!("rustler effect remove {clip} {id}"))).collect();
                    (!ids.is_empty()).then(|| ids.join(" && "))
                }
                (Ok(_), _) => Some(format!("rustler history undo {clip}   # undoes the latest revision; undo later edits first")),
                (Err(_), _) => None,
            };
            out.push(AppliedFix { action: fx.action.clone(), command: part.into(), ok: r.is_ok(), error: r.err().map(|e| e.message), undo });
        }
    }
    out
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
        let fetched = x.call("source.fetch_transcript", fetch_in).await.map_err(|e| {
            let hint = format!(
                "{} — or import the video yourself (`rustler source import {u} --sections MM:SS-MM:SS`), run `rustler analyze quick <src>` and then `rustler clips auto --source-id <src>`",
                e.hint.clone().unwrap_or_default()
            );
            e.hint(hint)
        })?;
        let pid = fetched.get("pending_id").and_then(Value::as_str).map(String::from).unwrap_or_else(|| pending_id(u));
        warnings.extend(fetched.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)));
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
            SourceSummary { kind: "url".into(), url: Some(u.clone()), ranked_on: pid.clone(), title: Some(s(&info, "title")), duration: f(&info, "duration").unwrap_or(0.0) },
            pid,
            None,
        )
    } else {
        let sid = i.source_id.clone().unwrap_or_else(SourceId::new);
        let src = load_source_checked(&store, &sid)?;
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
            SourceSummary { kind: "source".into(), url: None, ranked_on: sid.to_string(), title: Some(src.name.clone()), duration: src.probe.duration },
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
        if ranges.len() > count {
            warnings.push(format!("{} ranges given; count {count} ignored (every range is made)", ranges.len()));
        }
    } else if data.is_some() {
        let want = (count * 3).max(count + 5).min(100);
        let cands = x
            .call("highlights.candidates", json!({"source_id": rank_id, "count": want, "min_len": min_len, "max_len": max_len, "platform": platform}))
            .await?;
        warnings.extend(cands.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(|w| format!("candidates: {w}"))));
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
    };

    // Per-clip skeletons.
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
                name: format!("auto-{}-{}", k + 1, stamp(p.start)),
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
                c.plan.push(format!("rustler source import {u} --sections {a:.1}-{b:.1}{}", i.download_quality.as_ref().map(|q| format!(" --quality {q}")).unwrap_or_default()));
                c.plan.push("rustler analyze quick <new src_id>".into());
                c.plan.push(create_cmd(c, p, "<new src_id>", a));
            } else {
                c.plan.push(create_cmd(c, p, &rank_id, 0.0));
            }
            if let Some(cp) = &campaign {
                c.plan.push(format!("rustler clip meta-set <clip_id> --campaign-id {}", cp.id));
            }
            c.plan.push("rustler clip retention <clip_id> --timeline-only --per-second false".into());
            if fixes_on {
                c.plan.push("apply the retention fixes trim_head / trim_tail / remove_pauses / punch_in when suggested".into());
            }
            c.plan.push(format!("rustler render batch --clip-ids <clip_ids> --quality {}", quality_str(quality)));
            c.plan.push("rustler clip retention <clip_id> --per-second false".into());
            c.plan.push("rustler qc run <clip_id> --allow-fail true".into());
            if cover_on {
                c.plan.push("rustler clip cover <clip_id> --auto true   # best effort".into());
            }
            c.plan.push("rustler export clip <clip_id>   # writes <name>.post.json with empty copy for you to write".into());
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

    // ---- 3. create + retention fixes (sequential: each writes project.json) ----------------
    let total = clips.len().max(1);
    for (k, (c, p)) in clips.iter_mut().zip(&picks).enumerate() {
        if ctx.cancel.is_cancelled() {
            fail(c, "create", RustlerError::new(ErrorCode::Cancelled, STAGE, "cancelled"));
            continue;
        }
        let base = 0.15 + 0.45 * (k as f32 / total as f32);
        emit(&ctx, "create", Some(base), format!("{} ({}/{}) {}", c.name, k + 1, total, c.mmss));
        // URL: import the padded section and analyse it.
        let (src_id, offset) = if let (Some(u), Some((a, b))) = (&url, c.section) {
            let mut imp = json!({"input": u, "sections": [format!("{a:.2}-{b:.2}")], "name": c.name});
            if let Some(q) = &i.download_quality {
                imp["quality"] = json!(q);
            }
            let out = match x.call("source.import", imp).await {
                Ok(v) => v,
                Err(e) => {
                    fail(c, "import", e);
                    continue;
                }
            };
            let Some(sid) = out.pointer("/sources/0/id").and_then(Value::as_str).map(String::from) else {
                fail(c, "import", RustlerError::new(ErrorCode::Internal, STAGE, "source.import returned no source"));
                continue;
            };
            c.warnings.extend(out.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.as_str().map(String::from)));
            emit(&ctx, "analyze", Some(base), format!("{}: analyze quick {sid}", c.name));
            if let Err(e) = x.call("analyze.quick", json!({"source_id": sid})).await {
                fail(c, "analyze", e);
                c.source_id = Some(sid);
                continue;
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
                let mut suggested = retention_fixes(&r0);
                if fixes_on {
                    let mut applied = apply_fixes(&reg, &ctx, &suggested, SILENCE_FIXES).await;
                    if applied.iter().any(|a| a.ok) {
                        // Times changed: measure again before placing punch-ins.
                        if let Ok(r1) = x.call("clip.retention", ret_in.clone()).await {
                            suggested = retention_fixes(&r1);
                        }
                    }
                    applied.extend(apply_fixes(&reg, &ctx, &suggested, VISUAL_FIXES).await);
                    if !applied.is_empty() {
                        if let Ok(r2) = x.call("clip.retention", ret_in.clone()).await {
                            summary.score = r2.get("retention_score").and_then(Value::as_u64).map(|v| v as u32);
                            summary.dead_zones_left = dead_zones(&r2);
                            suggested = retention_fixes(&r2);
                        }
                    }
                    for a in applied.iter().filter(|a| a.ok) {
                        if let Some(u) = &a.undo {
                            c.change.push(Change { decision: format!("fix {} ({})", a.action, a.command), command: u.clone() });
                        }
                    }
                    c.fixes_applied = applied;
                    suggested.retain(|f| !SILENCE_FIXES.contains(&f.action.as_str()) && !VISUAL_FIXES.contains(&f.action.as_str()));
                }
                c.fixes_suggested = suggested;
                c.retention = Some(summary);
            }
            Err(e) => c.warnings.push(format!("retention not measured: {}", e.message)),
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
        match reg.dispatch("render.batch", json!({"clip_ids": to_render, "quality": quality}), rctx).await {
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
        "Optional one-call pipeline over existing commands: resolve a URL (cached info + captions, sections only) or a source, rank moments (highlights.candidates + first-2-s hook), create clips (smart defaults + template), apply cheap retention fixes, render.batch with ETA, qc.run, export.clip (post.json skeleton, copy left to you). Per-clip report with why, retention, QC, export paths and the commands to change each decision; one clip failing never stops the others. --dry-run plans only",
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
}
