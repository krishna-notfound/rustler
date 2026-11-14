//! Commands: clip.create, clip.add_segment, clip.set_layout, clip.set_captions (alias
//! caption.set), clip.show, clip.list, caption.list_styles, caption.export_srt,
//! layout.list_presets, audio.add_music, audio.remove_music (Task 10b).
//!
//! Also home of the helpers the render/see commands share: transcript loading, unit -> time
//! padding, `expect_text` checking, caption-word building (through `TimeMap`, with the
//! romanization choice) and the `extras.json` sidecar (music bed + crop paths, which the
//! frozen `Clip` has no field for).

use chrono::Utc;
use rustler_captions::{CaptionWord, PresetInfo, write_srt};
use rustler_core::layout::{self, LayoutParams};
use rustler_core::units::{build_units, find_quote};
use rustler_core::{
    Actor, AudioSettings, CaptionScript, CaptionTrack, Clip, ClipId, ErrorCode, Event, Layout, ProjectStore,
    RustlerError, Segment, SegmentId, Source, SourceId, TimeMap, Transcript, Unit, Word,
};
use rustler_media::compile::{CropKey, MusicBed};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Seconds of lead-in before a unit's first word (never reaching into the previous word).
pub const START_PAD: f64 = 0.08;
/// Seconds of tail after a unit's last word (never reaching into the next word).
pub const END_PAD: f64 = 0.15;
/// A segment end may exceed the probed source duration by this much.
const END_TOLERANCE: f64 = 0.05;

type Result<T> = rustler_core::Result<T>;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested below)
// ---------------------------------------------------------------------------------------------

/// Slice index of the word whose `Word::i` is `i`.
fn word_pos(words: &[Word], i: u32) -> Option<usize> {
    match words.get(i as usize) {
        Some(w) if w.i == i => Some(i as usize),
        _ => words.iter().position(|w| w.i == i),
    }
}

/// Source-time range for transcript units `start_unit..=end_unit`: starts `START_PAD` before the
/// first word but never before the previous word ended; ends `END_PAD` after the last word but
/// never after the next word starts; clamped to `[0, duration]`.
pub fn unit_range(words: &[Word], units: &[Unit], start_unit: u32, end_unit: u32, duration: f64) -> Result<(f64, f64)> {
    if end_unit < start_unit {
        return Err(validation("clip.create", format!("end_unit {end_unit} is before start_unit {start_unit}")));
    }
    let find = |id: u32| {
        units.iter().find(|u| u.id == id).ok_or_else(|| {
            let last = units.last().map_or(0, |u| u.id);
            validation("clip.create", format!("unit {id} does not exist (units are 0..={last})"))
                .hint("list units with `rustler transcript get <source_id>`")
        })
    };
    let first = find(start_unit)?;
    let last = find(end_unit)?;
    let prev_end = word_pos(words, first.word_start)
        .and_then(|p| p.checked_sub(1))
        .map_or(0.0, |p| words[p].end.min(first.start));
    let next_start = word_pos(words, last.word_end)
        .and_then(|p| words.get(p + 1))
        .map_or(duration, |w| w.start.max(last.end));
    let start = (first.start - START_PAD).max(prev_end).max(0.0);
    let end = (last.end + END_PAD).min(next_start).min(duration.max(last.end));
    Ok((start, end))
}

/// Errors with `Validation` "opening words do not match: transcript says '...'" unless the words
/// starting at `from_word` match `expect` (see `find_quote`).
pub fn check_expect_text(words: &[Word], from_word: u32, expect: &str) -> Result<()> {
    if find_quote(words, from_word, expect) {
        return Ok(());
    }
    let n = expect.split_whitespace().count().clamp(1, 12);
    let said: Vec<&str> = word_pos(words, from_word)
        .map(|p| words[p..].iter().take(n).map(|w| w.text.as_str()).collect())
        .unwrap_or_default();
    Err(validation(
        "clip.create",
        format!("opening words do not match: transcript says '{}'", said.join(" ")),
    )
    .hint("check the unit ids with `rustler transcript get <source_id>` or fix --expect-text"))
}

/// Index (`Word::i`) of the first word still playing at source time `t`.
fn word_at(words: &[Word], t: f64) -> Option<u32> {
    words.iter().find(|w| w.end > t + 1e-6).map(|w| w.i)
}

/// Picks the words matching the caption script the clip wants (the romanization choice).
/// `transcript` is `transcript.json` (romanized when the project script is latin); `raw` is the
/// original-script `transcript.raw.json`, if any. Returns the words plus a warning when the
/// wanted script is not available.
pub fn select_script_words(
    transcript: &Transcript,
    raw: Option<&Transcript>,
    want: CaptionScript,
) -> (Vec<Word>, Option<String>) {
    if want == CaptionScript::AsTranscribed || want == transcript.script {
        return (transcript.words.clone(), None);
    }
    if let Some(r) = raw.filter(|r| r.script == want) {
        return (r.words.clone(), None);
    }
    let warn = format!(
        "captions want {want:?} script but the transcript of {} is {:?} and no {want:?} version exists; \
         using the transcript as is (re-run `rustler analyze transcribe` after changing the project caption script)",
        transcript.source_id, transcript.script
    );
    (transcript.words.clone(), Some(warn))
}

/// Caption words for `clip` in output time: for every segment (via `TimeMap`), the words whose
/// midpoint lies inside it, clipped to the segment and shifted to the output timeline.
pub fn build_caption_words(clip: &Clip, words_by_source: &HashMap<SourceId, Vec<Word>>) -> Vec<CaptionWord> {
    let tm = TimeMap::from_segments(&clip.segments);
    let mut out = Vec::new();
    for seg in tm.segments() {
        let Some(words) = words_by_source.get(&seg.source_id) else { continue };
        for w in words {
            let text = w.text.trim();
            if text.is_empty() {
                continue;
            }
            let mid = (w.start + w.end) / 2.0;
            if mid < seg.src_start || mid >= seg.src_end {
                continue;
            }
            let s = w.start.max(seg.src_start);
            let e = w.end.min(seg.src_end).max(s + 0.02);
            out.push(CaptionWord {
                text: text.to_string(),
                out_start: seg.out_start + (s - seg.src_start),
                out_end: (seg.out_start + (e - seg.src_start)).min(seg.out_end.max(seg.out_start + 0.02)),
            });
        }
    }
    out.sort_by(|a, b| a.out_start.total_cmp(&b.out_start));
    out
}

pub(crate) fn fmt_ts(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor() as u64;
    format!("{m:02}:{:04.1}", t - (m as f64) * 60.0)
}

// ---------------------------------------------------------------------------------------------
// Shared IO helpers (pub(crate): see_cmds / render_cmds use them)
// ---------------------------------------------------------------------------------------------

pub(crate) fn transcript_rel(id: &SourceId) -> PathBuf {
    Path::new("analysis").join(id.as_str()).join("transcript.json")
}

fn raw_transcript_rel(id: &SourceId) -> PathBuf {
    Path::new("analysis").join(id.as_str()).join("transcript.raw.json")
}

pub(crate) fn extras_rel(id: &ClipId) -> PathBuf {
    Path::new("clips").join(id.as_str()).join("extras.json")
}

/// `transcript.json` of a source, `None` when it has not been transcribed yet.
pub(crate) fn load_transcript(store: &ProjectStore, id: &SourceId) -> Result<Option<Transcript>> {
    let rel = transcript_rel(id);
    if !store.resolve(&rel).is_file() {
        return Ok(None);
    }
    store.read_json(&rel).map(Some)
}

fn load_raw_transcript(store: &ProjectStore, id: &SourceId) -> Option<Transcript> {
    let rel = raw_transcript_rel(id);
    if !store.resolve(&rel).is_file() {
        return None;
    }
    store.read_json(&rel).ok()
}

/// Appends an event to the project log (actor from the context).
pub(crate) fn log_event(ctx: &Ctx, store: &ProjectStore, kind: &str, data: serde_json::Value) -> Result<()> {
    let project_revision = store.load_project()?.revision;
    let actor: Actor = ctx.actor;
    store.append_event(&Event { ts: Utc::now(), actor, kind: kind.to_string(), project_revision, data })
}

/// Words per source, in the clip's caption script, for every source used by `clip`.
pub(crate) fn caption_words_for_clip(store: &ProjectStore, clip: &Clip) -> Result<(Vec<CaptionWord>, Vec<String>)> {
    let mut by_source: HashMap<SourceId, Vec<Word>> = HashMap::new();
    let mut warnings = Vec::new();
    for seg in &clip.segments {
        if by_source.contains_key(&seg.source_id) {
            continue;
        }
        match load_transcript(store, &seg.source_id)? {
            None => warnings.push(format!(
                "source {} has no transcript; its segments get no captions (run `rustler analyze transcribe {}`)",
                seg.source_id, seg.source_id
            )),
            Some(tr) => {
                let raw = load_raw_transcript(store, &seg.source_id);
                let (words, warn) = select_script_words(&tr, raw.as_ref(), clip.captions.script);
                warnings.extend(warn);
                by_source.insert(seg.source_id.clone(), words);
            }
        }
    }
    Ok((build_caption_words(clip, &by_source), warnings))
}

/// Sidecar `clips/<id>/extras.json`: data the frozen `Clip` has no field for.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct ClipExtras {
    #[serde(default)]
    pub music: Option<MusicBed>,
    /// Crop path (keyframes of the crop's left edge) per segment id.
    #[serde(default)]
    pub crop_paths: BTreeMap<String, Vec<CropKey>>,
    /// Every other extras key (overlays, sfx, transitions, effects, …) owned by other command
    /// modules — carried through untouched so saving music/crop data never clobbers them.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

pub(crate) fn load_extras(store: &ProjectStore, id: &ClipId) -> Result<ClipExtras> {
    let rel = extras_rel(id);
    if !store.resolve(&rel).is_file() {
        return Ok(ClipExtras::default());
    }
    store.read_json(&rel)
}

/// Extras in the shape the compiler takes; crop paths of deleted segments are dropped.
pub(crate) fn compile_extras(extras: &ClipExtras, clip: &Clip) -> rustler_media::compile::CompileExtras {
    let mut out = rustler_media::compile::CompileExtras { music: extras.music.clone(), ..Default::default() };
    for seg in &clip.segments {
        if let Some(keys) = extras.crop_paths.get(seg.id.as_str()) {
            out.crop_paths.insert(seg.id.clone(), keys.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------------------------

/// A clip plus derived info; returned by every command that creates or changes a clip.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct ClipView {
    #[serde(flatten)]
    pub clip: Clip,
    /// Output duration in seconds.
    pub duration: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    /// Suggested follow-up commands.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn view(clip: Clip, dry: bool, next: Vec<String>, warnings: Vec<String>) -> ClipView {
    let duration = TimeMap::from_segments(&clip.segments).duration();
    ClipView { clip, duration, dry_run: dry.then_some(true), next, warnings }
}

fn default_next(clip: &Clip) -> Vec<String> {
    let id = &clip.id;
    let mid = TimeMap::from_segments(&clip.segments).duration() / 2.0;
    vec![
        format!("rustler render preview-frame {id} --out-t {mid:.1}"),
        format!("rustler render run {id} --quality draft"),
        "rustler caption list-styles".to_string(),
    ]
}

// ---------------------------------------------------------------------------------------------
// Range resolution (clip.create / clip.add_segment)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct RangeSpec {
    start_unit: Option<u32>,
    end_unit: Option<u32>,
    start: Option<f64>,
    end: Option<f64>,
    expect_text: Option<String>,
}

struct ResolvedRange {
    start: f64,
    end: f64,
    /// True when the source has transcript words (captions possible).
    has_words: bool,
}

fn resolve_range(store: &ProjectStore, source: &Source, spec: &RangeSpec, stage: &str) -> Result<ResolvedRange> {
    if !source.probe.has_video {
        return Err(validation(stage, format!("source {} has no video", source.id))
            .hint("audio-only sources cannot become clips; import a video source"));
    }
    let transcript = load_transcript(store, &source.id)?;
    let words: &[Word] = transcript.as_ref().map_or(&[], |t| t.words.as_slice());
    let duration = source.probe.duration;

    let (start, end, from_word) = match (spec.start_unit, spec.end_unit) {
        (None, Some(_)) => return Err(validation(stage, "end_unit needs start_unit")),
        (Some(su), eu) => {
            let units = build_units(words);
            if units.is_empty() {
                return Err(validation(stage, format!("source {} has no transcript units", source.id))
                    .hint("run `rustler analyze transcribe <source>` or pass --start/--end"));
            }
            let (s, e) = unit_range(words, &units, su, eu.unwrap_or(su), duration)?;
            let from = units.iter().find(|u| u.id == su).map(|u| u.word_start);
            (s, e, from)
        }
        (None, None) => {
            let (Some(s), Some(e)) = (spec.start, spec.end) else {
                return Err(validation(stage, "give a range: --start-unit/--end-unit or --start/--end")
                    .hint("prefer unit ids from `rustler transcript get <source_id>`"));
            };
            (s, e, word_at(words, s))
        }
    };

    if !start.is_finite() || !end.is_finite() || start < 0.0 {
        return Err(validation(stage, format!("invalid range {start}..{end}")));
    }
    if end <= start {
        return Err(validation(stage, format!("segment end {end:.2} must be after start {start:.2}")));
    }
    if end > duration + END_TOLERANCE {
        return Err(validation(
            stage,
            format!("segment end {end:.2}s is past the end of the source ({duration:.2}s)"),
        ));
    }
    let end = end.min(duration.max(start + 0.001));

    if let Some(expect) = spec.expect_text.as_deref().filter(|t| !t.trim().is_empty()) {
        match from_word {
            Some(fw) => check_expect_text(words, fw, expect)?,
            None => {
                return Err(validation(
                    stage,
                    "opening words do not match: transcript says '' (no words at that position)",
                )
                .hint("transcribe the source first or drop --expect-text"));
            }
        }
    }
    Ok(ResolvedRange { start, end, has_words: !words.is_empty() })
}

fn make_layout(
    source: &Source,
    canvas: &rustler_core::Canvas,
    preset: &str,
    params: &LayoutParams,
) -> Result<Layout> {
    layout::preset(preset, source.probe.width, source.probe.height, canvas, params)
}

#[cfg_attr(not(test), allow(dead_code))]
fn check_caption_preset(stage: &str, name: &str) -> Result<()> {
    if rustler_captions::PRESETS.contains(&name) {
        Ok(())
    } else {
        Err(validation(stage, format!("unknown caption style '{name}'"))
            .hint(format!("available styles: {} (see `rustler caption list-styles`)", rustler_captions::PRESETS.join(", "))))
    }
}

// ---------------------------------------------------------------------------------------------
// clip.create
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ClipCreateIn {
    pub name: String,
    pub source_id: SourceId,
    /// First transcript unit (preferred over raw times).
    pub start_unit: Option<u32>,
    /// Last transcript unit (default: same as start_unit).
    pub end_unit: Option<u32>,
    /// Source seconds; used when no units are given.
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// The first words you expect at the start; mismatch is a validation error.
    pub expect_text: Option<String>,
    /// center-crop | blur-fit | split-vertical (default center-crop).
    pub layout: Option<String>,
    /// focus_x for center-crop; top/bottom rects (+split) for split-vertical.
    pub layout_params: Option<LayoutParams>,
    /// Captions on (default true; auto-off when the source has no words).
    pub captions: Option<bool>,
    /// Caption style preset name (see caption.list_styles).
    pub caption_style: Option<String>,
}

async fn clip_create(ctx: Ctx, i: ClipCreateIn) -> Result<ClipView> {
    let store = ctx.store()?;
    let mut project = store.load_project()?;
    let source = store.load_source(&i.source_id)?;
    let range = resolve_range(
        &store,
        &source,
        &RangeSpec {
            start_unit: i.start_unit,
            end_unit: i.end_unit,
            start: i.start,
            end: i.end,
            expect_text: i.expect_text.clone(),
        },
        "clip.create",
    )?;
    let canvas = project.settings.canvas;
    let preset = i.layout.as_deref().unwrap_or("center-crop");
    let layout = make_layout(&source, &canvas, preset, &i.layout_params.unwrap_or_default())?;

    let mut captions = CaptionTrack { script: project.settings.caption_script, ..Default::default() };
    if let Some(style) = &i.caption_style {
        crate::asset_cmds::check_caption_style(&ctx, "clip.create", style)?;
        captions.preset = style.clone();
    }
    let mut warnings = Vec::new();
    captions.enabled = i.captions.unwrap_or(true);
    if captions.enabled && !range.has_words {
        captions.enabled = false;
        warnings.push(format!(
            "source {} has no transcript words; captions disabled (transcribe, then `rustler caption set <clip> --enabled true`)",
            source.id
        ));
    }

    let mut clip = Clip {
        schema: "rustler.clip@1".to_string(),
        id: ClipId::new(),
        name: i.name,
        revision: 0,
        canvas,
        segments: vec![Segment {
            id: SegmentId::new(),
            source_id: source.id.clone(),
            start: range.start,
            end: range.end,
            layout,
        }],
        captions,
        audio: AudioSettings::default(),
        renders: Vec::new(),
        created_at: Utc::now(),
    };
    let next = default_next(&clip);
    if ctx.dry_run {
        return Ok(view(clip, true, next, warnings));
    }
    store.save_clip(&mut clip, 0)?;
    let base = project.revision;
    project.clips.push(clip.id.clone());
    store.save_project(&mut project, base)?;
    log_event(
        &ctx,
        &store,
        "clip.create",
        serde_json::json!({"clip_id": clip.id, "source_id": source.id, "start": range.start, "end": range.end}),
    )?;
    Ok(view(clip, false, next, warnings))
}

// ---------------------------------------------------------------------------------------------
// Generic load-modify-save for existing clips
// ---------------------------------------------------------------------------------------------

/// Loads the clip, applies `f`, saves with `base_revision` (default: the revision just loaded) and
/// logs `kind`. `f` returns warnings. Honors `--dry-run`.
fn mutate_clip<F>(ctx: &Ctx, clip_id: &ClipId, base_revision: Option<u64>, kind: &str, f: F) -> Result<ClipView>
where
    F: FnOnce(&ProjectStore, &mut Clip) -> Result<Vec<String>>,
{
    let store = ctx.store()?;
    let mut clip = store.load_clip(clip_id)?;
    let base = base_revision.unwrap_or(clip.revision);
    let warnings = f(&store, &mut clip)?;
    if ctx.dry_run {
        return Ok(view(clip, true, Vec::new(), warnings));
    }
    store.save_clip(&mut clip, base)?;
    log_event(ctx, &store, kind, serde_json::json!({"clip_id": clip.id, "revision": clip.revision}))?;
    Ok(view(clip, false, Vec::new(), warnings))
}

// ---------------------------------------------------------------------------------------------
// clip.add_segment
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AddSegmentIn {
    pub clip_id: ClipId,
    pub source_id: SourceId,
    pub start_unit: Option<u32>,
    pub end_unit: Option<u32>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub expect_text: Option<String>,
    /// Defaults to the preset of the clip's last segment (else center-crop).
    pub layout: Option<String>,
    pub layout_params: Option<LayoutParams>,
    /// Fail with revision_conflict unless the clip is at this revision.
    pub base_revision: Option<u64>,
}

async fn clip_add_segment(ctx: Ctx, i: AddSegmentIn) -> Result<ClipView> {
    let spec = RangeSpec {
        start_unit: i.start_unit,
        end_unit: i.end_unit,
        start: i.start,
        end: i.end,
        expect_text: i.expect_text.clone(),
    };
    let mut v = mutate_clip(&ctx, &i.clip_id, i.base_revision, "clip.add_segment", |store, clip| {
        let source = store.load_source(&i.source_id)?;
        let range = resolve_range(store, &source, &spec, "clip.add_segment")?;
        let preset = i
            .layout
            .clone()
            .or_else(|| clip.segments.last().and_then(|s| s.layout.preset.clone()))
            .unwrap_or_else(|| "center-crop".to_string());
        let layout = make_layout(&source, &clip.canvas, &preset, &i.layout_params.unwrap_or_default())?;
        clip.segments.push(Segment {
            id: SegmentId::new(),
            source_id: source.id.clone(),
            start: range.start,
            end: range.end,
            layout,
        });
        Ok(Vec::new())
    })?;
    v.next = default_next(&v.clip);
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// clip.set_layout
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct SetLayoutIn {
    pub clip_id: ClipId,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// center-crop | blur-fit | split-vertical (see layout.list_presets).
    pub preset: String,
    pub params: Option<LayoutParams>,
    pub base_revision: Option<u64>,
}

async fn clip_set_layout(ctx: Ctx, i: SetLayoutIn) -> Result<ClipView> {
    let mut v = mutate_clip(&ctx, &i.clip_id, i.base_revision, "clip.set_layout", |store, clip| {
        let params = i.params.unwrap_or_default();
        let canvas = clip.canvas;
        let mut touched = 0;
        for seg in clip.segments.iter_mut().filter(|s| i.segment_id.as_ref().is_none_or(|id| id == &s.id)) {
            let source = store.load_source(&seg.source_id)?;
            seg.layout = make_layout(&source, &canvas, &i.preset, &params)?;
            touched += 1;
        }
        if touched == 0 {
            return Err(RustlerError::new(
                ErrorCode::NotFound,
                "clip.set_layout",
                format!("clip {} has no such segment", i.clip_id),
            )
            .hint("list segment ids with `rustler clip show <clip_id>`"));
        }
        Ok(Vec::new())
    })?;
    v.next = default_next(&v.clip);
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// clip.set_captions / caption.set
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct SetCaptionsIn {
    pub clip_id: ClipId,
    pub enabled: Option<bool>,
    /// Caption style preset (see caption.list_styles). `preset` is accepted as an alias.
    #[serde(alias = "preset")]
    pub style: Option<String>,
    /// Vertical position of the caption baseline, 0 (top) .. 1 (bottom); default 0.70.
    pub position_y: Option<f64>,
    pub max_words_per_line: Option<u32>,
    /// latin | devanagari | as_transcribed
    pub script: Option<CaptionScript>,
    pub base_revision: Option<u64>,
}

async fn clip_set_captions(ctx: Ctx, i: SetCaptionsIn) -> Result<ClipView> {
    mutate_clip(&ctx, &i.clip_id, i.base_revision, "clip.set_captions", |_, clip| {
        if let Some(style) = &i.style {
            crate::asset_cmds::check_caption_style(&ctx, "clip.set_captions", style)?;
            clip.captions.preset = style.clone();
        }
        if let Some(y) = i.position_y {
            if !(0.0..=1.0).contains(&y) {
                return Err(validation("clip.set_captions", "position_y must be between 0 and 1"));
            }
            clip.captions.position_y = y;
        }
        if let Some(n) = i.max_words_per_line {
            if n == 0 || n > 12 {
                return Err(validation("clip.set_captions", "max_words_per_line must be 1..=12"));
            }
            clip.captions.max_words_per_line = n;
        }
        if let Some(s) = i.script {
            clip.captions.script = s;
        }
        if let Some(e) = i.enabled {
            clip.captions.enabled = e;
        }
        Ok(Vec::new())
    })
}

// ---------------------------------------------------------------------------------------------
// clip.show / clip.list
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ClipIdIn {
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema)]
pub struct TimelineSegment {
    pub segment_id: SegmentId,
    pub source_id: SourceId,
    pub src_start: f64,
    pub src_end: f64,
    pub out_start: f64,
    pub out_end: f64,
    pub layout: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ClipShow {
    pub clip: Clip,
    pub duration: f64,
    pub timeline: Vec<TimelineSegment>,
    pub extras: ClipExtras,
    pub summary: String,
}

/// One-line human summary: `"My clip: 2 segments, 31.4s, captions bold-karaoke, 1 render"`.
pub fn clip_summary(clip: &Clip, duration: f64) -> String {
    let n = clip.segments.len();
    let caps = if clip.captions.enabled { format!("captions {}", clip.captions.preset) } else { "no captions".into() };
    format!(
        "{}: {n} segment{}, {duration:.1}s, {caps}, {} render{}",
        clip.name,
        if n == 1 { "" } else { "s" },
        clip.renders.len(),
        if clip.renders.len() == 1 { "" } else { "s" },
    )
}

async fn clip_show(ctx: Ctx, i: ClipIdIn) -> Result<ClipShow> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let tm = TimeMap::from_segments(&clip.segments);
    let timeline = tm
        .segments()
        .iter()
        .zip(&clip.segments)
        .map(|(m, s)| TimelineSegment {
            segment_id: m.segment_id.clone(),
            source_id: m.source_id.clone(),
            src_start: m.src_start,
            src_end: m.src_end,
            out_start: m.out_start,
            out_end: m.out_end,
            layout: s.layout.preset.clone(),
        })
        .collect();
    let extras = load_extras(&store, &clip.id)?;
    let duration = tm.duration();
    Ok(ClipShow { summary: clip_summary(&clip, duration), clip, duration, timeline, extras })
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct EmptyIn {}

#[derive(Serialize, JsonSchema)]
pub struct ClipListItem {
    pub id: ClipId,
    pub name: String,
    pub revision: u64,
    pub duration: f64,
    pub segments: usize,
    pub renders: usize,
    pub summary: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ClipListOut {
    pub clips: Vec<ClipListItem>,
}

async fn clip_list(ctx: Ctx, _i: EmptyIn) -> Result<ClipListOut> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let mut clips = Vec::new();
    for id in &project.clips {
        let c = store.load_clip(id)?;
        let duration = TimeMap::from_segments(&c.segments).duration();
        clips.push(ClipListItem {
            id: c.id.clone(),
            name: c.name.clone(),
            revision: c.revision,
            duration,
            segments: c.segments.len(),
            renders: c.renders.len(),
            summary: clip_summary(&c, duration),
        });
    }
    Ok(ClipListOut { clips })
}

// ---------------------------------------------------------------------------------------------
// caption.list_styles / caption.export_srt
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct ListStylesOut {
    pub styles: Vec<PresetInfo>,
}

async fn caption_list_styles(ctx: Ctx, _i: EmptyIn) -> Result<ListStylesOut> {
    Ok(ListStylesOut { styles: crate::asset_cmds::preset_infos(&ctx.home, ctx.project_dir.as_deref()) })
}

#[derive(Serialize, JsonSchema)]
pub struct ExportSrtOut {
    pub path: PathBuf,
    pub cues_words: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

async fn caption_export_srt(ctx: Ctx, i: ClipIdIn) -> Result<ExportSrtOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let (words, mut warnings) = caption_words_for_clip(&store, &clip)?;
    if words.is_empty() {
        warnings.push("no caption words fall inside this clip; the SRT is empty".to_string());
    }
    let rel = Path::new("clips").join(clip.id.as_str()).join("captions.srt");
    let path = store.resolve(&rel);
    if !ctx.dry_run {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| RustlerError::new(ErrorCode::Internal, "caption.export_srt", format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&path, write_srt(&words, clip.captions.max_words_per_line))
            .map_err(|e| RustlerError::new(ErrorCode::Internal, "caption.export_srt", format!("{}: {e}", path.display())))?;
        log_event(&ctx, &store, "caption.export_srt", serde_json::json!({"clip_id": clip.id}))?;
    }
    Ok(ExportSrtOut { path, cues_words: words.len(), warnings })
}

// ---------------------------------------------------------------------------------------------
// layout.list_presets
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
pub struct LayoutPresetInfo {
    pub name: String,
    pub description: String,
    /// Names of the `layout_params` / `params` fields the preset reads.
    pub params: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct LayoutPresetsOut {
    pub presets: Vec<LayoutPresetInfo>,
}

/// Description and params of a layout preset (empty strings for names this build does not know).
pub fn layout_preset_info(name: &str) -> LayoutPresetInfo {
    let (description, params): (&str, &[&str]) = match name {
        "center-crop" => (
            "Crop a vertical window out of a landscape source and fill the whole 9:16 canvas. Best for a single speaker.",
            &["focus_x (0..1, default 0.5)"],
        ),
        "blur-fit" => (
            "Whole source frame fitted inside the canvas over a blurred, darkened copy of itself. Nothing is cropped.",
            &[],
        ),
        "split-vertical" => (
            "Two panes stacked: `top` and `bottom` source rects (normalized 0..1) on the upper and lower part of the canvas. Good for gameplay + facecam.",
            &["top {x,y,w,h}", "bottom {x,y,w,h}", "split (top height fraction, default 0.5)"],
        ),
        _ => ("", &[]),
    };
    LayoutPresetInfo {
        name: name.to_string(),
        description: description.to_string(),
        params: params.iter().map(|s| s.to_string()).collect(),
    }
}

async fn layout_list_presets(_ctx: Ctx, _i: EmptyIn) -> Result<LayoutPresetsOut> {
    Ok(LayoutPresetsOut { presets: layout::PRESETS.iter().map(|n| layout_preset_info(n)).collect() })
}

// ---------------------------------------------------------------------------------------------
// audio.add_music / audio.remove_music
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AddMusicIn {
    pub clip_id: ClipId,
    /// Local audio file (mp3/wav/m4a/...) or `asset:<id>` from the asset library. Looped to the clip length.
    pub path: PathBuf,
    /// Music gain before mixing (default -18).
    pub volume_db: Option<f64>,
    /// Duck the music under speech (default true).
    pub duck: Option<bool>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct MusicOut {
    pub clip_id: ClipId,
    pub music: Option<MusicBed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

fn save_extras(store: &ProjectStore, id: &ClipId, extras: &ClipExtras) -> Result<()> {
    store.write_json(&extras_rel(id), extras)
}

async fn audio_add_music(ctx: Ctx, i: AddMusicIn) -> Result<MusicOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    // `asset:<id>` resolves to a library file (see asset.search --kind music).
    let src = crate::asset_cmds::resolve_asset_path(&store, &ctx.home, &i.path.to_string_lossy())?;
    if !src.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, "audio.add_music", format!("no such file: {}", src.display()))
            .hint("pass an existing local audio file or asset:<id> (see `rustler asset search --kind music`)"));
    }
    let path = std::path::absolute(&src).unwrap_or_else(|_| src.clone());
    let mut bed = MusicBed::new(path);
    if let Some(v) = i.volume_db {
        if !(-60.0..=6.0).contains(&v) {
            return Err(validation("audio.add_music", "volume_db must be between -60 and 6"));
        }
        bed.volume_db = v;
    }
    if let Some(d) = i.duck {
        bed.duck = d;
    }
    for (name, v, slot) in [("fade_in", i.fade_in, &mut bed.fade_in), ("fade_out", i.fade_out, &mut bed.fade_out)] {
        if let Some(v) = v {
            if !(0.0..=30.0).contains(&v) {
                return Err(validation("audio.add_music", format!("{name} must be between 0 and 30 seconds")));
            }
            *slot = v;
        }
    }
    let next = vec![
        format!("rustler render preview-frame {} --out-t 1.0", clip.id),
        format!("rustler render run {} --quality draft", clip.id),
    ];
    if ctx.dry_run {
        return Ok(MusicOut { clip_id: clip.id, music: Some(bed), dry_run: Some(true), next });
    }
    let mut extras = load_extras(&store, &clip.id)?;
    extras.music = Some(bed.clone());
    save_extras(&store, &clip.id, &extras)?;
    log_event(&ctx, &store, "audio.add_music", serde_json::json!({"clip_id": clip.id, "path": bed.path}))?;
    Ok(MusicOut { clip_id: clip.id, music: Some(bed), dry_run: None, next })
}

async fn audio_remove_music(ctx: Ctx, i: ClipIdIn) -> Result<MusicOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    if ctx.dry_run {
        return Ok(MusicOut { clip_id: clip.id, music: None, dry_run: Some(true), next: Vec::new() });
    }
    let mut extras = load_extras(&store, &clip.id)?;
    extras.music = None;
    save_extras(&store, &clip.id, &extras)?;
    log_event(&ctx, &store, "audio.remove_music", serde_json::json!({"clip_id": clip.id}))?;
    Ok(MusicOut { clip_id: clip.id, music: None, dry_run: None, next: Vec::new() })
}

// ---------------------------------------------------------------------------------------------
// crop.track_face / crop.clear
// ---------------------------------------------------------------------------------------------

/// Face sampling rate for `crop.track_face`.
const TRACK_FPS: f64 = 4.0;

#[derive(Deserialize, JsonSchema)]
pub struct TrackFaceIn {
    pub clip_id: ClipId,
    /// All center-crop segments when omitted.
    pub segment_id: Option<SegmentId>,
    /// Crop width as a fraction of the source width; default: the width of the segment's first
    /// layout region.
    pub crop_w: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct TrackedSegment {
    pub segment_id: SegmentId,
    pub keys: Vec<CropKey>,
    /// Share of the sampled frames in which a face was found (0..1).
    pub faces_found_ratio: f64,
    pub samples: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct TrackFaceOut {
    pub clip_id: ClipId,
    pub segments: Vec<TrackedSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
struct FacesTrackResult {
    samples: Vec<rustler_media::facepath::FaceSample>,
}

/// Which segments a crop command touches; NotFound when `segment_id` names none.
fn select_segments<'a>(clip: &'a Clip, segment_id: Option<&SegmentId>, stage: &str) -> Result<Vec<&'a Segment>> {
    let segs: Vec<&Segment> = clip.segments.iter().filter(|s| segment_id.is_none_or(|id| id == &s.id)).collect();
    if segs.is_empty() {
        return Err(RustlerError::new(ErrorCode::NotFound, stage, format!("clip {} has no such segment", clip.id))
            .hint("list segment ids with `rustler clip show <clip_id>`"));
    }
    Ok(segs)
}

/// Crop width (fraction of the source) for face tracking, from `crop_w` or the first region;
/// Validation unless the segment uses a center-crop layout that actually crops.
pub fn tracking_crop_width(seg: &Segment, override_w: Option<f64>) -> Result<f64> {
    let stage = "crop.track_face";
    if seg.layout.preset.as_deref() != Some("center-crop") {
        return Err(validation(
            stage,
            format!("segment {} uses layout {:?}; face tracking only drives center-crop layouts", seg.id, seg.layout.preset),
        )
        .hint("run `rustler clip set-layout <clip_id> --preset center-crop` first"));
    }
    let w = override_w.or_else(|| seg.layout.regions.first().map(|r| r.src.w)).unwrap_or(1.0);
    if !(0.05..0.999).contains(&w) {
        return Err(validation(stage, format!("crop width {w:.3} leaves nothing to track (source is already as narrow as the crop)"))
            .hint("face tracking is useful for landscape sources cropped to 9:16"));
    }
    Ok(w)
}

async fn crop_track_face(ctx: Ctx, i: TrackFaceIn) -> Result<TrackFaceOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let segs = select_segments(&clip, i.segment_id.as_ref(), "crop.track_face")?;
    let widths: Vec<f64> = segs.iter().map(|s| tracking_crop_width(s, i.crop_w)).collect::<Result<_>>()?;
    let next = vec![
        format!("rustler render preview-frame {} --out-t 1.0", clip.id),
        format!("rustler render run {} --quality draft", clip.id),
    ];
    if ctx.dry_run {
        return Ok(TrackFaceOut { clip_id: clip.id, segments: Vec::new(), dry_run: Some(true), next, warnings: Vec::new() });
    }
    let paths = rustler_tools::ToolPaths::from_home(&ctx.home);
    let engine = rustler_engine_bridge::EngineClient::spawn(&paths).await?;
    let mut extras = load_extras(&store, &clip.id)?;
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut failure = None;
    for (seg, crop_w) in segs.iter().zip(widths) {
        let source = store.load_source(&seg.source_id)?;
        let video = source
            .proxy
            .as_ref()
            .map(|p| store.resolve(p))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| store.resolve(&source.media_path));
        let params = serde_json::json!({
            "video_path": video, "start": seg.start, "end": seg.end, "fps": TRACK_FPS,
        });
        let res: Result<FacesTrackResult> = engine.call("faces.track", params, &ctx.progress, &ctx.cancel).await;
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                failure = Some(e);
                break;
            }
        };
        let found = res.samples.iter().filter(|s| !s.faces.is_empty()).count();
        let ratio = if res.samples.is_empty() { 0.0 } else { found as f64 / res.samples.len() as f64 };
        if ratio < 0.3 {
            warnings.push(format!(
                "segment {}: faces found in only {:.0}% of samples; the crop will mostly hold the centre",
                seg.id,
                ratio * 100.0
            ));
        }
        let keys = rustler_media::facepath::crop_keys(&res.samples, crop_w, &Default::default());
        extras.crop_paths.insert(seg.id.as_str().to_string(), keys.clone());
        out.push(TrackedSegment { segment_id: seg.id.clone(), keys, faces_found_ratio: ratio, samples: res.samples.len() });
    }
    engine.shutdown().await;
    if let Some(e) = failure {
        return Err(e);
    }
    save_extras(&store, &clip.id, &extras)?;
    log_event(&ctx, &store, "crop.track_face", serde_json::json!({"clip_id": clip.id, "segments": out.len()}))?;
    Ok(TrackFaceOut { clip_id: clip.id, segments: out, dry_run: None, next, warnings })
}

#[derive(Deserialize, JsonSchema)]
pub struct CropClearIn {
    pub clip_id: ClipId,
    /// All segments when omitted.
    pub segment_id: Option<SegmentId>,
}

#[derive(Serialize, JsonSchema)]
pub struct CropClearOut {
    pub clip_id: ClipId,
    pub cleared: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn crop_clear(ctx: Ctx, i: CropClearIn) -> Result<CropClearOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let segs = select_segments(&clip, i.segment_id.as_ref(), "crop.clear")?;
    let mut extras = load_extras(&store, &clip.id)?;
    let ids: Vec<String> = segs.iter().map(|s| s.id.as_str().to_string()).collect();
    let cleared = ids.iter().filter(|id| extras.crop_paths.contains_key(*id)).count();
    if ctx.dry_run {
        return Ok(CropClearOut { clip_id: clip.id, cleared, dry_run: Some(true) });
    }
    for id in &ids {
        extras.crop_paths.remove(id);
    }
    save_extras(&store, &clip.id, &extras)?;
    log_event(&ctx, &store, "crop.clear", serde_json::json!({"clip_id": clip.id, "cleared": cleared}))?;
    Ok(CropClearOut { clip_id: clip.id, cleared, dry_run: None })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

const READ: Annotations = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false };
const WRITE: Annotations = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "clip.create",
        "Create a clip from a source by transcript unit ids (preferred) or start/end seconds",
        &[],
        WRITE,
        vec![
            Example {
                description: "clip from units 12..14, checking the opening words, blur-fit layout",
                cli: "rustler clip create --name hook --source-id src_x --start-unit 12 --end-unit 14 --expect-text \"fellow Americans\" --layout blur-fit",
            },
            Example {
                description: "split-vertical with explicit panes",
                cli: "rustler clip create --name sp --source-id src_x --start 10 --end 40 --layout split-vertical --layout-params '{\"top\":{\"x\":0.25,\"y\":0,\"w\":0.5,\"h\":0.6},\"bottom\":{\"x\":0,\"y\":0.6,\"w\":1,\"h\":0.4}}'",
            },
        ],
        clip_create,
    ));
    reg.register(typed(
        "clip.add_segment",
        "Append another segment (same or another source) to a clip",
        &["clip_id"],
        WRITE,
        vec![Example { description: "append units 40..41", cli: "rustler clip add-segment clp_x --source-id src_x --start-unit 40 --end-unit 41" }],
        clip_add_segment,
    ));
    reg.register(typed(
        "clip.set_layout",
        "Set the layout preset (and params) for one segment or all segments",
        &["clip_id"],
        WRITE,
        vec![Example { description: "focus the crop at 30% of the width", cli: "rustler clip set-layout clp_x --preset center-crop --params '{\"focus_x\":0.3}'" }],
        clip_set_layout,
    ));
    reg.register(typed(
        "clip.set_captions",
        "Change caption settings of a clip (alias of caption.set)",
        &["clip_id"],
        WRITE,
        vec![],
        clip_set_captions,
    ));
    reg.register(typed(
        "caption.set",
        "Change caption settings of a clip: enabled, style, position_y, max_words_per_line, script",
        &["clip_id"],
        WRITE,
        vec![Example { description: "bigger-impact style, two words per line", cli: "rustler caption set clp_x --style hormozi-pop --max-words-per-line 2" }],
        clip_set_captions,
    ));
    reg.register(typed(
        "clip.show",
        "Show a clip with its output timeline, extras (music, crop paths) and summary",
        &["clip_id"],
        READ,
        vec![],
        clip_show,
    ));
    reg.register(typed("clip.list", "List clips of the project", &[], READ, vec![], clip_list));
    reg.register(typed(
        "caption.list_styles",
        "List caption style presets (use the name with --caption-style / caption set --style)",
        &[],
        READ,
        vec![],
        caption_list_styles,
    ));
    reg.register(typed(
        "caption.export_srt",
        "Write clips/<id>/captions.srt for a clip",
        &["clip_id"],
        WRITE,
        vec![],
        caption_export_srt,
    ));
    reg.register(typed(
        "layout.list_presets",
        "List layout presets with descriptions and the params each reads",
        &[],
        READ,
        vec![],
        layout_list_presets,
    ));
    reg.register(typed(
        "audio.add_music",
        "Add a background music bed (ducked under speech) to a clip",
        &["clip_id"],
        WRITE,
        vec![Example { description: "quiet ducked bed", cli: "rustler audio add-music clp_x --path bed.mp3 --volume-db -20" }],
        audio_add_music,
    ));
    reg.register(typed("audio.remove_music", "Remove the music bed from a clip", &["clip_id"], WRITE, vec![], audio_remove_music));
    reg.register(typed(
        "crop.track_face",
        "Follow the speaker's face with the crop of center-crop segments (face detection at 4 fps, smoothed)",
        &["clip_id"],
        Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: true },
        vec![Example { description: "track all segments", cli: "rustler crop track-face clp_x" }],
        crop_track_face,
    ));
    reg.register(typed(
        "crop.clear",
        "Remove tracked crop paths (back to the static focus_x crop)",
        &["clip_id"],
        WRITE,
        vec![],
        crop_clear,
    ));
}

// ---------------------------------------------------------------------------------------------
// Unit tests (pure logic)
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_core::{Background, Fit, Rect, Region, WordTiming};

    fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
        Word { i, text: text.into(), start, end, conf: None, speaker: None }
    }

    fn sample_words() -> Vec<Word> {
        vec![
            w(0, "Hello", 1.0, 1.4),
            w(1, "there.", 1.45, 1.9),
            w(2, "Fellow", 3.0, 3.4),
            w(3, "Americans,", 3.45, 4.0),
            w(4, "ask", 4.05, 4.3),
            w(5, "not.", 4.31, 5.0),
        ]
    }

    #[test]
    fn unit_range_pads_and_snaps() {
        let words = sample_words();
        let units = build_units(&words);
        assert!(units.len() >= 2, "{units:?}");
        // first unit: lead-in clamps at 0 / previous word, tail stops before next word
        let (s, e) = unit_range(&words, &units, 0, 0, 10.0).unwrap();
        assert!((s - 0.92).abs() < 1e-9, "start {s}");
        assert!((e - 2.05).abs() < 1e-9, "end {e}");
        // second unit starts 3.0: previous word ends 1.9 -> pad 0.08 is fine
        let (s, _) = unit_range(&words, &units, 1, 1, 10.0).unwrap();
        assert!((s - 2.92).abs() < 1e-9, "start {s}");
    }

    #[test]
    fn unit_range_never_crosses_neighbour_words() {
        // gap between words is only 0.03 s: both pads are clipped to the neighbours.
        let words = vec![w(0, "a.", 0.0, 1.0), w(1, "b.", 1.03, 2.0), w(2, "c.", 2.02, 3.0)];
        let units = build_units(&words);
        let u = units.iter().find(|u| u.word_start == 1).unwrap();
        let (s, e) = unit_range(&words, &units, u.id, u.id, 10.0).unwrap();
        assert!(s >= 1.0 && s <= 1.03 + 1e-9, "start {s}");
        assert!(e <= 2.02 + 1e-9 && e >= 2.0, "end {e}");
    }

    #[test]
    fn unit_range_clamps_to_duration_and_rejects_bad_ids() {
        let words = sample_words();
        let units = build_units(&words);
        let last = units.last().unwrap().id;
        let (_, e) = unit_range(&words, &units, last, last, 5.05).unwrap();
        assert!((e - 5.05).abs() < 1e-9, "end {e}");
        assert_eq!(unit_range(&words, &units, 99, 99, 10.0).unwrap_err().code, ErrorCode::Validation);
        assert_eq!(unit_range(&words, &units, 1, 0, 10.0).unwrap_err().code, ErrorCode::Validation);
    }

    #[test]
    fn expect_text_ok_and_mismatch() {
        let words = sample_words();
        check_expect_text(&words, 2, "fellow americans").unwrap();
        check_expect_text(&words, 2, "Fellow Americans, ask").unwrap();
        let e = check_expect_text(&words, 0, "fellow americans").unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation);
        assert!(e.message.contains("opening words do not match: transcript says 'Hello there.'"), "{}", e.message);
    }

    fn clip_with(segs: &[(&SourceId, f64, f64)]) -> Clip {
        let layout = Layout {
            preset: Some("center-crop".into()),
            regions: vec![Region {
                id: "main".into(),
                src: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
                dst: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
                z: 0,
                fit: Fit::Cover,
            }],
            background: Background::Black,
        };
        Clip {
            schema: "rustler.clip@1".into(),
            id: ClipId::new(),
            name: "t".into(),
            revision: 1,
            canvas: Default::default(),
            segments: segs
                .iter()
                .map(|(s, a, b)| Segment { id: SegmentId::new(), source_id: (*s).clone(), start: *a, end: *b, layout: layout.clone() })
                .collect(),
            captions: Default::default(),
            audio: Default::default(),
            renders: vec![],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn caption_words_go_through_timemap() {
        let src = SourceId::new();
        let words = sample_words();
        // seg 1: 0.9..2.0 ("Hello there."), seg 2: 2.9..4.35 ("Fellow Americans, ask")
        let clip = clip_with(&[(&src, 0.9, 2.0), (&src, 2.9, 4.35)]);
        let by = HashMap::from([(src.clone(), words)]);
        let cw = build_caption_words(&clip, &by);
        let texts: Vec<_> = cw.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Hello", "there.", "Fellow", "Americans,", "ask"]);
        assert!((cw[0].out_start - 0.1).abs() < 1e-9, "{}", cw[0].out_start);
        // second segment starts at out 1.1; "Fellow" at src 3.0 -> 1.1 + 0.1
        assert!((cw[2].out_start - 1.2).abs() < 1e-9, "{}", cw[2].out_start);
        assert!(cw.windows(2).all(|p| p[0].out_start <= p[1].out_start));
        // "ask" is clipped to the segment end (4.35 -> out 2.55)
        assert!(cw[4].out_end <= 2.55 + 1e-9, "{}", cw[4].out_end);
    }

    #[test]
    fn caption_words_skip_words_outside_segments() {
        let src = SourceId::new();
        let clip = clip_with(&[(&src, 3.0, 4.0)]);
        let by = HashMap::from([(src.clone(), sample_words())]);
        let cw = build_caption_words(&clip, &by);
        assert_eq!(cw.len(), 2);
        assert_eq!(cw[0].text, "Fellow");
        assert!(cw[0].out_start.abs() < 1e-9);
    }

    fn transcript(script: CaptionScript, text: &str) -> Transcript {
        Transcript {
            schema: "rustler.transcript@1".into(),
            source_id: SourceId::new(),
            language: "hi".into(),
            engine: "test".into(),
            script,
            words: vec![w(0, text, 0.0, 1.0)],
            word_timing: WordTiming::Exact,
        }
    }

    #[test]
    fn romanization_choice() {
        let latin = transcript(CaptionScript::Latin, "namaste");
        let deva = transcript(CaptionScript::Devanagari, "नमस्ते");
        // same script -> transcript
        let (w1, warn) = select_script_words(&latin, Some(&deva), CaptionScript::Latin);
        assert_eq!(w1[0].text, "namaste");
        assert!(warn.is_none());
        // want devanagari, raw available -> raw
        let (w2, warn) = select_script_words(&latin, Some(&deva), CaptionScript::Devanagari);
        assert_eq!(w2[0].text, "नमस्ते");
        assert!(warn.is_none());
        // want devanagari, no raw -> transcript + warning
        let (w3, warn) = select_script_words(&latin, None, CaptionScript::Devanagari);
        assert_eq!(w3[0].text, "namaste");
        assert!(warn.is_some());
        // as_transcribed never warns
        let (w4, warn) = select_script_words(&deva, None, CaptionScript::AsTranscribed);
        assert_eq!(w4[0].text, "नमस्ते");
        assert!(warn.is_none());
    }

    #[test]
    fn tracking_requires_cropping_center_crop() {
        let src = SourceId::new();
        let mut clip = clip_with(&[(&src, 0.0, 10.0)]);
        // clip_with regions are full-frame (w = 1.0): nothing to track
        assert_eq!(tracking_crop_width(&clip.segments[0], None).unwrap_err().code, ErrorCode::Validation);
        clip.segments[0].layout.regions[0].src.w = 0.316;
        assert!((tracking_crop_width(&clip.segments[0], None).unwrap() - 0.316).abs() < 1e-9);
        assert!((tracking_crop_width(&clip.segments[0], Some(0.5)).unwrap() - 0.5).abs() < 1e-9);
        clip.segments[0].layout.preset = Some("blur-fit".into());
        let e = tracking_crop_width(&clip.segments[0], None).unwrap_err();
        assert!(e.hint.unwrap().contains("set-layout"));
    }

    #[test]
    fn summaries_and_presets() {
        let src = SourceId::new();
        let clip = clip_with(&[(&src, 0.0, 10.0), (&src, 20.0, 25.0)]);
        assert_eq!(clip_summary(&clip, 15.0), "t: 2 segments, 15.0s, captions bold-karaoke, 0 renders");
        assert_eq!(fmt_ts(65.25), "01:05.2");
        for p in layout::PRESETS {
            assert!(!layout_preset_info(p).description.is_empty(), "{p}");
        }
        assert!(check_caption_preset("t", "bold-karaoke").is_ok());
        assert_eq!(check_caption_preset("t", "nope").unwrap_err().code, ErrorCode::Validation);
    }
}
