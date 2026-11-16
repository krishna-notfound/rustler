//! Timeline v2 document <-> storage (plan Task 1.1).
//!
//! Storage stays `clips/<id>/clip.json` (frozen v1 `Clip`) + `clips/<id>/extras.json` (one key
//! per layer kind, owned by its command module). This module is the only place that turns the
//! two files into a [`Timeline`] ([`assemble`]) and back ([`disassemble`]); every key maps 1:1:
//!
//! | extras key | document |
//! |---|---|
//! | (clip.json segments) + `crop_paths`, `segment_speed` | `tracks.video[]` (`crop_path`, `speed`) |
//! | `freezes` | `tracks.freezes` |
//! | `cutaways` / `images` / `masks` | `tracks.cutaways` / `tracks.images` / `tracks.masks` |
//! | `overlays` | `tracks.text` |
//! | (clip.json captions) + `caption_word_overrides`, `speaker_colors` | `tracks.captions` |
//! | (clip.json audio) + `audio_fx`, `music`, `sfx` | `tracks.audio` |
//! | `effects` / `transitions` / `progress_bar` | same names under `tracks` |
//! | `meta` (`score`, `retention`, …), `post`, `variant_of`, `variant_info` | `meta` |
//! | `template_ref` | `template_ref` |
//! | anything else | `tracks.extra` (verbatim) |
//!
//! [`disassemble`] writes both files under the clip's revision check: it snapshots the extras of
//! the revision it supersedes to `history/<rev>.extras.json` (next to the clip snapshot
//! `ProjectStore::save_clip` writes), so `history undo/restore` brings back every layer.

use crate::audio_cmds::{AudioFx, FX_KEY, SFX_KEY, SfxEntry};
use crate::clip_cmds::load_transcript;
use crate::director_cmds::META_KEY;
use crate::extras;
use crate::fx_cmds::{EFFECTS_KEY, PROGRESS_BAR_KEY, TRANSITIONS_KEY};
use crate::speaker_cmds::{COLORS_KEY, SpeakerColors as StoredSpeakerColors};
use crate::variant_cmds::{VARIANT_INFO, VARIANT_OF};
use chrono::Utc;
use rustler_core::timeline::{
    Anchor, AudioTrack, CaptionsTrack, CropPoint, CutawayItem, DenoiseLevel, EffectItem, FreezeItem, ImageItem,
    ItemTiming, MaskItem, MusicItem, ProgressBarItem, SfxItem, SpeakerColors, TemplateRef, TextItem, TransitionItem,
    WordOverride,
};
use rustler_core::{
    Clip, ClipId, ErrorCode, ProjectStore, RustlerError, Segment, SegmentId, SourceId, TIMELINE_SCHEMA, TimeMap,
    Timeline, TimelineMeta, Tracks, VideoSeg, Word,
};
use rustler_media::compile::{
    Effect, Freeze, MusicBed, ProgressBar, TextOverlay, Transition, TransitionMap, check_transitions_on, ffmpeg_color,
};
use rustler_media::facepath::CropKey;
use rustler_media::layers::{
    self, CAPTION_WORDS_KEY, CUTAWAYS_KEY, CaptionWordOverride, Cutaway, IMAGES_KEY, ImageLayer, MASKS_KEY, MaskLayer,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

type Result<T> = rustler_core::Result<T>;

/// extras key: playback speed per segment id (0.5..2.0).
pub const SEGMENT_SPEED_KEY: &str = "segment_speed";
/// extras key: freeze frames.
pub const FREEZES_KEY: &str = "freezes";
/// extras key: the template the clip was styled from.
pub const TEMPLATE_REF_KEY: &str = "template_ref";
const MUSIC_KEY: &str = "music";
const CROP_PATHS_KEY: &str = "crop_paths";
const OVERLAYS_KEY: &str = crate::overlay_cmds::KEY;
const POST_KEY: &str = "post";
const STAGE: &str = "timeline";
/// End times may exceed the edit duration by this much (rounding).
pub const END_TOLERANCE: f64 = 0.05;

/// extras keys the document maps to typed fields (everything else goes to `tracks.extra`).
pub const KNOWN_KEYS: &[&str] = &[
    MUSIC_KEY,
    CROP_PATHS_KEY,
    SEGMENT_SPEED_KEY,
    FREEZES_KEY,
    CUTAWAYS_KEY,
    IMAGES_KEY,
    MASKS_KEY,
    OVERLAYS_KEY,
    CAPTION_WORDS_KEY,
    COLORS_KEY,
    FX_KEY,
    SFX_KEY,
    EFFECTS_KEY,
    TRANSITIONS_KEY,
    PROGRESS_BAR_KEY,
    META_KEY,
    POST_KEY,
    VARIANT_OF,
    VARIANT_INFO,
    TEMPLATE_REF_KEY,
];

/// Serializes assemble/disassemble cycles inside this process (GUI + agent in one server).
static LOCK: Mutex<()> = Mutex::new(());

fn verr(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg)
}

fn internal(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, STAGE, msg)
}

fn to_value<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| internal(format!("serialize: {e}")))
}

fn clip_rel(id: &ClipId) -> PathBuf {
    Path::new("clips").join(id.as_str()).join("clip.json")
}

fn extras_snapshot_rel(id: &ClipId, revision: u64) -> PathBuf {
    Path::new("clips").join(id.as_str()).join("history").join(format!("{revision}.extras.json"))
}

// ---------------------------------------------------------------------------------------------
// assemble
// ---------------------------------------------------------------------------------------------

/// The clip as one document plus warnings (stale references: transitions after a missing or
/// last segment, speeds / volumes of deleted segments).
pub fn assemble(store: &ProjectStore, clip: &ClipId) -> Result<(Timeline, Vec<String>)> {
    let c = store.load_clip(clip).map_err(|e| {
        if e.code == ErrorCode::NotFound {
            RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {clip} not found")).hint("run `rustler clip list` to see clip ids")
        } else {
            e
        }
    })?;
    let map = extras::load(store, clip)?;
    assemble_from(&c, &map)
}

/// Pure part of [`assemble`].
pub fn assemble_from(clip: &Clip, map: &Map<String, Value>) -> Result<(Timeline, Vec<String>)> {
    let mut warnings = Vec::new();
    let crop: BTreeMap<String, Vec<CropPoint>> = extras::from_map(map, CROP_PATHS_KEY)?.unwrap_or_default();
    let speed: BTreeMap<String, f64> = extras::from_map(map, SEGMENT_SPEED_KEY)?.unwrap_or_default();
    let ids: BTreeSet<&str> = clip.segments.iter().map(|s| s.id.as_str()).collect();
    for id in speed.keys().filter(|k| !ids.contains(k.as_str())) {
        warnings.push(format!("speed of deleted segment {id} is ignored (dropped on the next timeline apply)"));
    }
    let video = clip
        .segments
        .iter()
        .map(|s| VideoSeg {
            id: s.id.clone(),
            source_id: s.source_id.clone(),
            src_in: s.start,
            src_out: s.end,
            layout: s.layout.clone(),
            crop_path: crop.get(s.id.as_str()).cloned(),
            speed: speed.get(s.id.as_str()).copied(),
        })
        .collect();

    let fx: AudioFx = extras::from_map(map, FX_KEY)?.unwrap_or_default();
    for id in fx.segment_volumes.keys().filter(|k| !ids.contains(k.as_str())) {
        warnings.push(format!("volume of deleted segment {id} is ignored"));
    }
    let audio = AudioTrack {
        master: clip.audio,
        denoise: convert(&fx.denoise)?,
        volume_db: fx.volume_db,
        segment_volumes: fx.segment_volumes.clone(),
        music: extras::from_map::<MusicItem>(map, MUSIC_KEY)?,
        sfx: extras::from_map(map, SFX_KEY)?.unwrap_or_default(),
    };
    let captions = CaptionsTrack {
        settings: clip.captions.clone(),
        word_overrides: extras::from_map(map, CAPTION_WORDS_KEY)?.unwrap_or_default(),
        speaker_colors: extras::from_map::<SpeakerColors>(map, COLORS_KEY)?,
    };
    let transitions: Vec<TransitionItem> = extras::from_map(map, TRANSITIONS_KEY)?.unwrap_or_default();
    for t in &transitions {
        match clip.segments.iter().position(|s| s.id == t.after_segment_id) {
            None => warnings.push(format!("transition after {} is ignored: that segment no longer exists", t.after_segment_id)),
            Some(k) if k + 1 == clip.segments.len() => {
                warnings.push(format!("transition after {} is ignored: it is the last segment", t.after_segment_id))
            }
            _ => {}
        }
    }

    let mut meta = TimelineMeta::default();
    match map.get(META_KEY) {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            let mut m = m.clone();
            meta.score = m.remove("score").filter(|v| !v.is_null());
            meta.retention = m.remove("retention").filter(|v| !v.is_null());
            meta.other = m;
        }
        Some(_) => return Err(internal("extras.json key 'meta' is not an object").hint("fix or remove the key")),
    }
    meta.post = map.get(POST_KEY).filter(|v| !v.is_null()).cloned();
    meta.variant_of = map.get(VARIANT_OF).filter(|v| !v.is_null()).cloned();
    meta.variant_info = map.get(VARIANT_INFO).filter(|v| !v.is_null()).cloned();

    let extra: BTreeMap<String, Value> =
        map.iter().filter(|(k, _)| !KNOWN_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();

    let tracks = Tracks {
        video,
        freezes: extras::from_map(map, FREEZES_KEY)?.unwrap_or_default(),
        cutaways: extras::from_map(map, CUTAWAYS_KEY)?.unwrap_or_default(),
        images: extras::from_map(map, IMAGES_KEY)?.unwrap_or_default(),
        masks: extras::from_map(map, MASKS_KEY)?.unwrap_or_default(),
        text: extras::from_map(map, OVERLAYS_KEY)?.unwrap_or_default(),
        captions,
        audio,
        effects: extras::from_map(map, EFFECTS_KEY)?.unwrap_or_default(),
        transitions,
        progress_bar: extras::from_map::<ProgressBarItem>(map, PROGRESS_BAR_KEY)?,
        extra,
    };
    let t = Timeline {
        schema: TIMELINE_SCHEMA.to_string(),
        id: clip.id.clone(),
        name: clip.name.clone(),
        revision: clip.revision,
        canvas: clip.canvas,
        template_ref: extras::from_map::<TemplateRef>(map, TEMPLATE_REF_KEY)?,
        tracks,
        meta,
    };
    Ok((t, warnings))
}

/// Same-shape conversion through JSON (enums that exist in two crates).
fn convert<A: Serialize, B: DeserializeOwned>(a: &A) -> Result<B> {
    serde_json::from_value(to_value(a)?).map_err(|e| internal(format!("convert: {e}")))
}

// ---------------------------------------------------------------------------------------------
// disassemble
// ---------------------------------------------------------------------------------------------

/// The two files the document is stored as. `disk` is the clip on disk (keeps `schema`,
/// `created_at`, `renders`); `disk_map` the extras on disk (crop paths of segments that are not
/// in the document are kept, so undo can bring them back).
pub fn to_storage(t: &Timeline, disk: Option<&Clip>, disk_map: &Map<String, Value>) -> Result<(Clip, Map<String, Value>)> {
    let mut clip = match disk {
        Some(c) => c.clone(),
        None => Clip {
            schema: "rustler.clip@1".to_string(),
            id: t.id.clone(),
            name: t.name.clone(),
            revision: 0,
            canvas: t.canvas,
            segments: Vec::new(),
            captions: Default::default(),
            audio: Default::default(),
            renders: Vec::new(),
            created_at: Utc::now(),
        },
    };
    clip.id = t.id.clone();
    clip.name = t.name.clone();
    clip.canvas = t.canvas;
    clip.segments = t
        .tracks
        .video
        .iter()
        .map(|v| Segment { id: v.id.clone(), source_id: v.source_id.clone(), start: v.src_in, end: v.src_out, layout: v.layout.clone() })
        .collect();
    clip.captions = t.tracks.captions.settings.clone();
    clip.audio = t.tracks.audio.master;

    let mut m = Map::new();
    let put = |m: &mut Map<String, Value>, key: &str, v: Value| {
        m.insert(key.to_string(), v);
    };
    // Crop paths: the document's segments, plus paths of segments it does not have (undo).
    let doc_ids: BTreeSet<&str> = t.tracks.video.iter().map(|v| v.id.as_str()).collect();
    let mut crop = Map::new();
    if let Some(Value::Object(old)) = disk_map.get(CROP_PATHS_KEY) {
        for (k, v) in old.iter().filter(|(k, _)| !doc_ids.contains(k.as_str())) {
            crop.insert(k.clone(), v.clone());
        }
    }
    let mut speed = Map::new();
    for v in &t.tracks.video {
        if let Some(p) = &v.crop_path {
            crop.insert(v.id.as_str().to_string(), to_value(p)?);
        }
        if let Some(f) = v.speed
            && (f - 1.0).abs() > 1e-9
        {
            speed.insert(v.id.as_str().to_string(), to_value(&f)?);
        }
    }
    if !crop.is_empty() {
        put(&mut m, CROP_PATHS_KEY, Value::Object(crop));
    }
    if !speed.is_empty() {
        put(&mut m, SEGMENT_SPEED_KEY, Value::Object(speed));
    }
    let tr = &t.tracks;
    macro_rules! list {
        ($key:expr, $v:expr) => {
            if !$v.is_empty() {
                put(&mut m, $key, to_value(&$v)?);
            }
        };
    }
    list!(FREEZES_KEY, tr.freezes);
    list!(CUTAWAYS_KEY, tr.cutaways);
    list!(IMAGES_KEY, tr.images);
    list!(MASKS_KEY, tr.masks);
    list!(OVERLAYS_KEY, tr.text);
    list!(CAPTION_WORDS_KEY, tr.captions.word_overrides);
    list!(SFX_KEY, tr.audio.sfx);
    list!(EFFECTS_KEY, tr.effects);
    list!(TRANSITIONS_KEY, tr.transitions);
    if let Some(sc) = &tr.captions.speaker_colors {
        put(&mut m, COLORS_KEY, to_value(sc)?);
    }
    let a = &tr.audio;
    if a.denoise != DenoiseLevel::Off || a.volume_db != 0.0 || !a.segment_volumes.is_empty() {
        let fx = AudioFx { denoise: convert(&a.denoise)?, volume_db: a.volume_db, segment_volumes: a.segment_volumes.clone() };
        put(&mut m, FX_KEY, to_value(&fx)?);
    }
    if let Some(mu) = &a.music {
        put(&mut m, MUSIC_KEY, to_value(mu)?);
    }
    if let Some(pb) = &tr.progress_bar {
        put(&mut m, PROGRESS_BAR_KEY, to_value(pb)?);
    }
    if let Some(r) = &t.template_ref {
        put(&mut m, TEMPLATE_REF_KEY, to_value(r)?);
    }
    let mut meta = t.meta.other.clone();
    if let Some(s) = &t.meta.score {
        meta.insert("score".into(), s.clone());
    }
    if let Some(r) = &t.meta.retention {
        meta.insert("retention".into(), r.clone());
    }
    if !meta.is_empty() {
        put(&mut m, META_KEY, Value::Object(meta));
    }
    for (key, v) in [(POST_KEY, &t.meta.post), (VARIANT_OF, &t.meta.variant_of), (VARIANT_INFO, &t.meta.variant_info)] {
        if let Some(v) = v {
            put(&mut m, key, v.clone());
        }
    }
    for (k, v) in &tr.extra {
        if KNOWN_KEYS.contains(&k.as_str()) {
            return Err(verr(format!("/tracks/extra/{k}: '{k}' is a typed part of the document; edit it there, not in tracks.extra")));
        }
        m.insert(k.clone(), v.clone());
    }
    check_owner_types(&m)?;
    Ok((clip, m))
}

/// Every typed key must read back as the type its command module (and the renderer) uses.
fn check_owner_types(m: &Map<String, Value>) -> Result<()> {
    fn chk<T: DeserializeOwned>(m: &Map<String, Value>, key: &str) -> Result<()> {
        match m.get(key) {
            None => Ok(()),
            Some(v) => serde_json::from_value::<T>(v.clone())
                .map(|_| ())
                .map_err(|e| verr(format!("{key}: does not match what its commands store: {e}"))),
        }
    }
    chk::<BTreeMap<String, Vec<CropKey>>>(m, CROP_PATHS_KEY)?;
    chk::<HashMap<SegmentId, f64>>(m, SEGMENT_SPEED_KEY)?;
    chk::<Vec<Freeze>>(m, FREEZES_KEY)?;
    chk::<Vec<Cutaway>>(m, CUTAWAYS_KEY)?;
    chk::<Vec<ImageLayer>>(m, IMAGES_KEY)?;
    chk::<Vec<MaskLayer>>(m, MASKS_KEY)?;
    chk::<Vec<TextOverlay>>(m, OVERLAYS_KEY)?;
    chk::<Vec<CaptionWordOverride>>(m, CAPTION_WORDS_KEY)?;
    chk::<StoredSpeakerColors>(m, COLORS_KEY)?;
    chk::<AudioFx>(m, FX_KEY)?;
    chk::<MusicBed>(m, MUSIC_KEY)?;
    chk::<Vec<SfxEntry>>(m, SFX_KEY)?;
    chk::<Vec<Effect>>(m, EFFECTS_KEY)?;
    chk::<Vec<Transition>>(m, TRANSITIONS_KEY)?;
    chk::<ProgressBar>(m, PROGRESS_BAR_KEY)?;
    Ok(())
}

/// Writes the document as clip.json + extras.json. `RevisionConflict` unless the clip on disk is
/// at `base_revision` (a clip not on disk counts as 0: [`disassemble`] then creates it). The
/// extras of the superseded revision are snapshotted first; if saving the clip fails the extras
/// are put back. Returns the new revision.
pub fn disassemble(store: &ProjectStore, t: &Timeline, base_revision: u64) -> Result<u64> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let rel = clip_rel(&t.id);
    let disk: Option<Clip> = if store.resolve(&rel).is_file() { Some(store.read_json(&rel)?) } else { None };
    let on_disk = disk.as_ref().map_or(0, |c| c.revision);
    if on_disk != base_revision {
        return Err(RustlerError::new(
            ErrorCode::RevisionConflict,
            STAGE,
            format!("clip {} is at revision {on_disk} but base_revision was {base_revision}", t.id),
        )
        .hint(format!("someone changed the clip meanwhile: run `rustler timeline show {}` and re-apply on its revision", t.id)));
    }
    let disk_map = extras::load(store, &t.id)?;
    let (mut clip, map) = to_storage(t, disk.as_ref(), &disk_map)?;
    if disk.is_some() {
        snapshot_extras_map(store, &t.id, on_disk, &disk_map)?;
    }
    extras::save(store, &t.id, &map)?;
    if let Err(e) = store.save_clip(&mut clip, base_revision) {
        let _ = extras::save(store, &t.id, &disk_map);
        return Err(e);
    }
    Ok(clip.revision)
}

// ---------------------------------------------------------------------------------------------
// History of extras.json
// ---------------------------------------------------------------------------------------------

fn snapshot_extras_map(store: &ProjectStore, clip: &ClipId, revision: u64, map: &Map<String, Value>) -> Result<()> {
    store.write_json(&extras_snapshot_rel(clip, revision), map)
}

/// Saves the clip's extras as they are now as the layers of `revision` (call right before a
/// clip save supersedes `revision`).
pub fn snapshot_extras(store: &ProjectStore, clip: &ClipId, revision: u64) -> Result<()> {
    let map = extras::load(store, clip)?;
    snapshot_extras_map(store, clip, revision, &map)
}

/// Puts the extras snapshot of `revision` back; `false` when that revision has none (saved by
/// an older build or by a command that does not snapshot layers).
pub fn restore_extras(store: &ProjectStore, clip: &ClipId, revision: u64) -> Result<bool> {
    let rel = extras_snapshot_rel(clip, revision);
    if !store.resolve(&rel).is_file() {
        return Ok(false);
    }
    let map: Map<String, Value> = store.read_json(&rel)?;
    extras::save(store, clip, &map)?;
    Ok(true)
}

// ---------------------------------------------------------------------------------------------
// Timing: edit timeline -> rendered timeline, anchors
// ---------------------------------------------------------------------------------------------

/// Segment speeds and freezes stored in an extras map (for the render and the time map).
pub fn retime_of(map: &Map<String, Value>) -> Result<(HashMap<SegmentId, f64>, Vec<Freeze>)> {
    Ok((
        extras::from_map(map, SEGMENT_SPEED_KEY)?.unwrap_or_default(),
        extras::from_map(map, FREEZES_KEY)?.unwrap_or_default(),
    ))
}

/// Edit -> rendered map of a clip with these extras (transitions, speeds, freezes).
pub fn transition_map_from(clip: &Clip, map: &Map<String, Value>) -> Result<TransitionMap> {
    let ts: Vec<Transition> = extras::from_map(map, TRANSITIONS_KEY)?.unwrap_or_default();
    let (speeds, freezes) = retime_of(map)?;
    Ok(TransitionMap::with_retime(clip, &ts, &speeds, &freezes))
}

/// The document's storage form plus both timelines and the transcript words it anchors to.
pub struct DocEnv {
    pub clip: Clip,
    pub map: Map<String, Value>,
    pub tm: TimeMap,
    pub tmap: TransitionMap,
    pub words: HashMap<SourceId, Vec<Word>>,
}

impl DocEnv {
    pub fn new(store: &ProjectStore, t: &Timeline) -> Result<Self> {
        let rel = clip_rel(&t.id);
        let disk: Option<Clip> = if store.resolve(&rel).is_file() { Some(store.read_json(&rel)?) } else { None };
        let disk_map = if disk.is_some() { extras::load(store, &t.id)? } else { Map::new() };
        let (clip, map) = to_storage(t, disk.as_ref(), &disk_map)?;
        let tm = TimeMap::from_segments(&clip.segments);
        let tmap = transition_map_from(&clip, &map)?;
        let mut words = HashMap::new();
        let mut sources: Vec<SourceId> = clip.segments.iter().map(|s| s.source_id.clone()).collect();
        sources.extend(anchor_sources(t));
        for s in sources {
            if !words.contains_key(&s)
                && let Ok(Some(tr)) = load_transcript(store, &s)
            {
                words.insert(s, tr.words);
            }
        }
        Ok(Self { clip, map, tm, tmap, words })
    }

    pub fn edit_duration(&self) -> f64 {
        self.tm.duration()
    }

    /// Edit-timeline window of an item; `Err(reason)` when an anchor does not resolve.
    pub fn window(&self, t: &ItemTiming) -> std::result::Result<(f64, f64), String> {
        layers::resolve_timing(&media_timing(t), &self.tm, &self.words)
    }

    /// Edit-timeline span of a word (first occurrence in the clip).
    pub fn word(&self, a: &Anchor) -> std::result::Result<(f64, f64), String> {
        let m = layers::Anchor { source_id: a.source_id.clone(), word_i: a.word_i };
        layers::word_spans(&m, &self.tm, &self.words).map(|s| s[0])
    }

    /// Edit time of a sound effect.
    pub fn sfx_at(&self, s: &SfxItem) -> std::result::Result<f64, String> {
        match &s.at_word {
            Some(a) => self.word(&Anchor { source_id: a.source_id.clone(), word_i: a.word_index }).map(|w| w.0),
            None => s.at.ok_or_else(|| "sfx has neither at nor at_word".to_string()),
        }
    }

    pub fn word_text(&self, a: &Anchor) -> Option<String> {
        let ws = self.words.get(&a.source_id)?;
        ws.iter().find(|w| w.i == a.word_i).or_else(|| ws.get(a.word_i as usize)).map(|w| w.text.trim().to_string())
    }
}

fn media_timing(t: &ItemTiming) -> layers::Timing {
    let a = |x: &Option<Anchor>| x.as_ref().map(|a| layers::Anchor { source_id: a.source_id.clone(), word_i: a.word_i });
    layers::Timing { out_start: t.out_start, out_end: t.out_end, start_word: a(&t.start_word), end_word: a(&t.end_word) }
}

fn anchor_sources(t: &Timeline) -> Vec<SourceId> {
    let tr = &t.tracks;
    let mut out = Vec::new();
    let timings = tr
        .images
        .iter()
        .map(|x| &x.timing)
        .chain(tr.masks.iter().map(|x| &x.timing))
        .chain(tr.cutaways.iter().map(|x| &x.timing));
    for tm in timings {
        out.extend(tm.start_word.iter().chain(tm.end_word.iter()).map(|a| a.source_id.clone()));
    }
    out.extend(tr.captions.word_overrides.iter().map(|w| w.word_anchor.source_id.clone()));
    out.extend(tr.audio.sfx.iter().filter_map(|s| s.at_word.as_ref().map(|a| a.source_id.clone())));
    out
}

/// Placement of one segment.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SegTimes {
    pub id: SegmentId,
    pub out_start: f64,
    pub out_end: f64,
    pub rendered_start: f64,
    pub rendered_end: f64,
    pub speed: f64,
}

/// Where one item lands: edit timeline (`start`/`end`, what you write) and the rendered file.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct ItemTimes {
    /// images | masks | cutaways | text | effects | sfx | freezes | caption_word_overrides
    pub track: String,
    pub id: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub rendered_start: Option<f64>,
    pub rendered_end: Option<f64>,
    /// True when the timing comes from transcript words (follows re-cuts).
    pub anchored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why the item will not render (anchor cut out, outside the clip, inside a transition).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// Resolved times of the whole document.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Resolved {
    /// Sum of the segments' source lengths: the timeline every stored time is on.
    pub edit_duration: f64,
    /// Length of the rendered file (speeds, freezes and transitions applied).
    pub rendered_duration: f64,
    pub video: Vec<SegTimes>,
    pub items: Vec<ItemTimes>,
}

/// Every timed item with its edit and rendered times (word anchors resolved).
pub fn resolve(env: &DocEnv, t: &Timeline) -> Resolved {
    let tmap = &env.tmap;
    let dur = env.edit_duration();
    let video = env
        .tm
        .segments()
        .iter()
        .enumerate()
        .map(|(k, m)| SegTimes {
            id: m.segment_id.clone(),
            out_start: m.out_start,
            out_end: m.out_end,
            rendered_start: tmap.map_time(m.out_start),
            rendered_end: tmap.map_time(m.out_end),
            speed: tmap.speed(k),
        })
        .collect();
    let mut items = Vec::new();
    let mut span = |track: &str, id: &str, w: std::result::Result<(f64, f64), String>, anchored: bool, text: Option<String>| {
        let mut it = ItemTimes {
            track: track.into(),
            id: id.into(),
            start: None,
            end: None,
            rendered_start: None,
            rendered_end: None,
            anchored,
            text,
            problem: None,
        };
        match w {
            Ok((s, e)) => {
                it.start = Some(s);
                it.end = Some(e);
                if s >= dur {
                    it.problem = Some(format!("starts after the clip ends ({dur:.2}s)"));
                }
                match tmap.map_interval(s, e.min(dur.max(s))) {
                    Some((a, b)) => {
                        it.rendered_start = Some(a);
                        it.rendered_end = Some(b);
                    }
                    None if it.problem.is_none() => it.problem = Some("lies inside the hidden half of a transition".into()),
                    None => {}
                }
            }
            Err(p) => it.problem = Some(p),
        }
        items.push(it);
    };
    let tr = &t.tracks;
    for x in &tr.cutaways {
        span("cutaways", &x.id, env.window(&x.timing), x.timing.has_anchor(), None);
    }
    for x in &tr.images {
        span("images", &x.id, env.window(&x.timing), x.timing.has_anchor(), None);
    }
    for x in &tr.masks {
        span("masks", &x.id, env.window(&x.timing), x.timing.has_anchor(), None);
    }
    for x in &tr.text {
        span("text", &x.id, Ok((x.start, x.end)), false, Some(x.text.clone()));
    }
    for x in &tr.effects {
        span("effects", &x.id, Ok((x.start, x.end)), false, None);
    }
    for w in &tr.captions.word_overrides {
        let id = format!("{}:{}", w.word_anchor.source_id, w.word_anchor.word_i);
        span("caption_word_overrides", &id, env.word(&w.word_anchor), true, env.word_text(&w.word_anchor));
    }
    // Points: sfx and freezes.
    for s in &tr.audio.sfx {
        let mut it = ItemTimes {
            track: "sfx".into(),
            id: s.id.clone(),
            start: None,
            end: None,
            rendered_start: None,
            rendered_end: None,
            anchored: s.at_word.is_some(),
            text: None,
            problem: None,
        };
        match env.sfx_at(s) {
            Ok(at) => {
                it.start = Some(at);
                it.rendered_start = Some(tmap.map_time(at));
                if at >= dur {
                    it.problem = Some(format!("plays after the clip ends ({dur:.2}s)"));
                }
            }
            Err(p) => it.problem = Some(p),
        }
        items.push(it);
    }
    for f in &tr.freezes {
        let a = tmap.map_time(f.out_t);
        items.push(ItemTimes {
            track: "freezes".into(),
            id: f.id.clone(),
            start: Some(f.out_t),
            end: Some(f.out_t),
            rendered_start: Some(a),
            rendered_end: Some(a + f.duration),
            anchored: false,
            text: None,
            problem: None,
        });
    }
    Resolved { edit_duration: dur, rendered_duration: tmap.duration(), video, items }
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

/// Validates a document against the project (sources, files, transcripts). Errors are collected
/// with their JSON path (`/tracks/images/0/rect: ...`) and returned as one Validation error.
/// `current` is the document on disk: an anchor that already failed to resolve there (word cut
/// out by an earlier re-cut) is a warning, a new one an error. Returns warnings.
pub fn validate(store: &ProjectStore, t: &Timeline, current: Option<&Timeline>) -> Result<Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    let mut warns: Vec<String> = Vec::new();
    if t.schema != TIMELINE_SCHEMA {
        errs.push(format!("/schema: must be \"{TIMELINE_SCHEMA}\" (got \"{}\")", t.schema));
    }
    if t.name.trim().is_empty() {
        errs.push("/name: must not be empty".into());
    }
    let c = t.canvas;
    if c.width < 2 || c.height < 2 || !(c.fps.is_finite() && c.fps > 0.0 && c.fps <= 120.0) {
        errs.push(format!("/canvas: invalid {}x{} @ {} fps", c.width, c.height, c.fps));
    }
    let tr = &t.tracks;
    if tr.video.is_empty() {
        errs.push("/tracks/video: a clip needs at least one segment".into());
    }
    let mut seg_ids = BTreeSet::new();
    for (k, v) in tr.video.iter().enumerate() {
        let p = format!("/tracks/video/{k}");
        if !seg_ids.insert(v.id.as_str()) {
            errs.push(format!("{p}/id: duplicate segment id {}", v.id));
        }
        let src = match store.load_source(&v.source_id) {
            Ok(s) => Some(s),
            Err(_) => {
                errs.push(format!("{p}/source_id: source {} does not exist (`rustler source list`)", v.source_id));
                None
            }
        };
        if !(v.src_in.is_finite() && v.src_out.is_finite()) || v.src_in < 0.0 || v.src_out <= v.src_in + 0.01 {
            errs.push(format!("{p}: needs 0 <= src_in < src_out (got {} .. {})", v.src_in, v.src_out));
        } else if let Some(s) = &src {
            if v.src_out > s.probe.duration + END_TOLERANCE {
                errs.push(format!("{p}/src_out: {:.3} is past the end of source {} ({:.3}s)", v.src_out, s.id, s.probe.duration));
            }
            if !s.probe.has_video {
                errs.push(format!("{p}/source_id: source {} has no video", s.id));
            }
        }
        for (j, r) in v.layout.regions.iter().enumerate() {
            for (name, rect) in [("src", &r.src), ("dst", &r.dst)] {
                let ok = [rect.x, rect.y, rect.w, rect.h].iter().all(|x| x.is_finite()) && rect.w > 0.0 && rect.h > 0.0;
                if !ok {
                    errs.push(format!("{p}/layout/regions/{j}/{name}: rect needs finite values and w, h > 0"));
                }
            }
        }
        if let Some(f) = v.speed
            && !(f.is_finite() && (rustler_media::compile::MIN_SPEED..=rustler_media::compile::MAX_SPEED).contains(&f))
        {
            errs.push(format!("{p}/speed: {f} must be between 0.5 and 2.0"));
        }
        if let Some(cp) = &v.crop_path
            && cp.iter().any(|k| !(k.t.is_finite() && k.x.is_finite() && k.t >= -1e-6 && (0.0..=1.0).contains(&k.x)))
        {
            errs.push(format!("{p}/crop_path: keys need t >= 0 and x in 0..1"));
        }
    }
    if !errs.is_empty() {
        // Without valid segments the timings below cannot be checked.
        return Err(verr(errs.join("; ")).hint("fix the listed paths (`rustler timeline schema` documents every field)"));
    }

    let env = DocEnv::new(store, t)?;
    let dur = env.edit_duration();
    let old_env = match current {
        Some(cur) => DocEnv::new(store, cur).ok(),
        None => None,
    };
    let in_time = |p: &str, s: f64, e: f64, errs: &mut Vec<String>| {
        if !(s.is_finite() && e.is_finite()) || s < 0.0 || e <= s {
            errs.push(format!("{p}: needs 0 <= start < end (got {s} .. {e})"));
        } else if s >= dur {
            errs.push(format!("{p}: starts at {s:.2}s, after the clip ends ({dur:.2}s edit timeline)"));
        } else if e > dur + END_TOLERANCE {
            errs.push(format!("{p}: ends at {e:.2}s, after the clip ends ({dur:.2}s edit timeline)"));
        }
    };
    // Timing of a layer item: out times in range, anchors resolve (or already did not resolve).
    let timing = |p: &str, track: &str, id: &str, tm: &ItemTiming, errs: &mut Vec<String>, warns: &mut Vec<String>| {
        if let Some(e) = tm.out_end
            && tm.end_word.is_none()
            && e > dur + END_TOLERANCE
        {
            errs.push(format!("{p}/out_end: {e:.2}s is past the end of the clip ({dur:.2}s edit timeline)"));
            return;
        }
        if let Err(reason) = env.window(tm) {
            let was_broken = old_env.as_ref().is_some_and(|o| {
                let same = current.map(|c| item_timing(c, track, id)).unwrap_or(None);
                same.as_ref() == Some(tm) && o.window(tm).is_err()
            });
            if tm.has_anchor() && was_broken {
                warns.push(format!("{track} {id} will not render: {reason}"));
            } else {
                errs.push(format!("{p}: {reason}"));
            }
        }
    };
    let ids = |track: &str, list: Vec<&str>, errs: &mut Vec<String>| {
        let mut seen = BTreeSet::new();
        for (k, id) in list.into_iter().enumerate() {
            if id.trim().is_empty() {
                errs.push(format!("/tracks/{track}/{k}/id: must not be empty"));
            } else if !seen.insert(id.to_string()) {
                errs.push(format!("/tracks/{track}/{k}/id: duplicate id {id}"));
            }
        }
    };
    ids("freezes", tr.freezes.iter().map(|x| x.id.as_str()).collect(), &mut errs);
    ids("cutaways", tr.cutaways.iter().map(|x| x.id.as_str()).collect(), &mut errs);
    ids("images", tr.images.iter().map(|x| x.id.as_str()).collect(), &mut errs);
    ids("masks", tr.masks.iter().map(|x| x.id.as_str()).collect(), &mut errs);
    ids("text", tr.text.iter().map(|x| x.id.as_str()).collect(), &mut errs);
    ids("effects", tr.effects.iter().map(|x| x.id.as_str()).collect(), &mut errs);
    ids("audio/sfx", tr.audio.sfx.iter().map(|x| x.id.as_str()).collect(), &mut errs);

    for (k, f) in tr.freezes.iter().enumerate() {
        let p = format!("/tracks/freezes/{k}");
        if !(f.out_t.is_finite() && f.out_t >= 0.0 && f.out_t <= dur + END_TOLERANCE) {
            errs.push(format!("{p}/out_t: {} must be inside the clip (0 .. {dur:.2}s edit timeline)", f.out_t));
        }
        if !(f.duration.is_finite() && f.duration > 0.0 && f.duration <= rustler_media::compile::MAX_FREEZE) {
            errs.push(format!("{p}/duration: {} must be > 0 and <= 10 s", f.duration));
        }
    }
    for (k, x) in tr.images.iter().enumerate() {
        let p = format!("/tracks/images/{k}");
        match convert::<_, ImageLayer>(x) {
            Ok(im) => {
                if let Err(m) = layers::check_image(&im) {
                    errs.push(format!("{}: {m}", if m.starts_with("rect") { format!("{p}/rect") } else { p.clone() }));
                }
            }
            Err(e) => errs.push(format!("{p}: {}", e.message)),
        }
        if !x.file.is_file() {
            let what = x.asset.as_deref().map(|a| format!(" (asset {a})")).unwrap_or_default();
            errs.push(format!("{p}/file: image file{what} not found: {}", x.file.display()));
        }
        timing(&p, "images", &x.id, &x.timing, &mut errs, &mut warns);
    }
    for (k, x) in tr.masks.iter().enumerate() {
        let p = format!("/tracks/masks/{k}");
        match convert::<_, MaskLayer>(x) {
            Ok(m) => {
                if let Err(e) = layers::check_mask(&m) {
                    errs.push(format!("{}: {e}", if e.starts_with("rect") { format!("{p}/rect") } else { p.clone() }));
                }
            }
            Err(e) => errs.push(format!("{p}: {}", e.message)),
        }
        timing(&p, "masks", &x.id, &x.timing, &mut errs, &mut warns);
    }
    for (k, x) in tr.cutaways.iter().enumerate() {
        let p = format!("/tracks/cutaways/{k}");
        match convert::<_, Cutaway>(x) {
            Ok(c) => {
                if let Err(e) = layers::check_cutaway(&c) {
                    errs.push(format!("{}: {e}", if e.starts_with("rect") { format!("{p}/rect") } else { p.clone() }));
                }
            }
            Err(e) => errs.push(format!("{p}: {}", e.message)),
        }
        match (&x.media.source_id, &x.media.file) {
            (Some(sid), _) => {
                if store.load_source(sid).is_err() {
                    errs.push(format!("{p}/media/source_id: source {sid} does not exist"));
                }
            }
            (None, Some(f)) if !f.is_file() => {
                let what = x.media.asset.as_deref().map(|a| format!(" (asset {a})")).unwrap_or_default();
                errs.push(format!("{p}/media/file: cutaway media{what} not found: {}", f.display()));
            }
            (None, None) => errs.push(format!("{p}/media: needs source_id or file")),
            _ => {}
        }
        timing(&p, "cutaways", &x.id, &x.timing, &mut errs, &mut warns);
    }
    for (k, x) in tr.text.iter().enumerate() {
        let p = format!("/tracks/text/{k}");
        if let Err(e) = crate::overlay_cmds::check_text(STAGE, &x.text) {
            errs.push(format!("{p}/text: {}", e.message));
        }
        in_time(&p, x.start, x.end, &mut errs);
        if !(0.0..=1.0).contains(&x.position_y) {
            errs.push(format!("{p}/position_y: must be between 0 (top) and 1 (bottom)"));
        }
        if !(x.font_size_factor.is_finite() && (0.01..=0.2).contains(&x.font_size_factor)) {
            errs.push(format!("{p}/font_size_factor: must be between 0.01 and 0.2"));
        }
    }
    for (k, x) in tr.effects.iter().enumerate() {
        let p = format!("/tracks/effects/{k}");
        match convert::<_, Effect>(x) {
            Ok(e) => {
                if let Err(e) = crate::fx_cmds::check_effect(STAGE, &e, dur) {
                    errs.push(format!("{p}: {}", e.message));
                }
            }
            Err(e) => errs.push(format!("{p}: {}", e.message)),
        }
    }
    for (k, x) in tr.transitions.iter().enumerate() {
        let p = format!("/tracks/transitions/{k}");
        match tr.video.iter().position(|v| v.id == x.after_segment_id) {
            None => {
                let stale = current.is_some_and(|c| c.tracks.transitions.contains(x));
                let msg = format!("{p}/after_segment_id: segment {} is not in tracks.video", x.after_segment_id);
                if stale { warns.push(format!("{msg} (ignored)")) } else { errs.push(msg) }
            }
            Some(i) if i + 1 == tr.video.len() => warns.push(format!("{p}: after the last segment; ignored")),
            _ => {}
        }
        if !(x.duration.is_finite() && x.duration > 0.0 && x.duration <= 2.0) {
            errs.push(format!("{p}/duration: must be > 0 and <= 2 s"));
        }
    }
    if let Some(pb) = &tr.progress_bar
        && (ffmpeg_color(&pb.color).is_none() || !(2..=200).contains(&pb.height))
    {
        errs.push("/tracks/progress_bar: color must be #RRGGBB and height 2..200".into());
    }
    let cap = &tr.captions;
    if !(0.0..=1.0).contains(&cap.settings.position_y) {
        errs.push("/tracks/captions/position_y: must be between 0 and 1".into());
    }
    if !(1..=12).contains(&cap.settings.max_words_per_line) {
        errs.push("/tracks/captions/max_words_per_line: must be 1..12".into());
    }
    for (k, w) in cap.word_overrides.iter().enumerate() {
        let p = format!("/tracks/captions/word_overrides/{k}");
        if let Some(col) = &w.color
            && ffmpeg_color(col).is_none()
        {
            errs.push(format!("{p}/color: must be #RRGGBB (got '{col}')"));
        }
        if let Some(s) = w.scale
            && !(s.is_finite() && (0.5..=2.5).contains(&s))
        {
            errs.push(format!("{p}/scale: must be between 0.5 and 2.5"));
        }
        if let Err(reason) = env.word(&w.word_anchor) {
            let old = current.is_some_and(|c| c.tracks.captions.word_overrides.contains(w));
            if old || !reason.contains("does not exist") && !reason.contains("no transcript") {
                warns.push(format!("caption word {}:{} has no effect: {reason}", w.word_anchor.source_id, w.word_anchor.word_i));
            } else {
                errs.push(format!("{p}/word_anchor: {reason}"));
            }
        }
    }
    let a = &tr.audio;
    let m = &a.master;
    if !(m.target_lufs.is_finite() && m.true_peak.is_finite() && m.lra.is_finite()) {
        errs.push("/tracks/audio: loudness targets must be finite".into());
    }
    if !(a.volume_db.is_finite() && (-60.0..=24.0).contains(&a.volume_db)) {
        errs.push("/tracks/audio/volume_db: must be between -60 and 24".into());
    }
    for (id, db) in &a.segment_volumes {
        if !(db.is_finite() && (-60.0..=24.0).contains(db)) {
            errs.push(format!("/tracks/audio/segment_volumes/{id}: must be between -60 and 24"));
        }
        if !seg_ids.contains(id.as_str()) {
            warns.push(format!("audio volume of segment {id} is ignored: not in tracks.video"));
        }
    }
    if let Some(mu) = &a.music {
        if !mu.path.is_file() {
            errs.push(format!("/tracks/audio/music/path: music file not found: {}", mu.path.display()));
        }
        if !(mu.volume_db.is_finite() && mu.fade_in >= 0.0 && mu.fade_out >= 0.0) {
            errs.push("/tracks/audio/music: volume_db must be finite and fades >= 0".into());
        }
    }
    for (k, s) in a.sfx.iter().enumerate() {
        let p = format!("/tracks/audio/sfx/{k}");
        if !s.file.is_file() {
            errs.push(format!("{p}/file: sound file not found: {}", s.file.display()));
        }
        if !(s.volume_db.is_finite() && (-60.0..=12.0).contains(&s.volume_db)) {
            errs.push(format!("{p}/volume_db: must be between -60 and 12"));
        }
        match env.sfx_at(s) {
            Ok(at) if !(at >= 0.0 && at < dur) => errs.push(format!("{p}: plays at {at:.2}s, outside the clip (0 .. {dur:.2}s)")),
            Ok(_) => {}
            Err(reason) => {
                let old = current.is_some_and(|c| c.tracks.audio.sfx.contains(s));
                if old { warns.push(format!("sfx {} will not play: {reason}", s.id)) } else { errs.push(format!("{p}: {reason}")) }
            }
        }
    }
    if errs.is_empty() {
        // Transition overlaps against the (retimed) segment lengths.
        if let Err(e) = check_transitions_on(&env.clip, &env.tmap) {
            errs.push(format!("/tracks/transitions: {}", e.message));
        }
    }
    if !errs.is_empty() {
        return Err(verr(errs.join("; ")).hint("fix the listed paths (`rustler timeline schema` documents every field)"));
    }
    Ok(warns)
}

fn item_timing(t: &Timeline, track: &str, id: &str) -> Option<ItemTiming> {
    let tr = &t.tracks;
    match track {
        "images" => tr.images.iter().find(|x| x.id == id).map(|x| x.timing.clone()),
        "masks" => tr.masks.iter().find(|x| x.id == id).map(|x| x.timing.clone()),
        "cutaways" => tr.cutaways.iter().find(|x| x.id == id).map(|x| x.timing.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Human-readable diff
// ---------------------------------------------------------------------------------------------

/// What changed between two documents, one line per item ("text ovl_1: changed start, end").
pub fn diff(old: &Timeline, new: &Timeline) -> Vec<String> {
    let mut out = Vec::new();
    if old.name != new.name {
        out.push(format!("name: '{}' -> '{}'", old.name, new.name));
    }
    if old.canvas != new.canvas {
        out.push("canvas changed".into());
    }
    if old.template_ref != new.template_ref {
        out.push("template_ref changed".into());
    }
    let (o, n) = (to_value(&old.tracks).unwrap_or_default(), to_value(&new.tracks).unwrap_or_default());
    let keys: BTreeSet<&String> = o.as_object().into_iter().chain(n.as_object()).flat_map(|m| m.keys()).collect();
    for k in keys {
        let (a, b) = (o.get(k).unwrap_or(&Value::Null), n.get(k).unwrap_or(&Value::Null));
        if a == b {
            continue;
        }
        match (a, b) {
            (Value::Array(x), Value::Array(y)) => diff_list(k, x, y, &mut out),
            (Value::Object(x), Value::Object(y)) if k != "extra" => {
                for f in x.keys().chain(y.keys()).collect::<BTreeSet<_>>() {
                    let (p, q) = (x.get(f).unwrap_or(&Value::Null), y.get(f).unwrap_or(&Value::Null));
                    if p == q {
                        continue;
                    }
                    match (p, q) {
                        (Value::Array(pa), Value::Array(qa)) => diff_list(&format!("{k}.{f}"), pa, qa, &mut out),
                        _ => out.push(format!("{k}.{f} changed")),
                    }
                }
            }
            (Value::Object(x), Value::Object(y)) => {
                for f in x.keys().chain(y.keys()).collect::<BTreeSet<_>>() {
                    match (x.get(f), y.get(f)) {
                        (None, Some(_)) => out.push(format!("extra.{f}: added")),
                        (Some(_), None) => out.push(format!("extra.{f}: removed")),
                        (Some(p), Some(q)) if p != q => out.push(format!("extra.{f}: changed")),
                        _ => {}
                    }
                }
            }
            (Value::Null, _) => out.push(format!("{k}: added")),
            (_, Value::Null) => out.push(format!("{k}: removed")),
            _ => out.push(format!("{k} changed")),
        }
    }
    let (om, nm) = (to_value(&old.meta).unwrap_or_default(), to_value(&new.meta).unwrap_or_default());
    if om != nm {
        out.push("meta changed".into());
    }
    out
}

fn key_of(v: &Value) -> Option<String> {
    if let Some(id) = v.get("id").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    if let Some(s) = v.get("after_segment_id").and_then(Value::as_str) {
        return Some(format!("after {s}"));
    }
    v.get("word_anchor").map(|a| format!("{}:{}", a["source_id"].as_str().unwrap_or("?"), a["word_i"]))
}

fn diff_list(track: &str, old: &[Value], new: &[Value], out: &mut Vec<String>) {
    let ok = old.iter().chain(new).all(|v| key_of(v).is_some());
    if !ok {
        out.push(format!("{track} changed"));
        return;
    }
    let ok_keys: Vec<String> = old.iter().filter_map(key_of).collect();
    let nk_keys: Vec<String> = new.iter().filter_map(key_of).collect();
    for (k, v) in nk_keys.iter().zip(new) {
        match ok_keys.iter().position(|x| x == k) {
            None => out.push(format!("{track} {k}: added")),
            Some(i) => {
                let o = &old[i];
                if o != v {
                    let fields: BTreeSet<&String> = o.as_object().into_iter().chain(v.as_object()).flat_map(|m| m.keys()).collect();
                    let changed: Vec<&str> = fields.into_iter().filter(|f| o.get(f.as_str()) != v.get(f.as_str())).map(|f| f.as_str()).collect();
                    out.push(format!("{track} {k}: changed {}", changed.join(", ")));
                }
            }
        }
    }
    for k in ok_keys.iter().filter(|k| !nk_keys.contains(k)) {
        out.push(format!("{track} {k}: removed"));
    }
    let common_old: Vec<&String> = ok_keys.iter().filter(|k| nk_keys.contains(k)).collect();
    let common_new: Vec<&String> = nk_keys.iter().filter(|k| ok_keys.contains(k)).collect();
    if common_old != common_new {
        out.push(format!("{track}: order changed"));
    }
}

// ---------------------------------------------------------------------------------------------
// Re-cut helpers (insert, cold open, merge): segments with provenance, times that follow content
// ---------------------------------------------------------------------------------------------

/// A segment of a rebuilt timeline and the parts of the old timeline it shows:
/// (old segment id, source start, source end).
#[derive(Clone, Debug)]
pub struct Piece {
    pub seg: VideoSeg,
    pub origins: Vec<(SegmentId, f64, f64)>,
}

/// The part `[a, b)` (source seconds) of `old` as a segment; `keep_id` keeps the id, otherwise
/// a new one. The crop path is shifted to the new start and cut to the piece.
pub fn sub_segment(old: &VideoSeg, a: f64, b: f64, keep_id: bool) -> VideoSeg {
    let crop_path = old.crop_path.as_ref().map(|keys| {
        let shift = a - old.src_in;
        cut_points(&keys.iter().map(|k| CropPoint { t: k.t - shift, x: k.x }).collect::<Vec<_>>(), b - a)
    });
    VideoSeg {
        id: if keep_id { old.id.clone() } else { SegmentId::new() },
        source_id: old.source_id.clone(),
        src_in: a,
        src_out: b,
        layout: old.layout.clone(),
        crop_path,
        speed: old.speed,
    }
}

/// Keys inside `0..=len` with interpolated keys at both ends.
fn cut_points(keys: &[CropPoint], len: f64) -> Vec<CropPoint> {
    let mut keys: Vec<CropPoint> = keys.iter().copied().filter(|k| k.t.is_finite() && k.x.is_finite()).collect();
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    if keys.is_empty() {
        return keys;
    }
    let at = |t: f64| -> f64 {
        if t <= keys[0].t {
            return keys[0].x;
        }
        if t >= keys[keys.len() - 1].t {
            return keys[keys.len() - 1].x;
        }
        let k = keys.partition_point(|k| k.t <= t);
        let (a, b) = (keys[k - 1], keys[k]);
        let span = b.t - a.t;
        if span <= 0.0 { b.x } else { a.x + (b.x - a.x) * (t - a.t) / span }
    };
    let mut out = vec![CropPoint { t: 0.0, x: at(0.0) }];
    out.extend(keys.iter().copied().filter(|k| k.t > 1e-6 && k.t < len - 1e-6));
    if len > 1e-6 {
        out.push(CropPoint { t: len, x: at(len) });
    }
    out
}

/// Maps old edit-timeline times onto a rebuilt timeline, following the content.
pub struct Remap {
    /// Old segments: (id, source, src_in, out_start, out_end).
    old: Vec<(SegmentId, f64, f64, f64)>,
    /// New pieces: (origins, src_in, out_start, out_end).
    new: Vec<(Vec<(SegmentId, f64, f64)>, f64, f64, f64)>,
    new_duration: f64,
}

impl Remap {
    pub fn new(old: &[VideoSeg], pieces: &[Piece]) -> Self {
        let mut o = 0.0;
        let old = old
            .iter()
            .map(|s| {
                let len = s.src_out - s.src_in;
                let r = (s.id.clone(), s.src_in, o, o + len);
                o += len;
                r
            })
            .collect();
        let mut n = 0.0;
        let new = pieces
            .iter()
            .map(|p| {
                let len = p.seg.src_out - p.seg.src_in;
                let r = (p.origins.clone(), p.seg.src_in, n, n + len);
                n += len;
                r
            })
            .collect();
        Self { old, new, new_duration: n }
    }

    /// New time of old time `t`; `end` prefers the piece where `t` is an end (`(a, b]`).
    pub fn time(&self, t: f64, end: bool) -> Option<f64> {
        let total = self.old.last().map_or(0.0, |s| s.3);
        let k = self.old.iter().position(|s| if end { t > s.2 && t <= s.3 + 1e-9 } else { t >= s.2 && t < s.3 }).or_else(|| {
            // Exactly the clip end, or the start of an empty clip.
            if !self.old.is_empty() && (t - total).abs() < 1e-9 { Some(self.old.len() - 1) } else { None }
        })?;
        let (id, src_in, o0, _) = &self.old[k];
        let s = src_in + (t - o0);
        let hit = |strict_end: bool| {
            self.new.iter().find_map(|(origins, p_in, p0, _)| {
                origins
                    .iter()
                    .any(|(oid, a, b)| oid == id && if strict_end { s > *a - 1e-9 && s <= *b + 1e-9 } else { s >= *a - 1e-9 && s < *b - 1e-9 })
                    .then(|| p0 + (s - p_in))
            })
        };
        hit(end).or_else(|| hit(!end))
    }

    /// New interval of `[s, e]`; when its content was split apart the item keeps its length
    /// from the new start. `None` when the content is gone.
    pub fn interval(&self, s: f64, e: f64) -> Option<(f64, f64)> {
        let len = e - s;
        match (self.time(s, false), self.time(e, true)) {
            (Some(a), Some(b)) if b > a && ((b - a) - len).abs() < 0.05 => Some((a, b)),
            (Some(a), _) => Some((a, (a + len).min(self.new_duration))).filter(|(a, b)| b > a),
            (None, Some(b)) => Some(((b - len).max(0.0), b)).filter(|(a, b)| b > a),
            (None, None) => None,
        }
    }
}

/// Rebuilds `old` with new video `pieces`: time-based items follow their content (word-anchored
/// items keep their anchors), transitions and segment volumes follow their segment (onto its last
/// piece). Items whose content is gone are dropped with a warning.
pub fn rebuild(old: &Timeline, pieces: Vec<Piece>) -> (Timeline, Vec<String>) {
    let mut warnings = Vec::new();
    let r = Remap::new(&old.tracks.video, &pieces);
    let mut t = old.clone();
    let tr = &mut t.tracks;
    let gone = |what: &str, id: &str, w: &mut Vec<String>| {
        w.push(format!("{what} {id} removed: its part of the clip was cut out"));
    };
    let fix_timing = |tm: &mut ItemTiming| -> bool {
        match (tm.out_start, tm.out_end) {
            (Some(s), Some(e)) => match r.interval(s, e) {
                Some((a, b)) => {
                    tm.out_start = Some(a);
                    tm.out_end = Some(b);
                    true
                }
                None => tm.has_anchor(),
            },
            (Some(s), None) => match r.time(s, false) {
                Some(a) => {
                    tm.out_start = Some(a);
                    true
                }
                None => tm.has_anchor(),
            },
            (None, Some(e)) => match r.time(e, true) {
                Some(b) => {
                    tm.out_end = Some(b);
                    true
                }
                None => tm.has_anchor(),
            },
            (None, None) => true,
        }
    };
    tr.images.retain_mut(|x| fix_timing(&mut x.timing) || { gone("image", &x.id, &mut warnings); false });
    tr.masks.retain_mut(|x| fix_timing(&mut x.timing) || { gone("mask", &x.id, &mut warnings); false });
    tr.cutaways.retain_mut(|x| fix_timing(&mut x.timing) || { gone("cutaway", &x.id, &mut warnings); false });
    tr.text.retain_mut(|x| match r.interval(x.start, x.end) {
        Some((a, b)) => {
            x.start = a;
            x.end = b;
            true
        }
        None => {
            gone("text", &x.id, &mut warnings);
            false
        }
    });
    tr.effects.retain_mut(|x| match r.interval(x.start, x.end) {
        Some((a, b)) => {
            x.start = a;
            x.end = b;
            true
        }
        None => {
            gone("effect", &x.id, &mut warnings);
            false
        }
    });
    tr.audio.sfx.retain_mut(|x| match x.at {
        Some(at) if x.at_word.is_none() => match r.time(at, false) {
            Some(a) => {
                x.at = Some(a);
                true
            }
            None => {
                gone("sfx", &x.id, &mut warnings);
                false
            }
        },
        _ => true,
    });
    tr.freezes.retain_mut(|x| match r.time(x.out_t, false).or_else(|| r.time(x.out_t, true)) {
        Some(a) => {
            x.out_t = a;
            true
        }
        None => {
            gone("freeze", &x.id, &mut warnings);
            false
        }
    });
    // Segment-keyed data follows the old segment onto its last piece.
    let last_piece = |old_id: &SegmentId| -> Option<SegmentId> {
        pieces.iter().rev().find(|p| p.origins.iter().any(|o| &o.0 == old_id)).map(|p| p.seg.id.clone())
    };
    let new_ids: Vec<SegmentId> = pieces.iter().map(|p| p.seg.id.clone()).collect();
    tr.transitions = tr
        .transitions
        .iter()
        .filter_map(|x| {
            let to = last_piece(&x.after_segment_id)?;
            if new_ids.last() == Some(&to) {
                warnings.push(format!("transition after {} removed: its segment is now the last one", x.after_segment_id));
                return None;
            }
            Some(TransitionItem { after_segment_id: to, ..x.clone() })
        })
        .collect();
    let mut volumes = BTreeMap::new();
    for p in &pieces {
        if let Some(db) = p.origins.iter().find_map(|o| tr.audio.segment_volumes.get(o.0.as_str())) {
            volumes.insert(p.seg.id.as_str().to_string(), *db);
        }
    }
    tr.audio.segment_volumes = volumes;
    tr.video = pieces.into_iter().map(|p| p.seg).collect();
    (t, warnings)
}

/// Pieces of `old` unchanged (identity provenance).
pub fn identity_pieces(video: &[VideoSeg]) -> Vec<Piece> {
    video.iter().map(|v| Piece { seg: v.clone(), origins: vec![(v.id.clone(), v.src_in, v.src_out)] }).collect()
}

/// Next free `prefix_N` id among `ids`.
pub fn next_item_id<'a>(prefix: &str, ids: impl Iterator<Item = &'a str>) -> String {
    crate::audio_cmds::next_id(prefix, ids)
}

/// Every track item id of `t` with its track (for collision checks when merging).
pub fn item_ids(t: &Timeline) -> Vec<(&'static str, String)> {
    let tr = &t.tracks;
    let mut v: Vec<(&'static str, String)> = Vec::new();
    v.extend(tr.freezes.iter().map(|x| ("freezes", x.id.clone())));
    v.extend(tr.cutaways.iter().map(|x| ("cutaways", x.id.clone())));
    v.extend(tr.images.iter().map(|x| ("images", x.id.clone())));
    v.extend(tr.masks.iter().map(|x| ("masks", x.id.clone())));
    v.extend(tr.text.iter().map(|x| ("text", x.id.clone())));
    v.extend(tr.effects.iter().map(|x| ("effects", x.id.clone())));
    v.extend(tr.audio.sfx.iter().map(|x| ("sfx", x.id.clone())));
    v
}

// Keep the per-type imports referenced (used by tests and other modules through this path).
#[allow(dead_code)]
fn _types(_: &FreezeItem, _: &CutawayItem, _: &ImageItem, _: &MaskItem, _: &TextItem, _: &EffectItem, _: &WordOverride) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, a: f64, b: f64) -> VideoSeg {
        VideoSeg {
            id: format!("seg_{id}").parse().unwrap(),
            source_id: "src_a".parse().unwrap(),
            src_in: a,
            src_out: b,
            layout: rustler_core::Layout { preset: None, regions: vec![], background: rustler_core::Background::Black },
            crop_path: None,
            speed: None,
        }
    }

    #[test]
    fn remap_follows_content_through_a_cold_open() {
        // Old: A [0,10) ; hook = source [6,8) moved to the front.
        let a = seg("A", 0.0, 10.0);
        let hook = Piece { seg: sub_segment(&a, 6.0, 8.0, false), origins: vec![(a.id.clone(), 6.0, 8.0)] };
        let p1 = Piece { seg: sub_segment(&a, 0.0, 6.0, true), origins: vec![(a.id.clone(), 0.0, 6.0)] };
        let p2 = Piece { seg: sub_segment(&a, 8.0, 10.0, false), origins: vec![(a.id.clone(), 8.0, 10.0)] };
        let r = Remap::new(std::slice::from_ref(&a), &[hook, p1, p2]);
        assert_eq!(r.time(6.5, false), Some(0.5), "inside the hook -> front");
        assert_eq!(r.time(1.0, false), Some(3.0), "before the hook shifts by its length");
        assert_eq!(r.time(9.0, false), Some(9.0), "after the hook keeps its time");
        assert_eq!(r.interval(1.0, 2.0), Some((3.0, 4.0)));
        assert_eq!(r.time(10.0, true), Some(10.0));
    }

    #[test]
    fn crop_path_is_cut_and_shifted() {
        let mut a = seg("A", 10.0, 20.0);
        a.crop_path = Some(vec![CropPoint { t: 0.0, x: 0.1 }, CropPoint { t: 10.0, x: 0.3 }]);
        let b = sub_segment(&a, 15.0, 20.0, false);
        let cp = b.crop_path.unwrap();
        assert!((cp[0].x - 0.2).abs() < 1e-9 && cp[0].t == 0.0, "{cp:?}");
        assert!((cp.last().unwrap().t - 5.0).abs() < 1e-9);
    }
}
