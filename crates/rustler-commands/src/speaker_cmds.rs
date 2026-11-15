//! Commands: analyze.speakers, speaker.rename, caption.speaker_colors, crop.track_speaker.
//!
//! Who speaks when, for podcasts and interviews. Diarization runs locally in the Python engine
//! (sherpa-onnx + pyannote segmentation 3.0 + a WeSpeaker embedding model; see `engine/MODELS.md`),
//! nothing calls an AI API. Results:
//!
//! * `analysis/<source_id>/speakers.json`: the diarized turns (`S1`, `S2`, ...) and a `labels` map
//!   (`speaker.rename`). `transcript.json` words get a `speaker` field (max-overlap assignment);
//!   `transcript.raw.json` is never touched.
//! * `clips/<id>/extras.json` key `speaker_colors` (`caption.speaker_colors`): the renderer then
//!   writes speaker-coloured captions (`rustler_captions::write_ass_colored`).
//! * `crop.track_speaker` follows the ACTIVE speaker: the engine's `speakers.active` gives face
//!   boxes per sample (a person id that survives cuts, and a mouth-motion score); this module links
//!   speakers to people (close-ups, elimination, mouth movement on big faces) and builds the crop path.

use crate::analyze_cmds::spawn_engine;
use crate::clip_cmds::{load_transcript, tracking_crop_width, transcript_rel};
use crate::extras;
use crate::project_cmds::{Outcome, log_event, mmss, validation};
use crate::source_cmds::load_source_checked;
use rustler_captions::{CaptionWord, Preset, write_ass_colored_with, write_ass_with};
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, RustlerError, SegmentId, Source, SourceId, TimeMap, Word};
use rustler_media::compile::{CropKey, TransitionMap};
use rustler_media::facepath::{FaceBox, FaceSample, SmoothParams, crop_keys};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

pub const SPEAKERS_SCHEMA: &str = "rustler.speakers@1";
/// Key of `clips/<id>/extras.json` holding [`SpeakerColors`].
pub const COLORS_KEY: &str = "speaker_colors";

/// Words further than this from every turn stay unlabelled.
const NEAREST_TURN_SECONDS: f64 = 0.5;
/// Turns shorter than this do not move the crop (backchannels such as "mm-hm").
const MIN_TURN_SECONDS: f64 = 1.0;
/// The crop starts moving this long before the speaker's first word (a cut slightly early reads natural).
const SWITCH_LEAD_SECONDS: f64 = 0.2;
/// A lost face keeps its last box for this long.
const FACE_HOLD_SECONDS: f64 = 3.0;
/// A face counts for a turn only when it was visible in at least this share of the turn's samples.
const MIN_PRESENCE: f64 = 0.3;
/// Mouth motion of the best face must reach this (0..1 normalized per face) to be called speech.
const MIN_SPEAK: f64 = 0.3;
/// Best face must beat the runner-up by this much, else the choice is "ambiguous".
const MIN_MARGIN: f64 = 0.15;
/// Mouth movement is only believed on faces at least this wide (share of the frame width): on the
/// 4-6 % faces of a wide two-shot the lips are ~10 px and the "motion" is head and codec noise.
const MIN_MOTION_FACE_WIDTH: f64 = 0.07;
/// Face sampling rate for `speakers.active`.
const ACTIVE_FPS: f64 = 5.0;
/// A person needs this many close-up samples (only one face on screen) before a close-up can say
/// who they are.
const SOLO_MIN_SAMPLES: usize = 5;
/// Share of a person's close-up samples that must fall in one speaker's turns to link them.
const IDENTITY_SHARE: f64 = 0.6;
/// A person on screen for fewer samples than this is a false detection, not a candidate for elimination.
const MIN_PERSON_SAMPLES: usize = 10;
/// Within one shot a briefly lost face keeps its last box this long; a lost face in a new shot does not.
const SAME_SHOT_HOLD_SECONDS: f64 = 1.5;
/// Faces of two samples are "the same shot" when one person sits within this distance (frame widths).
const SHOT_MATCH_DX: f64 = 0.12;

/// Colours used for `S1, S2, ...` when the caller gives none (readable on dark video, distinct).
const PALETTE: [&str; 8] = ["#FFE000", "#5EE0FF", "#7CFF6B", "#FF7AC8", "#FFA040", "#B48CFF", "#FF5A5A", "#FFFFFF"];

fn rel(source: &SourceId) -> PathBuf {
    Path::new("analysis").join(source.as_str()).join("speakers.json")
}

fn round(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

// ---------------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------------

/// One diarized stretch of speech by one speaker, in source time.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct SpeakerTurn {
    pub start: f64,
    pub end: f64,
    /// `S1`, `S2`, ... in order of first appearance.
    pub speaker: String,
}

/// `analysis/<source_id>/speakers.json`
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SpeakersFile {
    pub schema: String,
    pub source_id: SourceId,
    /// `sherpa-onnx:...` or `heuristic`.
    pub model: String,
    pub num_speakers: u32,
    pub duration: f64,
    pub segments: Vec<SpeakerTurn>,
    /// Display names by speaker id (`speaker.rename`); missing = the id itself.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl SpeakersFile {
    pub fn name_of(&self, speaker: &str) -> String {
        self.labels.get(speaker).cloned().unwrap_or_else(|| speaker.to_string())
    }

    pub fn speaker_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.segments.iter().map(|s| s.speaker.clone()).collect();
        v.sort_by_key(|s| (speaker_number(s), s.clone()));
        v.dedup();
        v
    }
}

fn speaker_number(id: &str) -> u32 {
    id.trim_start_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(u32::MAX)
}

/// The colour a speaker gets when none is configured.
pub fn default_color(speaker: &str) -> &'static str {
    let n = speaker_number(speaker);
    if n == u32::MAX || n == 0 { PALETTE[0] } else { PALETTE[(n as usize - 1) % PALETTE.len()] }
}

/// Loads `speakers.json`; `None` when the source has not been diarized.
pub fn load_speakers(store: &ProjectStore, source: &SourceId) -> Result<Option<SpeakersFile>> {
    let p = rel(source);
    if !store.resolve(&p).is_file() {
        return Ok(None);
    }
    store.read_json(&p).map(Some)
}

fn require_speakers(store: &ProjectStore, source: &SourceId, stage: &str) -> Result<SpeakersFile> {
    load_speakers(store, source)?.ok_or_else(|| {
        validation(stage, format!("source {source} has no speaker analysis"), format!("run `rustler analyze speakers {source}` first"))
    })
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------------------------

/// Speaker with the largest overlap with `[a, b]`; ties go to the earlier turn. When nothing
/// overlaps, the nearest turn within [`NEAREST_TURN_SECONDS`] of the span's midpoint.
pub fn speaker_for_span(turns: &[SpeakerTurn], a: f64, b: f64) -> Option<String> {
    let mut best: Option<(f64, &SpeakerTurn)> = None;
    let mut by_speaker: BTreeMap<&str, f64> = BTreeMap::new();
    for t in turns {
        let ov = b.min(t.end) - a.max(t.start);
        if ov > 1e-6 {
            *by_speaker.entry(t.speaker.as_str()).or_default() += ov;
        }
    }
    for t in turns {
        if let Some(&ov) = by_speaker.get(t.speaker.as_str()) {
            if best.is_none_or(|(o, _)| ov > o + 1e-9) {
                best = Some((ov, t));
            }
        }
    }
    if let Some((_, t)) = best {
        return Some(t.speaker.clone());
    }
    let mid = (a + b) / 2.0;
    turns
        .iter()
        .map(|t| (if mid < t.start { t.start - mid } else if mid > t.end { mid - t.end } else { 0.0 }, t))
        .filter(|(d, _)| *d <= NEAREST_TURN_SECONDS)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, t)| t.speaker.clone())
}

/// Sets `Word::speaker` for every word (max-overlap assignment; `None` when no turn is near).
/// Returns how many words got a speaker.
pub fn assign_words(words: &mut [Word], turns: &[SpeakerTurn]) -> usize {
    let mut sorted: Vec<SpeakerTurn> = turns.to_vec();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut n = 0;
    for w in words.iter_mut() {
        w.speaker = speaker_for_span(&sorted, w.start, w.end.max(w.start + 0.01));
        if w.speaker.is_some() {
            n += 1;
        }
    }
    n
}

/// Consecutive turns of the same speaker merged (whatever the pause between them).
pub fn merge_turns(turns: &[SpeakerTurn]) -> Vec<SpeakerTurn> {
    let mut sorted: Vec<SpeakerTurn> = turns.iter().filter(|t| t.end > t.start).cloned().collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<SpeakerTurn> = Vec::new();
    for t in sorted {
        match out.last_mut() {
            Some(last) if last.speaker == t.speaker => last.end = last.end.max(t.end),
            _ => out.push(t),
        }
    }
    out
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SpeakerSummary {
    pub id: String,
    pub name: String,
    pub talk_seconds: f64,
    /// Share of all speech time (0..100).
    pub talk_pct: f64,
    /// Number of turns (a turn is a run of this speaker's segments without another speaker in between).
    pub turns: u32,
    /// Words labelled with this speaker in the transcript (0 without a transcript).
    pub words: u32,
}

/// Per-speaker talk time, turns and word counts.
pub fn summarize(file: &SpeakersFile, words: &[Word]) -> Vec<SpeakerSummary> {
    let total: f64 = file.segments.iter().map(|s| (s.end - s.start).max(0.0)).sum();
    let merged = merge_turns(&file.segments);
    file.speaker_ids()
        .into_iter()
        .map(|id| {
            let secs: f64 = file.segments.iter().filter(|s| s.speaker == id).map(|s| (s.end - s.start).max(0.0)).sum();
            SpeakerSummary {
                name: file.name_of(&id),
                talk_seconds: round(secs, 2),
                talk_pct: if total > 0.0 { round(secs / total * 100.0, 1) } else { 0.0 },
                turns: merged.iter().filter(|t| t.speaker == id).count() as u32,
                words: words.iter().filter(|w| w.speaker.as_deref() == Some(id.as_str())).count() as u32,
                id,
            }
        })
        .collect()
}

/// Stored in `extras.json` under [`COLORS_KEY`].
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct SpeakerColors {
    pub enabled: bool,
    /// `#RRGGBB` by speaker id.
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
}

/// Per caption word the colour of the speaker talking at that moment (`None` = no colour).
/// `turns` holds the diarized turns per source; words are mapped back to source time through the clip.
pub fn word_colors(
    clip: &Clip,
    words: &[CaptionWord],
    turns: &HashMap<SourceId, Vec<SpeakerTurn>>,
    colors: &BTreeMap<String, String>,
) -> Vec<Option<String>> {
    let tm = TimeMap::from_segments(&clip.segments);
    words
        .iter()
        .map(|w| {
            let mid = (w.out_start + w.out_end) / 2.0;
            let (sid, t) = tm.to_source(mid)?;
            let half = ((w.out_end - w.out_start) / 2.0).max(0.02);
            let speaker = speaker_for_span(turns.get(&sid)?, t - half, t + half)?;
            colors.get(&speaker).cloned()
        })
        .collect()
}

/// Speaker colour per caption word (words in the clip's edit timeline) when `caption.speaker_colors`
/// is enabled for the clip; `None` when it is off or no word matched a coloured speaker. Problems
/// that make the captions fall back to the preset colour are appended to `warnings`.
pub fn word_colors_for_clip(
    store: &ProjectStore,
    clip: &Clip,
    words: &[CaptionWord],
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<Option<String>>>> {
    let cfg: Option<SpeakerColors> = extras::get(store, &clip.id, COLORS_KEY)?;
    let Some(cfg) = cfg.filter(|c| c.enabled) else { return Ok(None) };
    let mut turns: HashMap<SourceId, Vec<SpeakerTurn>> = HashMap::new();
    for seg in &clip.segments {
        if turns.contains_key(&seg.source_id) {
            continue;
        }
        match load_speakers(store, &seg.source_id)? {
            Some(f) => {
                turns.insert(seg.source_id.clone(), f.segments);
            }
            None => warnings.push(format!(
                "speaker colours are on but source {} has no speaker analysis (run `rustler analyze speakers {}`); its captions keep the preset colour",
                seg.source_id, seg.source_id
            )),
        }
    }
    let colors = word_colors(clip, words, &turns, &cfg.colors);
    if colors.iter().all(Option::is_none) {
        if !turns.is_empty() {
            warnings.push("speaker colours are on but no caption word matched a coloured speaker; rendering with the preset colour".to_string());
        }
        return Ok(None);
    }
    Ok(Some(colors))
}

/// Moves caption words (and their colours) onto the rendered timeline through the transitions;
/// words hidden by a transition are dropped together with their colour.
pub fn map_words_with_colors(
    words: Vec<CaptionWord>,
    colors: Option<Vec<Option<String>>>,
    tmap: &TransitionMap,
) -> (Vec<CaptionWord>, Option<Vec<Option<String>>>) {
    if tmap.is_identity() {
        return (words, colors);
    }
    let Some(colors) = colors else { return (crate::fx_cmds::map_caption_words(words, tmap), None) };
    let mut out_words = Vec::with_capacity(words.len());
    let mut out_colors = Vec::with_capacity(words.len());
    for (w, c) in words.into_iter().zip(colors) {
        if let Some(m) = crate::fx_cmds::map_caption_words(vec![w], tmap).into_iter().next() {
            out_words.push(m);
            out_colors.push(c);
        }
    }
    (out_words, Some(out_colors))
}

/// ASS text for the captions: speaker-coloured when `colors` is given, else the plain preset look.
/// `custom` = custom caption presets (`asset_cmds::custom_presets`).
pub fn write_captions_ass(words: &[CaptionWord], colors: Option<&[Option<String>]>, clip: &Clip, font: &str, custom: &[Preset]) -> Result<String> {
    match colors {
        Some(c) => write_ass_colored_with(words, c, &clip.captions, &clip.canvas, font, custom),
        None => write_ass_with(words, &clip.captions, &clip.canvas, font, custom),
    }
}

/// [`word_colors_for_clip`] + [`write_captions_ass`] for words that are already on the edit
/// timeline and no custom presets (what the tests and simple callers need).
pub fn write_clip_ass(store: &ProjectStore, clip: &Clip, words: &[CaptionWord], font: &str, warnings: &mut Vec<String>) -> Result<String> {
    let colors = word_colors_for_clip(store, clip, words, warnings)?;
    write_captions_ass(words, colors.as_deref(), clip, font, &[])
}

// ---------------------------------------------------------------------------------------------
// Active-speaker face choice (pure)
// ---------------------------------------------------------------------------------------------

/// A face in one `speakers.active` sample (`id` is stable across samples, numbered left to right;
/// `speak` is the mouth-motion score 0..1 normalized within the face's own track).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ActiveFace {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub score: f64,
    pub id: u32,
    #[serde(default)]
    pub speak: f64,
    #[serde(default)]
    pub motion: f64,
    /// Positional track (one face in one shot); `id` is the person, linked across cuts by appearance.
    #[serde(default)]
    pub track: u32,
}

impl ActiveFace {
    fn area(&self) -> f64 {
        self.w * self.h
    }
    fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ActiveSample {
    pub t: f64,
    pub faces: Vec<ActiveFace>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ActiveTrack {
    pub id: u32,
    #[serde(default)]
    pub samples: u32,
    #[serde(default)]
    pub mean_x: f64,
    #[serde(default)]
    pub mean_area: f64,
    /// No mouth movement beyond noise (a photo, a poster, a person who never spoke).
    #[serde(default, rename = "static")]
    pub is_static: bool,
}

#[derive(Deserialize)]
struct ActiveResult {
    samples: Vec<ActiveSample>,
    #[serde(default)]
    tracks: Vec<ActiveTrack>,
}

/// The face picked for one turn.
#[derive(Serialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TurnChoice {
    pub speaker: String,
    /// Source time.
    pub start: f64,
    pub end: f64,
    /// `None`: no face could be chosen (the crop holds its position).
    pub face_id: Option<u32>,
    /// Horizontal centre of the chosen face during the turn (0..1).
    pub face_x: Option<f64>,
    /// Mean mouth-motion score of the chosen face during the turn.
    pub score: f64,
    /// Lead over the runner-up face.
    pub margin: f64,
    /// `only-face` (one person on screen), `face-id` (linked through close-ups), `elimination`
    /// (the only person left), `mouth-motion`, `speaker-map` (the face this speaker owns in other
    /// turns), `dominant` (ambiguous: the largest face), `short-turn`, `no-face`.
    pub reason: String,
}

#[derive(Clone, Copy, Default)]
struct FaceStat {
    n: usize,
    speak: f64,
    area: f64,
    cx: f64,
    w: f64,
}

fn samples_in(samples: &[ActiveSample], start: f64, end: f64) -> impl Iterator<Item = &ActiveSample> {
    samples.iter().filter(move |s| s.t >= start && s.t < end)
}

/// Per face id: sample count, summed mouth score / area / centre over `[start, end)`.
fn turn_stats(samples: &[ActiveSample], start: f64, end: f64) -> (usize, BTreeMap<u32, FaceStat>) {
    let mut n = 0;
    let mut m: BTreeMap<u32, FaceStat> = BTreeMap::new();
    for s in samples_in(samples, start, end) {
        n += 1;
        for f in &s.faces {
            let e = m.entry(f.id).or_default();
            e.n += 1;
            e.speak += f.speak;
            e.area += f.area();
            e.cx += f.cx();
            e.w += f.w;
        }
    }
    (n, m)
}

/// Faces seen in at least [`MIN_PRESENCE`] of a turn's samples, with mean score/area/centre.
fn present_faces(samples: &[ActiveSample], t: &SpeakerTurn) -> Vec<(u32, FaceStat)> {
    let (n, stats) = turn_stats(samples, t.start, t.end);
    stats
        .into_iter()
        .filter(|(_, s)| n > 0 && s.n as f64 / n as f64 >= MIN_PRESENCE)
        .map(|(id, s)| {
            let k = s.n as f64;
            (id, FaceStat { n: s.n, speak: s.speak / k, area: s.area / k, cx: s.cx / k, w: s.w / k })
        })
        .collect()
}

/// For each speaker the face whose mouth moves when (and only when) that speaker talks.
/// Contrast = mean mouth score during the speaker's turns minus during everyone else's turns.
fn speaker_face_map(samples: &[ActiveSample], turns: &[SpeakerTurn]) -> BTreeMap<String, u32> {
    let long: Vec<&SpeakerTurn> = turns.iter().filter(|t| t.end - t.start >= MIN_TURN_SECONDS).collect();
    let mean_over = |pred: &dyn Fn(&SpeakerTurn) -> bool| -> BTreeMap<u32, f64> {
        let mut acc: BTreeMap<u32, (f64, usize)> = BTreeMap::new();
        for t in long.iter().filter(|t| pred(t)) {
            for s in samples_in(samples, t.start, t.end) {
                for f in s.faces.iter().filter(|f| f.w >= MIN_MOTION_FACE_WIDTH) {
                    let e = acc.entry(f.id).or_default();
                    e.0 += f.speak;
                    e.1 += 1;
                }
            }
        }
        acc.into_iter().map(|(k, (s, n))| (k, s / n as f64)).collect()
    };
    let mut cands: Vec<(f64, String, u32, f64)> = Vec::new(); // (contrast, speaker, face, runner-up contrast gap)
    let mut speakers: Vec<&str> = long.iter().map(|t| t.speaker.as_str()).collect();
    speakers.sort();
    speakers.dedup();
    for sp in speakers {
        let own = mean_over(&|t| t.speaker == sp);
        let others = mean_over(&|t| t.speaker != sp);
        let mut contrast: Vec<(u32, f64)> = own.iter().map(|(id, m)| (*id, m - others.get(id).copied().unwrap_or(0.0))).collect();
        contrast.sort_by(|a, b| b.1.total_cmp(&a.1));
        if let Some(&(id, c)) = contrast.first() {
            let second = contrast.get(1).map_or(0.0, |x| x.1);
            if c >= 0.2 && c - second >= MIN_MARGIN {
                cands.push((c, sp.to_string(), id, c - second));
            }
        }
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut map = BTreeMap::new();
    let mut taken: Vec<u32> = Vec::new();
    for (_, sp, face, _) in cands {
        if !taken.contains(&face) {
            taken.push(face);
            map.insert(sp, face);
        }
    }
    map
}

/// Face id with the largest total area over all samples.
pub fn dominant_face(samples: &[ActiveSample]) -> Option<u32> {
    let mut area: BTreeMap<u32, f64> = BTreeMap::new();
    for s in samples {
        for f in &s.faces {
            *area.entry(f.id).or_default() += f.area();
        }
    }
    area.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(id, _)| id)
}

/// The one face of a close-up: the only face, or one at least 3x the area of the next (the rest
/// being false detections in the background).
fn solo_face(s: &ActiveSample) -> Option<&ActiveFace> {
    let mut by_area: Vec<&ActiveFace> = s.faces.iter().collect();
    by_area.sort_by(|a, b| b.area().total_cmp(&a.area()));
    match by_area.as_slice() {
        [] => None,
        [only] => Some(only),
        [first, second, ..] => (first.area() >= 3.0 * second.area()).then_some(*first),
    }
}

/// Who is who, from close-ups: a person whose close-ups fall (>= 60 %) in one speaker's turns is
/// that speaker. An edit cuts to whoever talks, so the face alone on screen is the speaker's.
/// Reaction shots of the listener dilute this, so the strongest link wins each speaker. This is what
/// ties a speaker to a face in wide shots where the faces are too small for mouth movement.
fn identity_map(samples: &[ActiveSample], turns: &[SpeakerTurn]) -> BTreeMap<String, u32> {
    let long: Vec<&SpeakerTurn> = turns.iter().filter(|t| t.end - t.start >= MIN_TURN_SECONDS).collect();
    let mut counts: BTreeMap<(u32, String), usize> = BTreeMap::new();
    let mut solo_total: BTreeMap<u32, usize> = BTreeMap::new();
    for t in &long {
        for s in samples_in(samples, t.start, t.end) {
            if let Some(f) = solo_face(s) {
                *counts.entry((f.id, t.speaker.clone())).or_default() += 1;
                *solo_total.entry(f.id).or_default() += 1;
            }
        }
    }
    let mut cands: Vec<(f64, u32, String)> = Vec::new(); // (weight, person, speaker)
    for (&person, &n) in &solo_total {
        if n < SOLO_MIN_SAMPLES {
            continue;
        }
        let best = counts.iter().filter(|((p, _), _)| *p == person).max_by_key(|(_, c)| **c);
        if let Some(((_, sp), &c)) = best {
            let share = c as f64 / n as f64;
            if share >= IDENTITY_SHARE {
                cands.push((c as f64 * share, person, sp.clone()));
            }
        }
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut map = BTreeMap::new();
    let mut taken: Vec<u32> = Vec::new();
    for (_, person, sp) in cands {
        if !map.contains_key(&sp) && !taken.contains(&person) {
            taken.push(person);
            map.insert(sp, person);
        }
    }
    map
}

/// How a speaker's face was found, strongest first.
#[derive(Clone, Copy, PartialEq, Debug)]
enum OwnerSource {
    /// Linked through close-ups ([`identity_map`]).
    Identity,
    /// The only unclaimed person left for the only unclaimed speaker.
    Elimination,
    /// Mouth movement in the speaker's turns ([`speaker_face_map`]).
    Motion,
}

/// Speaker -> person: close-up identity first, then mouth movement, then elimination.
fn owner_map(samples: &[ActiveSample], turns: &[SpeakerTurn]) -> BTreeMap<String, (u32, OwnerSource)> {
    let mut owner: BTreeMap<String, (u32, OwnerSource)> = BTreeMap::new();
    for (sp, f) in identity_map(samples, turns) {
        owner.insert(sp, (f, OwnerSource::Identity));
    }
    for (sp, f) in speaker_face_map(samples, turns) {
        if !owner.contains_key(&sp) && !owner.values().any(|(p, _)| *p == f) {
            owner.insert(sp, (f, OwnerSource::Motion));
        }
    }
    let mut speakers: Vec<&str> = turns.iter().filter(|t| t.end - t.start >= MIN_TURN_SECONDS).map(|t| t.speaker.as_str()).collect();
    speakers.sort();
    speakers.dedup();
    let unmapped: Vec<&str> = speakers.into_iter().filter(|s| !owner.contains_key(*s)).collect();
    let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
    for s in samples {
        for f in &s.faces {
            *seen.entry(f.id).or_default() += 1;
        }
    }
    let free: Vec<u32> = seen.into_iter().filter(|(id, n)| *n >= MIN_PERSON_SAMPLES && !owner.values().any(|(p, _)| p == id)).map(|(id, _)| id).collect();
    if let ([sp], [person]) = (unmapped.as_slice(), free.as_slice()) {
        owner.insert((*sp).to_string(), (*person, OwnerSource::Elimination));
    }
    owner
}

/// Picks the active face for every turn. `turns` must be merged (see [`merge_turns`]) and sorted.
///
/// Order of evidence for a turn with several faces on screen: the person the speaker was linked to
/// through close-ups (`face-id`) or by elimination (`elimination`); a clear winner by mouth movement
/// (score >= 0.3 and a lead of 0.15; `mouth-motion`); the face that moves when this speaker talks in
/// other turns (`speaker-map`); the dominant (largest) face, reported as ambiguous (`dominant`).
pub fn choose_faces(samples: &[ActiveSample], turns: &[SpeakerTurn]) -> Vec<TurnChoice> {
    let owner = owner_map(samples, turns);
    let dominant = dominant_face(samples);
    let mut out: Vec<TurnChoice> = Vec::new();
    for t in turns {
        let mut c = TurnChoice {
            speaker: t.speaker.clone(),
            start: round(t.start, 3),
            end: round(t.end, 3),
            face_id: None,
            face_x: None,
            score: 0.0,
            margin: 0.0,
            reason: "no-face".to_string(),
        };
        if t.end - t.start < MIN_TURN_SECONDS {
            c.reason = "short-turn".to_string();
            out.push(c);
            continue;
        }
        let faces = present_faces(samples, t);
        if faces.is_empty() {
            out.push(c);
            continue;
        }
        let mut ranked = faces.clone();
        ranked.sort_by(|a, b| b.1.speak.total_cmp(&a.1.speak));
        let (best_id, best) = ranked[0];
        let second = ranked.get(1).map_or(0.0, |r| r.1.speak);
        c.margin = round(best.speak - second, 3);
        let mine = owner.get(&t.speaker).filter(|(f, _)| faces.iter().any(|x| x.0 == *f)).copied();
        let pick: (u32, &str) = if faces.len() == 1 {
            (best_id, "only-face")
        } else if let Some((f, src)) = mine.filter(|(_, src)| *src != OwnerSource::Motion) {
            (f, if src == OwnerSource::Identity { "face-id" } else { "elimination" })
        } else if best.w >= MIN_MOTION_FACE_WIDTH && best.speak >= MIN_SPEAK && best.speak - second >= MIN_MARGIN {
            (best_id, "mouth-motion")
        } else if let Some((f, _)) = mine {
            (f, "speaker-map")
        } else {
            let dom = faces
                .iter()
                .max_by(|a, b| a.1.area.total_cmp(&b.1.area))
                .map(|f| f.0)
                .or(dominant)
                .unwrap_or(best_id);
            (dom, "dominant")
        };
        let stat = faces.iter().find(|f| f.0 == pick.0).map(|f| f.1).unwrap_or_default();
        c.face_id = Some(pick.0);
        c.face_x = Some(round(stat.cx, 4));
        c.score = round(stat.speak, 3);
        c.reason = pick.1.to_string();
        out.push(c);
    }
    // Short turns follow the previous decided face (or the next one at the very start).
    let mut last: Option<(u32, f64)> = None;
    for c in out.iter_mut() {
        match c.face_id {
            Some(id) => last = Some((id, c.face_x.unwrap_or(0.5))),
            None if c.reason == "short-turn" => {
                if let Some((id, x)) = last {
                    c.face_id = Some(id);
                    c.face_x = Some(x);
                }
            }
            None => {}
        }
    }
    let first_decided = out.iter().find(|c| c.face_id.is_some()).map(|c| (c.face_id, c.face_x));
    for c in out.iter_mut().take_while(|c| c.face_id.is_none() && c.reason == "short-turn") {
        if let Some((id, x)) = first_decided {
            c.face_id = id;
            c.face_x = x;
        }
    }
    out
}

/// True when `now` looks like the same camera shot as `prev`: some person is on screen in both at
/// nearly the same place and size. A cut to a close-up or to the other camera breaks this.
fn same_shot(prev: &ActiveSample, now: &ActiveSample) -> bool {
    if prev.faces.is_empty() || now.faces.is_empty() {
        return true; // nothing to compare: a dropout is not a cut
    }
    now.faces.iter().any(|a| {
        prev.faces.iter().any(|b| a.id == b.id && (a.cx() - b.cx()).abs() <= SHOT_MATCH_DX && a.w.max(b.w) <= 1.5 * a.w.min(b.w).max(1e-6))
    })
}

/// One face per sample: the active speaker's face at that time (the crop path then follows it).
/// The switch to a turn's face happens [`SWITCH_LEAD_SECONDS`] early; turns without a decided face
/// and gaps between turns keep the previous face. When the chosen person is not on screen:
/// * within the same shot (a detection dropout) their last box is kept for [`SAME_SHOT_HOLD_SECONDS`];
/// * after a cut (a reaction shot, the other camera) the largest face that *is* on screen is
///   followed, because a crop position from the old shot means nothing in the new one;
/// * with no face at all (a graphic) the last box is kept for [`FACE_HOLD_SECONDS`].
/// Without any decision the dominant face is followed.
pub fn speaker_face_samples(samples: &[ActiveSample], choices: &[TurnChoice]) -> Vec<FaceSample> {
    let fallback = dominant_face(samples);
    // person -> (time, box, shot number)
    let mut last_box: BTreeMap<u32, (f64, FaceBox, u32)> = BTreeMap::new();
    let mut current: Option<u32> = choices.iter().find_map(|c| c.face_id).or(fallback);
    let mut shot = 0u32;
    let mut prev_faces: Option<&ActiveSample> = None;
    let mut out = Vec::with_capacity(samples.len());
    for s in samples {
        if let Some(prev) = prev_faces {
            if !same_shot(prev, s) {
                shot += 1;
            }
        }
        if !s.faces.is_empty() {
            prev_faces = Some(s);
        }
        if let Some(c) = choices.iter().rev().find(|c| c.start - SWITCH_LEAD_SECONDS <= s.t) {
            if let Some(id) = c.face_id {
                current = Some(id);
            }
        }
        for f in &s.faces {
            last_box.insert(f.id, (s.t, FaceBox { x: f.x, y: f.y, w: f.w, h: f.h, score: f.score }, shot));
        }
        let target = current.and_then(|id| last_box.get(&id));
        let faces = match target {
            Some((t, b, _)) if *t == s.t => vec![*b],
            _ if !s.faces.is_empty() => match target {
                Some((t, b, sh)) if *sh == shot && s.t - t <= SAME_SHOT_HOLD_SECONDS => vec![*b],
                _ => s
                    .faces
                    .iter()
                    .max_by(|a, b| a.area().total_cmp(&b.area()))
                    .map(|f| vec![FaceBox { x: f.x, y: f.y, w: f.w, h: f.h, score: f.score }])
                    .unwrap_or_default(),
            },
            Some((t, b, _)) if s.t - t <= FACE_HOLD_SECONDS => vec![*b],
            _ => Vec::new(),
        };
        out.push(FaceSample { t: s.t, faces });
    }
    out
}

/// Smoothing for speaker-following crops: quicker than `crop.track_face` so a change of speaker
/// is a short pan, not a drift.
pub fn speaker_smooth() -> SmoothParams {
    SmoothParams { median_window: 3, min_cutoff: 2.0, beta: 0.05, dead_zone: 0.02, max_speed: 1.6, hold_keys: true }
}

// ---------------------------------------------------------------------------------------------
// analyze.speakers
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SpeakersIn {
    pub source_id: SourceId,
    /// Exact number of speakers when known; otherwise it is estimated.
    pub num_speakers: Option<u32>,
    /// Lower bound for the estimate.
    pub min_speakers: Option<u32>,
    /// Upper bound for the estimate.
    pub max_speakers: Option<u32>,
}

#[derive(Deserialize, Debug)]
struct DiarizeResult {
    segments: Vec<SpeakerTurn>,
    #[serde(default)]
    duration: f64,
    model: String,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct TurnPreview {
    pub speaker: String,
    pub start: f64,
    pub end: f64,
    pub mmss: String,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SpeakersOut {
    pub source_id: SourceId,
    pub model: String,
    pub num_speakers: u32,
    pub duration: f64,
    pub speakers: Vec<SpeakerSummary>,
    /// Total turns (speaker changes + 1).
    pub turns: u32,
    /// The first turns, merged per speaker.
    pub timeline: Vec<TurnPreview>,
    /// Transcript words that got a `speaker` (0 when the source has no transcript yet).
    pub words_labelled: u32,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Writes `speaker` into `analysis/<id>/transcript.json` (never `transcript.raw.json`).
/// Returns `(labelled, total)` words, `None` when the source has no transcript.
pub fn label_transcript(store: &ProjectStore, source: &SourceId, turns: &[SpeakerTurn]) -> Result<Option<(usize, usize)>> {
    let Some(mut tr) = load_transcript(store, source)? else { return Ok(None) };
    let n = assign_words(&mut tr.words, turns);
    store.write_json(&transcript_rel(source), &tr)?;
    Ok(Some((n, tr.words.len())))
}

fn timeline(file: &SpeakersFile) -> Vec<TurnPreview> {
    merge_turns(&file.segments)
        .into_iter()
        .take(12)
        .map(|t| TurnPreview { mmss: mmss(t.start), speaker: file.name_of(&t.speaker), start: round(t.start, 2), end: round(t.end, 2) })
        .collect()
}

fn audio_of(store: &ProjectStore, source: &Source) -> Result<PathBuf> {
    source.audio_wav.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()).ok_or_else(|| {
        validation(
            "analyze.speakers",
            format!("source {} has no audio track (or its 16 kHz wav is missing)", source.id),
            "diarization needs speech audio; re-import the source if the wav was deleted",
        )
    })
}

pub async fn analyze_speakers(ctx: Ctx, input: SpeakersIn) -> Result<Outcome<SpeakersOut>> {
    let stage = "analyze.speakers";
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if !source.probe.has_audio {
        return Err(validation(stage, format!("source {} has no audio track", source.id), "nothing to diarize in a silent video"));
    }
    for (name, v) in [("num_speakers", input.num_speakers), ("min_speakers", input.min_speakers), ("max_speakers", input.max_speakers)] {
        if v.is_some_and(|n| !(1..=32).contains(&n)) {
            return Err(validation(stage, format!("{name} must be between 1 and 32"), "omit it to estimate the number of speakers"));
        }
    }
    if let (Some(lo), Some(hi)) = (input.min_speakers, input.max_speakers) {
        if lo > hi {
            return Err(validation(stage, "min_speakers is greater than max_speakers", "swap them"));
        }
    }
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"speakers": source.id, "num_speakers": input.num_speakers})));
    }
    let wav = audio_of(&store, &source)?;
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let params = json!({
        "audio_path": wav, "num_speakers": input.num_speakers,
        "min_speakers": input.min_speakers, "max_speakers": input.max_speakers,
    });
    let res: Result<DiarizeResult> = engine.call("speakers.diarize", params, &ctx.progress, &ctx.cancel).await;
    engine.shutdown().await;
    let res = res?;

    let mut warnings = res.warnings;
    let old = load_speakers(&store, &source.id)?;
    if old.as_ref().is_some_and(|o| !o.labels.is_empty()) {
        warnings.push("speaker names from an earlier run were dropped (speaker ids can change between runs); rename again with `rustler speaker rename`".to_string());
    }
    let mut file = SpeakersFile {
        schema: SPEAKERS_SCHEMA.to_string(),
        source_id: source.id.clone(),
        model: res.model,
        num_speakers: 0,
        duration: if res.duration > 0.0 { res.duration } else { source.probe.duration },
        segments: res.segments,
        labels: BTreeMap::new(),
    };
    file.num_speakers = file.speaker_ids().len() as u32;
    if file.model == "heuristic" {
        warnings.push("heuristic diarizer (no sherpa-onnx models available): speaker changes are approximate".to_string());
    }
    if file.segments.is_empty() {
        warnings.push("no speech found in the audio".to_string());
    }
    let path = rel(&source.id);
    store.write_json(&path, &file)?;

    let id = source.id.to_string();
    let labelled = label_transcript(&store, &source.id, &file.segments)?;
    let words = load_transcript(&store, &source.id)?.map(|t| t.words).unwrap_or_default();
    let mut next = Vec::new();
    match labelled {
        None => {
            warnings.push(format!("source {id} has no transcript yet; run `rustler analyze transcribe {id}` then this command again to label words"));
            next.push(format!("rustler analyze transcribe {id}"));
        }
        Some((n, total)) if n < total => warnings.push(format!("{} of {total} words are further than {NEAREST_TURN_SECONDS} s from any speaker turn and stay unlabelled", total - n)),
        Some(_) => {}
    }
    if file.num_speakers > 1 {
        next.push(format!("rustler speaker rename {id} S1 \"Host\""));
        next.push("rustler caption speaker-colors <clip_id> --enabled true".to_string());
        next.push("rustler crop track-speaker <clip_id>   (center-crop clips: follow the active speaker)".to_string());
    } else if file.num_speakers == 1 {
        warnings.push("only one speaker found; if the recording has more, pass --num-speakers".to_string());
    }
    let out = SpeakersOut {
        source_id: source.id.clone(),
        model: file.model.clone(),
        num_speakers: file.num_speakers,
        duration: round(file.duration, 2),
        speakers: summarize(&file, &words),
        turns: merge_turns(&file.segments).len() as u32,
        timeline: timeline(&file),
        words_labelled: labelled.map_or(0, |l| l.0 as u32),
        path,
        warnings,
        next,
    };
    log_event(&ctx, &store, "analyze.speakers", json!({"source_id": id, "model": out.model, "speakers": out.num_speakers, "turns": out.turns}))?;
    Ok(Outcome::Done(out))
}

// ---------------------------------------------------------------------------------------------
// speaker.rename
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct RenameIn {
    pub source_id: SourceId,
    /// Speaker id such as `S1` (see `analyze speakers`).
    pub speaker: String,
    /// Display name; empty resets to the id.
    pub name: String,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct RenameOut {
    pub source_id: SourceId,
    pub speaker: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

pub async fn speaker_rename(ctx: Ctx, i: RenameIn) -> Result<RenameOut> {
    let stage = "speaker.rename";
    let store = ctx.store()?;
    let mut file = require_speakers(&store, &i.source_id, stage)?;
    let ids = file.speaker_ids();
    let speaker = i.speaker.trim().to_uppercase();
    if !ids.contains(&speaker) {
        return Err(validation(stage, format!("no speaker '{}' in source {} (found: {})", i.speaker, i.source_id, ids.join(", ")), "use one of the listed ids"));
    }
    let name = i.name.trim().to_string();
    if name.chars().count() > 40 {
        return Err(validation(stage, "name is longer than 40 characters", "use a short display name"));
    }
    if name.is_empty() || name == speaker {
        file.labels.remove(&speaker);
    } else {
        file.labels.insert(speaker.clone(), name.clone());
    }
    if ctx.dry_run {
        return Ok(RenameOut { source_id: i.source_id, speaker, name, labels: file.labels, dry_run: Some(true) });
    }
    store.write_json(&rel(&i.source_id), &file)?;
    log_event(&ctx, &store, "speaker.rename", json!({"source_id": i.source_id, "speaker": speaker, "name": name}))?;
    let shown = file.name_of(&speaker);
    Ok(RenameOut { source_id: i.source_id, speaker, name: shown, labels: file.labels, dry_run: None })
}

// ---------------------------------------------------------------------------------------------
// caption.speaker_colors
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SpeakerColorsIn {
    pub clip_id: ClipId,
    pub enabled: bool,
    /// `#RRGGBB` by speaker id, e.g. {"S1": "#FFE000", "S2": "#5EE0FF"}. Speakers you leave out
    /// keep their current colour or get a default one.
    pub colors: Option<BTreeMap<String, String>>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SpeakerColorsOut {
    pub clip_id: ClipId,
    pub enabled: bool,
    pub colors: BTreeMap<String, String>,
    /// Speakers found in the clip's sources.
    pub speakers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn norm_hex(stage: &str, speaker: &str, c: &str) -> Result<String> {
    let v = rustler_captions::parse_hex_colour(c)
        .ok_or_else(|| validation(stage, format!("colour '{c}' for {speaker} is not #RRGGBB"), "use a hex colour such as #FFE000"))?;
    let (b, g, r) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    Ok(format!("#{r:02X}{g:02X}{b:02X}"))
}

pub async fn caption_speaker_colors(ctx: Ctx, i: SpeakerColorsIn) -> Result<SpeakerColorsOut> {
    let stage = "caption.speaker_colors";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let mut found: Vec<String> = Vec::new();
    let mut missing: Vec<SourceId> = Vec::new();
    let mut seen: Vec<SourceId> = Vec::new();
    for seg in &clip.segments {
        if seen.contains(&seg.source_id) {
            continue;
        }
        seen.push(seg.source_id.clone());
        match load_speakers(&store, &seg.source_id)? {
            Some(f) => found.extend(f.speaker_ids()),
            None => missing.push(seg.source_id.clone()),
        }
    }
    found.sort_by_key(|s| (speaker_number(s), s.clone()));
    found.dedup();
    let mut warnings = Vec::new();
    if i.enabled && found.is_empty() {
        let s = missing.first().map(|s| s.to_string()).unwrap_or_else(|| "<source_id>".to_string());
        return Err(validation(stage, format!("clip {} has no speaker analysis, so there is nothing to colour", clip.id), format!("run `rustler analyze speakers {s}` first")));
    }
    if i.enabled {
        for s in &missing {
            warnings.push(format!("source {s} has no speaker analysis; its captions keep the preset colour"));
        }
    }
    let current: SpeakerColors = extras::get(&store, &clip.id, COLORS_KEY)?.unwrap_or_default();
    let mut colors = current.colors;
    if let Some(given) = &i.colors {
        for (sp, c) in given {
            let sp = sp.trim().to_uppercase();
            if i.enabled && !found.contains(&sp) {
                warnings.push(format!("speaker {sp} does not appear in this clip's sources (found: {})", found.join(", ")));
            }
            colors.insert(sp.clone(), norm_hex(stage, &sp, c)?);
        }
    }
    if i.enabled {
        for sp in &found {
            colors.entry(sp.clone()).or_insert_with(|| default_color(sp).to_string());
        }
    }
    let cfg = SpeakerColors { enabled: i.enabled, colors: colors.clone() };
    let next = vec![format!("rustler render preview-frame {} --out-t 1.0", clip.id), format!("rustler render run {} --quality draft", clip.id)];
    if ctx.dry_run {
        return Ok(SpeakerColorsOut { clip_id: clip.id, enabled: i.enabled, colors, speakers: found, dry_run: Some(true), warnings, next });
    }
    extras::update(&store, &clip.id, COLORS_KEY, |_| Ok(Some(serde_json::to_value(&cfg).expect("serializable"))))?;
    log_event(&ctx, &store, "caption.speaker_colors", json!({"clip_id": clip.id, "enabled": i.enabled}))?;
    Ok(SpeakerColorsOut { clip_id: clip.id, enabled: i.enabled, colors, speakers: found, dry_run: None, warnings, next })
}

// ---------------------------------------------------------------------------------------------
// crop.track_speaker
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct TrackSpeakerIn {
    pub clip_id: ClipId,
    /// All center-crop segments when omitted.
    pub segment_id: Option<SegmentId>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct TurnReport {
    pub speaker: String,
    pub name: String,
    /// Source time.
    pub start: f64,
    pub end: f64,
    /// Output (clip) time.
    pub out_start: f64,
    pub out_end: f64,
    pub face_id: Option<u32>,
    pub face_x: Option<f64>,
    pub score: f64,
    pub margin: f64,
    pub reason: String,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct TrackedSpeakerSegment {
    pub segment_id: SegmentId,
    pub keys: Vec<CropKey>,
    /// The face chosen for every speaker turn inside the segment.
    pub turns: Vec<TurnReport>,
    /// Faces seen in the segment, numbered left to right.
    pub faces: Vec<ActiveTrack>,
    pub samples: usize,
    /// Share of samples with at least one face (0..1).
    pub faces_found_ratio: f64,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct TrackSpeakerOut {
    pub clip_id: ClipId,
    pub segments: Vec<TrackedSpeakerSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Turns of `file` clipped to the source window `[start, end]`, merged and sorted.
fn turns_in(file: &SpeakersFile, start: f64, end: f64) -> Vec<SpeakerTurn> {
    let clipped: Vec<SpeakerTurn> = file
        .segments
        .iter()
        .filter_map(|t| {
            let (s, e) = (t.start.max(start), t.end.min(end));
            (e - s > 0.05).then(|| SpeakerTurn { start: s, end: e, speaker: t.speaker.clone() })
        })
        .collect();
    merge_turns(&clipped)
}

fn video_of(store: &ProjectStore, source: &Source) -> PathBuf {
    source.proxy.as_ref().map(|p| store.resolve(p)).filter(|p| p.is_file()).unwrap_or_else(|| store.resolve(&source.media_path))
}

pub async fn crop_track_speaker(ctx: Ctx, i: TrackSpeakerIn) -> Result<TrackSpeakerOut> {
    let stage = "crop.track_speaker";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let segs: Vec<_> = clip.segments.iter().filter(|s| i.segment_id.as_ref().is_none_or(|id| id == &s.id)).collect();
    if segs.is_empty() {
        return Err(RustlerError::new(ErrorCode::NotFound, stage, format!("clip {} has no such segment", clip.id)).hint("list segment ids with `rustler clip show <clip_id>`"));
    }
    let widths: Vec<f64> = segs.iter().map(|s| tracking_crop_width(s, None)).collect::<Result<_>>()?;
    let mut diarized: HashMap<SourceId, SpeakersFile> = HashMap::new();
    for s in &segs {
        if !diarized.contains_key(&s.source_id) {
            diarized.insert(s.source_id.clone(), require_speakers(&store, &s.source_id, stage)?);
        }
    }
    let next = vec![format!("rustler render preview-frame {} --out-t 1.0", clip.id), format!("rustler render run {} --quality draft", clip.id)];
    if ctx.dry_run {
        return Ok(TrackSpeakerOut { clip_id: clip.id, segments: Vec::new(), dry_run: Some(true), next, warnings: Vec::new() });
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let engine = spawn_engine(&paths).await?;
    let tm = TimeMap::from_segments(&clip.segments);
    let mut out: Vec<TrackedSpeakerSegment> = Vec::new();
    let mut warnings = Vec::new();
    let mut failure = None;
    for (seg, crop_w) in segs.iter().zip(widths) {
        let source = store.load_source(&seg.source_id)?;
        let params = json!({"video_path": video_of(&store, &source), "start": seg.start, "end": seg.end, "fps": ACTIVE_FPS});
        let res: Result<ActiveResult> = engine.call("speakers.active", params, &ctx.progress, &ctx.cancel).await;
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                failure = Some(e);
                break;
            }
        };
        let file = &diarized[&seg.source_id];
        let turns = turns_in(file, seg.start, seg.end);
        let found = res.samples.iter().filter(|s| !s.faces.is_empty()).count();
        let ratio = if res.samples.is_empty() { 0.0 } else { found as f64 / res.samples.len() as f64 };
        if ratio < 0.3 {
            warnings.push(format!("segment {}: faces found in only {:.0}% of samples; the crop will mostly hold the centre", seg.id, ratio * 100.0));
        }
        if turns.is_empty() {
            warnings.push(format!("segment {}: nobody speaks in this window according to the diarization; following the largest face", seg.id));
        }
        let choices = choose_faces(&res.samples, &turns);
        let ambiguous = choices.iter().filter(|c| c.reason == "dominant").count();
        if ambiguous > 0 {
            warnings.push(format!(
                "segment {}: {ambiguous} turn(s) had no clear mouth movement; the largest face was used (see turns[].reason)",
                seg.id
            ));
        }
        if res.tracks.iter().all(|t| t.is_static) && res.tracks.len() > 1 {
            warnings.push(format!("segment {}: no face shows mouth movement (profile views, tiny faces or very low frame rate?); speaker choice is a guess", seg.id));
        }
        let face_samples = speaker_face_samples(&res.samples, &choices);
        let keys = crop_keys(&face_samples, crop_w, &speaker_smooth());
        let mapped = tm.segments().iter().find(|m| m.segment_id == seg.id);
        let to_out = |t: f64| mapped.map_or(t - seg.start, |m| m.out_start + (t - seg.start));
        let turns_out = choices
            .iter()
            .map(|c| TurnReport {
                name: file.name_of(&c.speaker),
                out_start: round(to_out(c.start), 3),
                out_end: round(to_out(c.end), 3),
                speaker: c.speaker.clone(),
                start: c.start,
                end: c.end,
                face_id: c.face_id,
                face_x: c.face_x,
                score: c.score,
                margin: c.margin,
                reason: c.reason.clone(),
            })
            .collect();
        out.push(TrackedSpeakerSegment { segment_id: seg.id.clone(), keys, turns: turns_out, faces: res.tracks, samples: res.samples.len(), faces_found_ratio: round(ratio, 3) });
    }
    engine.shutdown().await;
    if let Some(e) = failure {
        return Err(e);
    }
    // Same storage as crop.track_face: extras.json crop_paths[segment id].
    extras::update(&store, &clip.id, "crop_paths", |cur| {
        let mut map = match cur {
            Some(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        for s in &out {
            map.insert(s.segment_id.as_str().to_string(), serde_json::to_value(&s.keys).expect("serializable"));
        }
        Ok(Some(serde_json::Value::Object(map)))
    })?;
    log_event(&ctx, &store, "crop.track_speaker", json!({"clip_id": clip.id, "segments": out.len()}))?;
    Ok(TrackSpeakerOut { clip_id: clip.id, segments: out, dry_run: None, next, warnings })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

const WRITE: Annotations = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let long = Annotations { long_running: true, ..Default::default() };
    reg.register(typed(
        "analyze.speakers",
        "Who speaks when (local diarization: sherpa-onnx + pyannote segmentation + WeSpeaker): writes speakers.json, labels transcript words with a speaker, returns talk time and turns per speaker",
        &["source_id"],
        long,
        vec![Example { description: "Diarize a podcast; pass --num-speakers when you know it", cli: "rustler analyze speakers src_01k... --num-speakers 2" }],
        analyze_speakers,
    ));
    reg.register(typed(
        "speaker.rename",
        "Give a speaker (S1, S2, ...) a display name; stored in speakers.json",
        &["source_id", "speaker", "name"],
        WRITE,
        vec![Example { description: "Name the first speaker", cli: "rustler speaker rename src_01k... S1 Host" }],
        speaker_rename,
    ));
    reg.register(typed(
        "caption.speaker_colors",
        "Colour captions by speaker (needs analyze.speakers); stored in the clip's extras, honoured by render.run",
        &["clip_id"],
        WRITE,
        vec![Example { description: "Yellow host, cyan guest", cli: "rustler caption speaker-colors clip_01k... --enabled true --colors '{\"S1\":\"#FFE000\",\"S2\":\"#5EE0FF\"}'" }],
        caption_speaker_colors,
    ));
    reg.register(typed(
        "crop.track_speaker",
        "Make the center-crop follow the ACTIVE speaker (diarized turns + mouth movement of each face); reports the face chosen per turn",
        &["clip_id"],
        long,
        vec![Example { description: "Follow whoever is talking in a two-person interview", cli: "rustler crop track-speaker clip_01k..." }],
        crop_track_speaker,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f64, end: f64, speaker: &str) -> SpeakerTurn {
        SpeakerTurn { start, end, speaker: speaker.to_string() }
    }

    fn word(i: u32, start: f64, end: f64) -> Word {
        Word { i, text: format!("w{i}"), start, end, conf: None, speaker: None }
    }

    #[test]
    fn words_take_the_speaker_with_most_overlap() {
        let turns = vec![turn(0.0, 2.0, "S1"), turn(2.0, 5.0, "S2")];
        let mut words = vec![
            word(0, 0.2, 0.6),   // inside S1
            word(1, 1.8, 2.5),   // 0.2 s in S1, 0.5 s in S2 -> S2
            word(2, 2.6, 3.0),   // S2
            word(3, 4.9, 5.2),   // 0.1 in S2, then past the end -> S2
            word(4, 5.4, 5.6),   // 0.4 s after the last turn -> nearest S2
            word(5, 9.0, 9.3),   // far from everything -> none
        ];
        assert_eq!(assign_words(&mut words, &turns), 5);
        let got: Vec<Option<&str>> = words.iter().map(|w| w.speaker.as_deref()).collect();
        assert_eq!(got, vec![Some("S1"), Some("S2"), Some("S2"), Some("S2"), Some("S2"), None]);
    }

    #[test]
    fn same_speaker_overlap_adds_up_and_ties_go_to_the_earlier_turn() {
        let turns = vec![turn(0.0, 1.0, "S1"), turn(1.0, 1.4, "S2"), turn(1.4, 2.0, "S1")];
        // S1 overlaps 1.0 + 0.6 = 1.6 s of [0,2]; S2 only 0.4 s
        assert_eq!(speaker_for_span(&turns, 0.0, 2.0).as_deref(), Some("S1"));
        // exact tie between S1 (0.5) and S2 (0.5)
        let tie = vec![turn(0.0, 1.0, "S1"), turn(1.0, 2.0, "S2")];
        assert_eq!(speaker_for_span(&tie, 0.5, 1.5).as_deref(), Some("S1"));
    }

    #[test]
    fn merge_and_summary_count_turns_and_talk_time() {
        let file = SpeakersFile {
            schema: SPEAKERS_SCHEMA.into(),
            source_id: SourceId::new(),
            model: "test".into(),
            num_speakers: 2,
            duration: 12.0,
            segments: vec![turn(0.0, 2.0, "S1"), turn(2.5, 4.0, "S1"), turn(4.0, 8.0, "S2"), turn(9.0, 10.0, "S1")],
            labels: BTreeMap::from([("S2".to_string(), "Guest".to_string())]),
        };
        let merged = merge_turns(&file.segments);
        assert_eq!(merged.len(), 3);
        assert_eq!((merged[0].start, merged[0].end), (0.0, 4.0));
        let mut words = vec![word(0, 0.1, 0.4), word(1, 5.0, 5.4), word(2, 5.5, 5.9)];
        assign_words(&mut words, &file.segments);
        let s = summarize(&file, &words);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].id.as_str(), s[0].name.as_str(), s[0].turns, s[0].words), ("S1", "S1", 2, 1));
        assert_eq!((s[1].id.as_str(), s[1].name.as_str(), s[1].turns, s[1].words), ("S2", "Guest", 1, 2));
        assert_eq!(s[0].talk_seconds, 4.5);
        assert_eq!(s[1].talk_seconds, 4.0);
        assert_eq!(s[0].talk_pct, 52.9);
        assert_eq!(s[1].talk_pct, 47.1);
    }

    #[test]
    fn speaker_ids_sort_numerically_and_default_colours_cycle() {
        let mut file = SpeakersFile {
            schema: String::new(),
            source_id: SourceId::new(),
            model: String::new(),
            num_speakers: 0,
            duration: 0.0,
            segments: vec![turn(0.0, 1.0, "S10"), turn(1.0, 2.0, "S2"), turn(2.0, 3.0, "S1")],
            labels: BTreeMap::new(),
        };
        assert_eq!(file.speaker_ids(), vec!["S1", "S2", "S10"]);
        assert_eq!(default_color("S1"), "#FFE000");
        assert_eq!(default_color("S2"), "#5EE0FF");
        assert_eq!(default_color("S9"), default_color("S1"));
        file.segments.clear();
        assert!(file.speaker_ids().is_empty());
    }

    // --- active-speaker choice -------------------------------------------------------------

    fn face(id: u32, cx: f64, speak: f64) -> ActiveFace {
        ActiveFace { x: cx - 0.075, y: 0.2, w: 0.15, h: 0.3, score: 0.9, id, speak, motion: speak / 10.0, track: id }
    }

    /// Two faces (0 left at 0.2, 1 right at 0.8). `talking(t)` says which face moves its mouth.
    fn samples(seconds: f64, talking: impl Fn(f64) -> Option<u32>) -> Vec<ActiveSample> {
        (0..(seconds * ACTIVE_FPS) as usize)
            .map(|i| {
                let t = i as f64 / ACTIVE_FPS;
                let on = talking(t);
                ActiveSample {
                    t,
                    faces: vec![face(0, 0.2, if on == Some(0) { 0.9 } else { 0.05 }), face(1, 0.8, if on == Some(1) { 0.9 } else { 0.05 })],
                }
            })
            .collect()
    }

    fn alternating(t: f64) -> Option<u32> {
        Some(if t < 4.0 { 0 } else if t < 8.0 { 1 } else { 0 })
    }

    fn three_turns() -> Vec<SpeakerTurn> {
        vec![turn(0.0, 4.0, "S1"), turn(4.0, 8.0, "S2"), turn(8.0, 12.0, "S1")]
    }

    #[test]
    fn mouth_motion_picks_the_talking_face_per_turn() {
        let s = samples(12.0, |t| if t < 8.0 { alternating(t) } else { Some(0) });
        let c = choose_faces(&s, &three_turns());
        assert_eq!(c.len(), 3);
        assert_eq!(c.iter().map(|c| c.face_id).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(0)]);
        assert!(c.iter().all(|c| c.reason == "mouth-motion" && c.margin > 0.5), "{c:?}");
        assert!((c[1].face_x.unwrap() - 0.8).abs() < 1e-6);
    }

    #[test]
    fn ambiguous_motion_uses_the_speaker_map_then_the_largest_face() {
        // Turn 3 (S1 again) has no clear motion on either face: S1 owns face 0 from turn 1.
        let s = samples(12.0, |t| if t < 8.0 { alternating(t) } else { None });
        let c = choose_faces(&s, &three_turns());
        assert_eq!((c[2].face_id, c[2].reason.as_str()), (Some(0), "speaker-map"));

        // No mouth motion anywhere and a larger face on the right: dominant face, flagged.
        let mut flat = samples(8.0, |_| None);
        for smp in &mut flat {
            smp.faces[1].w = 0.25;
            smp.faces[1].h = 0.45;
        }
        let c = choose_faces(&flat, &[turn(0.0, 4.0, "S1"), turn(4.0, 8.0, "S2")]);
        assert!(c.iter().all(|c| c.face_id == Some(1) && c.reason == "dominant"), "{c:?}");
    }

    #[test]
    fn short_turns_and_missing_faces_hold_the_previous_face() {
        let s = samples(12.0, |t| if t < 6.0 { Some(0) } else { Some(1) });
        let turns = vec![turn(0.0, 6.0, "S1"), turn(6.0, 6.3, "S2"), turn(6.3, 12.0, "S1")];
        let c = choose_faces(&s, &turns);
        assert_eq!((c[1].face_id, c[1].reason.as_str()), (Some(0), "short-turn"));
        // a segment where no face exists at all
        let empty: Vec<ActiveSample> = (0..20).map(|i| ActiveSample { t: i as f64 / 5.0, faces: vec![] }).collect();
        let c = choose_faces(&empty, &[turn(0.0, 4.0, "S1")]);
        assert_eq!((c[0].face_id, c[0].reason.as_str()), (None, "no-face"));
    }

    #[test]
    fn a_single_face_is_the_only_face() {
        let s: Vec<ActiveSample> = (0..40).map(|i| ActiveSample { t: i as f64 / 5.0, faces: vec![face(0, 0.5, 0.1)] }).collect();
        let c = choose_faces(&s, &[turn(0.0, 4.0, "S1"), turn(4.0, 8.0, "S2")]);
        assert!(c.iter().all(|c| c.face_id == Some(0) && c.reason == "only-face"));
    }

    #[test]
    fn crop_keys_switch_sides_with_the_speaker() {
        let s = samples(12.0, |t| if t < 8.0 { alternating(t) } else { Some(0) });
        let choices = choose_faces(&s, &three_turns());
        let face_samples = speaker_face_samples(&s, &choices);
        assert!(face_samples.iter().all(|f| f.faces.len() == 1));
        let crop_w = 0.3164; // 9:16 out of 16:9
        let keys = crop_keys(&face_samples, crop_w, &speaker_smooth());
        let at = |t: f64| keys.iter().rev().find(|k| k.t <= t).map_or(keys[0].x, |k| k.x);
        // face 0 centred at 0.2 -> crop left edge clamps to 0.04; face 1 at 0.8 -> 0.642
        assert!(at(2.0) < 0.1, "{keys:?}");
        assert!(at(7.0) > 0.55, "{keys:?}");
        assert!(at(11.0) < 0.15, "{keys:?}");
        assert!(at(5.5) > 0.55, "{keys:?}");
        assert!(at(9.5) < 0.15, "{keys:?}");
    }

    #[test]
    fn face_boxes_are_held_when_the_speaker_briefly_leaves_the_frame() {
        let mut s = samples(6.0, |_| Some(1));
        for smp in s.iter_mut().filter(|x| x.t >= 2.0 && x.t < 3.0) {
            smp.faces.retain(|f| f.id == 0);
        }
        let choices = vec![TurnChoice {
            speaker: "S1".into(),
            start: 0.0,
            end: 6.0,
            face_id: Some(1),
            face_x: Some(0.8),
            score: 0.9,
            margin: 0.8,
            reason: "mouth-motion".into(),
        }];
        let fs = speaker_face_samples(&s, &choices);
        assert!(fs.iter().all(|f| f.faces.len() == 1 && (f.faces[0].x + f.faces[0].w / 2.0 - 0.8).abs() < 1e-9));
    }

    // --- real two-person footage (public-domain White House fireside chat, Wikimedia Commons) -----
    //
    // `vance_wide_active.json` is the recorded `speakers.active` output (5 fps) of a 95 s excerpt:
    // a wide two-shot (host left, guest right, faces ~4 % of the frame width, mouth movement is
    // noise at that size) with cuts to a close-up of the guest. `vance_wide_turns.json` is the
    // recorded `speakers.diarize` result (S1 = guest, S2 = host).

    #[derive(Deserialize)]
    struct Fixture {
        samples: Vec<ActiveSample>,
    }

    fn real_fixture() -> (Vec<ActiveSample>, Vec<SpeakerTurn>) {
        let f: Fixture = serde_json::from_str(include_str!("../tests/fixtures/speakers/vance_wide_active.json")).unwrap();
        let t: Vec<SpeakerTurn> = serde_json::from_str(include_str!("../tests/fixtures/speakers/vance_wide_turns.json")).unwrap();
        (f.samples, merge_turns(&t))
    }

    #[test]
    fn real_footage_close_ups_identify_the_guest_and_elimination_the_host() {
        let (samples, turns) = real_fixture();
        let owner = owner_map(&samples, &turns);
        let (guest, src) = owner["S1"];
        assert_eq!(src, OwnerSource::Identity, "{owner:?}");
        let (host, src) = owner["S2"];
        assert_eq!(src, OwnerSource::Elimination, "{owner:?}");
        assert_ne!(guest, host);
        let c = choose_faces(&samples, &turns);
        let host_turn = c.iter().find(|c| c.speaker == "S2" && c.end - c.start > 10.0).expect("host turn");
        assert_eq!(host_turn.face_id, Some(host), "{c:?}");
        assert!(host_turn.face_x.unwrap() < 0.35, "the host sits on the left: {host_turn:?}");
        let guest_turn = c.iter().find(|c| c.speaker == "S1" && c.end - c.start > 10.0).unwrap();
        assert_eq!(guest_turn.face_id, Some(guest));
    }

    /// Share of 1-second samples (inside `truth` spans: start, end, person id) where the person's
    /// face centre is inside the crop (interpolated like the renderer does).
    fn crop_hit_rate(samples: &[ActiveSample], keys: &[rustler_media::facepath::CropKey], crop_w: f64, truth: &[(f64, f64, u32)]) -> f64 {
        let at = |t: f64| {
            let k = keys.partition_point(|k| k.t <= t);
            if k == 0 {
                keys[0].x
            } else if k == keys.len() {
                keys[k - 1].x
            } else {
                let (a, b) = (keys[k - 1], keys[k]);
                a.x + (b.x - a.x) * (t - a.t) / (b.t - a.t)
            }
        };
        let face_cx = |id: u32, t: f64| {
            samples
                .iter()
                .filter(|s| (s.t - t).abs() <= 1.0)
                .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
                .and_then(|s| s.faces.iter().find(|f| f.id == id))
                .map(|f| f.cx())
        };
        let (mut hit, mut n) = (0, 0);
        for &(a, b, id) in truth {
            let mut t = a + 0.5;
            while t < b {
                n += 1;
                if let Some(cx) = face_cx(id, t) {
                    let x = at(t);
                    if cx >= x + 0.02 && cx <= x + crop_w - 0.02 {
                        hit += 1;
                    }
                }
                t += 1.0;
            }
        }
        hit as f64 / n as f64
    }

    #[test]
    fn real_footage_the_crop_sits_on_the_talking_person() {
        let (samples, turns) = real_fixture();
        let choices = choose_faces(&samples, &turns);
        let keys = crop_keys(&speaker_face_samples(&samples, &choices), 0.3164, &speaker_smooth());
        let owner = owner_map(&samples, &turns);
        let (guest, host) = (owner["S1"].0, owner["S2"].0);
        // Truth from the transcript and the frames: host speaks 17.2-39.8 s, guest 8.3-16.5 s and 41-95 s.
        let rate = crop_hit_rate(&samples, &keys, 0.3164, &[(8.3, 16.5, guest), (17.2, 39.8, host), (41.0, 95.0, guest)]);
        assert!(rate >= 0.9, "crop contained the speaker in only {:.0}% of samples: {keys:?}", rate * 100.0);
        // Without the hold keys the crop drifts for seconds before each move and the rate collapses.
        let drifting = SmoothParams { hold_keys: false, ..speaker_smooth() };
        let keys = crop_keys(&speaker_face_samples(&samples, &choices), 0.3164, &drifting);
        let bad = crop_hit_rate(&samples, &keys, 0.3164, &[(8.3, 16.5, guest), (17.2, 39.8, host), (41.0, 95.0, guest)]);
        assert!(bad < rate, "{bad} vs {rate}");
    }

    /// Second real excerpt (public-domain White House fireside chat, medium and very wide shots where
    /// the camera slowly zooms, faces 2-4 % of the width, close-ups of the guest at the end):
    /// guest speaks 0-6 s and 46-105 s, host 6-45 s.
    #[test]
    fn real_footage_medium_two_shot_with_zooms() {
        let f: Fixture = serde_json::from_str(include_str!("../tests/fixtures/speakers/boyle_medium_active.json")).unwrap();
        let t: Vec<SpeakerTurn> = serde_json::from_str(include_str!("../tests/fixtures/speakers/boyle_medium_turns.json")).unwrap();
        let (samples, turns) = (f.samples, merge_turns(&t));
        let owner = owner_map(&samples, &turns);
        let (guest, host) = (owner["S1"].0, owner["S2"].0);
        assert_ne!(guest, host);
        assert_eq!(owner["S1"].1, OwnerSource::Identity, "{owner:?}");
        let choices = choose_faces(&samples, &turns);
        assert!(choices.iter().all(|c| c.reason != "dominant" && c.reason != "mouth-motion"), "{choices:?}");
        let keys = crop_keys(&speaker_face_samples(&samples, &choices), 0.3164, &speaker_smooth());
        let rate = crop_hit_rate(&samples, &keys, 0.3164, &[(0.5, 5.7, guest), (6.3, 44.8, host), (45.8, 104.9, guest)]);
        assert!(rate >= 0.95, "crop contained the speaker in only {:.0}% of samples: {keys:?}", rate * 100.0);
        // The old behaviour (largest face, no identity) put the crop on the host in the first turn.
        let first = &choices[0];
        assert_eq!(first.face_id, Some(guest));
    }
}
