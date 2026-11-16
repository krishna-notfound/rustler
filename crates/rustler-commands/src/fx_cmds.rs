//! Commands: transition.set, transition.list, transition.clear, effect.add, effect.list,
//! effect.remove, overlay.progress_bar.
//!
//! Stored in `clips/<id>/extras.json` (keys `transitions`, `effects`, `progress_bar`) through
//! [`crate::extras`]; `render.run` compiles them into the same single encode.
//!
//! Time convention: overlay, sfx and effect times are on the *edit timeline* (the clip's
//! TimeMap, what `clip show` prints). A transition overlaps the two segments around a cut, so
//! the rendered file gets shorter by the overlap. The render maps every edit-timeline time to
//! the rendered timeline ([`TransitionMap`]): the cut point sits in the middle of the overlap,
//! material before it keeps its time, material after it moves earlier by the overlap. Captions
//! go through the same map, so words, overlays, effects and sound effects stay on the picture.

use crate::audio_cmds::{change, next_id};
use crate::clip_cmds::log_event;
use crate::extras;
use rustler_captions::CaptionWord;
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, RustlerError, SegmentId, SourceId, TimeMap, Word};
use rustler_media::compile::{
    BarPosition, CompileExtras, Effect, EffectKind, ProgressBar, Transition, TransitionKind, TransitionMap,
    check_transitions, ffmpeg_color,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

type Result<T> = rustler_core::Result<T>;

pub const TRANSITIONS_KEY: &str = "transitions";
pub const EFFECTS_KEY: &str = "effects";
pub const PROGRESS_BAR_KEY: &str = "progress_bar";
const MAX_TRANSITION: f64 = 2.0;
const END_TOLERANCE: f64 = 0.05;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Loaders shared with render / QC
// ---------------------------------------------------------------------------------------------

pub fn load_transitions(store: &ProjectStore, clip: &ClipId) -> Result<Vec<Transition>> {
    Ok(extras::get(store, clip, TRANSITIONS_KEY)?.unwrap_or_default())
}

pub fn load_effects(store: &ProjectStore, clip: &ClipId) -> Result<Vec<Effect>> {
    Ok(extras::get(store, clip, EFFECTS_KEY)?.unwrap_or_default())
}

/// Transition map of a clip as stored now (edit timeline -> rendered timeline).
pub fn transition_map(store: &ProjectStore, clip: &Clip) -> Result<TransitionMap> {
    Ok(TransitionMap::new(clip, &load_transitions(store, &clip.id)?))
}

/// Duration of the rendered file (edit duration minus transition overlaps). QC should compare
/// the file against this, not against the bare TimeMap duration.
pub fn effective_duration_of(store: &ProjectStore, clip: &Clip) -> Result<f64> {
    Ok(transition_map(store, clip)?.duration())
}

/// Caption words (edit timeline) moved onto the rendered timeline; words hidden in the far
/// half of a transition are dropped.
pub fn map_caption_words(words: Vec<CaptionWord>, tmap: &TransitionMap) -> Vec<CaptionWord> {
    if tmap.is_identity() {
        return words;
    }
    words
        .into_iter()
        .filter_map(|w| {
            let (s, e) = tmap.map_interval(w.out_start, w.out_end)?;
            Some(CaptionWord { out_start: s, out_end: e, ..w })
        })
        .collect()
}

/// Transitions, effects and the progress bar into the compile extras; warnings for stale
/// transitions (their segment was deleted or is now the last one).
pub(crate) fn apply_to_compile(
    map: &serde_json::Map<String, serde_json::Value>,
    clip: &Clip,
    cx: &mut CompileExtras,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let ts: Vec<Transition> = extras::from_map(map, TRANSITIONS_KEY)?.unwrap_or_default();
    for t in &ts {
        if let Some(p) = transition_problem(clip, t) {
            warnings.push(format!("transition after {} ignored: {p}", t.after_segment_id));
        }
    }
    cx.transitions = ts;
    cx.effects = extras::from_map(map, EFFECTS_KEY)?.unwrap_or_default();
    cx.progress_bar = extras::from_map(map, PROGRESS_BAR_KEY)?;
    let duration = TimeMap::from_segments(&clip.segments).duration();
    for e in &cx.effects {
        if e.start >= duration {
            warnings.push(format!("{} starts after the clip ends ({duration:.2}s); skipped", e.id));
        }
    }
    Ok(warnings)
}

fn transition_problem(clip: &Clip, t: &Transition) -> Option<String> {
    match clip.segments.iter().position(|s| s.id == t.after_segment_id) {
        None => Some("its segment no longer exists".into()),
        Some(k) if k + 1 == clip.segments.len() => Some("its segment is now the last one".into()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// transition.set / transition.list / transition.clear
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct TransitionView {
    pub after_segment_id: SegmentId,
    pub kind: TransitionKind,
    /// Requested overlap (seconds).
    pub duration: f64,
    /// Overlap actually used, in frames and seconds (0 when inactive).
    pub frames: u64,
    pub overlap: f64,
    /// Edit-timeline time of the cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// Rendered window of the transition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendered_start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendered_end: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TransitionsOut {
    pub clip_id: ClipId,
    pub transitions: Vec<TransitionView>,
    /// Edit-timeline duration (overlay/sfx/effect times refer to it).
    pub duration: f64,
    /// Duration of the rendered file.
    pub rendered_duration: f64,
    /// Seconds removed by the transitions.
    pub removed: f64,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
}

const TIME_NOTE: &str = "times you give (overlays, sfx, effects, preview out_t) stay on the edit timeline; the render shifts them with the picture, captions included";

fn transitions_out(clip: &Clip, ts: &[Transition], dry: bool, next: Vec<String>) -> TransitionsOut {
    let tm = TimeMap::from_segments(&clip.segments);
    let tmap = TransitionMap::new(clip, ts);
    let views = ts
        .iter()
        .map(|t| {
            let k = clip.segments.iter().position(|s| s.id == t.after_segment_id);
            let active = k.and_then(|k| tmap.at_boundary(k).map(|(_, f)| (k, f)));
            let at = k.and_then(|k| tm.segments().get(k)).map(|s| s.out_end);
            let (frames, overlap, window) = match (active, at) {
                (Some((_, f)), Some(at)) => {
                    let ov = f as f64 / clip.canvas.fps;
                    let mid = tmap.map_time(at);
                    (f, ov, Some((mid - ov / 2.0, mid + ov / 2.0)))
                }
                _ => (0, 0.0, None),
            };
            TransitionView {
                after_segment_id: t.after_segment_id.clone(),
                kind: t.kind,
                duration: t.duration,
                frames,
                overlap,
                at,
                rendered_start: window.map(|w| w.0),
                rendered_end: window.map(|w| w.1),
                problem: transition_problem(clip, t),
            }
        })
        .collect();
    TransitionsOut {
        clip_id: clip.id.clone(),
        transitions: views,
        duration: tm.duration(),
        rendered_duration: tmap.duration(),
        removed: tmap.total_overlap(),
        note: TIME_NOTE.into(),
        dry_run: dry.then_some(true),
        next,
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SetIn {
    pub clip_id: ClipId,
    /// The transition goes from this segment into the next one.
    pub after_segment_id: SegmentId,
    /// cut | fade (dip to black) | dissolve (cross-blend) | wipeleft | wiperight | slideup | zoomin | flash (0.1 s white flash) | flashblack (0.1 s black flash) | whip (0.2 s horizontal smear)
    pub kind: TransitionKind,
    /// Overlap in seconds (default 0.3; flash/flashblack 0.1, whip 0.2; rounded to whole frames; max 2).
    pub duration: Option<f64>,
}

async fn transition_set(ctx: Ctx, i: SetIn) -> Result<TransitionsOut> {
    const STAGE: &str = "transition.set";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let ids: Vec<&str> = clip.segments.iter().map(|s| s.id.as_str()).collect();
    let Some(k) = clip.segments.iter().position(|s| s.id == i.after_segment_id) else {
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no segment {}", clip.id, i.after_segment_id))
            .hint(format!("segments in order: {}", ids.join(", "))));
    };
    if k + 1 == clip.segments.len() {
        return Err(validation(STAGE, format!("segment {} is the last one; a transition needs a next segment", i.after_segment_id))
            .hint(format!("segments in order: {}", ids.join(", "))));
    }
    let d = i.duration.unwrap_or_else(|| i.kind.default_duration());
    let one_frame = 1.0 / clip.canvas.fps;
    if i.kind != TransitionKind::Cut && !(d.is_finite() && d >= one_frame - 1e-9 && d <= MAX_TRANSITION) {
        return Err(validation(STAGE, format!("duration must be between one frame ({one_frame:.3}s) and {MAX_TRANSITION}s")));
    }
    let new = Transition { after_segment_id: i.after_segment_id.clone(), kind: i.kind, duration: d };
    let ts = change(&ctx, &store, &clip.id, TRANSITIONS_KEY, Vec::is_empty, |list: &mut Vec<Transition>| {
        list.retain(|t| t.after_segment_id != new.after_segment_id);
        if new.kind != TransitionKind::Cut {
            list.push(new.clone());
        }
        // Keep the list in timeline order.
        list.sort_by_key(|t| clip.segments.iter().position(|s| s.id == t.after_segment_id).unwrap_or(usize::MAX));
        check_transitions(&clip, list).map_err(|mut e| {
            e.stage = STAGE.into();
            e
        })?;
        Ok(list.clone())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "after_segment_id": i.after_segment_id, "kind": i.kind, "duration": d}))?;
    }
    let cut_at = TimeMap::from_segments(&clip.segments).segments()[k].out_end;
    let next = vec![
        format!("rustler render preview-frame {} --out-t {cut_at:.2}", clip.id),
        format!("rustler render run {} --quality draft", clip.id),
    ];
    Ok(transitions_out(&clip, &ts, ctx.dry_run, next))
}

#[derive(Deserialize, JsonSchema)]
pub struct ClipIn {
    pub clip_id: ClipId,
}

async fn transition_list(ctx: Ctx, i: ClipIn) -> Result<TransitionsOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let ts = load_transitions(&store, &clip.id)?;
    Ok(transitions_out(&clip, &ts, false, Vec::new()))
}

#[derive(Deserialize, JsonSchema)]
pub struct ClearIn {
    pub clip_id: ClipId,
    /// Only the transition after this segment (default: all).
    pub after_segment_id: Option<SegmentId>,
}

async fn transition_clear(ctx: Ctx, i: ClearIn) -> Result<TransitionsOut> {
    const STAGE: &str = "transition.clear";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let ts = change(&ctx, &store, &clip.id, TRANSITIONS_KEY, Vec::is_empty, |list: &mut Vec<Transition>| {
        match &i.after_segment_id {
            Some(sid) => {
                if !list.iter().any(|t| &t.after_segment_id == sid) {
                    return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("no transition after segment {sid}"))
                        .hint("`rustler transition list <clip_id>` shows them"));
                }
                list.retain(|t| &t.after_segment_id != sid);
            }
            None => list.clear(),
        }
        Ok(list.clone())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "after_segment_id": i.after_segment_id}))?;
    }
    Ok(transitions_out(&clip, &ts, ctx.dry_run, Vec::new()))
}

// ---------------------------------------------------------------------------------------------
// effect.add / effect.list / effect.remove
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct EffectAddIn {
    pub clip_id: ClipId,
    /// punch-in (instant zoom) | zoom-slow (1.0 -> amount) | shake | flash (white flash)
    pub kind: EffectKind,
    /// Edit-timeline start (seconds). Give start + end, or `words`.
    pub start: Option<f64>,
    /// Edit-timeline end (seconds).
    pub end: Option<f64>,
    /// Zoom factor for punch-in / zoom-slow (1.01..2, default 1.15); shake amplitude as a
    /// fraction of the frame (0.002..0.08, default 0.02); not used by flash.
    pub amount: Option<f64>,
    /// Words to land the effect on instead of start/end (comma list ok: `amazing,look`; case and
    /// punctuation ignored). Every occurrence inside the clip gets one effect over
    /// `[word.start - 0.05, word.end + 0.25]`; words not found in the clip are reported in `warnings`.
    pub words: Option<Vec<String>>,
    /// With `words`: only look in segments from this source.
    pub source_id: Option<SourceId>,
}

#[derive(Serialize, JsonSchema)]
pub struct EffectView {
    #[serde(flatten)]
    pub effect: Effect,
    /// The amount used (default by kind when not given).
    pub amount_used: f64,
    /// Rendered window (after transitions); None when it falls outside the clip.
    pub rendered_start: Option<f64>,
    pub rendered_end: Option<f64>,
}

fn effect_view(e: Effect, tmap: &TransitionMap) -> EffectView {
    let w = tmap.map_interval(e.start, e.end.min(tmap.edit_duration())).filter(|(s, _)| *s < tmap.duration());
    EffectView { amount_used: e.amount_or_default(), rendered_start: w.map(|w| w.0), rendered_end: w.map(|w| w.1), effect: e }
}

#[derive(Serialize, JsonSchema)]
pub struct EffectOut {
    pub clip_id: ClipId,
    /// The created effect (start/end mode only; with `words` see `effect_ids`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectView>,
    /// Every created effect id (one per word occurrence with `words`), in time order.
    pub effect_ids: Vec<String>,
    pub effects: Vec<EffectView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Validates an effect against the clip's edit duration.
pub fn check_effect(stage: &str, e: &Effect, duration: f64) -> Result<()> {
    if !(e.start.is_finite() && e.end.is_finite()) || e.start < 0.0 || e.end <= e.start {
        return Err(validation(stage, format!("need 0 <= start < end (got {} .. {})", e.start, e.end)));
    }
    if e.start >= duration || e.end > duration + END_TOLERANCE {
        return Err(validation(stage, format!("{:.2} .. {:.2}s is outside the clip (0 .. {duration:.2}s)", e.start, e.end))
            .hint("`rustler clip show <clip_id>` prints the edit timeline"));
    }
    let (lo, hi) = e.kind.amount_range();
    if let Some(a) = e.amount
        && !(a.is_finite() && (lo..=hi).contains(&a))
    {
        return Err(validation(stage, format!("amount for {:?} must be between {lo} and {hi}", e.kind)));
    }
    Ok(())
}

/// Lower-case alphanumerics of a word, for matching `Friends.` against `friends`.
fn norm_word(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Effect window around a spoken word: from just before it starts to a beat after it ends.
const WORD_LEAD: f64 = 0.05;
const WORD_TAIL: f64 = 0.25;

/// Edit-timeline `(start, end)` windows for every occurrence of each requested word inside the
/// clip (through the TimeMap), in time order, plus warnings for words that are not in the clip.
fn word_windows(
    store: &ProjectStore,
    clip: &Clip,
    wanted: &[String],
    source: Option<&SourceId>,
    duration: f64,
) -> Result<(Vec<(f64, f64)>, Vec<String>)> {
    const STAGE: &str = "effect.add";
    if let Some(src) = source
        && !clip.segments.iter().any(|s| &s.source_id == src)
    {
        return Err(validation(STAGE, format!("clip {} has no segment from source {src}", clip.id))
            .hint("`rustler clip show <clip_id>` lists the segments and their sources"));
    }
    let mut warnings = Vec::new();
    let mut by_source: std::collections::HashMap<SourceId, Vec<Word>> = std::collections::HashMap::new();
    for seg in &clip.segments {
        if source.is_some_and(|s| s != &seg.source_id) || by_source.contains_key(&seg.source_id) {
            continue;
        }
        match crate::clip_cmds::load_transcript(store, &seg.source_id)? {
            None => warnings.push(format!(
                "source {} has no transcript, so its words cannot be found (run `rustler analyze transcribe {}`)",
                seg.source_id, seg.source_id
            )),
            Some(tr) => {
                by_source.insert(seg.source_id.clone(), crate::clip_cmds::select_script_words(&tr, None, clip.captions.script).0);
            }
        }
    }
    let caption_words = crate::clip_cmds::build_caption_words(clip, &by_source);
    let mut windows: Vec<(f64, f64)> = Vec::new();
    for want in wanted {
        let key = norm_word(want);
        let mut found = 0;
        for w in caption_words.iter().filter(|w| norm_word(&w.text) == key) {
            found += 1;
            let start = (w.out_start - WORD_LEAD).max(0.0);
            let end = (w.out_end + WORD_TAIL).min(duration);
            if end - start < 0.05 {
                warnings.push(format!("'{want}' at {:.2}s is too close to the clip end for an effect; skipped", w.out_start));
                continue;
            }
            windows.push((start, end));
        }
        if found == 0 {
            warnings.push(format!("'{want}' not found in the clip (not spoken, or cut out); no effect added for it"));
        }
    }
    windows.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok((windows, warnings))
}

async fn effect_add(ctx: Ctx, i: EffectAddIn) -> Result<EffectOut> {
    const STAGE: &str = "effect.add";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let duration = TimeMap::from_segments(&clip.segments).duration();
    let mut warnings = Vec::new();
    // (start, end) windows on the edit timeline, as given or resolved from words.
    let windows: Vec<(f64, f64)> = match (&i.words, i.start, i.end) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err(validation(STAGE, "give either words or start + end, not both")
                .hint("e.g. --words amazing,look  or  --start 4.2 --end 6.0"));
        }
        (Some(words), None, None) => {
            let wanted: Vec<String> =
                words.iter().flat_map(|w| w.split(',')).map(str::trim).filter(|w| !w.is_empty()).map(String::from).collect();
            if wanted.is_empty() {
                return Err(validation(STAGE, "words is empty").hint("e.g. --words amazing,look"));
            }
            let (w, warn) = word_windows(&store, &clip, &wanted, i.source_id.as_ref(), duration)?;
            warnings = warn;
            w
        }
        (None, Some(s), Some(e)) => {
            if i.source_id.is_some() {
                warnings.push("source_id only applies with words; ignored".into());
            }
            vec![(s, e)]
        }
        (None, _, _) => {
            return Err(validation(STAGE, "give start and end, or words")
                .hint("e.g. --start 4.2 --end 6.0  or  --words amazing,look"));
        }
    };
    let by_words = i.words.is_some();
    let mut effects: Vec<Effect> = Vec::new();
    for &(s, e) in &windows {
        let eff = Effect { id: String::new(), kind: i.kind, start: s, end: e.min(duration), amount: i.amount };
        check_effect(STAGE, &Effect { end: e, ..eff.clone() }, duration)?;
        effects.push(eff);
    }
    if i.kind == EffectKind::Flash && i.amount.is_some() {
        warnings.push("flash does not use amount (it always starts fully white)".into());
    }
    if i.kind == EffectKind::Flash && effects.iter().any(|e| e.end - e.start > 1.0) {
        warnings.push("a flash longer than 1 s reads as a fade; 0.15-0.4 s is typical".into());
    }
    let created = if effects.is_empty() {
        Vec::new()
    } else {
        change(&ctx, &store, &clip.id, EFFECTS_KEY, Vec::is_empty, |list: &mut Vec<Effect>| {
            for e in &mut effects {
                e.id = next_id("fx", list.iter().map(|x| x.id.as_str()));
                list.push(e.clone());
            }
            Ok(effects.clone())
        })?
    };
    if !ctx.dry_run {
        for e in &created {
            log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "effect_id": e.id, "kind": e.kind}))?;
        }
    }
    let mut next = Vec::new();
    if let Some(e) = created.first() {
        let mid = (e.start + e.end) / 2.0;
        next.push(format!("rustler render preview-frame {} --out-t {mid:.2}", clip.id));
        next.push(format!("rustler render run {} --quality draft", clip.id));
    }
    let tmap = transition_map(&store, &clip)?;
    let mut views: Vec<EffectView> = created.into_iter().map(|e| effect_view(e, &tmap)).collect();
    let effect_ids: Vec<String> = views.iter().map(|v| v.effect.id.clone()).collect();
    let (effect_id, effect) = if by_words || views.len() != 1 {
        (None, None)
    } else {
        // Start/end mode: keep the single-effect fields v1 callers read.
        let v = views.remove(0);
        let single = EffectView { effect: v.effect.clone(), amount_used: v.amount_used, rendered_start: v.rendered_start, rendered_end: v.rendered_end };
        views.push(v);
        (Some(single.effect.id.clone()), Some(single))
    };
    Ok(EffectOut { clip_id: clip.id.clone(), effect_id, effect, effect_ids, effects: views, dry_run: ctx.dry_run.then_some(true), warnings, next })
}

#[derive(Serialize, JsonSchema)]
pub struct EffectListOut {
    pub clip_id: ClipId,
    pub duration: f64,
    pub rendered_duration: f64,
    pub effects: Vec<EffectView>,
    pub progress_bar: Option<ProgressBar>,
}

async fn effect_list(ctx: Ctx, i: ClipIn) -> Result<EffectListOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let tmap = transition_map(&store, &clip)?;
    let effects = load_effects(&store, &clip.id)?.into_iter().map(|e| effect_view(e, &tmap)).collect();
    Ok(EffectListOut {
        clip_id: clip.id.clone(),
        duration: tmap.edit_duration(),
        rendered_duration: tmap.duration(),
        effects,
        progress_bar: extras::get(&store, &clip.id, PROGRESS_BAR_KEY)?,
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct EffectRemoveIn {
    pub clip_id: ClipId,
    pub effect_id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct EffectRemoveOut {
    pub clip_id: ClipId,
    pub removed: String,
    pub remaining: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn effect_remove(ctx: Ctx, i: EffectRemoveIn) -> Result<EffectRemoveOut> {
    const STAGE: &str = "effect.remove";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let remaining = change(&ctx, &store, &clip.id, EFFECTS_KEY, Vec::is_empty, |list: &mut Vec<Effect>| {
        let Some(pos) = list.iter().position(|e| e.id == i.effect_id) else {
            let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
            return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no effect '{}'", clip.id, i.effect_id))
                .hint(if ids.is_empty() { "the clip has no effects".to_string() } else { format!("effects: {}", ids.join(", ")) }));
        };
        list.remove(pos);
        Ok(list.iter().map(|e| e.id.clone()).collect::<Vec<_>>())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "effect_id": i.effect_id}))?;
    }
    Ok(EffectRemoveOut { clip_id: clip.id, removed: i.effect_id, remaining, dry_run: ctx.dry_run.then_some(true) })
}

// ---------------------------------------------------------------------------------------------
// overlay.progress_bar
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ProgressBarIn {
    pub clip_id: ClipId,
    /// true shows the bar, false removes it.
    pub enabled: bool,
    /// `#RRGGBB` (default #FFE000).
    pub color: Option<String>,
    /// Pixels, 2..200 (default 8).
    pub height: Option<u32>,
    /// top (default) | bottom
    pub position: Option<BarPosition>,
}

#[derive(Serialize, JsonSchema)]
pub struct ProgressBarOut {
    pub clip_id: ClipId,
    pub progress_bar: Option<ProgressBar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

async fn progress_bar(ctx: Ctx, i: ProgressBarIn) -> Result<ProgressBarOut> {
    const STAGE: &str = "overlay.progress_bar";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let bar = if i.enabled {
        let current: ProgressBar = extras::get(&store, &clip.id, PROGRESS_BAR_KEY)?.unwrap_or_default();
        let bar = ProgressBar {
            color: i.color.unwrap_or(current.color),
            height: i.height.unwrap_or(current.height),
            position: i.position.unwrap_or(current.position),
        };
        if ffmpeg_color(&bar.color).is_none() {
            return Err(validation(STAGE, format!("color '{}' is not #RRGGBB", bar.color)).hint("e.g. --color \"#FFE000\""));
        }
        if !(2..=200).contains(&bar.height) {
            return Err(validation(STAGE, "height must be between 2 and 200 pixels"));
        }
        Some(bar)
    } else {
        None
    };
    if !ctx.dry_run {
        let value = bar.clone();
        extras::update(&store, &clip.id, PROGRESS_BAR_KEY, |_| {
            value.map(|b| serde_json::to_value(b).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))).transpose()
        })?;
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "enabled": i.enabled}))?;
    }
    let half = TimeMap::from_segments(&clip.segments).duration() / 2.0;
    Ok(ProgressBarOut {
        next: vec![format!("rustler render preview-frame {} --out-t {half:.2}", clip.id)],
        clip_id: clip.id,
        progress_bar: bar,
        dry_run: ctx.dry_run.then_some(true),
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
        "transition.set",
        "Set the transition from a segment into the next (cut|fade|dissolve|wipeleft|wiperight|slideup|zoomin|flash|flashblack|whip, frame-exact xfade + acrossfade); the clip gets shorter by the overlap",
        &["clip_id", "after_segment_id", "kind"],
        write,
        vec![
            Example { description: "0.3 s cross-dissolve", cli: "rustler transition set clp_x seg_a dissolve" },
            Example { description: "half-second dip to black", cli: "rustler transition set clp_x seg_a fade --duration 0.5" },
            Example { description: "0.1 s white flash at a cut (pacing)", cli: "rustler transition set clp_x seg_a flash" },
            Example { description: "fast horizontal whip into the next shot", cli: "rustler transition set clp_x seg_a whip" },
            Example { description: "back to a hard cut", cli: "rustler transition set clp_x seg_a cut" },
        ],
        transition_set,
    ));
    reg.register(typed(
        "transition.list",
        "List a clip's transitions with their rendered windows, and the edit vs rendered duration",
        &["clip_id"],
        read,
        vec![Example { description: "transitions of a clip", cli: "rustler transition list clp_x" }],
        transition_list,
    ));
    reg.register(typed(
        "transition.clear",
        "Remove one transition (after_segment_id) or all of them (hard cuts again)",
        &["clip_id"],
        Annotations { destructive: true, ..write },
        vec![Example { description: "remove all transitions", cli: "rustler transition clear clp_x" }],
        transition_clear,
    ));
    reg.register(typed(
        "effect.add",
        "Add a camera effect over an edit-timeline range (start + end) or on spoken words (words): punch-in (instant zoom), zoom-slow, shake, flash; applied to the composed frame under the captions",
        &["clip_id", "kind"],
        write,
        vec![
            Example { description: "punch in on the punchline", cli: "rustler effect add clp_x punch-in --start 4.2 --end 6.0" },
            Example { description: "punch in on every 'amazing' and 'look' (0.05 s before to 0.25 s after the word)", cli: "rustler effect add clp_x punch-in --words amazing,look" },
            Example { description: "slow 1.0 -> 1.2 zoom", cli: "rustler effect add clp_x zoom-slow --start 0 --end 5 --amount 1.2" },
            Example { description: "white flash at a cut", cli: "rustler effect add clp_x flash --start 3.0 --end 3.25" },
        ],
        effect_add,
    ));
    reg.register(typed(
        "effect.list",
        "List a clip's effects (with rendered windows) and its progress bar",
        &["clip_id"],
        read,
        vec![Example { description: "effects of a clip", cli: "rustler effect list clp_x" }],
        effect_list,
    ));
    reg.register(typed(
        "effect.remove",
        "Remove an effect from a clip",
        &["clip_id", "effect_id"],
        Annotations { destructive: true, ..write },
        vec![Example { description: "remove fx_1", cli: "rustler effect remove clp_x fx_1" }],
        effect_remove,
    ));
    reg.register(typed(
        "overlay.progress_bar",
        "Show (enabled true) or remove a progress bar that fills the canvas width over the clip, on top of everything",
        &["clip_id"],
        write,
        vec![
            Example { description: "yellow bar at the top", cli: "rustler overlay progress-bar clp_x --enabled true" },
            Example { description: "white bar at the bottom", cli: "rustler overlay progress-bar clp_x --enabled true --color \"#FFFFFF\" --position bottom" },
            Example { description: "remove it", cli: "rustler overlay progress-bar clp_x --enabled false" },
        ],
        progress_bar,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_checks() {
        let e = |kind, s, en, a| Effect { id: "fx_1".into(), kind, start: s, end: en, amount: a };
        assert!(check_effect("t", &e(EffectKind::PunchIn, 1.0, 2.0, None), 10.0).is_ok());
        assert!(check_effect("t", &e(EffectKind::Shake, 1.0, 10.04, Some(0.05)), 10.0).is_ok());
        for bad in [
            e(EffectKind::PunchIn, 2.0, 1.0, None),
            e(EffectKind::PunchIn, 10.0, 11.0, None),
            e(EffectKind::PunchIn, 1.0, 12.0, None),
            e(EffectKind::PunchIn, 1.0, 2.0, Some(3.0)),
            e(EffectKind::Shake, 1.0, 2.0, Some(0.5)),
            e(EffectKind::ZoomSlow, -1.0, 2.0, None),
        ] {
            assert_eq!(check_effect("t", &bad, 10.0).unwrap_err().code, ErrorCode::Validation, "{bad:?}");
        }
    }
}
