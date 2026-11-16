//! Commands: clip.retention, clip.score_set, campaign.analyze (director layer, design §5.9).
//!
//! Rustler never judges content: it measures where a clip can lose a viewer (retention map,
//! dead zones, ending type, a documented 0-100 retention score) and turns a brief or campaign
//! into hard rules, a strategy and a checklist. The agent decides and records its own 4-part
//! score (hook / flow / value / trend) with `clip.score_set`.
//!
//! Storage: `clips/<id>/extras.json` key `meta` (an object; other sub-keys are preserved):
//! `meta.score` (agent score) and `meta.retention` (last retention score, written by
//! `clip.retention` unless `--dry-run`). `clip.show` prints extras, `export.clip` embeds both
//! in its report through [`report_score`].
//!
//! Time base: `clip.retention` works on the **rendered** timeline (after transitions); the
//! fix commands it prints are converted back to the edit timeline that `effect.add`,
//! `overlay.add_text` and `edit.*` take.

use crate::clip_cmds::{caption_words_for_clip, log_event};
use crate::extras;
use crate::fx_cmds::{map_caption_words, transition_map};
use chrono::{DateTime, Utc};
use rustler_core::{
    AudioEvents, Clip, ClipId, ErrorCode, LoudnessPoint, ProjectStore, RenderId, RenderRecord, RustlerError, Scenes,
    SourceId, TimeMap,
};
use rustler_media::compile::TransitionMap;
use rustler_media::punchin::{self, PunchConfig, PunchInput, PunchWord};
use rustler_media::retention::{
    self, DeadKind, DeadZone, Ending, Factor, HookCheck, Pacing, RetentionInput, SecondView, Span, TimedText, VisualChange,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::{Tool, ToolPaths};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// extras.json key holding `score` and `retention`.
pub const META_KEY: &str = "meta";
/// Highest value of one part of the agent score.
const PART_MAX: f64 = 25.0;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

// ---------------------------------------------------------------------------------------------
// meta helpers (shared with export)
// ---------------------------------------------------------------------------------------------

/// Sets `meta.<sub>`, keeping the other sub-keys of `meta` and every other extras key.
fn set_meta(store: &ProjectStore, clip: &ClipId, sub: &str, value: Value) -> Result<()> {
    extras::update(store, clip, META_KEY, |cur| {
        let mut m = match cur {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        m.insert(sub.to_string(), value);
        Ok(Some(Value::Object(m)))
    })
}

fn meta_of(map: &Map<String, Value>, sub: &str) -> Option<Value> {
    map.get(META_KEY).and_then(|m| m.get(sub)).filter(|v| !v.is_null()).cloned()
}

/// `{"agent": meta.score, "retention": meta.retention}` for the export report; `null` when the
/// clip has neither.
pub fn report_score(store: &ProjectStore, clip: &ClipId) -> Value {
    let Ok(map) = extras::load(store, clip) else { return Value::Null };
    let (agent, ret) = (meta_of(&map, "score"), meta_of(&map, "retention"));
    if agent.is_none() && ret.is_none() {
        return Value::Null;
    }
    json!({ "agent": agent, "retention": ret })
}

// ---------------------------------------------------------------------------------------------
// Lenient extras readers
// ---------------------------------------------------------------------------------------------

fn items<'a>(map: &'a Map<String, Value>, key: &str) -> Vec<&'a Value> {
    match map.get(key) {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Object(o)) => o.values().collect(),
        _ => Vec::new(),
    }
}

fn num(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_f64)).filter(|x| x.is_finite())
}

fn text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(str::to_string)
}

const START_KEYS: &[&str] = &["start", "start_s", "out_start", "t0", "at"];
const END_KEYS: &[&str] = &["end", "end_s", "out_end", "t1"];

/// Edit-timeline time for rendered time `t` (inverse of `TransitionMap::map_time`).
fn edit_time(tmap: &TransitionMap, t: f64) -> f64 {
    let ed = tmap.edit_duration();
    if tmap.is_identity() {
        return t.clamp(0.0, ed);
    }
    let (mut lo, mut hi) = (0.0, ed);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if tmap.map_time(mid) < t { lo = mid } else { hi = mid }
    }
    hi
}

// ---------------------------------------------------------------------------------------------
// clip.retention
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct RetentionIn {
    pub clip_id: ClipId,
    /// Render to measure (frame-difference spikes, loudness, silences). Default: the latest
    /// render, when no edit was made after it.
    pub render_id: Option<RenderId>,
    /// Measure the edit timeline only, even when a render exists (no ffmpeg pass).
    #[serde(default)]
    pub timeline_only: bool,
    /// Include the per-second timeline (default true).
    pub per_second: Option<bool>,
}

/// One suggested fix for a dead zone.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Fix {
    /// punch_in | punch_in_skipped (no command: nothing fits) | text_pop | cutaway | remove_pauses | trim_head | trim_tail |
    /// bad_take (in `take_fixes`) | hook_title / cold_open (clips.auto suggestions)
    pub action: String,
    /// Exact command to run (edit-timeline times), when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub why: String,
}

#[derive(Serialize, JsonSchema)]
pub struct DeadZoneOut {
    #[serde(flatten)]
    pub zone: DeadZone,
    /// The zone on the edit timeline (what effect/overlay times refer to).
    pub edit_start: f64,
    pub edit_end: f64,
    pub fixes: Vec<Fix>,
}

#[derive(Serialize, JsonSchema)]
pub struct Inputs {
    pub transcript: bool,
    /// render | source | none
    pub audio: String,
    pub scenes: bool,
    pub frame_scores: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct RetentionOut {
    pub clip_id: ClipId,
    /// render (frame differences + render audio measured) | timeline (edit data only)
    pub measured_on: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_id: Option<RenderId>,
    /// Rendered duration (all times below are on this timeline).
    pub duration: f64,
    pub edit_duration: f64,
    pub retention_score: u32,
    pub breakdown: Vec<Factor>,
    pub formula: String,
    pub first_frame_hook: HookCheck,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_start_delay: Option<f64>,
    pub dead_zones: Vec<DeadZoneOut>,
    /// Whole-clip fixes not tied to a dead zone: restarts and repeats (`bad_take`, from
    /// `edit.find_bad_takes`; exact word timings only). Suggestions: never applied by clips.auto.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub take_fixes: Vec<Fix>,
    pub ending: Ending,
    pub pacing: Pacing,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption_density: Option<f64>,
    pub silence_total_s: f64,
    pub visual_changes: Vec<VisualChange>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<SecondView>,
    /// The agent's score (`clip.score_set`), when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_score: Option<Value>,
    pub inputs: Inputs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Event kinds that do not change the picture or the sound of a clip.
const NON_EDIT_PREFIXES: &[&str] =
    &["qc.", "export.", "render.", "see.", "analyze.", "brief.", "clip.retention", "clip.score_set", "clip.cover", "clip.meta_set", "fix.suggest", "variant.", "campaign."];

/// The latest edit of `clip` logged after `since`, as `"<kind> at <ts>"`.
fn edited_after(store: &ProjectStore, clip: &ClipId, since: DateTime<Utc>) -> Option<String> {
    let text = std::fs::read_to_string(store.root.join("events.ndjson")).ok()?;
    let mut last = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("data").and_then(|d| d.get("clip_id")).and_then(Value::as_str) != Some(clip.as_str()) {
            continue;
        }
        let kind = v.get("kind").and_then(Value::as_str).unwrap_or_default();
        if NON_EDIT_PREFIXES.iter().any(|p| kind.starts_with(p)) {
            continue;
        }
        let Some(ts) = v.get("ts").and_then(Value::as_str).and_then(|s| s.parse::<DateTime<Utc>>().ok()) else { continue };
        if ts > since {
            last = Some(format!("{kind} at {}", ts.format("%H:%M:%S")));
        }
    }
    last
}

/// Measured on a render: frame scores + audio.
struct Measured {
    render_id: RenderId,
    frames: rustler_media::retention::FrameScores,
    audio: AudioEvents,
}

async fn measure(ctx: &Ctx, path: &Path, render: &RenderRecord) -> Result<Measured> {
    let paths = ToolPaths::from_home(&ctx.home);
    paths.require(Tool::Ffmpeg)?;
    paths.require(Tool::Ffprobe)?;
    let frames = retention::frame_scores(&paths, path, retention::SCENE_SPIKE, &ctx.cancel).await?;
    let audio = rustler_media::analyze_audio(&paths, path, &ctx.cancel, &ctx.progress).await?;
    Ok(Measured { render_id: render.id.clone(), frames, audio })
}

fn read_analysis<T: serde::de::DeserializeOwned>(store: &ProjectStore, id: &SourceId, file: &str) -> Option<T> {
    let rel = Path::new("analysis").join(id.as_str()).join(file);
    if !store.resolve(&rel).is_file() {
        return None;
    }
    store.read_json(&rel).ok()
}

/// Merges spans that touch (within 0.05 s).
fn merge_spans(mut s: Vec<Span>) -> Vec<Span> {
    s.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<Span> = Vec::new();
    for x in s {
        match out.last_mut() {
            Some(l) if x.start <= l.end + 0.05 => l.end = l.end.max(x.end),
            _ => out.push(x),
        }
    }
    out
}

/// Everything the timeline itself says: cuts, transitions, layout changes, scene cuts, effects,
/// overlays, images, cutaways (rendered times) + motion spans + overlays for the hook.
fn timeline_events(
    store: &ProjectStore,
    clip: &Clip,
    map: &Map<String, Value>,
    tmap: &TransitionMap,
    use_scenes: bool,
    warnings: &mut Vec<String>,
) -> (Vec<VisualChange>, Vec<TimedText>, Vec<TimedText>, bool) {
    let tm = TimeMap::from_segments(&clip.segments);
    let mut ev = Vec::new();
    let mut push = |t: f64, kind: &str, detail: String| ev.push(VisualChange { t: r2(tmap.map_time(t)), kind: kind.into(), detail });
    let ms = tm.segments();
    for k in 1..clip.segments.len() {
        let (prev, cur) = (&clip.segments[k - 1], &clip.segments[k]);
        let at = ms[k].out_start;
        let gap = cur.start - prev.end;
        if let Some((kind, _)) = tmap.at_boundary(k - 1) {
            push(at, "transition", format!("{kind:?} into {}", cur.id).to_lowercase());
        } else if prev.source_id != cur.source_id {
            push(at, "cut", format!("cut to {} (another source)", cur.id));
        } else if prev.layout != cur.layout {
            push(at, "layout_change", format!("layout {} -> {}", prev.layout.preset.as_deref().unwrap_or("custom"), cur.layout.preset.as_deref().unwrap_or("custom")));
        } else if gap.abs() > 1.5 {
            push(at, "cut", format!("cut to {} ({gap:+.1} s in the source)", cur.id));
        } else if gap.abs() > 0.25 {
            push(at, "jump_cut", format!("{gap:.2} s removed before {}", cur.id));
        }
    }
    // Scene cuts inside segments.
    let mut scenes_ok = true;
    if use_scenes {
        let mut cache: HashMap<SourceId, Option<Scenes>> = HashMap::new();
        for m in ms {
            let sc = cache.entry(m.source_id.clone()).or_insert_with(|| read_analysis(store, &m.source_id, "scenes.json"));
            match sc {
                None => scenes_ok = false,
                Some(sc) => {
                    for s in &sc.scenes {
                        if s.start > m.src_start + 0.2 && s.start < m.src_end - 0.2 {
                            push(m.out_start + (s.start - m.src_start), "scene_cut", format!("scene cut at source {:.2} s", s.start));
                        }
                    }
                }
            }
        }
        if !scenes_ok {
            warnings.push("a source has no scenes.json: camera cuts inside segments are not counted (run `rustler analyze scenes <source_id>`, or render and measure the render)".into());
        }
    }
    let mut motion = Vec::new();
    for e in items(map, "effects") {
        let (Some(s), Some(en)) = (num(e, START_KEYS), num(e, END_KEYS)) else { continue };
        let kind = text(e, &["kind"]).unwrap_or_else(|| "effect".into());
        push(s, "effect", format!("{kind} {}", text(e, &["id"]).unwrap_or_default()).trim().to_string());
        match kind.as_str() {
            "punch-in" => push(en, "effect_end", format!("{kind} ends")),
            "zoom-slow" | "shake" => motion.push(TimedText { text: kind.clone(), start: tmap.map_time(s), end: tmap.map_time(en) }),
            _ => {}
        }
    }
    let mut overlays = Vec::new();
    for key in ["overlays", "texts"] {
        for o in items(map, key) {
            let (Some(s), Some(en)) = (num(o, START_KEYS), num(o, END_KEYS)) else { continue };
            let t = text(o, &["text"]).unwrap_or_default();
            push(s, "text", format!("'{t}'"));
            overlays.push(TimedText { text: t, start: tmap.map_time(s), end: tmap.map_time(en) });
        }
    }
    for o in items(map, "images") {
        if let Some(s) = num(o, START_KEYS) {
            push(s, "image", text(o, &["id", "path", "asset"]).unwrap_or_default());
        }
    }
    for o in items(map, "cutaways") {
        if let Some(s) = num(o, START_KEYS) {
            push(s, "cutaway", text(o, &["id", "source_id"]).unwrap_or_default());
            if let Some(en) = num(o, END_KEYS) {
                push(en, "cutaway_end", "back to the main shot".into());
            }
        }
    }
    (ev, motion, overlays, scenes_ok)
}

/// Source audio_events mapped onto the rendered timeline; `None` when any source lacks them.
fn source_audio(store: &ProjectStore, clip: &Clip, tmap: &TransitionMap) -> Option<(Vec<Span>, Vec<LoudnessPoint>)> {
    let tm = TimeMap::from_segments(&clip.segments);
    let mut cache: HashMap<SourceId, Option<AudioEvents>> = HashMap::new();
    let mut sil = Vec::new();
    let mut by_sec: HashMap<i64, f64> = HashMap::new();
    for m in tm.segments() {
        let ae = cache.entry(m.source_id.clone()).or_insert_with(|| read_analysis(store, &m.source_id, "audio_events.json")).as_ref()?;
        for s in &ae.silences {
            let (a, b) = (s.start.max(m.src_start), s.end.min(m.src_end));
            if b > a {
                sil.push(Span { start: tmap.map_time(m.out_start + a - m.src_start), end: tmap.map_time(m.out_start + b - m.src_start) });
            }
        }
        for p in &ae.curve {
            if p.t >= m.src_start - 0.5 && p.t < m.src_end {
                let t = tmap.map_time((m.out_start + p.t - m.src_start).max(0.0)).floor() as i64;
                let e = by_sec.entry(t).or_insert(f64::NEG_INFINITY);
                *e = e.max(p.lufs);
            }
        }
    }
    let mut curve: Vec<LoudnessPoint> = by_sec.into_iter().map(|(t, l)| LoudnessPoint { t: t as f64, lufs: l }).collect();
    curve.sort_by(|a, b| a.t.total_cmp(&b.t));
    Some((merge_spans(sil), curve))
}

fn longest_word(said: &str) -> Option<String> {
    said.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>())
        .filter(|w| w.chars().count() >= 4)
        .max_by_key(|w| w.chars().count())
        .map(|w| w.to_uppercase())
}

/// Fix suggestions for one dead zone (rendered times in, edit-timeline commands out).
/// Punch-in commands for one static dead zone from the shared planner (edit-timeline window).
fn punch_fix(id: &str, punch: &mut PunchInput, (s, e): (f64, f64), len: f64) -> (Fix, Option<f64>) {
    let input = PunchInput { window: Some((s, e)), ..punch.clone() };
    let planned = punchin::plan(&input, &PunchConfig::default());
    if planned.is_empty() {
        return (
            Fix {
                action: "punch_in_skipped".into(),
                command: None,
                why: format!(
                    "{len:.1} s with no visual change, but no punch-in fits here (graphic / blur-fit segment, a cut or transition within 0.5 s, an existing zoom, or already {} changes per 10 s); use a text pop or a cutaway",
                    PunchConfig::default().max_changes_per_10s
                ),
            },
            None,
        );
    }
    let cmds: Vec<String> = planned
        .iter()
        .map(|p| {
            let focus = match (p.focus_x, p.focus_y) {
                (Some(x), Some(y)) => format!(" --focus-x {x:.3} --focus-y {y:.3}"),
                _ => String::new(),
            };
            format!("rustler effect add {id} punch-in --start {:.2} --end {:.2} --amount {:.2}{focus}", p.start, p.end, p.amount)
        })
        .collect();
    let words: Vec<String> = planned.iter().filter_map(|p| p.word.clone()).collect();
    let fix = Fix {
        action: "punch_in".into(),
        command: Some(cmds.join(" && ")),
        why: format!(
            "{len:.1} s with no visual change; {} punch-in(s) from the punch-in planner{} (>= 0.5 s from cuts, not on graphics, at most {} changes per 10 s, varied zoom{})",
            planned.len(),
            if words.is_empty() { String::new() } else { format!(" on stressed words ({})", words.join(", ")) },
            PunchConfig::default().max_changes_per_10s,
            if planned.iter().any(|p| p.focus_x.is_some()) { ", centred on the face" } else { "" }
        ),
    };
    // Later dead zones plan around these (pacing counts across zone boundaries).
    punch.existing.extend(planned.iter().map(|p| (p.start, p.end)));
    (fix, planned.first().map(|p| p.start))
}

fn fixes_for(clip: &Clip, z: &DeadZone, punch: &mut PunchInput, tmap: &TransitionMap, duration: f64, cutaway_cmd: bool) -> Vec<Fix> {
    let id = clip.id.as_str();
    let ed = tmap.edit_duration();
    let et = |t: f64| r2(edit_time(tmap, t).min(ed));
    let mut out = Vec::new();
    match z.kind {
        DeadKind::Static => {
            let len = z.end - z.start;
            let (fix, first) = punch_fix(id, punch, (et(z.start), et(z.end)), len);
            out.push(fix);
            // Text pop times are edit times already; keep the rendered-time fallback otherwise.
            let at_edit = first.unwrap_or_else(|| et(z.start + 0.5));
            let kw = longest_word(&z.said);
            out.push(Fix {
                action: "text_pop".into(),
                command: Some(format!(
                    "rustler overlay add-text {id} \"{}\" --style plain --start {:.2} --end {:.2}",
                    kw.clone().unwrap_or_else(|| "KEY WORD".into()),
                    at_edit,
                    r2((at_edit + 1.2).min(ed))
                )),
                why: format!(
                    "a 1-3 word text pop on the key word of the line{} (replace the text with the word that matters)",
                    kw.map(|k| format!(" (longest word spoken here: {k})")).unwrap_or_default()
                ),
            });
            out.push(Fix {
                action: "cutaway".into(),
                command: cutaway_cmd.then(|| "rustler commands describe cutaway.add".to_string()),
                why: format!(
                    "cover {:.2}-{:.2} s (edit {:.2}-{:.2}) with a cutaway: B-roll, a reaction shot, or a screenshot of what is said{}",
                    z.start,
                    z.end,
                    et(z.start),
                    et(z.end),
                    if cutaway_cmd { "" } else { " (no cutaway command in this build; a layout change or another camera angle via a segment split also breaks the shot)" }
                ),
            });
        }
        DeadKind::Silence => {
            let len = z.end - z.start;
            if z.start < 0.05 {
                out.push(Fix {
                    action: "trim_head".into(),
                    command: Some(format!("rustler edit trim {id} --start-delta {:.2}", (len - 0.1).max(0.05))),
                    why: format!("the clip opens with {len:.1} s of silence; start on the first word"),
                });
            } else if z.end > duration - 0.05 {
                let last = clip.segments.last().map(|s| s.id.to_string()).unwrap_or_default();
                out.push(Fix {
                    action: "trim_tail".into(),
                    command: Some(format!("rustler edit trim {id} --segment-id {last} --end-delta -{:.2}", (len - 0.15).max(0.05))),
                    why: format!("{len:.1} s of silence after the last word; end on the line (or loop back)"),
                });
            } else {
                out.push(Fix {
                    action: "remove_pauses".into(),
                    command: Some(format!("rustler edit remove-pauses {id} --min-gap 0.8")),
                    why: format!("{len:.1} s of dead air at {:.2} s (edit {:.2}); tightens every word gap >= 0.8 s to 0.12 s", z.start, et(z.start)),
                });
            }
        }
    }
    out
}

fn registry_has(name: &str) -> bool {
    let mut reg = Registry::new();
    crate::register_all(&mut reg);
    reg.describe(name).is_some()
}

async fn clip_retention(ctx: Ctx, i: RetentionIn) -> Result<RetentionOut> {
    const STAGE: &str = "clip.retention";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    if clip.segments.is_empty() {
        return Err(validation(STAGE, format!("clip {} has no segments", clip.id)).hint("add one: rustler clip add-segment <clip_id> ..."));
    }
    let tmap = transition_map(&store, &clip)?;
    let map = extras::load(&store, &clip.id)?;
    let mut warnings = Vec::new();

    // Which render (if any) to measure.
    let render = match (&i.render_id, i.timeline_only) {
        (Some(_), true) => {
            warnings.push("timeline_only given: render_id ignored".into());
            None
        }
        (_, true) => None,
        (Some(id), false) => Some(clip.renders.iter().find(|r| &r.id == id).cloned().ok_or_else(|| {
            RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no render {id}", clip.id))
                .hint(format!("see renders with `rustler clip show {}`", clip.id))
        })?),
        (None, false) => clip.renders.last().cloned(),
    };
    let mut measured: Option<Measured> = None;
    if let Some(r) = &render {
        let path = store.resolve(&r.path);
        let stale = edited_after(&store, &clip.id, r.created_at);
        if !path.is_file() {
            warnings.push(format!("render {} file is missing ({}); measured the edit timeline only", r.id, path.display()));
        } else if i.render_id.is_none() && stale.is_some() {
            warnings.push(format!(
                "render {} is older than the last edit ({}); measured the edit timeline only (re-render to measure frames and loudness)",
                r.id,
                stale.unwrap_or_default()
            ));
        } else {
            if let Some(s) = stale {
                warnings.push(format!("render {} predates the last edit ({s}); its frames and audio may not match the timeline events", r.id));
            }
            match measure(&ctx, &path, r).await {
                Ok(m) => {
                    let n = m.frames.per_second.len() as f64;
                    if (n - tmap.duration().ceil()).abs() > 1.5 {
                        warnings.push(format!("render {} lasts ~{n:.0} s but the timeline is {:.1} s; it may be out of date", r.id, tmap.duration()));
                    }
                    measured = Some(m);
                }
                Err(e) if e.code == ErrorCode::Cancelled => return Err(e),
                Err(e) => warnings.push(format!("could not measure render {}: {}; measured the edit timeline only", r.id, e.message)),
            }
        }
    }

    let (mut changes, motion_spans, overlays, scenes_ok) = timeline_events(&store, &clip, &map, &tmap, measured.is_none(), &mut warnings);
    if let Some(m) = &measured {
        for (t, s) in &m.frames.spikes {
            changes.push(VisualChange { t: r2(*t), kind: "frame_change".into(), detail: format!("frame difference {s:.2}") });
        }
    }
    let (cw, w) = caption_words_for_clip(&store, &clip)?;
    warnings.extend(w);
    // Planner input for punch_in fixes (edit timeline): the whole timeline when it assembles,
    // otherwise just the words.
    let mut punch = match crate::timeline_doc::punch_input_for(&store, &clip) {
        Ok((p, w)) => {
            warnings.extend(w.into_iter().filter(|w| w.starts_with("extras key")));
            p
        }
        Err(e) => {
            warnings.push(format!("punch-in fixes see only the words (timeline did not assemble: {})", e.message));
            PunchInput {
                duration: tmap.edit_duration(),
                words: cw.iter().map(|w| PunchWord { text: w.text.clone(), start: w.out_start, end: w.out_end }).collect(),
                ..Default::default()
            }
        }
    };
    let words: Vec<TimedText> =
        map_caption_words(cw, &tmap).into_iter().map(|w| TimedText { text: w.text, start: w.out_start, end: w.out_end }).collect();
    let (silences, loudness, audio_from) = match &measured {
        Some(m) => (Some(m.audio.silences.iter().map(|s| Span { start: s.start, end: s.end }).collect()), Some(m.audio.curve.clone()), "render"),
        None => match source_audio(&store, &clip, &tmap) {
            Some((s, c)) => (Some(s), Some(c), "source"),
            None => {
                warnings.push("no audio_events.json for a source: silence from word gaps, energy neutral (run `rustler analyze audio <source_id>`)".into());
                (None, None, "none")
            }
        },
    };
    let duration = tmap.duration();
    let input = RetentionInput {
        duration,
        changes,
        motion_spans,
        words: words.clone(),
        silences,
        loudness,
        motion: measured.as_ref().map(|m| m.frames.per_second.clone()),
        overlays,
        captions_enabled: clip.captions.enabled,
    };
    let a = retention::analyze(&input);

    let cutaway_cmd = registry_has("cutaway.add");
    let dead_zones: Vec<DeadZoneOut> = a
        .dead_zones
        .iter()
        .map(|z| DeadZoneOut {
            edit_start: r2(edit_time(&tmap, z.start)),
            edit_end: r2(edit_time(&tmap, z.end)),
            fixes: fixes_for(&clip, z, &mut punch, &tmap, duration, cutaway_cmd),
            zone: z.clone(),
        })
        .collect();

    let measured_on = if measured.is_some() { "render" } else { "timeline" };
    let render_id = measured.as_ref().map(|m| m.render_id.clone());
    if !ctx.dry_run {
        set_meta(
            &store,
            &clip.id,
            "retention",
            json!({"score": a.retention_score, "measured_on": measured_on, "render_id": render_id, "dead_zones": a.dead_zones.len(), "at": Utc::now()}),
        )?;
    }
    let agent_score = meta_of(&map, "score");
    let id = clip.id.as_str();
    let mut next = Vec::new();
    if measured.is_none() {
        next.push(format!("rustler render run {id} --quality draft"));
        next.push(format!("rustler clip retention {id}"));
    }
    if let Some(f) = dead_zones.iter().flat_map(|z| z.fixes.iter()).find_map(|f| f.command.clone()) {
        next.push(f);
        next.push(format!("rustler clip retention {id}"));
    }
    let take_fixes = crate::takes_cmds::retention_fixes(&store, &clip);
    if !take_fixes.is_empty() {
        next.push(format!("{} restart(s)/repeat(s) found: read them with `rustler edit find-bad-takes {id}` and cut the ones you agree with", take_fixes.len()));
    }
    if agent_score.is_none() {
        next.push(format!("rustler clip score-set {id} --hook 0 --flow 0 --value 0 --trend 0 --notes \"why\""));
    }
    next.dedup();
    Ok(RetentionOut {
        clip_id: clip.id.clone(),
        measured_on: measured_on.into(),
        render_id,
        duration: a.duration,
        edit_duration: r2(tmap.edit_duration()),
        retention_score: a.retention_score,
        breakdown: a.breakdown,
        formula: retention::SCORE_FORMULA.into(),
        first_frame_hook: a.first_frame_hook,
        speech_start_delay: a.speech_start_delay,
        dead_zones,
        take_fixes,
        ending: a.ending,
        pacing: a.pacing,
        caption_density: a.caption_density,
        silence_total_s: a.silence_total_s,
        visual_changes: a.changes,
        timeline: if i.per_second.unwrap_or(true) { a.timeline } else { Vec::new() },
        agent_score,
        inputs: Inputs { transcript: !words.is_empty(), audio: audio_from.into(), scenes: measured.is_none() && scenes_ok, frame_scores: measured.is_some() },
        dry_run: ctx.dry_run.then_some(true),
        warnings,
        next,
    })
}

// ---------------------------------------------------------------------------------------------
// clip.score_set
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ScoreSetIn {
    pub clip_id: ClipId,
    /// Opening grabs a stranger in the first second and is on topic (0-25).
    pub hook: f64,
    /// Logical progression and a satisfying end (0-25).
    pub flow: f64,
    /// Value / emotion / connection for the viewer (0-25).
    pub value: f64,
    /// Fit with current trends and the audience (0-25; low confidence, offline judgement).
    pub trend: f64,
    /// Why: quote the line that earns each part.
    pub notes: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ScoreSetOut {
    pub clip_id: ClipId,
    pub score: Value,
    /// Last `clip.retention` result, when one was saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

async fn score_set(ctx: Ctx, i: ScoreSetIn) -> Result<ScoreSetOut> {
    const STAGE: &str = "clip.score_set";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    for (name, v) in [("hook", i.hook), ("flow", i.flow), ("value", i.value), ("trend", i.trend)] {
        if !(v.is_finite() && (0.0..=PART_MAX).contains(&v)) {
            return Err(validation(STAGE, format!("{name} must be between 0 and 25 (got {v})"))
                .hint("each of hook, flow, value, trend is 0-25; the total is out of 100"));
        }
    }
    let total = i.hook + i.flow + i.value + i.trend;
    let by = serde_json::to_value(ctx.actor).unwrap_or(json!("agent"));
    let mut score = json!({"hook": i.hook, "flow": i.flow, "value": i.value, "trend": i.trend, "total": total, "by": by, "at": Utc::now()});
    if let Some(n) = i.notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        score["notes"] = json!(n);
    }
    if !ctx.dry_run {
        set_meta(&store, &clip.id, "score", score.clone())?;
        log_event(&ctx, &store, STAGE, json!({"clip_id": clip.id, "total": total}))?;
    }
    let retention = meta_of(&extras::load(&store, &clip.id)?, "retention");
    let id = clip.id.as_str();
    let mut next = Vec::new();
    if retention.is_none() {
        next.push(format!("rustler clip retention {id}"));
    }
    next.push(format!("rustler qc run {id}"));
    next.push(format!("rustler export clip {id}"));
    Ok(ScoreSetOut { clip_id: clip.id, score, retention, dry_run: ctx.dry_run.then_some(true), next })
}

// ---------------------------------------------------------------------------------------------
// campaign.analyze
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct CampaignIn {
    /// Campaign to read from `<project>/campaigns/` (file stem, `id` or `name`).
    pub campaign_id: Option<String>,
    /// A brief given inline (same fields as brief.json / a campaign file); default: the
    /// campaign, else `brief.json`, else the only campaign file.
    pub brief: Option<Value>,
    /// Length of the source material in minutes (default: each project source's duration).
    pub source_minutes: Option<f64>,
}

#[derive(Serialize, JsonSchema, Default)]
pub struct DurationWindow {
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// brief | platform_default | none
    pub from: String,
}

#[derive(Serialize, JsonSchema, Default)]
pub struct HardRules {
    pub must: Vec<String>,
    pub must_not: Vec<String>,
}

#[derive(Serialize, JsonSchema, Default)]
pub struct Required {
    pub mentions: Vec<String>,
    pub hashtags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark_text: Option<String>,
    pub text: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct SourceCount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub minutes: f64,
    pub min: u32,
    pub max: u32,
    pub basis: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ClipCount {
    pub per_source: Vec<SourceCount>,
    pub total_min: u32,
    pub total_max: u32,
    pub note: String,
}

#[derive(Serialize, JsonSchema)]
pub struct Strategy {
    /// per_view | per_clip | unknown
    pub payout_model: String,
    pub optimise: Vec<String>,
    pub hints: Vec<String>,
    pub recommended_clips: ClipCount,
}

#[derive(Serialize, JsonSchema)]
pub struct CheckItem {
    pub id: String,
    pub text: String,
    /// rustler (a command checks it) | agent (you judge it)
    pub by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct CampaignOut {
    pub found: bool,
    /// campaign | brief | inline | none
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub duration_window: DurationWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payout: Option<Value>,
    pub hard_rules: HardRules,
    pub required: Required,
    pub strategy: Strategy,
    pub checklist: Vec<CheckItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn strs(v: &Value, keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in keys {
        match v.get(*k) {
            Some(Value::Array(a)) => out.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string).or_else(|| text(x, &["text", "name"])))),
            Some(Value::String(s)) => out.extend(s.split(',').map(|x| x.trim().to_string())),
            _ => {}
        }
    }
    out.retain(|s| !s.trim().is_empty());
    out.dedup();
    out
}

fn tagged(items: Vec<String>, sigil: char) -> Vec<String> {
    items.into_iter().map(|s| format!("{sigil}{}", s.trim().trim_start_matches(sigil))).collect()
}

fn is_negative_rule(t: &str) -> bool {
    let l = format!(" {} ", t.to_lowercase());
    [" no ", " don't ", " dont ", " do not ", " never ", " must not ", " mustn't ", " avoid ", " without ", " not allowed", " banned", " forbidden"]
        .iter()
        .any(|p| l.contains(p))
}

/// `(min, max, basis)` clips for a source of `m` minutes under a payout model.
pub fn recommended_count(m: f64, per_clip: bool) -> (u32, u32, String) {
    let lerp = |a: f64, b: f64, f: f64| (a + (b - a) * f.clamp(0.0, 1.0)).round() as u32;
    if per_clip {
        return match m {
            m if m < 30.0 => (1, 3, "per-clip payout: a few polished clips (Rustler heuristic)".into()),
            m if m <= 120.0 => (3, 6, "per-clip payout: a few polished clips (Rustler heuristic)".into()),
            _ => (5, 8, "per-clip payout: a few polished clips (Rustler heuristic)".into()),
        };
    }
    match m {
        m if m < 3.0 => (1, 3, "under 3 min: Rustler heuristic".into()),
        m if m <= 10.0 => (3, 14, "OpusClip: 3-10 min -> 3-14 clips".into()),
        m if m < 30.0 => (lerp(3.0, 23.0, (m - 10.0) / 20.0), lerp(14.0, 32.0, (m - 10.0) / 20.0), "interpolated between OpusClip's 3-10 min and 30-60 min buckets".into()),
        m if m <= 60.0 => (23, 32, "OpusClip: 30-60 min -> 23-32 clips".into()),
        m if m <= 120.0 => (lerp(23.0, 42.0, (m - 60.0) / 60.0), lerp(32.0, 55.0, (m - 60.0) / 60.0), "interpolated between OpusClip's 30-60 min and >120 min buckets".into()),
        _ => (42, 55, "OpusClip: over 120 min -> 42-55 clips".into()),
    }
}

/// Default duration window when the brief is silent (finding-hooks guide, "Length guidance").
fn platform_window(p: Option<&str>) -> Option<(f64, f64)> {
    match p?.to_lowercase().as_str() {
        "shorts" | "youtube" | "youtube_shorts" => Some((20.0, 45.0)),
        "reels" | "instagram" => Some((15.0, 45.0)),
        "tiktok" => Some((25.0, 60.0)),
        _ => None,
    }
}

/// The campaign/brief JSON to analyze and where it came from.
fn find_campaign(store: &ProjectStore, i: &CampaignIn) -> Result<(String, Option<PathBuf>, Option<Value>)> {
    const STAGE: &str = "campaign.analyze";
    let dir = store.root.join("campaigns");
    let files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
        .unwrap_or_default();
    let read = |p: &Path| -> Option<Value> { serde_json::from_slice(&std::fs::read(p).ok()?).ok() };
    if let Some(id) = &i.campaign_id {
        for f in &files {
            let Some(v) = read(f) else { continue };
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            if stem == id || text(&v, &["id"]).as_deref() == Some(id.as_str()) || text(&v, &["name"]).is_some_and(|n| n.eq_ignore_ascii_case(id)) {
                return Ok(("campaign".into(), Some(f.clone()), Some(v)));
            }
        }
        let names: Vec<String> = files.iter().filter_map(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("no campaign '{id}' in {}", dir.display())).hint(if names.is_empty() {
            "no campaigns yet; set a brief with `rustler brief set ...` or pass --input-json @brief.json with a \"brief\" object".to_string()
        } else {
            format!("campaigns: {}", names.join(", "))
        }));
    }
    if let Some(b) = &i.brief {
        if !b.is_object() {
            return Err(validation(STAGE, "brief must be a JSON object").hint("use --input-json @file.json with {\"brief\": {...}}"));
        }
        return Ok(("inline".into(), None, Some(b.clone())));
    }
    let bp = store.root.join(crate::brief_cmds::BRIEF_FILE);
    if let Some(v) = bp.is_file().then(|| read(&bp)).flatten() {
        return Ok(("brief".into(), Some(bp), Some(v)));
    }
    if files.len() == 1
        && let Some(v) = read(&files[0])
    {
        return Ok(("campaign".into(), Some(files[0].clone()), Some(v)));
    }
    Ok(("none".into(), None, None))
}

async fn campaign_analyze(ctx: Ctx, i: CampaignIn) -> Result<CampaignOut> {
    const STAGE: &str = "campaign.analyze";
    let store = ctx.store()?;
    let (from, path, v) = find_campaign(&store, &i)?;
    let mut warnings = Vec::new();
    let empty = json!({});
    let b = v.as_ref().unwrap_or(&empty);
    let has_brief_file = store.root.join(crate::brief_cmds::BRIEF_FILE).is_file();

    let platform = text(b, &["platform"]);
    let dur = b.get("duration").cloned().unwrap_or(Value::Null);
    let min = num(b, &["min_duration", "min_duration_s", "min_s"]).or_else(|| num(&dur, &["min", "min_s"]));
    let max = num(b, &["max_duration", "max_duration_s", "max_s"]).or_else(|| num(&dur, &["max", "max_s"]));
    let duration_window = if min.is_some() || max.is_some() {
        DurationWindow { min, max, from: "brief".into() }
    } else if let Some((lo, hi)) = platform_window(platform.as_deref()) {
        DurationWindow { min: Some(lo), max: Some(hi), from: "platform_default".into() }
    } else {
        DurationWindow { min: None, max: None, from: "none".into() }
    };
    let payout = b.get("payout").filter(|p| p.is_object()).cloned().or_else(|| text(b, &["payout_model"]).map(|m| json!({"model": m})));
    let model = payout.as_ref().and_then(|p| text(p, &["model"])).map(|m| m.to_lowercase().replace(['-', ' '], "_"));
    let per_clip = model.as_deref() == Some("per_clip");
    let model_name = match model.as_deref() {
        Some("per_view") | Some("per_clip") => model.clone().unwrap_or_default(),
        Some(other) => {
            warnings.push(format!("unknown payout model '{other}'; planning as per_view"));
            "unknown".into()
        }
        None => "unknown".into(),
    };

    let required = Required {
        mentions: tagged(strs(b, &["required_mentions", "mentions_required", "mentions"]), '@'),
        hashtags: tagged(strs(b, &["required_hashtags", "hashtags_required", "hashtags"]), '#'),
        credit_text: text(b, &["credit_text", "credit"]),
        watermark_text: text(b, &["watermark_text", "watermark"]),
        text: strs(b, &["required_text"]),
    };
    let banned = strs(b, &["banned_words", "banned", "forbidden_words"]);

    let mut rules = HardRules::default();
    match (min, max) {
        (Some(lo), Some(hi)) => rules.must.push(format!("duration {lo}-{hi} s")),
        (Some(lo), None) => rules.must.push(format!("duration at least {lo} s")),
        (None, Some(hi)) => rules.must.push(format!("duration at most {hi} s")),
        (None, None) => {}
    }
    if let Some(p) = &platform {
        rules.must.push(format!("platform: {p}"));
    }
    for (k, label) in [("aspect", "aspect"), ("layout", "layout preset"), ("caption_style", "caption style")] {
        if let Some(x) = text(b, &[k]) {
            rules.must.push(format!("{label}: {x}"));
        }
    }
    if let Some(c) = &required.credit_text {
        rules.must.push(format!("on-screen credit overlay: \"{c}\""));
    }
    if let Some(w) = &required.watermark_text {
        rules.must.push(format!("on-screen watermark overlay: \"{w}\""));
    }
    for t in &required.text {
        rules.must.push(format!("say or show: \"{t}\""));
    }
    if !required.mentions.is_empty() {
        rules.must.push(format!("mention {} in the post", required.mentions.join(" ")));
    }
    if !required.hashtags.is_empty() {
        rules.must.push(format!("hashtags {} in the post", required.hashtags.join(" ")));
    }
    for w in &banned {
        rules.must_not.push(format!("say, caption or show \"{w}\""));
    }
    let mut camp_rules: Vec<(String, String, bool)> = Vec::new();
    if let Some(Value::Array(rs)) = b.get("rules") {
        for (k, r) in rs.iter().enumerate() {
            let (id, t, machine) = match r {
                Value::String(s) => (format!("rule_{}", k + 1), s.clone(), false),
                o => (
                    text(o, &["id"]).unwrap_or_else(|| format!("rule_{}", k + 1)),
                    text(o, &["text", "rule", "description"]).unwrap_or_default(),
                    text(o, &["kind"]).as_deref() == Some("machine"),
                ),
            };
            if t.trim().is_empty() {
                continue;
            }
            if is_negative_rule(&t) { rules.must_not.push(t.clone()) } else { rules.must.push(t.clone()) }
            camp_rules.push((id, t, machine));
        }
    }
    for k in ["must", "do"] {
        rules.must.extend(strs(b, &[k]));
    }
    for k in ["must_not", "dont", "do_not"] {
        rules.must_not.extend(strs(b, &[k]));
    }

    // Clip counts per source.
    let mut per_source = Vec::new();
    match i.source_minutes {
        Some(m) if m.is_finite() && m > 0.0 => {
            let (lo, hi, basis) = recommended_count(m, per_clip);
            per_source.push(SourceCount { source_id: None, name: None, minutes: r2(m), min: lo, max: hi, basis });
        }
        Some(m) => return Err(validation(STAGE, format!("source_minutes must be > 0 (got {m})"))),
        None => {
            for sid in store.load_project()?.sources {
                let Ok(s) = store.load_source(&sid) else { continue };
                let m = s.probe.duration / 60.0;
                if m <= 0.0 {
                    continue;
                }
                let (lo, hi, basis) = recommended_count(m, per_clip);
                per_source.push(SourceCount { source_id: Some(sid), name: Some(s.name), minutes: r2(m), min: lo, max: hi, basis });
            }
            if per_source.is_empty() {
                warnings.push("no sources in the project and no source_minutes: clip count not estimated".into());
            }
        }
    }
    let total_min = per_source.iter().map(|s| s.min).sum();
    let total_max = per_source.iter().map(|s| s.max).sum();
    let recommended_clips = ClipCount {
        per_source,
        total_min,
        total_max,
        note: "a volume ceiling, not a quota: ship only clips that score >= 70 on the finding-hooks rubric and pass QC; diversify topics (no two clips making the same point)".into(),
    };
    let (optimise, mut hints): (Vec<String>, Vec<String>) = if per_clip {
        (
            vec!["acceptance: match the creator's style exactly".into(), "polish over volume".into(), "zero rule violations".into()],
            vec![
                "every accepted clip pays and a rejected one pays nothing: fewer, polished clips".into(),
                "study 3-5 of the creator's own shorts first (pacing, caption style, hook format) and copy that look on every clip".into(),
                "run the checklist on every clip before export; one broken hard rule = rejected".into(),
            ],
        )
    } else {
        (
            vec!["retention (watch time, completion, replays)".into(), "volume of distinct clips".into(), "hook in the first second".into()],
            vec![
                "views pay: a viewer who stays to the end is worth more than any polish; measure every clip with `rustler clip retention` and fix dead zones".into(),
                "open on the strongest line (bold claim, result first, question); a text hook on screen from frame 0".into(),
                "a visual change every 2-3 s (punch-in, cut, text pop, layout change); no silence over 0.8 s".into(),
                "end on a loopable line or a clean sentence; never mid-word".into(),
                "volume: one look for all clips (template / caption style) and `rustler render batch` to ship them".into(),
            ],
        )
    };
    if model_name == "unknown" {
        hints.push("no payout model in the brief: planned as per_view (retention + volume); set it with `rustler brief set --input-json @brief.json` (payout.model per_view|per_clip)".into());
    }
    if let Some(r) = payout.as_ref().and_then(|p| num(p, &["rate"])) {
        let cur = payout.as_ref().and_then(|p| text(p, &["currency"])).unwrap_or_default();
        hints.push(if per_clip { format!("rate {r} {cur} per accepted clip") } else { format!("rate {r} {cur} per 1000 views") }.replace("  ", " "));
    }

    // Checklist.
    let camp_id = (from == "campaign").then(|| text(b, &["id"])).flatten();
    let check_cmd = if has_brief_file { Some("rustler brief check <clip_id>".to_string()) } else { None };
    let mut checklist = Vec::new();
    let mut add = |id: &str, t: String, by: &str, cmd: Option<String>| checklist.push(CheckItem { id: id.into(), text: t, by: by.into(), command: cmd });
    if let (Some(lo), Some(hi)) = (duration_window.min, duration_window.max) {
        add("duration", format!("duration {lo}-{hi} s ({})", duration_window.from), "rustler", check_cmd.clone().or(Some("rustler qc run <clip_id>".into())));
    }
    if let Some(c) = &required.credit_text {
        add("credit", format!("credit overlay \"{c}\" on screen"), "rustler", Some(format!("rustler overlay add-text <clip_id> \"{c}\" --style lower-third")));
    }
    if let Some(w) = &required.watermark_text {
        add("watermark", format!("watermark \"{w}\" on screen"), "rustler", check_cmd.clone());
    }
    if !required.text.is_empty() {
        add("required_text", format!("say or show: {}", required.text.join("; ")), "rustler", check_cmd.clone());
    }
    if !banned.is_empty() {
        add("banned_words", format!("none of: {}", banned.join(", ")), "rustler", check_cmd.clone());
    }
    if !required.mentions.is_empty() || !required.hashtags.is_empty() {
        let tags: Vec<String> = required.mentions.iter().chain(&required.hashtags).cloned().collect();
        add("post_tags", format!("post text includes {}", tags.join(" ")), "rustler", has_brief_file.then(|| "rustler brief description <clip_id>".to_string()));
    }
    if let Some(cid) = &camp_id {
        add(
            "attach",
            format!("clip attached to campaign {cid} and posting copy written (export writes <name>.post.json)"),
            "rustler",
            Some(format!("rustler clip meta-set <clip_id> --campaign-id {cid} --input-json @meta.json")),
        );
    }
    for (id, t, machine) in &camp_rules {
        let cmd = if *machine {
            Some("rustler qc run <clip_id>".to_string())
        } else {
            camp_id.as_ref().map(|_| format!("rustler campaign mark <clip_id> {id} --pass true --reason \"what you saw\""))
        };
        add(id, t.clone(), if *machine { "rustler" } else { "agent" }, cmd);
    }
    add("hook", "a hook in the first second (text on screen at 0, speech before 0.5 s, or motion)".into(), "rustler", Some("rustler clip retention <clip_id>".into()));
    add("dead_zones", "no dead zone: nothing static > 2.5 s, no silence >= 0.8 s".into(), "rustler", Some("rustler clip retention <clip_id>".into()));
    add("retention", "retention_score >= 70".into(), "rustler", Some("rustler clip retention <clip_id>".into()));
    add("ending", "ends on a full sentence, loopable if possible".into(), "agent", Some("rustler clip retention <clip_id>".into()));
    add("score", "your hook/flow/value/trend score recorded with reasons".into(), "agent", Some("rustler clip score-set <clip_id> --hook H --flow F --value V --trend T --notes \"why\"".into()));
    add("qc", "QC passed and the contact sheet looked at".into(), "rustler", Some("rustler qc run <clip_id>".into()));

    if from == "none" {
        warnings.push("no campaign or brief found: generic retention strategy; set the job's rules with `rustler brief set ...`".into());
    }
    let mut next = vec!["rustler highlights candidates <source_id> --count 10".to_string(), "rustler prompt show direct_campaign".to_string()];
    if from == "none" {
        next.insert(0, "rustler brief set --min-duration 20 --max-duration 60 --credit-text \"Clip from @creator\"".into());
    }
    Ok(CampaignOut {
        found: from != "none",
        from,
        path: path.map(|p| store.relativize(&p)),
        name: text(b, &["name", "campaign"]),
        creator: text(b, &["creator"]),
        platform,
        duration_window,
        payout,
        hard_rules: rules,
        required,
        strategy: Strategy { payout_model: model_name, optimise, hints, recommended_clips },
        checklist,
        notes: text(b, &["notes", "description"]),
        warnings,
        next,
    })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    let read = Annotations { read_only: true, ..write };
    reg.register(typed(
        "clip.retention",
        "Retention map of the output clip: per-second visual changes, speech, silence, caption density and loudness; first-frame hook, speech start delay, dead zones (> 2.5 s static, >= 0.8 s silence) with exact fix commands, ending type, pacing and a 0-100 retention score with its breakdown. Measures the latest current render (frame differences + audio) when there is one",
        &["clip_id"],
        write,
        vec![
            Example { description: "measure a clip (uses the latest render when it is current)", cli: "rustler clip retention clp_x" },
            Example { description: "edit timeline only, no ffmpeg pass, no per-second rows", cli: "rustler clip retention clp_x --timeline-only --per-second false" },
        ],
        clip_retention,
    ));
    reg.register(typed(
        "clip.score_set",
        "Record your 4-part score of a clip (hook, flow, value, trend: 0-25 each, total /100) with notes; stored in extras meta.score, shown by clip.show and in the export report",
        &["clip_id", "hook", "flow", "value", "trend"],
        write,
        vec![Example {
            description: "score a clip",
            cli: "rustler clip score-set clp_x --hook 22 --flow 18 --value 20 --trend 12 --notes \"opens on the 10x claim; clean payoff\"",
        }],
        score_set,
    ));
    reg.register(typed(
        "campaign.analyze",
        "Read the campaign (campaigns/<id>.json) or brief (brief.json or inline) and return its hard rules (must / must-not), duration window, platform, payout model -> strategy and recommended clip count per source, required mentions/hashtags/credit, and the checklist each clip must pass",
        &[],
        read,
        vec![
            Example { description: "analyze the project's brief", cli: "rustler campaign analyze" },
            Example { description: "analyze one campaign", cli: "rustler campaign analyze --campaign-id summer-drop" },
            Example { description: "plan for a 45-minute source", cli: "rustler campaign analyze --source-minutes 45" },
        ],
        campaign_analyze,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_follow_opusclip_buckets() {
        assert_eq!(recommended_count(5.0, false).0..=recommended_count(5.0, false).1, 3..=14);
        assert_eq!((recommended_count(45.0, false).0, recommended_count(45.0, false).1), (23, 32));
        assert_eq!((recommended_count(150.0, false).0, recommended_count(150.0, false).1), (42, 55));
        let (lo, hi, b) = recommended_count(20.0, false);
        assert!((3..=23).contains(&lo) && (14..=32).contains(&hi) && b.contains("interpolated"));
        assert!(recommended_count(45.0, true).1 <= 6);
    }

    #[test]
    fn negative_rules() {
        assert!(is_negative_rule("No competitor logos"));
        assert!(is_negative_rule("Do not use music"));
        assert!(!is_negative_rule("Tag the creator"));
    }

    #[test]
    fn longest_word_picks_keyword() {
        assert_eq!(longest_word("we made ten million dollars"), Some("DOLLARS".into()));
        assert_eq!(longest_word("a b c"), None);
    }
}
