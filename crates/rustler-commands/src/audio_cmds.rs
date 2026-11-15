//! Commands: audio.denoise, audio.volume, audio.add_sfx, audio.remove_sfx, audio.list.
//! (`audio.add_music` / `audio.remove_music` live in `clip_cmds`.)
//!
//! Stored in `clips/<id>/extras.json` through [`crate::extras`] (other keys untouched):
//! `audio_fx` = `{denoise, volume_db, segment_volumes}` and `sfx` = list of [`SfxEntry`].
//! `render.run` folds them into the single encode: per-segment gain in each segment's audio
//! chain, `highpass` + `afftdn` (noise floor measured in a fast audio-only pass) and the clip
//! gain on the joined dialogue, then the sound effects (`adelay` in samples) and the music bed
//! are mixed before loudnorm. Word-anchored effects are re-resolved through the clip's
//! `TimeMap` at every render, so they move with recuts.

use crate::clip_cmds::{ClipExtras, load_transcript, log_event};
use crate::extras;
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, RustlerError, SegmentId, SourceId, TimeMap};
use rustler_media::compile::{CompileExtras, Denoise, MusicBed, SfxCue, TransitionMap};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

type Result<T> = rustler_core::Result<T>;

/// extras.json keys.
pub const FX_KEY: &str = "audio_fx";
pub const SFX_KEY: &str = "sfx";
const DEFAULT_SFX_DB: f64 = -6.0;
const SFX_EXTS: &[&str] = &["wav", "mp3", "m4a", "aac", "ogg", "opus", "flac", "webm", "mp4", "mov"];

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Stored types
// ---------------------------------------------------------------------------------------------

/// `audio_fx`: dialogue clean-up and gains.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct AudioFx {
    /// off | light | medium | strong
    #[serde(default)]
    pub denoise: Denoise,
    /// Whole-dialogue gain in dB (before the mix; loudnorm, when on, sets the final level).
    #[serde(default)]
    pub volume_db: f64,
    /// Gain in dB per segment id.
    #[serde(default)]
    pub segment_volumes: BTreeMap<String, f64>,
}

impl AudioFx {
    fn is_default(&self) -> bool {
        self.denoise == Denoise::Off && self.volume_db == 0.0 && self.segment_volumes.is_empty()
    }
}

/// A word of a source transcript (`transcript get` prints the indices).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct WordAnchor {
    pub source_id: SourceId,
    pub word_index: u32,
}

/// One sound effect: at a fixed edit-timeline time or on a transcript word.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct SfxEntry {
    pub id: String,
    /// Absolute path of the audio file.
    pub file: PathBuf,
    /// Edit-timeline time (seconds) when not word-anchored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// Plays at the start of this word wherever it lands after edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_word: Option<WordAnchor>,
    pub volume_db: f64,
}

// ---------------------------------------------------------------------------------------------
// Storage helpers (also used by fx_cmds)
// ---------------------------------------------------------------------------------------------

/// Read-modify-write of one extras key holding `T`; the key is removed when `empty(&T)`.
pub(crate) fn modify_key<T, R>(
    store: &ProjectStore,
    clip: &ClipId,
    key: &str,
    empty: impl Fn(&T) -> bool,
    f: impl FnOnce(&mut T) -> Result<R>,
) -> Result<R>
where
    T: Serialize + DeserializeOwned + Default,
{
    let mut out = None;
    extras::update(store, clip, key, |cur| {
        let mut v: T = match cur {
            None | Some(serde_json::Value::Null) => T::default(),
            Some(v) => serde_json::from_value(v).map_err(|e| {
                RustlerError::new(ErrorCode::Internal, "extras", format!("extras.json key '{key}' is invalid: {e}"))
                    .hint(format!("fix or remove '{key}' in clips/{clip}/extras.json"))
            })?,
        };
        out = Some(f(&mut v)?);
        if empty(&v) {
            return Ok(None);
        }
        serde_json::to_value(&v).map(Some).map_err(|e| RustlerError::new(ErrorCode::Internal, "extras", e.to_string()))
    })?;
    Ok(out.expect("closure ran"))
}

/// Runs `f` on the stored value: for real (saved) or, in dry-run, on a copy (nothing saved).
pub(crate) fn change<T, R>(
    ctx: &Ctx,
    store: &ProjectStore,
    clip: &ClipId,
    key: &str,
    empty: impl Fn(&T) -> bool,
    f: impl FnOnce(&mut T) -> Result<R>,
) -> Result<R>
where
    T: Serialize + DeserializeOwned + Default,
{
    if ctx.dry_run {
        let mut v: T = extras::get(store, clip, key)?.unwrap_or_default();
        f(&mut v)
    } else {
        modify_key(store, clip, key, empty, f)
    }
}

/// Next free `<prefix>_<n>` id.
pub(crate) fn next_id<'a>(prefix: &str, ids: impl Iterator<Item = &'a str>) -> String {
    let n = ids
        .filter_map(|id| id.strip_prefix(prefix).and_then(|s| s.strip_prefix('_')).and_then(|s| s.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    format!("{prefix}_{}", n + 1)
}

pub fn load_audio_fx(store: &ProjectStore, clip: &ClipId) -> Result<AudioFx> {
    Ok(extras::get(store, clip, FX_KEY)?.unwrap_or_default())
}

pub fn load_sfx(store: &ProjectStore, clip: &ClipId) -> Result<Vec<SfxEntry>> {
    Ok(extras::get(store, clip, SFX_KEY)?.unwrap_or_default())
}

fn edit_duration(clip: &Clip) -> f64 {
    TimeMap::from_segments(&clip.segments).duration()
}

// ---------------------------------------------------------------------------------------------
// Resolution (pure; unit-tested)
// ---------------------------------------------------------------------------------------------

/// Edit-timeline time of an effect: `at`, or the start of its word mapped through the TimeMap.
/// `Err(reason)` when the word is not in the clip (cut out) or unknown.
pub fn resolve_sfx_time(
    e: &SfxEntry,
    tm: &TimeMap,
    words: &HashMap<SourceId, Vec<rustler_core::Word>>,
) -> std::result::Result<f64, String> {
    if let Some(a) = &e.at_word {
        let Some(ws) = words.get(&a.source_id) else {
            return Err(format!("source {} has no transcript", a.source_id));
        };
        let Some(w) = ws.iter().find(|w| w.i == a.word_index).or_else(|| ws.get(a.word_index as usize)) else {
            return Err(format!("word {} does not exist in source {}", a.word_index, a.source_id));
        };
        // Midpoint inside the clip, start clamped to its segment (a word cut at its head still plays).
        let mid = (w.start + w.end) / 2.0;
        for s in tm.segments().iter().filter(|s| s.source_id == a.source_id) {
            if mid >= s.src_start && mid < s.src_end {
                return Ok(s.out_start + (w.start.max(s.src_start) - s.src_start));
            }
        }
        return Err(format!("word {} ('{}') is not in the clip any more", a.word_index, w.text.trim()));
    }
    e.at.ok_or_else(|| "sfx has neither at nor at_word".to_string())
}

/// The stored sound effects; entries that do not parse are skipped with a warning (a broken
/// entry must not block renders or preview frames).
fn parse_sfx(map: &serde_json::Map<String, serde_json::Value>, warnings: &mut Vec<String>) -> Vec<SfxEntry> {
    let Some(serde_json::Value::Array(items)) = map.get(SFX_KEY) else { return Vec::new() };
    items
        .iter()
        .enumerate()
        .filter_map(|(n, v)| match serde_json::from_value::<SfxEntry>(v.clone()) {
            Ok(e) => Some(e),
            Err(err) => {
                let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                warnings.push(format!("sfx entry {n} ({id}) in extras.json is invalid and ignored: {err}"));
                None
            }
        })
        .collect()
}

/// Compile-ready gains, denoise and sound effects; warnings for effects that will not play.
pub(crate) fn apply_to_compile(
    store: &ProjectStore,
    clip: &Clip,
    map: &serde_json::Map<String, serde_json::Value>,
    cx: &mut CompileExtras,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let fx: AudioFx = extras::from_map(map, FX_KEY)?.unwrap_or_default();
    cx.denoise = fx.denoise;
    cx.speech_gain_db = fx.volume_db;
    for seg in &clip.segments {
        if let Some(db) = fx.segment_volumes.get(seg.id.as_str()) {
            cx.segment_gain_db.insert(seg.id.clone(), *db);
        }
    }
    let entries = parse_sfx(map, &mut warnings);
    if entries.is_empty() {
        return Ok(warnings);
    }
    let tm = TimeMap::from_segments(&clip.segments);
    let duration = tm.duration();
    let mut words = HashMap::new();
    for e in &entries {
        if let Some(a) = &e.at_word
            && !words.contains_key(&a.source_id)
            && let Some(t) = load_transcript(store, &a.source_id)?
        {
            words.insert(a.source_id.clone(), t.words);
        }
    }
    for e in entries {
        match resolve_sfx_time(&e, &tm, &words) {
            Ok(at) if at < duration => {
                if !e.file.is_file() {
                    return Err(RustlerError::new(ErrorCode::NotFound, "render", format!("sfx file is missing: {}", e.file.display()))
                        .hint(format!("`rustler audio remove-sfx {} {}` or restore the file", clip.id, e.id)));
                }
                cx.sfx.push(SfxCue { id: e.id, path: e.file, at, volume_db: e.volume_db });
            }
            Ok(at) => warnings.push(format!("{} at {at:.2}s is past the end of the clip ({duration:.2}s); skipped", e.id)),
            Err(reason) => warnings.push(format!("{} skipped: {reason}", e.id)),
        }
    }
    Ok(warnings)
}

// ---------------------------------------------------------------------------------------------
// audio.denoise
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct DenoiseIn {
    pub clip_id: ClipId,
    /// light | medium (default) | strong | off
    pub strength: Option<Denoise>,
}

#[derive(Serialize, JsonSchema)]
pub struct AudioFxOut {
    pub clip_id: ClipId,
    pub audio_fx: AudioFx,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn render_next(clip: &ClipId) -> Vec<String> {
    vec![format!("rustler render run {clip} --quality draft"), format!("rustler audio list {clip}")]
}

async fn denoise(ctx: Ctx, i: DenoiseIn) -> Result<AudioFxOut> {
    const STAGE: &str = "audio.denoise";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let strength = i.strength.unwrap_or(Denoise::Medium);
    let fx = change(&ctx, &store, &clip.id, FX_KEY, AudioFx::is_default, |fx: &mut AudioFx| {
        fx.denoise = strength;
        Ok(fx.clone())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "strength": strength}))?;
    }
    let warnings = if strength == Denoise::Strong {
        vec!["strong denoise can sound watery on speech; listen to the draft render and step down to medium if it does".into()]
    } else {
        Vec::new()
    };
    Ok(AudioFxOut { next: render_next(&clip.id), clip_id: clip.id, audio_fx: fx, dry_run: ctx.dry_run.then_some(true), warnings })
}

// ---------------------------------------------------------------------------------------------
// audio.volume
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct VolumeIn {
    pub clip_id: ClipId,
    /// Segment to change; leave out for the whole dialogue.
    pub segment_id: Option<SegmentId>,
    /// Gain in dB (-60 .. 24; 0 resets).
    pub db: f64,
}

async fn volume(ctx: Ctx, i: VolumeIn) -> Result<AudioFxOut> {
    const STAGE: &str = "audio.volume";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    if !(i.db.is_finite() && (-60.0..=24.0).contains(&i.db)) {
        return Err(validation(STAGE, "db must be between -60 and 24").hint("e.g. --db -6 makes a segment quieter, --db 3 louder"));
    }
    if let Some(sid) = &i.segment_id
        && !clip.segments.iter().any(|s| &s.id == sid)
    {
        let ids: Vec<&str> = clip.segments.iter().map(|s| s.id.as_str()).collect();
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no segment {sid}", clip.id))
            .hint(format!("segments: {}", ids.join(", "))));
    }
    let fx = change(&ctx, &store, &clip.id, FX_KEY, AudioFx::is_default, |fx: &mut AudioFx| {
        match &i.segment_id {
            Some(sid) if i.db == 0.0 => {
                fx.segment_volumes.remove(sid.as_str());
            }
            Some(sid) => {
                fx.segment_volumes.insert(sid.to_string(), i.db);
            }
            None => fx.volume_db = i.db,
        }
        Ok(fx.clone())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "segment_id": i.segment_id, "db": i.db}))?;
    }
    let mut warnings = Vec::new();
    if i.segment_id.is_none() && clip.audio.loudnorm && i.db != 0.0 {
        warnings.push("loudnorm is on, so a whole-clip gain changes little; per-segment gains still balance segments".into());
    }
    Ok(AudioFxOut { next: render_next(&clip.id), clip_id: clip.id, audio_fx: fx, dry_run: ctx.dry_run.then_some(true), warnings })
}

// ---------------------------------------------------------------------------------------------
// audio.add_sfx / audio.remove_sfx
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AddSfxIn {
    pub clip_id: ClipId,
    /// Local audio file (wav/mp3/m4a/ogg/flac...), `asset:<id>`, an `http(s)` link to an audio file
    /// (downloaded to the project's assets cache), or `source:<source_id>[@start-end]` (audio of an imported video).
    pub file: PathBuf,
    /// Edit-timeline time in seconds (what `clip show` prints). Give this or at_word.
    pub at: Option<f64>,
    /// Play on a transcript word; follows the word when the clip is re-cut.
    pub at_word: Option<WordAnchor>,
    /// Gain in dB (default -6).
    pub volume_db: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct SfxView {
    #[serde(flatten)]
    pub sfx: SfxEntry,
    /// Edit-timeline time it plays at now (None when it will not play).
    pub resolved_at: Option<f64>,
    /// Time in the rendered file (after transitions).
    pub rendered_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct SfxOut {
    pub clip_id: ClipId,
    pub sfx_id: String,
    pub sfx: SfxView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

fn words_for(store: &ProjectStore, sources: impl Iterator<Item = SourceId>) -> Result<HashMap<SourceId, Vec<rustler_core::Word>>> {
    let mut words = HashMap::new();
    for s in sources {
        if !words.contains_key(&s)
            && let Some(t) = load_transcript(store, &s)?
        {
            words.insert(s, t.words);
        }
    }
    Ok(words)
}

fn sfx_view(e: SfxEntry, tm: &TimeMap, tmap: &TransitionMap, words: &HashMap<SourceId, Vec<rustler_core::Word>>) -> SfxView {
    let duration = tm.duration();
    let (resolved_at, problem) = match resolve_sfx_time(&e, tm, words) {
        Ok(t) if t < duration => (Some(t), None),
        Ok(t) => (None, Some(format!("at {t:.2}s is past the end of the clip ({duration:.2}s)"))),
        Err(r) => (None, Some(r)),
    };
    let problem = problem.or_else(|| (!e.file.is_file()).then(|| format!("file is missing: {}", e.file.display())));
    SfxView { rendered_at: resolved_at.map(|t| tmap.map_time(t)), resolved_at, problem, sfx: e }
}

async fn add_sfx(ctx: Ctx, mut i: AddSfxIn) -> Result<SfxOut> {
    const STAGE: &str = "audio.add_sfx";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    // A path, `asset:<id>` (`rustler asset list --kind sfx`), an http(s) URL or `source:<id>[@a-b]`.
    let resolved = crate::media_ref::resolve(&ctx, STAGE, &i.file.to_string_lossy(), crate::media_ref::Want::Audio).await?;
    i.file = resolved.path.clone();
    if !resolved.pending && !i.file.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("no such file: {}", i.file.display()))
            .hint("pass an existing local audio file (wav/mp3/m4a/ogg/flac)"));
    }
    let ext = i.file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !SFX_EXTS.contains(&ext.as_str()) {
        return Err(validation(STAGE, format!("'{}' does not look like an audio file", i.file.display()))
            .hint(format!("supported extensions: {}", SFX_EXTS.join(", "))));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    if paths.ffprobe.is_file() && !resolved.pending {
        let p = rustler_media::probe(&paths, &i.file).await.map_err(|e| {
            validation(STAGE, format!("cannot read {}: {}", i.file.display(), e.message))
        })?;
        if !p.has_audio {
            return Err(validation(STAGE, format!("{} has no audio stream", i.file.display())));
        }
    }
    let volume_db = i.volume_db.unwrap_or(DEFAULT_SFX_DB);
    if !(volume_db.is_finite() && (-60.0..=12.0).contains(&volume_db)) {
        return Err(validation(STAGE, "volume_db must be between -60 and 12"));
    }
    let tm = TimeMap::from_segments(&clip.segments);
    let duration = tm.duration();
    let mut entry = SfxEntry {
        id: String::new(),
        file: std::path::absolute(&i.file).unwrap_or_else(|_| i.file.clone()),
        at: None,
        at_word: None,
        volume_db,
    };
    match (i.at, i.at_word) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(validation(STAGE, "give exactly one of at (seconds) or at_word ({source_id, word_index})")
                .hint("e.g. --at 2.5, or --at-word '{\"source_id\":\"src_x\",\"word_index\":42}'"));
        }
        (Some(at), None) => {
            if !(at.is_finite() && at >= 0.0 && at < duration) {
                return Err(validation(STAGE, format!("at must be inside the clip (0 .. {duration:.2}s)"))
                    .hint("`rustler clip show <clip_id>` prints the timeline"));
            }
            entry.at = Some(at);
        }
        (None, Some(a)) => {
            let words = words_for(&store, std::iter::once(a.source_id.clone()))?;
            entry.at_word = Some(a);
            if let Err(reason) = resolve_sfx_time(&entry, &tm, &words) {
                return Err(validation(STAGE, reason)
                    .hint("pick a word inside the clip: `rustler transcript get <source_id>` lists word indices"));
            }
        }
    }
    let e = change(&ctx, &store, &clip.id, SFX_KEY, Vec::is_empty, |list: &mut Vec<SfxEntry>| {
        entry.id = next_id("sfx", list.iter().map(|e| e.id.as_str()));
        list.push(entry.clone());
        Ok(entry.clone())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "sfx_id": e.id, "file": e.file}))?;
    }
    let words = words_for(&store, e.at_word.iter().map(|a| a.source_id.clone()))?;
    let tmap = TransitionMap::new(&clip, &crate::fx_cmds::load_transitions(&store, &clip.id)?);
    let v = sfx_view(e, &tm, &tmap, &words);
    Ok(SfxOut {
        clip_id: clip.id.clone(),
        sfx_id: v.sfx.id.clone(),
        sfx: v,
        dry_run: ctx.dry_run.then_some(true),
        next: render_next(&clip.id),
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct RemoveSfxIn {
    pub clip_id: ClipId,
    pub sfx_id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct RemoveSfxOut {
    pub clip_id: ClipId,
    pub removed: String,
    pub remaining: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn remove_sfx(ctx: Ctx, i: RemoveSfxIn) -> Result<RemoveSfxOut> {
    const STAGE: &str = "audio.remove_sfx";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let remaining = change(&ctx, &store, &clip.id, SFX_KEY, Vec::is_empty, |list: &mut Vec<SfxEntry>| {
        let Some(pos) = list.iter().position(|e| e.id == i.sfx_id) else {
            let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
            return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no sound effect '{}'", clip.id, i.sfx_id))
                .hint(if ids.is_empty() { "the clip has no sound effects".to_string() } else { format!("sound effects: {}", ids.join(", ")) }));
        };
        list.remove(pos);
        Ok(list.iter().map(|e| e.id.clone()).collect::<Vec<_>>())
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "sfx_id": i.sfx_id}))?;
    }
    Ok(RemoveSfxOut { clip_id: clip.id, removed: i.sfx_id, remaining, dry_run: ctx.dry_run.then_some(true) })
}

// ---------------------------------------------------------------------------------------------
// audio.list
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ListIn {
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema)]
pub struct SegmentVolume {
    pub segment_id: String,
    pub db: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct AudioListOut {
    pub clip_id: ClipId,
    /// Edit-timeline duration (what overlay/sfx/effect times refer to).
    pub duration: f64,
    /// Rendered duration (shorter by the transition overlaps).
    pub rendered_duration: f64,
    pub loudnorm: bool,
    pub music: Option<MusicBed>,
    pub denoise: Denoise,
    /// Whole-dialogue gain (dB).
    pub volume_db: f64,
    pub segment_volumes: Vec<SegmentVolume>,
    pub sfx: Vec<SfxView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

async fn list(ctx: Ctx, i: ListIn) -> Result<AudioListOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let map = extras::load(&store, &clip.id)?;
    let typed_extras: ClipExtras = serde_json::from_value(serde_json::Value::Object(map.clone()))
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "audio.list", format!("extras.json is invalid: {e}")))?;
    let fx: AudioFx = extras::from_map(&map, FX_KEY)?.unwrap_or_default();
    let mut warnings = Vec::new();
    let entries = parse_sfx(&map, &mut warnings);
    let tm = TimeMap::from_segments(&clip.segments);
    let tmap = TransitionMap::new(&clip, &crate::fx_cmds::load_transitions(&store, &clip.id)?);
    let words = words_for(&store, entries.iter().filter_map(|e| e.at_word.as_ref().map(|a| a.source_id.clone())))?;
    let segment_volumes = fx
        .segment_volumes
        .iter()
        .filter_map(|(sid, db)| {
            if clip.segments.iter().any(|s| s.id.as_str() == sid) {
                Some(SegmentVolume { segment_id: sid.clone(), db: *db })
            } else {
                warnings.push(format!("volume for deleted segment {sid} is ignored"));
                None
            }
        })
        .collect();
    let sfx: Vec<SfxView> = entries.into_iter().map(|e| sfx_view(e, &tm, &tmap, &words)).collect();
    for s in &sfx {
        if let Some(p) = &s.problem {
            warnings.push(format!("{} will not play: {p}", s.sfx.id));
        }
    }
    Ok(AudioListOut {
        clip_id: clip.id.clone(),
        duration: edit_duration(&clip),
        rendered_duration: tmap.duration(),
        loudnorm: clip.audio.loudnorm,
        music: typed_extras.music,
        denoise: fx.denoise,
        volume_db: fx.volume_db,
        segment_volumes,
        sfx,
        warnings,
    })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    reg.register(typed(
        "audio.denoise",
        "Clean the dialogue: 80 Hz high-pass + FFT denoise tuned to the measured noise floor (light|medium|strong|off), applied at render before loudnorm",
        &["clip_id"],
        write,
        vec![
            Example { description: "medium denoise", cli: "rustler audio denoise clp_x" },
            Example { description: "turn it off", cli: "rustler audio denoise clp_x --strength off" },
        ],
        denoise,
    ));
    reg.register(typed(
        "audio.volume",
        "Gain in dB for one segment (balance a quiet take) or, without segment_id, for the whole dialogue; 0 resets",
        &["clip_id"],
        write,
        vec![
            Example { description: "segment 6 dB quieter", cli: "rustler audio volume clp_x --segment-id seg_x --db -6" },
            Example { description: "reset a segment", cli: "rustler audio volume clp_x --segment-id seg_x --db 0" },
        ],
        volume,
    ));
    reg.register(typed(
        "audio.add_sfx",
        "Add a sound effect at an edit-timeline time or on a transcript word (word-anchored effects follow the word through re-cuts); returns its id. The file can be a path, asset:<id>, an http(s) link or source:<source_id>[@start-end]",
        &["clip_id", "file"],
        write,
        vec![
            Example { description: "whoosh at 2.5 s", cli: "rustler audio add-sfx clp_x C:/sfx/whoosh.wav --at 2.5" },
            Example {
                description: "impact on word 42, quieter",
                cli: "rustler audio add-sfx clp_x C:/sfx/impact.wav --at-word '{\"source_id\":\"src_x\",\"word_index\":42}' --volume-db -10",
            },
        ],
        add_sfx,
    ));
    reg.register(typed(
        "audio.remove_sfx",
        "Remove a sound effect from a clip",
        &["clip_id", "sfx_id"],
        Annotations { destructive: true, ..write },
        vec![Example { description: "remove sfx_1", cli: "rustler audio remove-sfx clp_x sfx_1" }],
        remove_sfx,
    ));
    reg.register(typed(
        "audio.list",
        "Audio of a clip: music bed, sound effects (with the time each plays at now), denoise and volumes",
        &["clip_id"],
        Annotations { read_only: true, ..write },
        vec![Example { description: "audio overview", cli: "rustler audio list clp_x" }],
        list,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_core::{Background, Layout, Segment, Word};

    fn seg(src: &SourceId, s: f64, e: f64) -> Segment {
        Segment { id: SegmentId::new(), source_id: src.clone(), start: s, end: e, layout: Layout { preset: None, regions: vec![], background: Background::Black } }
    }

    fn w(i: u32, s: f64, e: f64) -> Word {
        Word { i, text: format!("w{i}"), start: s, end: e, conf: None, speaker: None }
    }

    #[test]
    fn word_anchor_follows_recuts() {
        let src = SourceId::new();
        let mut words = HashMap::new();
        words.insert(src.clone(), vec![w(0, 1.0, 1.4), w(1, 5.2, 5.6), w(2, 9.0, 9.5)]);
        let e = SfxEntry {
            id: "sfx_1".into(),
            file: "x.wav".into(),
            at: None,
            at_word: Some(WordAnchor { source_id: src.clone(), word_index: 1 }),
            volume_db: -6.0,
        };
        // Clip [4, 7]: word 1 starts 1.2 s in.
        let tm = TimeMap::from_segments(&[seg(&src, 4.0, 7.0)]);
        assert!((resolve_sfx_time(&e, &tm, &words).unwrap() - 1.2).abs() < 1e-9);
        // Re-cut: [0, 2] + [5, 7] -> 2.0 + 0.2.
        let tm = TimeMap::from_segments(&[seg(&src, 0.0, 2.0), seg(&src, 5.0, 7.0)]);
        assert!((resolve_sfx_time(&e, &tm, &words).unwrap() - 2.2).abs() < 1e-9);
        // Word head trimmed off: clamps to the segment start.
        let tm = TimeMap::from_segments(&[seg(&src, 5.3, 7.0)]);
        assert_eq!(resolve_sfx_time(&e, &tm, &words).unwrap(), 0.0);
        // Cut out entirely.
        let tm = TimeMap::from_segments(&[seg(&src, 0.0, 2.0)]);
        assert!(resolve_sfx_time(&e, &tm, &words).unwrap_err().contains("not in the clip"));
        // Fixed time.
        let f = SfxEntry { at: Some(3.5), at_word: None, ..e };
        assert_eq!(resolve_sfx_time(&f, &tm, &words).unwrap(), 3.5);
    }

    #[test]
    fn ids() {
        assert_eq!(next_id("sfx", [].into_iter()), "sfx_1");
        assert_eq!(next_id("sfx", ["sfx_2", "sfx_9", "fx_40"].into_iter()), "sfx_10");
    }
}
