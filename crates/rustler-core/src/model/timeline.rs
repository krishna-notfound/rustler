//! Timeline v2 (`rustler.timeline@2`): the whole clip as one document (design §5.1, plan Task 1.1).
//!
//! Storage stays `clips/<id>/clip.json` + `clips/<id>/extras.json`; `rustler-commands`
//! (`timeline_doc`) assembles this document from those two files and writes it back. Every
//! extras key maps 1:1 to a field here; keys this version does not know are kept verbatim under
//! [`Tracks::extra`], and unknown fields inside a known item are kept in its `extra` map, so a
//! round trip never loses data written by a newer or older build.
//!
//! Times: every item time (`start`/`end`, `out_start`/`out_end`, `at`, `out_t`) is on the *edit
//! timeline* — the sum of the segments' source lengths, what `clip show` prints. The render maps
//! edit time to the rendered file through segment speeds, freeze frames and transitions;
//! `timeline.show` prints both. Word anchors (`start_word`/`end_word`/`at_word`/`word_anchor`)
//! name a transcript word and follow re-cuts; an anchor whose word was cut out is dropped with a
//! warning, never placed at a wrong time.
//!
//! Field names: a track keeps the field names its extras key already stores (e.g. text items
//! keep `start`/`end` and `box` like `overlays`), so v1 data, v1 commands and the document agree.

use crate::ids::{ClipId, SegmentId, SourceId};
use crate::model::clip::{AudioSettings, CaptionTrack, Canvas, Fit, Layout, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Schema id of the document.
pub const TIMELINE_SCHEMA: &str = "rustler.timeline@2";

/// Unknown fields of an item, kept as they are.
pub type ExtraFields = Map<String, Value>;

fn is_false(b: &bool) -> bool {
    !*b
}

/// One clip as a single editable document.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Timeline {
    /// Always `rustler.timeline@2`.
    pub schema: String,
    pub id: ClipId,
    pub name: String,
    /// Revision of the clip when the document was assembled (`timeline.apply` checks
    /// `base_revision` against the clip on disk and bumps it).
    pub revision: u64,
    pub canvas: Canvas,
    /// The template this clip was styled from (`template.apply`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_ref: Option<TemplateRef>,
    pub tracks: Tracks,
    #[serde(default)]
    pub meta: TimelineMeta,
}

/// extras key `template_ref`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TemplateRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// Every layer of the clip. Order of drawing: video -> full cutaways -> masks -> effects ->
/// PiP cutaways -> images below the captions (by `z`) -> captions -> text -> images with
/// `above_captions` (by `z`) -> progress bar.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct Tracks {
    /// Segments in play order (clip.json `segments`, plus extras `crop_paths` and
    /// `segment_speed` per segment).
    pub video: Vec<VideoSeg>,
    /// Freeze frames (extras `freezes`).
    #[serde(default)]
    pub freezes: Vec<FreezeItem>,
    /// B-roll over the speaker (extras `cutaways`).
    #[serde(default)]
    pub cutaways: Vec<CutawayItem>,
    /// Logos, stickers, cards, GIFs (extras `images`).
    #[serde(default)]
    pub images: Vec<ImageItem>,
    /// Blur / box / pixelate areas of the source picture (extras `masks`).
    #[serde(default)]
    pub masks: Vec<MaskItem>,
    /// Hook titles and labels (extras `overlays`).
    #[serde(default)]
    pub text: Vec<TextItem>,
    /// clip.json `captions` plus extras `caption_word_overrides` and `speaker_colors`.
    #[serde(default)]
    pub captions: CaptionsTrack,
    /// clip.json `audio` plus extras `audio_fx`, `music` and `sfx`.
    #[serde(default)]
    pub audio: AudioTrack,
    /// Camera effects: punch-in, zoom-slow, shake, flash (extras `effects`).
    #[serde(default)]
    pub effects: Vec<EffectItem>,
    /// Transitions between consecutive segments (extras `transitions`).
    #[serde(default)]
    pub transitions: Vec<TransitionItem>,
    /// Progress bar (extras `progress_bar`); null = none.
    #[serde(default)]
    pub progress_bar: Option<ProgressBarItem>,
    /// extras keys this build does not know, verbatim (written back unchanged).
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
}

/// One crop-path keyframe: at segment-relative source time `t` the crop's left edge is `x`
/// (0..1 of the source width).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct CropPoint {
    pub t: f64,
    pub x: f64,
}

/// A source range `[src_in, src_out)` placed on the timeline with its layout.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct VideoSeg {
    pub id: SegmentId,
    pub source_id: SourceId,
    /// Source seconds where the segment starts.
    pub src_in: f64,
    /// Source seconds where it ends (exclusive).
    pub src_out: f64,
    pub layout: Layout,
    /// Face-tracked crop path driving `layout.regions[0]` (extras `crop_paths[<id>]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_path: Option<Vec<CropPoint>>,
    /// Playback speed 0.5..2.0 (extras `segment_speed[<id>]`); missing = 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
}

/// extras `freezes[]`: the picture holds at `out_t` for `duration` seconds (silent).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct FreezeItem {
    pub id: String,
    /// Edit-timeline seconds.
    pub out_t: f64,
    /// Seconds (0 < duration <= 10).
    pub duration: f64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// A transcript word (`transcript get <source_id>` prints the indices).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Hash)]
pub struct Anchor {
    pub source_id: SourceId,
    /// Word index in the source transcript (`word_index` is accepted too).
    #[serde(alias = "word_index")]
    pub word_i: u32,
}

/// The sound-effect form of a word anchor (stored as `word_index`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Hash)]
pub struct SfxAnchor {
    pub source_id: SourceId,
    pub word_index: u32,
}

/// When a layer shows: edit-timeline seconds, or transcript words that follow re-cuts.
/// `start_word` wins over `out_start`, `end_word` over `out_end`. A `start_word` without an end
/// lasts until the end of that word. Missing start = 0, missing end = end of the clip.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct ItemTiming {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_end: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_word: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_word: Option<Anchor>,
}

impl ItemTiming {
    pub fn has_anchor(&self) -> bool {
        self.start_word.is_some() || self.end_word.is_some()
    }
}

/// Entrance / exit animation kind of an image.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnimKind {
    #[default]
    None,
    Fade,
    Pop,
    SlideUp,
}

fn anim_duration() -> f64 {
    0.3
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct Anim {
    #[serde(rename = "in", default)]
    pub in_: AnimKind,
    #[serde(default)]
    pub out: AnimKind,
    /// Seconds per animation (default 0.3).
    #[serde(default = "anim_duration")]
    pub duration: f64,
}

impl Default for Anim {
    fn default() -> Self {
        Self { in_: AnimKind::None, out: AnimKind::None, duration: anim_duration() }
    }
}

fn one() -> f64 {
    1.0
}
fn contain() -> Fit {
    Fit::Contain
}
fn cover() -> Fit {
    Fit::Cover
}
fn yes() -> bool {
    true
}

/// extras `images[]`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct ImageItem {
    pub id: String,
    #[serde(flatten)]
    pub timing: ItemTiming,
    /// Library asset id when it came from the asset library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Absolute path of the image (png, jpg, webp, gif, bmp).
    pub file: PathBuf,
    /// Box on the canvas, 0..1, sides > 0.02.
    pub rect: Rect,
    #[serde(default = "one")]
    pub scale: f64,
    /// 0..1.
    #[serde(default = "one")]
    pub opacity: f64,
    /// Degrees clockwise.
    #[serde(default)]
    pub rotation: f64,
    /// contain (default) | cover.
    #[serde(default = "contain")]
    pub fit: Fit,
    #[serde(default)]
    pub anim: Anim,
    /// Stacking order among images (higher = on top).
    #[serde(default)]
    pub z: i32,
    /// Draw above the captions and text.
    #[serde(default)]
    pub above_captions: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MaskKind {
    Blur,
    Box,
    Pixelate,
}

/// extras `masks[]`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct MaskItem {
    pub id: String,
    #[serde(flatten)]
    pub timing: ItemTiming,
    pub rect: Rect,
    pub kind: MaskKind,
    /// blur radius / pixel size in pixels, 1..100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
    /// box: `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CutawayMode {
    Full,
    Pip,
}

/// What a cutaway shows: an imported source, a library asset or a file.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct CutawayMedia {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Seconds into the media where the cutaway starts.
    #[serde(rename = "in", default)]
    pub in_: f64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// extras `cutaways[]`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct CutawayItem {
    pub id: String,
    #[serde(flatten)]
    pub timing: ItemTiming,
    pub media: CutawayMedia,
    pub mode: CutawayMode,
    /// pip: box on the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(default = "cover")]
    pub fit: Fit,
    /// Keep only the clip's speech (default true).
    #[serde(default = "yes")]
    pub mute: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// Look of a text item.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TextStyle {
    #[default]
    HookTop,
    LowerThird,
    Plain,
}

fn text_y() -> f64 {
    0.12
}
fn text_size() -> f64 {
    0.085
}

/// extras `overlays[]`: a hook title or label.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TextItem {
    pub id: String,
    pub text: String,
    /// Edit-timeline seconds.
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub style: TextStyle,
    /// Vertical centre, 0 = top .. 1 = bottom.
    #[serde(default = "text_y")]
    pub position_y: f64,
    /// Font size as a fraction of the canvas width (0.01..0.2).
    #[serde(default = "text_size")]
    pub font_size_factor: f64,
    /// Dark box behind the text.
    #[serde(rename = "box", default = "yes")]
    pub boxed: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// extras `caption_word_overrides[]`: per-word caption look.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct WordOverride {
    pub word_anchor: Anchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// 0.5..2.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// extras `speaker_colors`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct SpeakerColors {
    pub enabled: bool,
    /// `#RRGGBB` by speaker id.
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// Captions: the clip.json caption settings (flattened: `enabled`, `preset`, `script`,
/// `position_y`, `max_words_per_line`) plus word overrides and speaker colours.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct CaptionsTrack {
    #[serde(flatten)]
    pub settings: CaptionTrack,
    #[serde(default)]
    pub word_overrides: Vec<WordOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_colors: Option<SpeakerColors>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum DenoiseLevel {
    #[default]
    Off,
    Light,
    Medium,
    Strong,
}

/// extras `music`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct MusicItem {
    pub path: PathBuf,
    pub volume_db: f64,
    pub duck: bool,
    pub fade_in: f64,
    pub fade_out: f64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// extras `sfx[]`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct SfxItem {
    pub id: String,
    pub file: PathBuf,
    /// Edit-timeline seconds when not word-anchored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_word: Option<SfxAnchor>,
    pub volume_db: f64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// Audio: the clip.json loudness settings (flattened: `loudnorm`, `target_lufs`, `true_peak`,
/// `lra`), extras `audio_fx` (`denoise`, `volume_db`, `segment_volumes`), `music` and `sfx`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct AudioTrack {
    #[serde(flatten)]
    pub master: AudioSettings,
    #[serde(default)]
    pub denoise: DenoiseLevel,
    /// Whole-dialogue gain (dB).
    #[serde(default)]
    pub volume_db: f64,
    /// Gain (dB) per segment id.
    #[serde(default)]
    pub segment_volumes: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<MusicItem>,
    #[serde(default)]
    pub sfx: Vec<SfxItem>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    PunchIn,
    ZoomSlow,
    Shake,
    Flash,
}

/// extras `effects[]`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct EffectItem {
    pub id: String,
    pub kind: EffectKind,
    /// Edit-timeline seconds.
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TransitionKind {
    Cut,
    Fade,
    Dissolve,
    Wipeleft,
    Wiperight,
    Slideup,
    Zoomin,
}

/// extras `transitions[]`: into the segment after `after_segment_id`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TransitionItem {
    pub after_segment_id: SegmentId,
    pub kind: TransitionKind,
    /// Overlap in seconds.
    pub duration: f64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
}

fn bar_color() -> String {
    "#FFE000".into()
}
fn bar_height() -> u32 {
    8
}

/// extras `progress_bar`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct ProgressBarItem {
    #[serde(default = "bar_color")]
    pub color: String,
    #[serde(default = "bar_height")]
    pub height: u32,
    #[serde(default)]
    pub position: BarPosition,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: ExtraFields,
}

/// Clip-level data: scores, posting copy, variant links. Written by their own commands
/// (`clip.score_set`, `clip.retention`, `clip.meta_set`, `variant.*`); kept as JSON here.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct TimelineMeta {
    /// extras `meta.score` (the agent's hook/flow/value/trend score).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Value>,
    /// extras `meta.retention` (last `clip.retention` result).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<Value>,
    /// extras `post` (titles, descriptions, hashtags per platform; campaign).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<Value>,
    /// extras `variant_of` (root clip id of a variant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_of: Option<Value>,
    /// extras `variant_info`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_info: Option<Value>,
    /// Other fields of extras `meta`, verbatim.
    #[serde(flatten)]
    #[schemars(skip)]
    pub other: ExtraFields,
}

impl Timeline {
    /// Total edit-timeline duration (sum of the segments' source lengths).
    pub fn edit_duration(&self) -> f64 {
        self.tracks.video.iter().map(|s| (s.src_out - s.src_in).max(0.0)).sum()
    }
}
