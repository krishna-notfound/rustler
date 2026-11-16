//! Commands: template.create, template.show, template.list, template.update, template.delete,
//! template.apply, template.diff, template.export, template.import (plan Task 1.5, design Â§5.3).
//!
//! A template (`rustler.template@1`) is a clip's whole look without its content: caption style /
//! position / words per line, layout, hook title look, text looks, brand images, progress bar,
//! loudness / denoise / music / sound-effect patterns, effect patterns (punch-in every N s, at
//! cuts, at the start), transitions and canvas. Text content is captured only when asked
//! (`include_text`). A section that is absent leaves that part of the clip alone.
//!
//! Storage: `templates/<name>@<version>.json` in the project, or `<home>/templates/` (global).
//! Built-in starters (`podcast-viral`, `streamer-split`) are embedded. `template.apply` turns the
//! template into RFC 6902 JSON-Patch ops on the clip's timeline v2 document and goes through
//! `timeline.apply` (one revision per clip, validated, undoable with `history undo`). Items a
//! template adds carry `by_template: "<name>@<version>"`, so re-applying replaces them instead of
//! stacking. `template.export` bundles the template with the assets and files it references
//! (zip); `template.import` puts them back into a library.

use crate::asset_cmds::{self, Scope};
use crate::clip_cmds::log_event;
use crate::smart_cmds::{self, SMART_KEY, ScanFace, ScanSample, ShotClass};
use crate::timeline_cmds;
use crate::understand_cmds::FacesResult;
use crate::timeline_doc;
use rustler_core::layout::{self as core_layout, LayoutParams, LayoutPresetFile};
use rustler_core::timeline::{
    Anchor, Anim, BarPosition, CutawayItem, CutawayMedia, CutawayMode, DenoiseLevel, EffectItem, EffectKind, ImageItem, ItemTiming,
    MusicItem, ProgressBarItem, SfxItem, SpeakerColors, TemplateRef, TextItem, TextStyle, TransitionItem, TransitionKind,
    WordOverride,
};
use rustler_core::{
    Background, Canvas, CaptionScript, ClipId, ErrorCode, Fit, Layout, ProjectStore, Rect, Region, RenderQuality, RustlerError, Segment,
    Timeline, TimeMap,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::assets::{AssetEntry, AssetKind, assets_root, project_assets_root, update_manifest, upsert};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// Schema id of a template file.
pub const TEMPLATE_SCHEMA: &str = "rustler.template@1";
const BUNDLE_SCHEMA: &str = "rustler.template-bundle@1";
/// Field set on items a template adds (value `<name>@<version>`).
pub const BY_TEMPLATE: &str = "by_template";
const ROLE: &str = "template_role";

const STARTERS: &[(&str, &str)] = &[
    ("podcast-viral", include_str!("../starters/podcast-viral.json")),
    ("streamer-split", include_str!("../starters/streamer-split.json")),
];

fn verr(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn nf(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::NotFound, stage, msg)
}

fn io_err(stage: &str, what: &str, p: &Path, e: impl std::fmt::Display) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, stage, format!("cannot {what} {}: {e}", p.display()))
}

fn with_stage(mut e: RustlerError, stage: &str) -> RustlerError {
    e.stage = stage.into();
    e
}

// ---------------------------------------------------------------------------------------------
// The template document
// ---------------------------------------------------------------------------------------------

fn schema_id() -> String {
    TEMPLATE_SCHEMA.into()
}
fn one_u32() -> u32 {
    1
}

/// A reusable look (`templates/<name>@<version>.json`). Absent sections leave that part of a clip alone.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// Always `rustler.template@1`.
    #[serde(default = "schema_id")]
    pub schema: String,
    /// Lowercase letters, digits, `-` and `_`.
    pub name: String,
    #[serde(default = "one_u32")]
    pub version: u32,
    #[serde(default)]
    pub description: String,
    /// agent | user | built-in.
    #[serde(default)]
    pub created_by: String,
    /// Where the look came from, e.g. {"clip": "clp_x"} or {"reference_set": "my-style"}.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Value>,
    /// Advice for the agent applying it (not applied).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<Canvas>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<TplLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captions: Option<TplCaptions>,
    /// Look of the hook title (text items with style hook-top).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<TplHook>,
    /// Looks of the other text items by style (`lower-third`, `plain`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text_styles: BTreeMap<String, TextLook>,
    /// Fixed text items (only when captured with include_text, or written by hand).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texts: Option<Vec<TplText>>,
    /// Brand images (logo, watermark, badge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<TplImage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_bar: Option<TplBar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<TplAudio>,
    /// Effect patterns (punch-in at the start, at every cut, every N seconds ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<TplEffect>>,
    /// One transition for every seam between segments (kind cut removes them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transitions: Option<TplTransition>,
    /// Font asset ids the look needs (custom caption style fonts); exported with the template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<String>,
    /// Per-shot layout: the first rule whose `when` matches a segment's shot sets its layout and
    /// tracking (segments no rule matches get `layout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_rules: Option<Vec<LayoutRule>>,
    /// Fill weak visual stretches (punch-ins or tagged cutaways).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutaway_rules: Option<TplCutawayRules>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<TplBrand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<TplExport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_defaults: Option<TplPost>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplLayout {
    /// center-crop | blur-fit | split-vertical | a saved layout preset (`layout save-preset`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Parameters of a built-in preset (focus_x, top, bottom, split).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<LayoutParams>,
    /// Normalized regions (src rect of the source -> dst rect of the canvas), used instead of the preset's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<Vec<Region>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    /// Also re-lay segments that already use this preset (default false: their framing, crop path or
    /// detected webcam panes are kept).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplCaptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Caption style (built-in or custom, `caption styles`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_words_per_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<CaptionScript>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_colors: Option<SpeakerColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_highlight: Option<TplKeywords>,
    /// on: keep the emoji the agent set per word (`caption word --emoji`); off: remove them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<OnOff>,
}

/// Look of a text item.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TextLook {
    /// Vertical centre 0 (top) .. 1 (bottom).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_y: Option<f64>,
    /// Font size as a fraction of the canvas width (0.01..0.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_factor: Option<f64>,
    /// Dark box behind the text.
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub boxed: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplHook {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_factor: Option<f64>,
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub boxed: Option<bool>,
    /// Put the hook on the seam of a stacked layout (webcam over gameplay) when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_seam: Option<bool>,
    /// Seconds the hook stays on screen (from its start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Warn when a hook is longer than this many words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_words: Option<u32>,
    /// Hook text to add when the clip has no hook title (normally left out: hooks are per clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

fn plain() -> TextStyle {
    TextStyle::Plain
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplText {
    pub text: String,
    /// Edit-timeline seconds (clamped to the clip).
    pub start: f64,
    pub end: f64,
    #[serde(default = "plain")]
    pub style: TextStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_factor: Option<f64>,
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub boxed: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplImage {
    /// Library asset id (give this or file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<Anim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above_captions: Option<bool>,
    /// Edit-timeline seconds; missing = from the start / to the end of the clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplBar {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<BarPosition>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplAudio {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudnorm: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_lufs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub true_peak: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lra: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denoise: Option<DenoiseLevel>,
    /// Whole-dialogue gain (dB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_db: Option<f64>,
    /// Music bed; {"none": true} removes the clip's music.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<TplMusic>,
    /// Sound-effect patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<Vec<TplSfx>>,
}

fn music_db() -> f64 {
    -18.0
}
fn yes() -> bool {
    true
}
fn fade_in() -> f64 {
    0.5
}
fn fade_out() -> f64 {
    1.0
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplMusic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Pick a music asset from the library by tag (same pick for the same clip on re-apply).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Pick by mood: matches a library tag `<mood>` or `mood:<mood>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mood: Option<String>,
    /// true: remove the clip's music.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub none: bool,
    #[serde(default = "music_db")]
    pub volume_db: f64,
    #[serde(default = "yes")]
    pub duck: bool,
    #[serde(default = "fade_in")]
    pub fade_in: f64,
    #[serde(default = "fade_out")]
    pub fade_out: f64,
}

/// Where a pattern places its items on the edit timeline.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PlaceAt {
    /// At 0 s (the hook).
    Start,
    /// At every seam between segments (jump cuts).
    Cuts,
    /// Every `every_s` seconds from `every_s` on (not in the last 0.5 s). Punch-ins go through
    /// the punch-in planner with that spacing (snapped to stressed words, away from cuts).
    Every,
    /// At `t` seconds.
    Time,
    /// Punch-ins only: placed by the punch-in planner on stressed words, at most `per_10s`
    /// visual changes per 10 s, at least 0.5 s from cuts, never on graphics / blur-fit
    /// segments, amount varied between `amount` and `amount_max`, centred on the face.
    Emphasis,
}

fn sfx_db() -> f64 {
    -6.0
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplSfx {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub at: PlaceAt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    #[serde(default = "sfx_db")]
    pub volume_db: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplEffect {
    /// punch-in | zoom-slow | shake | flash.
    pub kind: EffectKind,
    pub at: PlaceAt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    /// Seconds each effect lasts (planned punch-ins: the target length, snapped to words).
    pub duration: f64,
    /// Zoom factor; for planned punch-ins (at = emphasis / every) the lower end of the range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    /// Planned punch-ins: upper end of the zoom range (varied per punch-in; default = amount).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_max: Option<f64>,
    /// Planned punch-ins: most visual changes (cuts, text pops, zoom in/out) in any 10 s (default 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_10s: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplTransition {
    /// cut (removes transitions) | fade | dissolve | wipeleft | wiperight | slideup | zoomin.
    pub kind: TransitionKind,
    #[serde(default = "tr_dur")]
    pub duration: f64,
}

fn tr_dur() -> f64 {
    0.3
}

/// `on` | `off`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum OnOff {
    On,
    Off,
}

/// Keyword highlight in the captions: the `caption.keywords --auto` picker marks numbers, emphasis
/// words and names in a colour (template-tagged caption word overrides; words the agent coloured
/// itself are left alone).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplKeywords {
    /// `#RRGGBB` (default: the caption preset's accent, else green / yellow like `caption keywords`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Most marked words per caption line (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_line: Option<u32>,
}

/// What a shot shows (from the smart defaults' shot plan or `analysis/<source>/faces.json`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShotWhen {
    /// One face on screen.
    #[serde(rename = "faces=1")]
    OneFace,
    /// Two or more faces.
    #[serde(rename = "faces>=2")]
    MultiFace,
    /// No face (graphics included).
    #[serde(rename = "no_face")]
    NoFace,
    /// No face and a screen / window / content box inside the frame (smart defaults chose zoom-fit), or graphics.
    #[serde(rename = "screen")]
    Screen,
    /// Flat graphics / slides.
    #[serde(rename = "graphics")]
    Graphics,
}

/// How a rule's center-crop follows the faces.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RuleTrack {
    /// Smooth face track (`crop track-face`).
    Face,
    /// Diarized speaker (`crop track-speaker`).
    Speaker,
    /// Light active speaker (`crop track-active`).
    Active,
    /// Static crop at the face (drops the crop path).
    None,
}

/// One per-shot layout rule; the first rule whose `when` matches a segment wins.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct LayoutRule {
    pub when: ShotWhen,
    /// center-crop | blur-fit | zoom-fit | split-vertical | a saved layout preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<LayoutParams>,
    /// Normalized regions instead of the preset's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<Vec<Region>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<RuleTrack>,
    /// Leave room in the look direction when tracking (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look_room: Option<bool>,
}

/// What fills a weak visual stretch.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CutawayKind {
    /// Punch-ins from the shared punch-in planner (its default pacing), inside each weak zone.
    PunchIn,
    /// A cutaway showing an image asset from the library with `tag`.
    AssetTag,
}

fn cutaway_s() -> f64 {
    1.5
}

/// Fills edit-timeline stretches longer than `on_weak_visual_after_s` without a visual change.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplCutawayRules {
    pub on_weak_visual_after_s: f64,
    pub kind: CutawayKind,
    /// asset_tag: library tag of the image assets to show (e.g. "broll").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// asset_tag: seconds each cutaway shows (default 1.5).
    #[serde(default = "cutaway_s")]
    pub duration: f64,
    /// asset_tag: full (default) | pip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<CutawayMode>,
    /// asset_tag + pip: box on the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
}

/// A CTA or outro card: an image asset (design it with `asset render-card`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TplCard {
    pub asset: String,
    /// Canvas box (default: CTA lower middle, outro the whole canvas).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// CTA only: edit-timeline second it appears (default: 40 % into the clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// Seconds on screen (default CTA 3, outro 2.5; the outro ends with the clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// Brand kit: logo (image layer over the whole clip), CTA card, outro card.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplBrand {
    /// Image asset id of the logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_asset: Option<String>,
    /// Logo box (default top right: x 0.78, y 0.03, w 0.18, h 0.08).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// Logo opacity 0..1 (default 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cta_card: Option<TplCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outro_card: Option<TplCard>,
}

/// Export defaults stored in the clip (extras `export`): `render run` uses `quality` when no
/// --quality is given, `qc run` checks the length window, `fps` sets the canvas fps.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplExport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<RenderQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    /// Longest clip in seconds (warning at apply time, QC check).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<f64>,
    /// Shortest clip in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_len: Option<f64>,
}

/// Posting defaults merged into the clip's posting copy (never replacing what the agent wrote).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct TplPost {
    /// Hashtags added to every platform's copy (with or without `#`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashtags_always: Vec<String>,
    /// Mentions added to every platform's copy (with or without `@`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    /// A line appended to each description that does not contain it yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cta_line: Option<String>,
}

/// extras key: the template's export defaults.
pub const EXPORT_KEY: &str = "export";
/// extras key: the template's posting defaults (merged again by `export.clip`).
pub const POST_DEFAULTS_KEY: &str = "post_defaults";

/// One file the template needs: a library asset or a plain file.
#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct TplRef {
    /// font | caption_style | image | music | sfx.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// JSON path inside the template.
    pub at: String,
}

impl Template {
    /// Every asset / file the template references (built-in caption styles are not listed).
    pub fn refs(&self) -> Vec<TplRef> {
        let mut v = Vec::new();
        let r = |kind: &str, asset: &Option<String>, file: &Option<PathBuf>, at: String| TplRef {
            kind: kind.into(),
            asset: asset.clone(),
            file: if asset.is_some() { None } else { file.clone() },
            at,
        };
        if let Some(p) = self.captions.as_ref().and_then(|c| c.preset.as_ref())
            && rustler_captions::presets::find_preset(p).is_none()
        {
            v.push(TplRef { kind: "caption_style".into(), asset: Some(format!("style-{p}")), file: None, at: "/captions/preset".into() });
        }
        for (k, f) in self.fonts.iter().enumerate() {
            v.push(TplRef { kind: "font".into(), asset: Some(f.clone()), file: None, at: format!("/fonts/{k}") });
        }
        for (k, im) in self.images.iter().flatten().enumerate() {
            v.push(r("image", &im.asset, &im.file, format!("/images/{k}")));
        }
        if let Some(b) = &self.brand {
            if let Some(a) = &b.logo_asset {
                v.push(TplRef { kind: "image".into(), asset: Some(a.clone()), file: None, at: "/brand/logo_asset".into() });
            }
            for (n, c) in [("cta_card", &b.cta_card), ("outro_card", &b.outro_card)] {
                if let Some(c) = c {
                    v.push(TplRef { kind: "image".into(), asset: Some(c.asset.clone()), file: None, at: format!("/brand/{n}/asset") });
                }
            }
        }
        if let Some(a) = &self.audio {
            // Music picked by tag / mood is chosen at apply time (not a fixed reference).
            if let Some(m) = a.music.as_ref().filter(|m| !m.none && (m.asset.is_some() || m.file.is_some())) {
                v.push(r("music", &m.asset, &m.file, "/audio/music".into()));
            }
            for (k, s) in a.sfx.iter().flatten().enumerate() {
                v.push(r("sfx", &s.asset, &s.file, format!("/audio/sfx/{k}")));
            }
        }
        v
    }
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && n.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
}

fn hex_ok(c: &str) -> bool {
    rustler_media::compile::ffmpeg_color(c).is_some()
}

fn rect_ok(r: &Rect) -> bool {
    [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite())
        && r.x >= 0.0
        && r.y >= 0.0
        && r.w > 0.02
        && r.h > 0.02
        && r.x + r.w <= 1.0 + 1e-9
        && r.y + r.h <= 1.0 + 1e-9
}

/// Checks a template's fields (ranges, names, exactly one of asset/file). Errors list JSON paths.
pub fn validate_template(t: &Template) -> Result<()> {
    const STAGE: &str = "template";
    let mut e: Vec<String> = Vec::new();
    if t.schema != TEMPLATE_SCHEMA {
        e.push(format!("/schema: must be \"{TEMPLATE_SCHEMA}\""));
    }
    if !valid_name(&t.name) {
        e.push(format!("/name: '{}' must be 1-64 lowercase letters, digits, '-' or '_'", t.name));
    }
    if t.version == 0 {
        e.push("/version: must be >= 1".into());
    }
    if let Some(c) = &t.canvas
        && (c.width < 2 || c.height < 2 || !(c.fps.is_finite() && c.fps > 0.0 && c.fps <= 120.0))
    {
        e.push(format!("/canvas: invalid {}x{} @ {} fps", c.width, c.height, c.fps));
    }
    if let Some(l) = &t.layout {
        if l.preset.as_deref().is_some_and(|p| p.trim().is_empty()) {
            e.push("/layout/preset: must not be empty".into());
        }
        if let Some(rs) = &l.regions {
            if rs.is_empty() {
                e.push("/layout/regions: give at least one region".into());
            }
            for (k, r) in rs.iter().enumerate() {
                for (n, rect) in [("src", &r.src), ("dst", &r.dst)] {
                    if !rect_ok(rect) {
                        e.push(format!("/layout/regions/{k}/{n}: rect must lie in 0..1 with w, h > 0.02"));
                    }
                }
            }
        }
        if l.preset.as_deref() == Some("split-vertical") && l.regions.is_none() {
            let p = l.params.unwrap_or_default();
            if p.top.is_none() || p.bottom.is_none() {
                e.push("/layout/params: split-vertical needs top and bottom source rects".into());
            }
        }
    }
    if let Some(c) = &t.captions {
        if let Some(y) = c.position_y
            && !(0.0..=1.0).contains(&y)
        {
            e.push("/captions/position_y: must be between 0 and 1".into());
        }
        if let Some(n) = c.max_words_per_line
            && !(1..=12).contains(&n)
        {
            e.push("/captions/max_words_per_line: must be 1..12".into());
        }
        if c.preset.as_deref().is_some_and(|p| p.trim().is_empty()) {
            e.push("/captions/preset: must not be empty".into());
        }
        if let Some(k) = &c.keyword_highlight {
            if k.color.as_deref().is_some_and(|c| !hex_ok(c)) {
                e.push("/captions/keyword_highlight/color: must be #RRGGBB".into());
            }
            if k.max_per_line.is_some_and(|n| !(1..=6).contains(&n)) {
                e.push("/captions/keyword_highlight/max_per_line: must be 1..6".into());
            }
        }
    }
    for (k, r) in t.layout_rules.iter().flatten().enumerate() {
        let p = format!("/layout_rules/{k}");
        if r.layout.is_none() && r.regions.is_none() && r.track.is_none() {
            e.push(format!("{p}: give layout, regions or track"));
        }
        if r.layout.as_deref().is_some_and(|l| l.trim().is_empty()) {
            e.push(format!("{p}/layout: must not be empty"));
        }
        if let Some(rs) = &r.regions {
            if rs.is_empty() {
                e.push(format!("{p}/regions: give at least one region"));
            }
            for (j, reg) in rs.iter().enumerate() {
                if !rect_ok(&reg.src) || !rect_ok(&reg.dst) {
                    e.push(format!("{p}/regions/{j}: rects must lie in 0..1 with w, h > 0.02"));
                }
            }
        }
        if r.layout.as_deref() == Some("split-vertical") && r.regions.is_none() {
            let pp = r.params.unwrap_or_default();
            if pp.top.is_none() || pp.bottom.is_none() {
                e.push(format!("{p}/params: split-vertical needs top and bottom source rects"));
            }
        }
        let crops = r.layout.as_deref().is_none_or(|l| l == "center-crop") && r.regions.is_none();
        if r.track.is_some_and(|tr| tr != RuleTrack::None) && !crops {
            e.push(format!("{p}/track: tracking needs layout center-crop"));
        }
        if t.layout_rules.iter().flatten().take(k).any(|x| x.when == r.when) {
            e.push(format!("{p}/when: an earlier rule already matches {:?}; it would never apply", r.when));
        }
    }
    if t.layout_rules.as_ref().is_some_and(Vec::is_empty) {
        e.push("/layout_rules: give at least one rule (or leave layout_rules out)".into());
    }
    if let Some(c) = &t.cutaway_rules {
        if !(c.on_weak_visual_after_s.is_finite() && (1.0..=30.0).contains(&c.on_weak_visual_after_s)) {
            e.push("/cutaway_rules/on_weak_visual_after_s: must be 1..30 s".into());
        }
        match c.kind {
            CutawayKind::AssetTag if c.tag.as_deref().is_none_or(|t| t.trim().is_empty()) => {
                e.push("/cutaway_rules/tag: kind asset_tag needs the library tag".into())
            }
            CutawayKind::PunchIn if c.tag.is_some() || c.mode.is_some() || c.rect.is_some() => {
                e.push("/cutaway_rules: tag / mode / rect are for kind asset_tag".into())
            }
            _ => {}
        }
        if !(c.duration.is_finite() && (0.5..=10.0).contains(&c.duration)) {
            e.push("/cutaway_rules/duration: must be 0.5..10 s".into());
        }
        if c.rect.as_ref().is_some_and(|r| !rect_ok(r)) {
            e.push("/cutaway_rules/rect: must lie in 0..1 with w, h > 0.02".into());
        }
    }
    if let Some(b) = &t.brand {
        if b.logo_asset.as_deref().is_some_and(|a| a.trim().is_empty()) {
            e.push("/brand/logo_asset: must not be empty".into());
        }
        if b.rect.as_ref().is_some_and(|r| !rect_ok(r)) {
            e.push("/brand/rect: must lie in 0..1 with w, h > 0.02".into());
        }
        if b.opacity.is_some_and(|o| !(0.0..=1.0).contains(&o)) {
            e.push("/brand/opacity: must be 0..1".into());
        }
        if (b.rect.is_some() || b.opacity.is_some()) && b.logo_asset.is_none() {
            e.push("/brand: rect / opacity are the logo's; give logo_asset".into());
        }
        for (n, c) in [("cta_card", &b.cta_card), ("outro_card", &b.outro_card)] {
            let Some(c) = c else { continue };
            let p = format!("/brand/{n}");
            if c.asset.trim().is_empty() {
                e.push(format!("{p}/asset: must not be empty"));
            }
            if c.rect.as_ref().is_some_and(|r| !rect_ok(r)) {
                e.push(format!("{p}/rect: must lie in 0..1 with w, h > 0.02"));
            }
            if c.duration.is_some_and(|d| !(d.is_finite() && (0.5..=30.0).contains(&d))) {
                e.push(format!("{p}/duration: must be 0.5..30 s"));
            }
            if c.at.is_some_and(|a| !(a.is_finite() && a >= 0.0)) {
                e.push(format!("{p}/at: must be >= 0"));
            }
            if n == "outro_card" && c.at.is_some() {
                e.push(format!("{p}/at: the outro card ends with the clip; give duration instead"));
            }
        }
    }
    if let Some(x) = &t.export {
        if x.fps.is_some_and(|f| !(f.is_finite() && f > 0.0 && f <= 120.0)) {
            e.push("/export/fps: must be > 0 and <= 120".into());
        }
        for (n, v) in [("max_len", x.max_len), ("min_len", x.min_len)] {
            if v.is_some_and(|v| !(v.is_finite() && v > 0.0 && v <= 3600.0)) {
                e.push(format!("/export/{n}: must be > 0 s"));
            }
        }
        if let (Some(a), Some(b)) = (x.min_len, x.max_len)
            && a > b
        {
            e.push("/export: min_len must be <= max_len".into());
        }
    }
    if let Some(p) = &t.post_defaults {
        for (n, list) in [("hashtags_always", &p.hashtags_always), ("mentions", &p.mentions)] {
            for (k, s) in list.iter().enumerate() {
                let s = s.trim().trim_start_matches(['#', '@']);
                if s.is_empty() || s.chars().any(char::is_whitespace) {
                    e.push(format!("/post_defaults/{n}/{k}: one word, no spaces"));
                }
            }
        }
        if p.cta_line.as_deref().is_some_and(|c| c.trim().is_empty() || c.chars().count() > 300) {
            e.push("/post_defaults/cta_line: 1..300 characters".into());
        }
    }
    let look = |p: &str, y: Option<f64>, f: Option<f64>, e: &mut Vec<String>| {
        if let Some(y) = y
            && !(0.0..=1.0).contains(&y)
        {
            e.push(format!("{p}/position_y: must be between 0 and 1"));
        }
        if let Some(f) = f
            && !(f.is_finite() && (0.01..=0.2).contains(&f))
        {
            e.push(format!("{p}/font_size_factor: must be between 0.01 and 0.2"));
        }
    };
    if let Some(h) = &t.hook {
        look("/hook", h.position_y, h.font_size_factor, &mut e);
        if let Some(d) = h.duration
            && !(d.is_finite() && d > 0.2 && d <= 30.0)
        {
            e.push("/hook/duration: must be between 0.2 and 30 s".into());
        }
        if h.max_words == Some(0) {
            e.push("/hook/max_words: must be >= 1".into());
        }
        if let Some(tx) = &h.text
            && let Err(x) = crate::overlay_cmds::check_text(STAGE, tx)
        {
            e.push(format!("/hook/text: {}", x.message));
        }
    }
    for (k, l) in &t.text_styles {
        if !matches!(k.as_str(), "lower-third" | "plain") {
            e.push(format!("/text_styles/{k}: style must be lower-third or plain (the hook-top look is /hook)"));
        }
        look(&format!("/text_styles/{k}"), l.position_y, l.font_size_factor, &mut e);
    }
    for (k, x) in t.texts.iter().flatten().enumerate() {
        let p = format!("/texts/{k}");
        if let Err(m) = crate::overlay_cmds::check_text(STAGE, &x.text) {
            e.push(format!("{p}/text: {}", m.message));
        }
        if !(x.start.is_finite() && x.end.is_finite() && x.start >= 0.0 && x.end > x.start) {
            e.push(format!("{p}: needs 0 <= start < end"));
        }
        look(&p, x.position_y, x.font_size_factor, &mut e);
    }
    let one_of = |p: &str, asset: &Option<String>, file: &Option<PathBuf>, e: &mut Vec<String>| match (asset, file) {
        (Some(a), _) if a.trim().is_empty() => e.push(format!("{p}/asset: must not be empty")),
        (None, None) => e.push(format!("{p}: give asset (library id) or file")),
        _ => {}
    };
    for (k, im) in t.images.iter().flatten().enumerate() {
        let p = format!("/images/{k}");
        one_of(&p, &im.asset, &im.file, &mut e);
        if !rect_ok(&im.rect) {
            e.push(format!("{p}/rect: must lie in 0..1 with w, h > 0.02"));
        }
        if let Some(o) = im.opacity
            && !(0.0..=1.0).contains(&o)
        {
            e.push(format!("{p}/opacity: must be 0..1"));
        }
        if let Some(s) = im.scale
            && !(s.is_finite() && s > 0.0 && s <= 4.0)
        {
            e.push(format!("{p}/scale: must be > 0 and <= 4"));
        }
        if let (Some(a), Some(b)) = (im.start, im.end)
            && b <= a
        {
            e.push(format!("{p}: end must be after start"));
        }
        if im.start.is_some_and(|s| !(s.is_finite() && s >= 0.0)) {
            e.push(format!("{p}/start: must be >= 0"));
        }
    }
    if let Some(b) = &t.progress_bar {
        if b.color.as_deref().is_some_and(|c| !hex_ok(c)) {
            e.push("/progress_bar/color: must be #RRGGBB".into());
        }
        if b.height.is_some_and(|h| !(2..=200).contains(&h)) {
            e.push("/progress_bar/height: must be 2..200".into());
        }
    }
    let place = |p: &str, at: PlaceAt, every: Option<f64>, tt: Option<f64>, e: &mut Vec<String>| match at {
        PlaceAt::Every if !every.is_some_and(|s| s.is_finite() && s >= 1.0) => e.push(format!("{p}/every_s: at=every needs every_s >= 1")),
        PlaceAt::Time if !tt.is_some_and(|s| s.is_finite() && s >= 0.0) => e.push(format!("{p}/t: at=time needs t >= 0")),
        PlaceAt::Emphasis if p.starts_with("/audio") => e.push(format!("{p}/at: emphasis is for punch-in effects only")),
        _ => {}
    };
    if let Some(a) = &t.audio {
        for (n, v) in [("target_lufs", a.target_lufs), ("true_peak", a.true_peak), ("lra", a.lra)] {
            if v.is_some_and(|v| !v.is_finite()) {
                e.push(format!("/audio/{n}: must be a number"));
            }
        }
        if a.volume_db.is_some_and(|v| !(v.is_finite() && (-60.0..=24.0).contains(&v))) {
            e.push("/audio/volume_db: must be between -60 and 24".into());
        }
        if let Some(m) = &a.music {
            let picks = [m.tag.as_deref(), m.mood.as_deref()];
            if picks.iter().flatten().any(|s| s.trim().is_empty()) {
                e.push("/audio/music: tag / mood must not be empty".into());
            }
            let by_tag = picks.iter().any(Option::is_some);
            if m.none && (by_tag || m.asset.is_some() || m.file.is_some()) {
                e.push("/audio/music: none removes the music; do not give asset, file, tag or mood with it".into());
            } else if by_tag && (m.asset.is_some() || m.file.is_some()) {
                e.push("/audio/music: give asset, file or tag/mood, not both".into());
            } else if !m.none && !by_tag {
                one_of("/audio/music", &m.asset, &m.file, &mut e);
            }
            if !(m.volume_db.is_finite() && (-60.0..=6.0).contains(&m.volume_db)) {
                e.push("/audio/music/volume_db: must be between -60 and 6".into());
            }
            if !((0.0..=30.0).contains(&m.fade_in) && (0.0..=30.0).contains(&m.fade_out)) {
                e.push("/audio/music: fades must be 0..30 s".into());
            }
        }
        for (k, s) in a.sfx.iter().flatten().enumerate() {
            let p = format!("/audio/sfx/{k}");
            one_of(&p, &s.asset, &s.file, &mut e);
            place(&p, s.at, s.every_s, s.t, &mut e);
            if !(s.volume_db.is_finite() && (-60.0..=12.0).contains(&s.volume_db)) {
                e.push(format!("{p}/volume_db: must be between -60 and 12"));
            }
        }
    }
    for (k, x) in t.effects.iter().flatten().enumerate() {
        let p = format!("/effects/{k}");
        place(&p, x.at, x.every_s, x.t, &mut e);
        if !(x.duration.is_finite() && x.duration >= 0.1 && x.duration <= 30.0) {
            e.push(format!("{p}/duration: must be 0.1..30 s"));
        }
        let (lo, hi) = x.kind.amount_range();
        if let Some(a) = x.amount
            && !(a.is_finite() && (lo..=hi).contains(&a))
        {
            e.push(format!("{p}/amount: must be between {lo} and {hi} for {:?}", x.kind));
        }
        if x.at == PlaceAt::Emphasis && x.kind != EffectKind::PunchIn {
            e.push(format!("{p}/at: emphasis is for punch-in effects only"));
        }
        if (x.amount_max.is_some() || x.per_10s.is_some()) && x.kind != EffectKind::PunchIn {
            e.push(format!("{p}: amount_max / per_10s apply to planned punch-ins only"));
        }
        if let Some(a) = x.amount_max
            && !(a.is_finite() && (lo..=hi).contains(&a) && x.amount.is_none_or(|m| a >= m))
        {
            e.push(format!("{p}/amount_max: must be between {lo} and {hi} and >= amount"));
        }
        if x.per_10s.is_some_and(|v| !(v.is_finite() && (1.0..=10.0).contains(&v))) {
            e.push(format!("{p}/per_10s: must be 1..10"));
        }
    }
    if let Some(tr) = &t.transitions
        && tr.kind != TransitionKind::Cut
        && !(tr.duration.is_finite() && tr.duration > 0.0 && tr.duration <= 2.0)
    {
        e.push("/transitions/duration: must be > 0 and <= 2 s".into());
    }
    for (k, f) in t.fonts.iter().enumerate() {
        if f.trim().is_empty() {
            e.push(format!("/fonts/{k}: must not be empty"));
        }
    }
    if e.is_empty() {
        Ok(())
    } else {
        Err(verr(STAGE, format!("template '{}' is invalid: {}", t.name, e.join("; "))).hint("`rustler template show podcast-viral` shows a valid template"))
    }
}

/// Effect kinds share the media crate's amount ranges.
trait AmountRange {
    fn amount_range(&self) -> (f64, f64);
}
impl AmountRange for EffectKind {
    fn amount_range(&self) -> (f64, f64) {
        match self {
            EffectKind::PunchIn | EffectKind::ZoomSlow => (1.01, 2.0),
            EffectKind::Shake => (0.002, 0.08),
            EffectKind::Flash => (0.0, 1.0),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------------------------

/// A stored or built-in template.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Stored {
    pub name: String,
    pub version: u32,
    /// project | global | built-in.
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(skip)]
    pub template: Option<Template>,
}

fn dir_of(ctx: &Ctx, scope: Scope) -> Result<PathBuf> {
    match scope {
        Scope::Global => Ok(ctx.home.join("templates")),
        Scope::Project => ctx
            .project_dir
            .as_ref()
            .map(|p| p.join("templates"))
            .ok_or_else(|| nf("template", "no project is open for scope \"project\"").hint("pass --scope global, or --project <dir>")),
    }
}

fn scope_label(s: Scope) -> &'static str {
    match s {
        Scope::Project => "project",
        Scope::Global => "global",
    }
}

fn default_scope(ctx: &Ctx) -> Scope {
    if ctx.project_dir.is_some() { Scope::Project } else { Scope::Global }
}

/// Built-in starter templates.
pub fn builtin_templates() -> Vec<Template> {
    STARTERS
        .iter()
        .map(|(n, body)| serde_json::from_str::<Template>(body).unwrap_or_else(|e| panic!("built-in template {n} does not parse: {e}")))
        .collect()
}

/// `(name, version)` from `<name>@<version>.json`.
fn parse_file_name(p: &Path) -> Option<(String, u32)> {
    if p.extension().and_then(|x| x.to_str()) != Some("json") {
        return None;
    }
    let stem = p.file_stem()?.to_str()?;
    let (n, v) = stem.rsplit_once('@')?;
    Some((n.to_string(), v.parse().ok()?))
}

fn scan(dir: &Path, scope: &str) -> Vec<Stored> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Stored> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let (name, version) = parse_file_name(&p)?;
            Some(Stored { name, version, scope: scope.into(), file: Some(p), template: None })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
    out
}

/// Every template: project, then global, then built-in (each sorted by name, version).
fn all_stored(ctx: &Ctx) -> Vec<Stored> {
    let mut out = Vec::new();
    if let Ok(d) = dir_of(ctx, Scope::Project) {
        out.extend(scan(&d, "project"));
    }
    out.extend(scan(&ctx.home.join("templates"), "global"));
    for t in builtin_templates() {
        out.push(Stored { name: t.name.clone(), version: t.version, scope: "built-in".into(), file: None, template: Some(t) });
    }
    out
}

fn read_template(stage: &str, p: &Path) -> Result<Template> {
    let bytes = std::fs::read(p).map_err(|e| io_err(stage, "read", p, e))?;
    serde_json::from_slice(&bytes).map_err(|e| verr(stage, format!("{} is not a valid template: {e}", p.display())).hint("fix or delete the file"))
}

fn load(stage: &str, s: &Stored) -> Result<Template> {
    match (&s.template, &s.file) {
        (Some(t), _) => Ok(t.clone()),
        (None, Some(f)) => read_template(stage, f),
        (None, None) => Err(RustlerError::new(ErrorCode::Internal, stage, "template without file")),
    }
}

/// `name` or `name@version`.
fn split_spec(stage: &str, spec: &str) -> Result<(String, Option<u32>)> {
    let spec = spec.trim();
    match spec.rsplit_once('@') {
        Some((n, v)) => {
            let v: u32 = v.parse().map_err(|_| verr(stage, format!("'{spec}': version after @ must be a number")).hint("e.g. podcast-viral@2"))?;
            Ok((n.to_string(), Some(v)))
        }
        None => Ok((spec.to_string(), None)),
    }
}

/// Resolves `name[@ver]`: the first scope (project, global, built-in) that has the name wins; its
/// highest version unless one is given (an explicit version is looked up in every scope).
pub fn resolve(ctx: &Ctx, stage: &str, spec: &str) -> Result<(Stored, Template)> {
    let (name, ver) = split_spec(stage, spec)?;
    let all = all_stored(ctx);
    let same: Vec<&Stored> = all.iter().filter(|s| s.name == name).collect();
    let pick = match ver {
        Some(v) => same.iter().find(|s| s.version == v).copied(),
        None => {
            let first_scope = same.first().map(|s| s.scope.clone());
            same.iter().filter(|s| Some(&s.scope) == first_scope.as_ref()).max_by_key(|s| s.version).copied()
        }
    };
    let Some(s) = pick else {
        let mut names: Vec<String> = all.iter().map(|s| s.name.clone()).collect();
        names.dedup();
        let msg = match (ver, same.is_empty()) {
            (Some(v), false) => format!("template '{name}' has no version {v} (versions: {})", same.iter().map(|s| s.version.to_string()).collect::<Vec<_>>().join(", ")),
            _ => format!("no template called '{name}'"),
        };
        return Err(nf(stage, msg).hint(format!("templates: {} (`rustler template list`)", names.join(", "))));
    };
    let t = load(stage, s)?;
    Ok((s.clone(), t))
}

fn write_template(stage: &str, dir: &Path, t: &Template) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| io_err(stage, "create", dir, e))?;
    let path = dir.join(format!("{}@{}.json", t.name, t.version));
    let tmp = dir.join(format!(".{}@{}.json.tmp{}", t.name, t.version, std::process::id()));
    let body = serde_json::to_vec_pretty(t).map_err(|e| RustlerError::new(ErrorCode::Internal, stage, e.to_string()))?;
    std::fs::write(&tmp, body).map_err(|e| io_err(stage, "write", &tmp, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(stage, "write", &path, e)
    })?;
    Ok(path)
}

fn latest_in(ctx: &Ctx, name: &str) -> Option<Stored> {
    let all = all_stored(ctx);
    let first_scope = all.iter().find(|s| s.name == name).map(|s| s.scope.clone())?;
    all.into_iter().filter(|s| s.name == name && s.scope == first_scope).max_by_key(|s| s.version)
}

fn parse_template_value(stage: &str, v: Value) -> Result<Template> {
    serde_json::from_value(v).map_err(|e| {
        verr(stage, format!("template JSON does not match rustler.template@1: {e}"))
            .hint("`rustler template show podcast-viral` shows the shape; unknown fields are rejected")
    })
}

// ---------------------------------------------------------------------------------------------
// Capturing a clip's look
// ---------------------------------------------------------------------------------------------

const NEAR: f64 = 0.06;

fn cut_times(t: &Timeline) -> Vec<f64> {
    let mut acc = 0.0;
    let mut out = Vec::new();
    let n = t.tracks.video.len();
    for (k, s) in t.tracks.video.iter().enumerate() {
        acc += (s.src_out - s.src_in).max(0.0);
        if k + 1 < n {
            out.push(acc);
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// Library rows of `kind` whose file is `path` (so a template stores the asset id, not the path).
fn asset_id_for(rows: &[asset_cmds::AssetRow], kind: AssetKind, path: &Path) -> Option<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let want = canon(path);
    rows.iter().find(|r| r.entry.kind == kind && r.file.as_ref().is_some_and(|f| canon(f) == want)).map(|r| r.entry.id.clone())
}

/// The look of a clip as a template (text content only with `include_text`). Returns notes on
/// what was not captured.
pub fn capture(ctx: &Ctx, store: &ProjectStore, doc: &Timeline, name: &str, include_text: bool) -> Result<(Template, Vec<String>)> {
    let rows = asset_cmds::load_rows(&ctx.home, Some(&store.root));
    let tr = &doc.tracks;
    let dur = doc.edit_duration();
    let cuts = cut_times(doc);
    let mut notes: Vec<String> = Vec::new();

    // Layout: the first segment's; center-crop / blur-fit framing is per source, so only the preset.
    let layout = tr.video.first().map(|s| {
        let preset = s.layout.preset.clone();
        let per_source = matches!(preset.as_deref(), Some("center-crop" | "blur-fit"));
        if tr.video.iter().any(|v| v.layout.preset != preset) {
            notes.push(format!("segments use different layouts; captured the first segment's ({})", preset.as_deref().unwrap_or("custom regions")));
        }
        TplLayout {
            preset: preset.clone(),
            params: None,
            regions: if per_source { None } else { Some(s.layout.regions.clone()) },
            background: Some(s.layout.background),
            force: false,
        }
    });

    let cs = &tr.captions.settings;
    let mut captions = TplCaptions {
        enabled: Some(cs.enabled),
        preset: Some(cs.preset.clone()),
        position_y: Some(cs.position_y),
        max_words_per_line: Some(cs.max_words_per_line),
        script: None,
        speaker_colors: tr.captions.speaker_colors.clone(),
        keyword_highlight: None,
        emoji: None,
    };
    if !tr.captions.word_overrides.is_empty() {
        notes.push(format!("{} caption word overrides are content, not captured", tr.captions.word_overrides.len()));
    }
    // Fonts of a custom caption style that live in the asset library.
    let mut fonts: Vec<String> = Vec::new();
    if rustler_captions::presets::find_preset(&cs.preset).is_none() {
        let custom = asset_cmds::custom_presets(&ctx.home, Some(&store.root));
        if let Some(p) = custom.iter().find(|p| p.name == cs.preset) {
            let fam = norm(p.font_family);
            for r in rows.iter().filter(|r| r.entry.kind == AssetKind::Font && r.exists) {
                let stem = r.file.as_ref().and_then(|f| f.file_stem()).map(|s| norm(&s.to_string_lossy())).unwrap_or_default();
                if !fam.is_empty() && (stem.contains(&fam) || norm(&r.entry.name).contains(&fam)) && !fonts.contains(&r.entry.id) {
                    fonts.push(r.entry.id.clone());
                }
            }
        }
    }

    // Hook: the first hook title (hook-top or a viral hook style).
    let hook_item = tr.text.iter().find(|x| x.style.is_hook());
    let hook = hook_item.map(|h| TplHook {
        position_y: Some(h.position_y),
        font_size_factor: Some(h.font_size_factor),
        boxed: Some(h.boxed),
        at_seam: None,
        duration: Some(((h.end - h.start) * 100.0).round() / 100.0),
        max_words: Some(7),
        text: include_text.then(|| h.text.clone()),
    });
    let mut text_styles = BTreeMap::new();
    let mut texts = Vec::new();
    for x in tr.text.iter().filter(|x| Some(*x) != hook_item) {
        let key = x.style.name();
        if !x.style.is_hook() && !text_styles.contains_key(key) {
            text_styles.insert(key.to_string(), TextLook { position_y: Some(x.position_y), font_size_factor: Some(x.font_size_factor), boxed: Some(x.boxed) });
        }
        if include_text {
            texts.push(TplText {
                text: x.text.clone(),
                start: x.start,
                end: x.end,
                style: x.style,
                position_y: Some(x.position_y),
                font_size_factor: Some(x.font_size_factor),
                boxed: Some(x.boxed),
            });
        }
    }
    let other_text = tr.text.iter().filter(|x| Some(*x) != hook_item).count();
    if !include_text && (other_text > 0 || hook_item.is_some()) {
        notes.push("text content (hook and labels) not captured; pass include_text to keep it".into());
    }

    // Brand images (time-anchored ones only; word-anchored images belong to the content). Library
    // images become the brand kit when they look like one: tag / role logo, cta, outro, or by
    // place (a small image over the whole clip = logo; one that runs to the end from past the
    // middle = outro card).
    let mut images = Vec::new();
    let mut brand = TplBrand::default();
    for im in &tr.images {
        if im.timing.has_anchor() {
            notes.push(format!("image {} is word-anchored (content), not captured", im.id));
            continue;
        }
        let asset = im.asset.clone().or_else(|| asset_id_for(&rows, AssetKind::Image, &im.file));
        if let Some(a) = &asset {
            let tags: Vec<String> = rows.iter().filter(|r| &r.entry.id == a).flat_map(|r| r.entry.tags.iter().map(|t| t.to_lowercase())).collect();
            let role = im.extra.get(ROLE).and_then(Value::as_str).unwrap_or("");
            let has = |k: &str| role == format!("brand_{k}") || tags.iter().any(|t| t == k) || a.to_lowercase().contains(k);
            let start = im.timing.out_start.unwrap_or(0.0);
            let to_end = im.timing.out_end.is_none_or(|e| e >= dur - NEAR);
            let whole = start <= NEAR && to_end;
            let small = im.rect.w * im.rect.h <= 0.12;
            let r2 = |x: f64| (x * 100.0).round() / 100.0;
            if brand.logo_asset.is_none() && (has("logo") || (whole && small && role.is_empty())) && whole {
                brand.logo_asset = Some(a.clone());
                brand.rect = Some(im.rect);
                brand.opacity = Some(im.opacity);
                continue;
            }
            if brand.outro_card.is_none() && to_end && start > dur * 0.5 && (has("outro") || !small) {
                brand.outro_card = Some(TplCard { asset: a.clone(), rect: Some(im.rect), at: None, duration: Some(r2(dur - start)) });
                continue;
            }
            if brand.cta_card.is_none() && has("cta") && !whole {
                let end = im.timing.out_end.unwrap_or(dur);
                brand.cta_card = Some(TplCard { asset: a.clone(), rect: Some(im.rect), at: Some(r2(start)), duration: Some(r2(end - start)) });
                continue;
            }
        }
        images.push(TplImage {
            file: if asset.is_some() { None } else { Some(im.file.clone()) },
            asset,
            rect: im.rect,
            scale: Some(im.scale),
            opacity: Some(im.opacity),
            rotation: (im.rotation != 0.0).then_some(im.rotation),
            fit: Some(im.fit),
            anim: (im.anim != Anim::default()).then_some(im.anim),
            z: (im.z != 0).then_some(im.z),
            above_captions: im.above_captions.then_some(true),
            start: im.timing.out_start.filter(|s| *s > NEAR),
            end: im.timing.out_end.filter(|e| *e < dur - NEAR),
        });
    }
    // Cutaways of tagged library images become an asset_tag rule (the most common tag).
    let mut tag_count: BTreeMap<String, usize> = BTreeMap::new();
    for c in &tr.cutaways {
        let Some(a) = &c.media.asset else { continue };
        for r in rows.iter().filter(|r| &r.entry.id == a && r.entry.kind == AssetKind::Image) {
            for tg in r.entry.tags.iter().filter(|t| !t.contains(':')) {
                *tag_count.entry(tg.to_lowercase()).or_default() += 1;
            }
        }
    }
    let cutaway_rules = tag_count.iter().max_by_key(|(t, n)| (**n, std::cmp::Reverse((*t).clone()))).map(|(tg, _)| {
        let first = tr.cutaways.iter().find(|c| c.media.asset.is_some());
        let len = first.and_then(|c| Some(c.timing.out_end? - c.timing.out_start?)).filter(|l| (0.5..=10.0).contains(l)).unwrap_or(cutaway_s());
        notes.push(format!("cutaway_rules: library cutaways tagged '{tg}' captured as an asset_tag rule; on_weak_visual_after_s 3 s is a default, change it to taste"));
        TplCutawayRules {
            on_weak_visual_after_s: 3.0,
            kind: CutawayKind::AssetTag,
            tag: Some(tg.clone()),
            duration: (len * 100.0).round() / 100.0,
            mode: first.map(|c| c.mode),
            rect: first.filter(|c| c.mode == CutawayMode::Pip).and_then(|c| c.rect),
        }
    });
    for (n, k) in [("masks", tr.masks.len()), ("cutaways", tr.cutaways.len()), ("freezes", tr.freezes.len())] {
        if k > 0 {
            notes.push(format!("{k} {n} are content, not captured as items"));
        }
    }

    // Per-shot layout rules from the smart defaults' shot plan or faces.json.
    let shots = shot_infos(store, doc);
    let mut layout_rules: Vec<LayoutRule> = Vec::new();
    for v in &tr.video {
        let Some(s) = shots.get(v.id.as_str()) else { continue };
        let when = if s.class == ShotClass::NoFace && s.zoom_fit { ShotWhen::Screen } else { when_of(s.class) };
        if layout_rules.iter().any(|r| r.when == when) {
            continue;
        }
        let preset = v.layout.preset.clone();
        let builtin = preset.as_deref().is_some_and(|p| core_layout::PRESETS.contains(&p) || p == "zoom-fit");
        let center = preset.as_deref() == Some("center-crop");
        layout_rules.push(LayoutRule {
            when,
            layout: preset.clone(),
            params: None,
            regions: (!builtin || preset.as_deref() == Some("split-vertical")).then(|| v.layout.regions.clone()).filter(|r| !r.is_empty()),
            background: (!center).then_some(v.layout.background),
            track: center.then(|| match (v.crop_path.is_some(), s.class) {
                (false, _) => RuleTrack::None,
                (true, ShotClass::MultiFace) => RuleTrack::Active,
                (true, _) => RuleTrack::Face,
            }),
            look_room: None,
        });
    }
    if !layout_rules.is_empty() && shots.len() < tr.video.len() {
        notes.push(format!("layout_rules: {} of {} segments had shot data", shots.len(), tr.video.len()));
    }

    // Caption keywords (coloured word overrides) and emoji.
    let mut keyword_highlight = None;
    let coloured: Vec<&WordOverride> = tr.captions.word_overrides.iter().filter(|o| o.color.is_some()).collect();
    if !coloured.is_empty() {
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for o in &coloured {
            *by.entry(o.color.clone().unwrap_or_default().to_uppercase()).or_default() += 1;
        }
        let color = by.iter().max_by_key(|(_, n)| **n).map(|(c, _)| c.clone());
        let wpl = cs.max_words_per_line.max(1) as usize;
        let per_line = crate::keyword_cmds::clip_words(store, &TimeMap::from_segments(&doc_segments(doc)))
            .ok()
            .map(|words| {
                let mut per: HashMap<usize, u32> = HashMap::new();
                for o in &coloured {
                    if let Some(k) = words.iter().position(|w| w.source_id == o.word_anchor.source_id && w.word_i == o.word_anchor.word_i) {
                        *per.entry(k / wpl).or_default() += 1;
                    }
                }
                per.values().copied().max().unwrap_or(1).clamp(1, 6)
            })
            .unwrap_or(1);
        keyword_highlight = Some(TplKeywords { color, max_per_line: Some(per_line) });
        notes.push("keyword_highlight: the words are picked again per clip by the caption keywords rules".into());
    }
    let emoji = (tr.captions.word_overrides.iter().any(|o| o.emoji.is_some()) || cs.preset == "emoji-karaoke").then_some(OnOff::On);

    // Export defaults: the clip's stored ones, else fps + the last render's quality.
    let stored_export: Option<TplExport> = tr.extra.get(EXPORT_KEY).cloned().and_then(|mut v| {
        if let Some(o) = v.as_object_mut() {
            o.remove(BY_TEMPLATE);
        }
        serde_json::from_value(v).ok()
    });
    let last_quality = store.load_clip(&doc.id).ok().and_then(|c| c.renders.last().map(|r| r.quality));
    let export = Some(match stored_export {
        Some(x) => x,
        None => TplExport { quality: last_quality, fps: Some(doc.canvas.fps), max_len: None, min_len: None },
    });

    // Posting defaults: the clip's stored ones, else hashtags / mentions every written platform shares.
    let post_defaults = tr.extra.get(POST_DEFAULTS_KEY).cloned().and_then(|mut v| {
        if let Some(o) = v.as_object_mut() {
            o.remove(BY_TEMPLATE);
        }
        serde_json::from_value::<TplPost>(v).ok()
    });
    let post_defaults = post_defaults.or_else(|| {
        let pl = doc.meta.post.as_ref()?.get("platforms")?.as_object()?;
        let lists = |k: &str| -> Vec<Vec<String>> {
            pl.values().filter(|c| c.is_object()).map(|c| c.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()).collect()
        };
        let shared = |ls: Vec<Vec<String>>| -> Vec<String> {
            let Some(first) = ls.first() else { return Vec::new() };
            first.iter().filter(|t| ls.iter().all(|l| l.iter().any(|x| tag_norm(x) == tag_norm(t)))).cloned().collect()
        };
        let (h, m) = (shared(lists("hashtags")), shared(lists("mentions")));
        (!h.is_empty() || !m.is_empty()).then(|| {
            notes.push("post_defaults: hashtags / mentions every platform's copy shares were captured".into());
            TplPost { hashtags_always: h, mentions: m, cta_line: None }
        })
    });

    let progress_bar = Some(match &tr.progress_bar {
        Some(b) => TplBar { enabled: true, color: Some(b.color.clone()), height: Some(b.height), position: Some(b.position) },
        None => TplBar { enabled: false, color: None, height: None, position: None },
    });

    // Patterns: at the start, at cuts, otherwise content-specific.
    let place_of = |s: f64| -> Option<PlaceAt> {
        if s < NEAR {
            Some(PlaceAt::Start)
        } else if cuts.iter().any(|c| (c - s).abs() < NEAR) {
            Some(PlaceAt::Cuts)
        } else {
            None
        }
    };
    let mut effects: Vec<TplEffect> = Vec::new();
    let mut loose_fx = 0;
    for x in &tr.effects {
        match place_of(x.start) {
            Some(at) => {
                if !effects.iter().any(|e| e.kind == x.kind && e.at == at) {
                    effects.push(TplEffect {
                        kind: x.kind,
                        at,
                        every_s: None,
                        t: None,
                        duration: ((x.end - x.start) * 100.0).round() / 100.0,
                        amount: x.amount,
                        amount_max: None,
                        per_10s: None,
                    });
                }
            }
            None => loose_fx += 1,
        }
    }
    if loose_fx > 0 {
        notes.push(format!("{loose_fx} effects at content times (not at the start or a cut) not captured; add an at=every pattern by hand if they were regular"));
    }
    let a = &tr.audio;
    let music = a.music.as_ref().map(|m| {
        let asset = asset_id_for(&rows, AssetKind::Music, &m.path);
        TplMusic {
            file: if asset.is_some() { None } else { Some(m.path.clone()) },
            asset,
            tag: None,
            mood: None,
            none: false,
            volume_db: m.volume_db,
            duck: m.duck,
            fade_in: m.fade_in,
            fade_out: m.fade_out,
        }
    });
    let mut sfx: Vec<TplSfx> = Vec::new();
    let mut loose_sfx = 0;
    for s in &a.sfx {
        let at = match (s.at, &s.at_word) {
            (Some(t), None) => place_of(t),
            _ => None,
        };
        let Some(at) = at else {
            loose_sfx += 1;
            continue;
        };
        let asset = asset_id_for(&rows, AssetKind::Sfx, &s.file);
        let item = TplSfx { file: if asset.is_some() { None } else { Some(s.file.clone()) }, asset, at, every_s: None, t: None, volume_db: s.volume_db };
        if !sfx.iter().any(|x| x.at == item.at && x.asset == item.asset && x.file == item.file) {
            sfx.push(item);
        }
    }
    if loose_sfx > 0 {
        notes.push(format!("{loose_sfx} sound effects at content times or words not captured"));
    }
    let audio = TplAudio {
        loudnorm: Some(a.master.loudnorm),
        target_lufs: Some(a.master.target_lufs),
        true_peak: Some(a.master.true_peak),
        lra: Some(a.master.lra),
        denoise: Some(a.denoise),
        volume_db: (a.volume_db != 0.0).then_some(a.volume_db),
        music,
        sfx: Some(sfx),
    };
    let transitions = {
        let kinds: Vec<(TransitionKind, f64)> = tr.transitions.iter().map(|x| (x.kind, x.duration)).collect();
        match kinds.first() {
            Some(&(k, d)) => {
                if kinds.iter().any(|x| x.0 != k) {
                    notes.push("transitions differ between seams; captured the first one".into());
                }
                Some(TplTransition { kind: k, duration: d })
            }
            None => None,
        }
    };
    captions.keyword_highlight = keyword_highlight;
    captions.emoji = emoji;
    let t = Template {
        schema: TEMPLATE_SCHEMA.into(),
        name: name.into(),
        version: 1,
        description: String::new(),
        created_by: "agent".into(),
        based_on: Some(json!({"clip": doc.id})),
        notes: Vec::new(),
        canvas: Some(doc.canvas),
        layout,
        captions: Some(captions),
        hook,
        text_styles,
        texts: include_text.then_some(texts),
        images: Some(images),
        progress_bar,
        audio: Some(audio),
        effects: Some(effects),
        transitions,
        fonts,
        layout_rules: (!layout_rules.is_empty()).then_some(layout_rules),
        cutaway_rules,
        brand: (brand != TplBrand::default()).then_some(brand),
        export,
        post_defaults,
    };
    Ok((t, notes))
}

// ---------------------------------------------------------------------------------------------
// Applying a template to a document
// ---------------------------------------------------------------------------------------------

/// Library paths of every asset / file the template references; `not_found` naming the first
/// missing one.
pub fn check_refs(ctx: &Ctx, stage: &str, t: &Template) -> Result<HashMap<String, PathBuf>> {
    let project = ctx.project_dir.as_deref();
    let mut out = HashMap::new();
    let hint = "import the template bundle (`rustler template import <file.zip>`) or add the file with `rustler asset add <file> --kind <kind>`";
    let rows = asset_cmds::load_rows(&ctx.home, project);
    for r in t.refs() {
        match (&r.asset, &r.file) {
            (Some(id), _) if r.kind == "caption_style" => {
                let name = id.trim_start_matches("style-");
                let custom = asset_cmds::custom_presets(&ctx.home, project);
                if rustler_captions::find_preset_in(name, &custom).is_none() {
                    return Err(nf(stage, format!("template '{}' needs caption style '{name}' (asset {id}), which is not in the project or global library ({})", t.name, r.at)).hint(hint));
                }
            }
            (Some(id), _) if r.kind == "font" => {
                let ok = rows.iter().any(|x| &x.entry.id == id && x.entry.kind == AssetKind::Font && x.exists);
                if !ok {
                    return Err(nf(stage, format!("template '{}' needs font asset '{id}', which is not in the project or global library ({})", t.name, r.at)).hint(hint));
                }
            }
            (Some(id), _) => {
                let p = asset_cmds::resolve_asset_path_in(project, &ctx.home, &format!("asset:{id}")).map_err(|e| {
                    nf(stage, format!("template '{}' needs {} asset '{id}' ({}): {}", t.name, r.kind, r.at, e.message)).hint(hint)
                })?;
                out.insert(format!("asset:{id}"), p);
            }
            (None, Some(f)) => {
                if !f.is_file() {
                    return Err(nf(stage, format!("template '{}' needs {} file '{}' ({}), which does not exist", t.name, r.kind, f.display(), r.at)).hint(hint));
                }
                out.insert(f.to_string_lossy().into_owned(), f.clone());
            }
            (None, None) => {}
        }
    }
    Ok(out)
}

fn path_of(paths: &HashMap<String, PathBuf>, asset: &Option<String>, file: &Option<PathBuf>) -> Option<PathBuf> {
    match (asset, file) {
        (Some(a), _) => paths.get(&format!("asset:{a}")).cloned(),
        (None, Some(f)) => Some(std::path::absolute(f).unwrap_or_else(|_| f.clone())),
        _ => None,
    }
}

fn instances(at: PlaceAt, every: Option<f64>, t: Option<f64>, cuts: &[f64], dur: f64) -> Vec<f64> {
    match at {
        PlaceAt::Start => vec![0.0],
        PlaceAt::Cuts => cuts.to_vec(),
        PlaceAt::Time => t.filter(|t| *t < dur).into_iter().collect(),
        // Planned by the punch-in planner (see `build`), not a fixed grid.
        PlaceAt::Emphasis => Vec::new(),
        PlaceAt::Every => {
            let step = every.unwrap_or(5.0).max(1.0);
            let mut v = Vec::new();
            let mut s = step;
            while s < dur - 0.5 {
                v.push(s);
                s += step;
            }
            v
        }
    }
}

/// Planner settings for a punch-in pattern: `emphasis` uses the planner's pacing; `every` keeps
/// its spacing (`every_s` between starts) but is snapped and guarded by the planner too.
pub fn punch_config(e: &TplEffect) -> rustler_media::punchin::PunchConfig {
    let mut c = rustler_media::punchin::PunchConfig::default();
    match (e.amount, e.amount_max) {
        (Some(lo), Some(hi)) => (c.amount_min, c.amount_max) = (lo, hi),
        (Some(a), None) => (c.amount_min, c.amount_max) = (a, a),
        (None, Some(hi)) => c.amount_max = hi,
        (None, None) => {}
    }
    c.len_s = e.duration;
    c.max_len_s = c.max_len_s.max(e.duration);
    c.min_len_s = c.min_len_s.min(e.duration);
    if let Some(p) = e.per_10s {
        c.max_changes_per_10s = p;
    }
    if e.at == PlaceAt::Every
        && let Some(step) = e.every_s
    {
        c.max_static_s = (step - e.duration).max(e.duration).max(c.min_spacing_s);
    }
    c
}

fn tagged(x: &Map<String, Value>, role: &str) -> bool {
    x.contains_key(BY_TEMPLATE) && x.get(ROLE).and_then(Value::as_str).unwrap_or("") == role
}

fn tag_map(tag: &str, role: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(BY_TEMPLATE.into(), json!(tag));
    m.insert(ROLE.into(), json!(role));
    m
}

/// Seam of a stacked layout (a panel that starts well below the top and spans most of the width).
fn seam_of(l: &Layout) -> Option<f64> {
    if l.regions.len() < 2 {
        return None;
    }
    l.regions.iter().filter(|r| r.dst.y > 0.2 && r.dst.y < 0.8 && r.dst.w > 0.8).map(|r| r.dst.y).min_by(|a, b| a.total_cmp(b))
}

// ---------------------------------------------------------------------------------------------
// Per-shot data, keywords, weak visual zones, posting defaults (helpers of build / capture)
// ---------------------------------------------------------------------------------------------

/// What is known about one segment's shot.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShotInfo {
    class: ShotClass,
    face_x: Option<f64>,
    /// Smart defaults found a content box (screen / window) and chose zoom-fit.
    zoom_fit: bool,
}

/// Shot class from a smart-defaults layout decision (`why` is the shot classifier's reason).
fn class_from_decision(what: &str, why: &str) -> Option<(ShotClass, bool)> {
    let zoom = what.starts_with("zoom-fit");
    // Merged shots read "<a>; then <b>" and are multi-face when any part is (active speaker).
    let class = if why.starts_with("graphics") {
        ShotClass::Graphics
    } else if why.contains("active speaker") {
        ShotClass::MultiFace
    } else if why.starts_with("one face") {
        ShotClass::OneFace
    } else if why.starts_with("faces in only") || why.starts_with("no frames") {
        ShotClass::NoFace
    } else {
        return None;
    };
    Some((class, zoom))
}

/// Per segment id: the shot class from the smart defaults' layout decisions (extras
/// `smart_defaults`), else from `analysis/<source>/faces.json` through the smart defaults' shot
/// classifier. Segments with neither are missing.
fn shot_infos(store: &ProjectStore, t: &Timeline) -> HashMap<String, ShotInfo> {
    let decisions: Vec<&Value> =
        t.tracks.extra.get(SMART_KEY).and_then(|v| v.get("decisions")).and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let mut faces: HashMap<String, Option<FacesResult>> = HashMap::new();
    let mut out = HashMap::new();
    // Later edits (split, trim, cold open) give segments new ids; decisions do not name their
    // source, so they are matched by source time only when the whole clip is one source.
    let one_source = t.tracks.video.windows(2).all(|w| w[0].source_id == w[1].source_id);
    let class_of = |d: &&Value| class_from_decision(d.get("what").and_then(Value::as_str).unwrap_or(""), d.get("why").and_then(Value::as_str).unwrap_or(""));
    let layout_decs: Vec<&&Value> = decisions.iter().filter(|d| d.get("kind").and_then(Value::as_str) == Some("layout")).collect();
    for v in &t.tracks.video {
        let id = v.id.as_str();
        let by_id = layout_decs
            .iter()
            .find(|d| d.get("segment_ids").and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|x| x.as_str() == Some(id))))
            .and_then(|d| class_of(d));
        let by_span = || {
            let mid = (v.src_in + v.src_out) / 2.0;
            layout_decs
                .iter()
                .find(|d| {
                    let num = |k: &str| d.get(k).and_then(Value::as_f64);
                    matches!((num("source_start"), num("source_end")), (Some(a), Some(b)) if mid >= a && mid < b)
                })
                .and_then(|d| class_of(d))
        };
        let dec = by_id.or_else(|| if one_source { by_span() } else { None });
        let f = faces.entry(v.source_id.as_str().to_string()).or_insert_with(|| {
            let rel = Path::new("analysis").join(v.source_id.as_str()).join("faces.json");
            store.resolve(&rel).is_file().then(|| store.read_json::<FacesResult>(&rel).ok()).flatten()
        });
        let plan = f.as_ref().and_then(|f| {
            let samples: Vec<ScanSample> = f
                .samples
                .iter()
                .filter(|s| s.t >= v.src_in && s.t < v.src_out)
                .map(|s| ScanSample {
                    t: s.t,
                    faces: s.faces.iter().map(|b| ScanFace { x: b.x, y: b.y, w: b.w, h: b.h, score: b.score, lm: None, mouth: 0.0, track: 0 }).collect(),
                    g: None,
                })
                .collect();
            if samples.is_empty() {
                return None;
            }
            let source = store.load_source(&v.source_id).ok()?;
            let refs: Vec<&ScanSample> = samples.iter().collect();
            Some(smart_cmds::classify(&refs, v.src_in, v.src_out, smart_cmds::frame_aspect(&source, &t.canvas)))
        });
        let info = match (dec, plan) {
            (Some((class, zoom_fit)), p) => ShotInfo { class, face_x: p.and_then(|p| p.face_x), zoom_fit },
            (None, Some(p)) => ShotInfo { class: p.class, face_x: p.face_x, zoom_fit: matches!(p.layout, smart_cmds::LayoutChoice::ZoomFit { .. }) },
            (None, None) => continue,
        };
        out.insert(id.to_string(), info);
    }
    out
}

fn shot_matches(when: ShotWhen, s: &ShotInfo) -> bool {
    match when {
        ShotWhen::OneFace => s.class == ShotClass::OneFace,
        ShotWhen::MultiFace => s.class == ShotClass::MultiFace,
        ShotWhen::NoFace => matches!(s.class, ShotClass::NoFace | ShotClass::Graphics),
        ShotWhen::Graphics => s.class == ShotClass::Graphics,
        ShotWhen::Screen => s.class == ShotClass::Graphics || (s.class == ShotClass::NoFace && s.zoom_fit),
    }
}

fn when_of(c: ShotClass) -> ShotWhen {
    match c {
        ShotClass::OneFace => ShotWhen::OneFace,
        ShotClass::MultiFace => ShotWhen::MultiFace,
        ShotClass::NoFace => ShotWhen::NoFace,
        ShotClass::Graphics => ShotWhen::Graphics,
    }
}

/// The command that writes a crop path for `track` (the template does not run the face engine).
fn track_cmd(clip: &ClipId, seg: &str, track: RuleTrack, look_room: bool) -> Option<String> {
    let lr = if look_room { "" } else { " --look-room false" };
    match track {
        RuleTrack::Face => Some(format!("rustler crop track-face {clip} --segment-id {seg}{lr}")),
        RuleTrack::Active => Some(format!("rustler crop track-active {clip} --segment-id {seg}{lr}")),
        RuleTrack::Speaker => Some(format!("rustler crop track-speaker {clip} --segment-id {seg}{lr}")),
        RuleTrack::None => None,
    }
}

/// Segments of the document as clip segments (for a [`TimeMap`]).
fn doc_segments(t: &Timeline) -> Vec<Segment> {
    t.tracks
        .video
        .iter()
        .map(|v| Segment { id: v.id.clone(), source_id: v.source_id.clone(), start: v.src_in, end: v.src_out, layout: v.layout.clone() })
        .collect()
}

/// Default keyword colour for a caption preset (as `caption keywords` picks it).
fn keyword_colour(ctx: &Ctx, store: &ProjectStore, preset: &str) -> String {
    let custom = asset_cmds::custom_presets(&ctx.home, Some(&store.root));
    match rustler_captions::find_preset_in(preset, &custom) {
        Some(p) if p.active == rustler_captions::presets::ActiveStyle::Static => rustler_captions::custom::hex_colour(p.active_colour & 0x00FF_FFFF),
        Some(p) => crate::keyword_cmds::default_keyword_colour(Some(p.active_colour & 0x00FF_FFFF)),
        None => crate::keyword_cmds::default_keyword_colour(None),
    }
}

/// Picks keywords with the `caption keywords --auto` rules, at most `per_line` in each caption
/// line of `words_per_line` words. Returns word indices.
fn pick_keywords(words: &[crate::keyword_cmds::ClipWord], words_per_line: u32, per_line: u32) -> Vec<usize> {
    let wpl = words_per_line.max(1) as usize;
    let lines = words.len().div_ceil(wpl);
    let proposed = crate::keyword_cmds::propose(words, lines * per_line as usize);
    let mut per: HashMap<usize, u32> = HashMap::new();
    let mut out = Vec::new();
    for (k, _) in proposed {
        let n = per.entry(k / wpl).or_default();
        if *n < per_line {
            *n += 1;
            out.push(k);
        }
    }
    out
}

/// Edit-timeline stretches longer than `after` s without a visual change (cuts, layout changes,
/// transitions, text / image / cutaway starts and ends, freezes, flashes; zoom effects count as
/// motion), like the retention map's static dead zones.
fn weak_zones(input: &rustler_media::punchin::PunchInput, after: f64, covered: &[(f64, f64)]) -> Vec<(f64, f64)> {
    use rustler_media::retention::{VisualChange, merge_change_times};
    let d = input.duration;
    let mut changes: Vec<VisualChange> =
        input.cuts.iter().chain(input.changes.iter()).map(|t| VisualChange { t: *t, kind: "change".into(), detail: String::new() }).collect();
    for (s, e) in &input.existing {
        changes.push(VisualChange { t: *s, kind: "effect".into(), detail: String::new() });
        changes.push(VisualChange { t: *e, kind: "effect".into(), detail: String::new() });
    }
    let mut bounds = vec![0.0];
    bounds.extend(merge_change_times(&changes, d));
    bounds.push(d);
    let mut out = Vec::new();
    for w in bounds.windows(2) {
        // Zoom effects are motion: subtract their spans.
        let mut parts = vec![(w[0], w[1])];
        for &(s, e) in input.existing.iter().chain(covered) {
            parts = parts.into_iter().flat_map(|(a, b)| if e <= a || s >= b { vec![(a, b)] } else { vec![(a, s.max(a)), (e.min(b), b)] }).filter(|(a, b)| b > a).collect();
        }
        out.extend(parts.into_iter().filter(|(a, b)| b - a > after));
    }
    out
}

/// The clip's export defaults (extras `export`, written by template.apply); `None` when unset or unreadable.
pub fn clip_export(store: &ProjectStore, clip: &ClipId) -> Option<TplExport> {
    let mut v: Value = crate::extras::get(store, clip, EXPORT_KEY).ok().flatten()?;
    if let Some(o) = v.as_object_mut() {
        o.remove(BY_TEMPLATE);
    }
    serde_json::from_value(v).ok()
}

/// The clip's posting defaults (extras `post_defaults`, written by template.apply).
pub fn clip_post_defaults(store: &ProjectStore, clip: &ClipId) -> Option<TplPost> {
    let mut v: Value = crate::extras::get(store, clip, POST_DEFAULTS_KEY).ok().flatten()?;
    if let Some(o) = v.as_object_mut() {
        o.remove(BY_TEMPLATE);
    }
    serde_json::from_value(v).ok()
}

fn tag_norm(s: &str) -> String {
    s.trim().trim_start_matches(['#', '@']).to_lowercase()
}

/// Adds posting defaults to one platform's copy (JSON object with title / description /
/// hashtags / mentions): missing hashtags and mentions are appended, the CTA line is appended to a
/// description that does not contain it. Nothing the agent wrote is replaced. True when changed.
pub fn merge_post_defaults(copy: &mut Value, d: &TplPost) -> bool {
    let Some(o) = copy.as_object_mut() else { return false };
    let mut changed = false;
    for (key, list, sigil) in [("hashtags", &d.hashtags_always, '#'), ("mentions", &d.mentions, '@')] {
        if list.is_empty() {
            continue;
        }
        let cur = o.entry(key).or_insert_with(|| json!([]));
        if !cur.is_array() {
            *cur = json!([]);
        }
        let arr = cur.as_array_mut().expect("array");
        for tag in list {
            let n = tag_norm(tag);
            if n.is_empty() || arr.iter().any(|x| x.as_str().is_some_and(|x| tag_norm(x) == n)) {
                continue;
            }
            arr.push(json!(format!("{sigil}{}", tag.trim().trim_start_matches(sigil))));
            changed = true;
        }
    }
    if let Some(cta) = d.cta_line.as_deref().map(str::trim).filter(|c| !c.is_empty())
        && let Some(desc) = o.get_mut("description").and_then(|v| v.as_str().map(str::to_string))
        && !desc.contains(cta)
    {
        let joined = if desc.trim().is_empty() { cta.to_string() } else { format!("{}\n\n{cta}", desc.trim_end()) };
        o.insert("description".into(), json!(joined));
        changed = true;
    }
    changed
}

/// Library rows of `kind` that exist and carry `tag` (case-insensitive; `mood:<tag>` matches a mood).
fn tagged_assets<'a>(rows: &'a [asset_cmds::AssetRow], kind: AssetKind, tag: &str, mood: bool) -> Vec<&'a asset_cmds::AssetRow> {
    let want = tag.trim().to_lowercase();
    let mut v: Vec<&asset_cmds::AssetRow> = rows
        .iter()
        .filter(|r| r.entry.kind == kind && r.exists && r.file.is_some())
        .filter(|r| {
            r.entry.tags.iter().any(|t| {
                let t = t.trim().to_lowercase();
                t == want || (mood && t == format!("mood:{want}"))
            })
        })
        .collect();
    v.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
    v.dedup_by(|a, b| a.entry.id == b.entry.id);
    v
}

/// A centred-right / centred box of `w` x `h` that keeps clear of the caption band at `cap_y`:
/// just above it when the captions sit low, just below when they sit high.
fn default_box(cap_y: f64, w: f64, h: f64) -> Rect {
    const BAND: f64 = 0.09;
    let y = if cap_y >= 0.5 { (cap_y - BAND - h).max(0.05) } else { (cap_y + BAND).min(0.95 - h) };
    let x = if w >= 0.7 { (1.0 - w) / 2.0 } else { 0.96 - w };
    Rect { x: (x * 1000.0).round() / 1000.0, y: (y * 1000.0).round() / 1000.0, w, h }
}

/// Stable index for `key` among `n` choices (the same clip gets the same pick on re-apply).
fn stable_pick(key: &str, n: usize) -> usize {
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    (h % n.max(1) as u64) as usize
}

/// The document with the template applied, plus warnings. Nothing is written.
pub fn build(ctx: &Ctx, store: &ProjectStore, tpl: &Template, paths: &HashMap<String, PathBuf>, old: &Timeline) -> Result<(Timeline, Vec<String>)> {
    const STAGE: &str = "template.apply";
    let mut t = old.clone();
    let mut w: Vec<String> = Vec::new();
    let tag = format!("{}@{}", tpl.name, tpl.version);
    let clip_key = old.id.as_str().to_string();
    // Items an earlier edit left past the end (older builds did not re-time layers) would fail
    // validation: drop / clamp them first and say so.
    w.extend(timeline_doc::prune_to_duration(&mut t));

    if let Some(c) = tpl.canvas {
        t.canvas = c;
    }
    if let Some(f) = tpl.export.as_ref().and_then(|x| x.fps) {
        t.canvas.fps = f;
    }
    // Per-shot layout rules first; `layout` styles the segments no rule matched.
    let mut ruled: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(rules) = &tpl.layout_rules {
        let shots = shot_infos(store, &t);
        let canvas = t.canvas;
        let clip_id = t.id.clone();
        let mut sources: HashMap<rustler_core::SourceId, rustler_core::Source> = HashMap::new();
        let (mut no_data, mut follow) = (Vec::new(), Vec::new());
        for seg in t.tracks.video.iter_mut() {
            let sid = seg.id.as_str().to_string();
            let Some(shot) = shots.get(&sid) else {
                no_data.push(sid);
                continue;
            };
            let Some(rule) = rules.iter().find(|r| shot_matches(r.when, shot)) else { continue };
            ruled.insert(sid.clone());
            let want = rule.layout.clone().or_else(|| (rule.regions.is_none() && rule.track.is_some()).then(|| "center-crop".to_string()));
            let keep = rule.regions.is_none() && want.is_some() && seg.layout.preset == want;
            if !keep {
                let new_layout = if let Some(rs) = &rule.regions {
                    Some(Layout { preset: rule.layout.clone(), regions: rs.clone(), background: rule.background.unwrap_or(Background::Black) })
                } else if let Some(p) = want.as_deref() {
                    if !sources.contains_key(&seg.source_id) {
                        sources.insert(seg.source_id.clone(), store.load_source(&seg.source_id)?);
                    }
                    let src = &sources[&seg.source_id];
                    let (sw, sh) = (src.probe.width, src.probe.height);
                    if p == "zoom-fit" {
                        // The content box is the smart defaults' (layout plan); a shot without a
                        // clear box is blur-fit (as the shot plan decides), else say how to get it.
                        if shot.zoom_fit {
                            w.push(format!(
                                "segment {sid}: zoom-fit needs the content box `rustler layout plan {clip_id} --apply` finds; blur-fit used instead"
                            ));
                        }
                        Some(core_layout::preset("blur-fit", sw, sh, &canvas, &LayoutParams::default()).map_err(|e| with_stage(e, STAGE))?)
                    } else if core_layout::PRESETS.contains(&p) {
                        let mut params = rule.params.unwrap_or_default();
                        if p == "center-crop" && params.focus_x.is_none() {
                            params.focus_x = shot.face_x.map(|x| x.clamp(0.0, 1.0));
                        }
                        Some(core_layout::preset(p, sw, sh, &canvas, &params).map_err(|e| with_stage(e, STAGE))?)
                    } else {
                        let rel = core_layout::preset_rel(p);
                        if !store.resolve(&rel).is_file() {
                            return Err(nf(STAGE, format!("template '{}' layout_rules need layout preset '{p}', which is not saved in this project", tpl.name))
                                .hint("save it with `rustler layout save-preset`, or give the rule regions"));
                        }
                        let f: LayoutPresetFile = store.read_json(&rel)?;
                        Some(Layout { preset: Some(p.to_string()), regions: f.regions, background: f.background })
                    }
                } else {
                    None
                };
                if let Some(mut l) = new_layout {
                    if let Some(bg) = rule.background {
                        l.background = bg;
                    }
                    if l != seg.layout {
                        seg.crop_path = None;
                        seg.layout = l;
                    }
                }
            } else if let Some(bg) = rule.background
                && want.as_deref() != Some("center-crop")
            {
                seg.layout.background = bg;
            }
            if seg.layout.preset.as_deref() == Some("center-crop") {
                match rule.track {
                    Some(RuleTrack::None) => seg.crop_path = None,
                    Some(tr) if seg.crop_path.is_none() => follow.extend(track_cmd(&clip_id, &sid, tr, rule.look_room.unwrap_or(true))),
                    _ => {}
                }
            }
        }
        if !no_data.is_empty() {
            w.push(format!(
                "layout_rules: no face / shot data for segment(s) {} (left unchanged); run `rustler layout plan {clip_id} --apply` or `rustler analyze faces <source_id>`, then apply again",
                no_data.join(", ")
            ));
        }
        if !follow.is_empty() {
            w.push(format!("layout_rules: these center-crop segments have no crop path yet (the template does not run face tracking): {}", follow.join(" && ")));
        }
    }
    if let Some(l) = &tpl.layout {
        let mut sources = HashMap::new();
        let canvas = t.canvas;
        let mut kept = 0;
        for seg in t.tracks.video.iter_mut() {
            if ruled.contains(seg.id.as_str()) {
                continue;
            }
            if !l.force && l.preset.is_some() && seg.layout.preset == l.preset {
                kept += 1;
                if let Some(bg) = l.background
                    && l.preset.as_deref() != Some("center-crop")
                {
                    seg.layout.background = bg;
                }
                continue;
            }
            let mut layout = if let Some(rs) = &l.regions {
                Layout { preset: l.preset.clone(), regions: rs.clone(), background: l.background.unwrap_or(Background::Black) }
            } else if let Some(p) = &l.preset {
                if core_layout::PRESETS.contains(&p.as_str()) {
                    if !sources.contains_key(&seg.source_id) {
                        sources.insert(seg.source_id.clone(), store.load_source(&seg.source_id)?);
                    }
                    let src = &sources[&seg.source_id];
                    core_layout::preset(p, src.probe.width, src.probe.height, &canvas, &l.params.unwrap_or_default()).map_err(|e| with_stage(e, STAGE))?
                } else {
                    let rel = core_layout::preset_rel(p);
                    if !store.resolve(&rel).is_file() {
                        return Err(nf(STAGE, format!("template '{}' needs layout preset '{p}', which is not saved in this project", tpl.name))
                            .hint("save it with `rustler layout save-preset`, or give the template layout.regions"));
                    }
                    let f: LayoutPresetFile = store.read_json(&rel)?;
                    Layout { preset: Some(p.clone()), regions: f.regions, background: f.background }
                }
            } else {
                seg.layout.clone()
            };
            if let Some(bg) = l.background {
                layout.background = bg;
            }
            if layout != seg.layout {
                if layout.preset.as_deref() != Some("center-crop") {
                    seg.crop_path = None;
                }
                seg.layout = layout;
            }
        }
        if kept > 0 {
            w.push(format!("{kept} segment(s) already use layout {} and keep their framing (layout.force re-lays them)", l.preset.as_deref().unwrap_or("")));
        }
    }
    if let Some(c) = &tpl.captions {
        let s = &mut t.tracks.captions.settings;
        if let Some(v) = c.enabled {
            s.enabled = v;
        }
        if let Some(v) = &c.preset {
            s.preset = v.clone();
        }
        if let Some(v) = c.position_y {
            s.position_y = v;
        }
        if let Some(v) = c.max_words_per_line {
            s.max_words_per_line = v;
        }
        if let Some(v) = c.script {
            s.script = v;
        }
        if let Some(v) = &c.speaker_colors {
            t.tracks.captions.speaker_colors = Some(v.clone());
        }
        if let Some(k) = &c.keyword_highlight {
            t.tracks.captions.word_overrides.retain(|o| !tagged(&o.extra, "keyword"));
            let tm = TimeMap::from_segments(&doc_segments(&t));
            match crate::keyword_cmds::clip_words(store, &tm) {
                Ok(words) if !words.is_empty() => {
                    let s = &t.tracks.captions.settings;
                    let color = k.color.clone().unwrap_or_else(|| keyword_colour(ctx, store, &s.preset));
                    let picks = pick_keywords(&words, s.max_words_per_line, k.max_per_line.unwrap_or(1));
                    let ovs = &mut t.tracks.captions.word_overrides;
                    for i in picks {
                        let a = Anchor { source_id: words[i].source_id.clone(), word_i: words[i].word_i };
                        // A word the agent styled itself (colour, emoji, size) keeps its look.
                        if ovs.iter().any(|o| o.word_anchor == a) {
                            continue;
                        }
                        ovs.push(WordOverride { word_anchor: a, color: Some(color.clone()), scale: None, emoji: None, hide: false, extra: tag_map(&tag, "keyword") });
                    }
                }
                Ok(_) => w.push(format!("keyword_highlight: clip {} has no transcript words; no keywords marked", t.id)),
                Err(e) => w.push(format!("keyword_highlight: transcript not readable ({}); no keywords marked", e.message)),
            }
        }
        match c.emoji {
            Some(OnOff::Off) => {
                let ovs = &mut t.tracks.captions.word_overrides;
                let n = ovs.iter_mut().filter_map(|o| o.emoji.take()).count();
                ovs.retain(|o| o.color.is_some() || o.scale.is_some() || o.hide || o.emoji.is_some());
                if n > 0 {
                    w.push(format!("emoji off: removed the emoji of {n} caption word(s)"));
                }
            }
            Some(OnOff::On) if !t.tracks.captions.word_overrides.iter().any(|o| o.emoji.is_some()) => w.push(format!(
                "emoji on: the agent picks the emoji per word; none set yet (`rustler caption word {} --source-id <src> --word-i <i> --emoji \"...\"`)",
                t.id
            )),
            _ => {}
        }
    }

    let dur = t.edit_duration();
    let cuts = cut_times(&t);
    let seam = t.tracks.video.first().and_then(|s| seam_of(&s.layout));

    if let Some(h) = &tpl.hook {
        t.tracks.text.retain(|x| !tagged(&x.extra, "hook"));
        let restyle = |x: &mut TextItem| {
            let at_seam = h.at_seam.unwrap_or(false);
            match (at_seam, seam) {
                (true, Some(y)) => x.position_y = y,
                _ => {
                    if let Some(y) = h.position_y {
                        x.position_y = y;
                    }
                }
            }
            // A viral hook style keeps its own look (box) and is never made smaller.
            let viral = x.style != TextStyle::HookTop;
            if let Some(f) = h.font_size_factor {
                x.font_size_factor = if viral { f.max(x.font_size_factor) } else { f };
            }
            if let Some(b) = h.boxed.filter(|_| !viral) {
                x.boxed = b;
            }
            if let Some(d) = h.duration {
                x.end = (x.start + d).min(dur);
            }
        };
        let mut n = 0;
        for x in t.tracks.text.iter_mut().filter(|x| x.style.is_hook()) {
            restyle(x);
            n += 1;
            if let Some(m) = h.max_words {
                let words = x.text.split_whitespace().count();
                if words > m as usize {
                    w.push(format!("hook {} has {words} words; this template wants <= {m} (`rustler overlay edit {} {} --text \"...\"`)", x.id, t.id, x.id));
                }
            }
        }
        if n == 0 {
            match &h.text {
                Some(text) => {
                    let id = timeline_doc::next_item_id("ovl", t.tracks.text.iter().map(|x| x.id.as_str()));
                    let mut x = TextItem {
                        id,
                        text: text.clone(),
                        start: 0.0,
                        end: h.duration.unwrap_or(3.0).min(dur),
                        style: TextStyle::HookTop,
                        position_y: 0.12,
                        font_size_factor: 0.085,
                        boxed: true,
                        extra: tag_map(&tag, "hook"),
                    };
                    restyle(&mut x);
                    t.tracks.text.push(x);
                }
                None => w.push(format!(
                    "clip {} has no hook title; add one (<= {} words) with `rustler overlay add-text {} \"...\"` and apply the template again",
                    t.id,
                    h.max_words.unwrap_or(7),
                    t.id
                )),
            }
        }
    }
    for (k, look) in &tpl.text_styles {
        let style = if k == "lower-third" { TextStyle::LowerThird } else { TextStyle::Plain };
        for x in t.tracks.text.iter_mut().filter(|x| x.style == style && !x.extra.contains_key(BY_TEMPLATE)) {
            if let Some(y) = look.position_y {
                x.position_y = y;
            }
            if let Some(f) = look.font_size_factor {
                x.font_size_factor = f;
            }
            if let Some(b) = look.boxed {
                x.boxed = b;
            }
        }
    }
    if let Some(texts) = &tpl.texts {
        t.tracks.text.retain(|x| !tagged(&x.extra, "text"));
        for x in texts {
            if x.start >= dur {
                w.push(format!("text \"{}\" starts at {:.2}s, after the clip ends ({dur:.2}s); skipped", x.text, x.start));
                continue;
            }
            let id = timeline_doc::next_item_id("ovl", t.tracks.text.iter().map(|x| x.id.as_str()));
            t.tracks.text.push(TextItem {
                id,
                text: x.text.clone(),
                start: x.start,
                end: x.end.min(dur),
                style: x.style,
                position_y: x.position_y.unwrap_or(match x.style {
                    TextStyle::LowerThird => 0.80,
                    TextStyle::Plain => 0.50,
                    _ => 0.12,
                }),
                font_size_factor: x.font_size_factor.unwrap_or(0.065),
                boxed: x.boxed.unwrap_or(x.style != TextStyle::Plain),
                extra: tag_map(&tag, "text"),
            });
        }
    }
    if let Some(images) = &tpl.images {
        t.tracks.images.retain(|x| !tagged(&x.extra, "image"));
        for im in images {
            let Some(file) = path_of(paths, &im.asset, &im.file) else { continue };
            let start = im.start.filter(|s| *s > 0.0);
            if start.is_some_and(|s| s >= dur) {
                w.push(format!("image {} starts after the clip ends; skipped", im.asset.clone().unwrap_or_else(|| file.display().to_string())));
                continue;
            }
            let id = timeline_doc::next_item_id("img", t.tracks.images.iter().map(|x| x.id.as_str()));
            t.tracks.images.push(ImageItem {
                id,
                timing: ItemTiming { out_start: start, out_end: im.end.map(|e| e.min(dur)), start_word: None, end_word: None },
                asset: im.asset.clone(),
                file,
                rect: im.rect,
                scale: im.scale.unwrap_or(1.0),
                opacity: im.opacity.unwrap_or(1.0),
                rotation: im.rotation.unwrap_or(0.0),
                fit: im.fit.unwrap_or(Fit::Contain),
                anim: im.anim.unwrap_or_default(),
                z: im.z.unwrap_or(0),
                above_captions: im.above_captions.unwrap_or(false),
                extra: tag_map(&tag, "image"),
            });
        }
    }
    if let Some(b) = &tpl.brand {
        const ROLES: [&str; 3] = ["brand_logo", "brand_cta", "brand_outro"];
        t.tracks.images.retain(|x| !ROLES.iter().any(|r| tagged(&x.extra, r)));
        // (role, asset, rect, opacity, start, end, above captions, z)
        let mut items: Vec<(&str, &String, Rect, f64, Option<f64>, Option<f64>, bool, i32)> = Vec::new();
        if let Some(a) = &b.logo_asset {
            let rect = b.rect.unwrap_or(Rect { x: 0.78, y: 0.03, w: 0.18, h: 0.08 });
            items.push(("brand_logo", a, rect, b.opacity.unwrap_or(0.9), None, None, false, 5));
        }
        if let Some(c) = &b.cta_card {
            let start = c.at.unwrap_or(((dur * 0.4) * 100.0).round() / 100.0);
            let end = (start + c.duration.unwrap_or(3.0)).min(dur);
            if end - start < 0.5 {
                w.push(format!("brand cta_card at {start:.2}s does not fit in the clip ({dur:.2}s); skipped"));
            } else {
                let rect = c.rect.unwrap_or_else(|| default_box(t.tracks.captions.settings.position_y, 0.8, 0.12));
                items.push(("brand_cta", &c.asset, rect, 1.0, Some(start), Some(end), true, 8));
            }
        }
        if let Some(c) = &b.outro_card {
            let d = c.duration.unwrap_or(2.5);
            if d >= dur - 0.5 {
                w.push(format!("brand outro_card ({d:.1}s) is as long as the clip ({dur:.2}s); skipped"));
            } else {
                let start = ((dur - d) * 100.0).round() / 100.0;
                let rect = c.rect.unwrap_or(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
                items.push(("brand_outro", &c.asset, rect, 1.0, Some(start), None, true, 9));
            }
        }
        for (role, asset, rect, opacity, start, end, above, z) in items {
            let Some(file) = paths.get(&format!("asset:{asset}")).cloned() else { continue };
            let id = timeline_doc::next_item_id("img", t.tracks.images.iter().map(|x| x.id.as_str()));
            t.tracks.images.push(ImageItem {
                id,
                timing: ItemTiming { out_start: start, out_end: end, start_word: None, end_word: None },
                asset: Some(asset.clone()),
                file,
                rect,
                scale: 1.0,
                opacity,
                rotation: 0.0,
                fit: Fit::Contain,
                anim: Anim::default(),
                z,
                above_captions: above,
                extra: tag_map(&tag, role),
            });
        }
    }
    if let Some(b) = &tpl.progress_bar {
        t.tracks.progress_bar = if b.enabled {
            let mut item = t.tracks.progress_bar.clone().unwrap_or(ProgressBarItem {
                color: "#FFE000".into(),
                height: 8,
                position: BarPosition::Top,
                extra: Map::new(),
            });
            if let Some(c) = &b.color {
                item.color = c.clone();
            }
            if let Some(h) = b.height {
                item.height = h;
            }
            if let Some(p) = b.position {
                item.position = p;
            }
            Some(item)
        } else {
            None
        };
    }
    if let Some(a) = &tpl.audio {
        let au = &mut t.tracks.audio;
        if let Some(v) = a.loudnorm {
            au.master.loudnorm = v;
        }
        if let Some(v) = a.target_lufs {
            au.master.target_lufs = v;
        }
        if let Some(v) = a.true_peak {
            au.master.true_peak = v;
        }
        if let Some(v) = a.lra {
            au.master.lra = v;
        }
        if let Some(v) = a.denoise {
            au.denoise = v;
        }
        if let Some(v) = a.volume_db {
            au.volume_db = v;
        }
        if let Some(m) = &a.music {
            au.music = if m.none {
                None
            } else if m.tag.is_some() || m.mood.is_some() {
                let rows = asset_cmds::load_rows(&ctx.home, Some(&store.root));
                let mut c = match &m.tag {
                    Some(tg) => tagged_assets(&rows, AssetKind::Music, tg, false),
                    None => tagged_assets(&rows, AssetKind::Music, m.mood.as_deref().unwrap_or(""), true),
                };
                if let (Some(_), Some(md)) = (&m.tag, &m.mood) {
                    let moods: Vec<String> = tagged_assets(&rows, AssetKind::Music, md, true).iter().map(|r| r.entry.id.clone()).collect();
                    c.retain(|r| moods.contains(&r.entry.id));
                }
                let what = [m.tag.as_ref().map(|x| format!("tag '{x}'")), m.mood.as_ref().map(|x| format!("mood '{x}'"))].into_iter().flatten().collect::<Vec<_>>().join(" and ");
                if c.is_empty() {
                    w.push(format!(
                        "audio.music: no music asset in the library with {what}; music left as it was (add one with `rustler asset add <file> --kind music --tags <tag> --license <license>`)"
                    ));
                    au.music.clone()
                } else {
                    let r = c[stable_pick(&clip_key, c.len())];
                    Some(MusicItem {
                        path: r.file.clone().unwrap_or_default(),
                        volume_db: m.volume_db,
                        duck: m.duck,
                        fade_in: m.fade_in,
                        fade_out: m.fade_out,
                        extra: Map::new(),
                    })
                }
            } else {
                path_of(paths, &m.asset, &m.file).map(|path| MusicItem {
                    path,
                    volume_db: m.volume_db,
                    duck: m.duck,
                    fade_in: m.fade_in,
                    fade_out: m.fade_out,
                    extra: Map::new(),
                })
            };
        }
        if let Some(list) = &a.sfx {
            au.sfx.retain(|x| !x.extra.contains_key(BY_TEMPLATE));
            for s in list {
                let Some(file) = path_of(paths, &s.asset, &s.file) else { continue };
                for at in instances(s.at, s.every_s, s.t, &cuts, dur) {
                    if at >= dur {
                        continue;
                    }
                    let id = timeline_doc::next_item_id("sfx", au.sfx.iter().map(|x| x.id.as_str()));
                    au.sfx.push(SfxItem { id, file: file.clone(), at: Some(at), at_word: None, volume_db: s.volume_db, extra: tag_map(&tag, "sfx") });
                }
            }
        }
    }
    if let Some(list) = &tpl.effects {
        t.tracks.effects.retain(|x| !x.extra.contains_key(BY_TEMPLATE));
        for e in list {
            if e.kind == EffectKind::PunchIn && matches!(e.at, PlaceAt::Emphasis | PlaceAt::Every) {
                // One planner for template and retention punch-ins: stressed words, away from
                // cuts, not on graphics, not stacked, paced, centred on the face.
                let (input, pw) = timeline_doc::punch_input(store, &t);
                w.extend(pw.into_iter().filter(|x| !x.contains("no transcript")));
                let cfg = punch_config(e);
                let planned = rustler_media::punchin::plan(&input, &cfg);
                for p in planned {
                    let id = timeline_doc::next_item_id("fx", t.tracks.effects.iter().map(|x| x.id.as_str()));
                    let mut extra = tag_map(&tag, "effect");
                    if let (Some(x), Some(y)) = (p.focus_x, p.focus_y) {
                        extra.insert("focus_x".into(), json!(x));
                        extra.insert("focus_y".into(), json!(y));
                    }
                    t.tracks.effects.push(EffectItem { id, kind: e.kind, start: p.start, end: p.end, amount: Some(p.amount), extra });
                }
                continue;
            }
            for s in instances(e.at, e.every_s, e.t, &cuts, dur) {
                let end = (s + e.duration).min(dur);
                if end - s < 0.1 {
                    continue;
                }
                let id = timeline_doc::next_item_id("fx", t.tracks.effects.iter().map(|x| x.id.as_str()));
                t.tracks.effects.push(EffectItem { id, kind: e.kind, start: s, end, amount: e.amount, extra: tag_map(&tag, "effect") });
            }
        }
    }
    if let Some(tr) = &tpl.transitions {
        t.tracks.transitions.clear();
        if tr.kind != TransitionKind::Cut {
            let v = &t.tracks.video;
            for k in 0..v.len().saturating_sub(1) {
                let (a, b) = (v[k].src_out - v[k].src_in, v[k + 1].src_out - v[k + 1].src_in);
                if a < 2.0 * tr.duration + 0.1 || b < 2.0 * tr.duration + 0.1 {
                    w.push(format!("no {:?} after segment {}: a neighbouring segment is too short for a {:.2}s overlap", tr.kind, v[k].id, tr.duration));
                    continue;
                }
                t.tracks.transitions.push(TransitionItem { after_segment_id: v[k].id.clone(), kind: tr.kind, duration: tr.duration, extra: Map::new() });
            }
        }
    }
    if let Some(cr) = &tpl.cutaway_rules {
        t.tracks.effects.retain(|x| !tagged(&x.extra, "weak_punch"));
        t.tracks.cutaways.retain(|x| !tagged(&x.extra, "cutaway"));
        let (mut input, pw) = timeline_doc::punch_input(store, &t);
        // The hook and the brand CTA / outro cards stay visible: no cutaway or zoom over them.
        let covered: Vec<(f64, f64)> = t
            .tracks
            .text
            .iter()
            .filter(|x| x.style.is_hook())
            .map(|x| (x.start, x.end))
            .chain(t.tracks.images.iter().filter(|x| tagged(&x.extra, "brand_cta") || tagged(&x.extra, "brand_outro")).map(|x| (x.timing.out_start.unwrap_or(0.0), x.timing.out_end.unwrap_or(dur))))
            .collect();
        let zones = weak_zones(&input, cr.on_weak_visual_after_s, &covered);
        match cr.kind {
            CutawayKind::PunchIn => {
                w.extend(pw.into_iter().filter(|x| !x.contains("no transcript")));
                // The planner's own (subtle) defaults; only inside the weak zones.
                let cfg = rustler_media::punchin::PunchConfig::default();
                let mut n = 0;
                for &(s, e) in &zones {
                    let planned = rustler_media::punchin::plan(&rustler_media::punchin::PunchInput { window: Some((s, e)), ..input.clone() }, &cfg);
                    input.existing.extend(planned.iter().map(|p| (p.start, p.end)));
                    for p in planned {
                        let id = timeline_doc::next_item_id("fx", t.tracks.effects.iter().map(|x| x.id.as_str()));
                        let mut extra = tag_map(&tag, "weak_punch");
                        if let (Some(x), Some(y)) = (p.focus_x, p.focus_y) {
                            extra.insert("focus_x".into(), json!(x));
                            extra.insert("focus_y".into(), json!(y));
                        }
                        t.tracks.effects.push(EffectItem { id, kind: EffectKind::PunchIn, start: p.start, end: p.end, amount: Some(p.amount), extra });
                        n += 1;
                    }
                }
                if !zones.is_empty() && n == 0 {
                    w.push(format!(
                        "cutaway_rules: {} weak visual stretch(es) longer than {:.1}s, but no punch-in fits there (graphics / blur-fit, near a cut, or pacing); use a cutaway or a text pop",
                        zones.len(),
                        cr.on_weak_visual_after_s
                    ));
                }
            }
            CutawayKind::AssetTag => {
                let tg = cr.tag.clone().unwrap_or_default();
                let rows = asset_cmds::load_rows(&ctx.home, Some(&store.root));
                let c = tagged_assets(&rows, AssetKind::Image, &tg, false);
                if c.is_empty() {
                    w.push(format!(
                        "cutaway_rules: no image asset in the library has tag '{tg}'; {} weak stretch(es) left as they are (`rustler asset add <file> --kind image --tags {tg} --license <license>`)",
                        zones.len()
                    ));
                } else {
                    let first = stable_pick(&clip_key, c.len());
                    let cap_y = t.tracks.captions.settings.position_y;
                    for (k, &(s, e)) in zones.iter().enumerate() {
                        let len = cr.duration.min(e - s - 0.2);
                        if len < 0.5 {
                            continue;
                        }
                        let start = (((s + e) / 2.0 - len / 2.0) * 100.0).round() / 100.0;
                        let r = c[(first + k) % c.len()];
                        let mode = cr.mode.unwrap_or(CutawayMode::Full);
                        let id = timeline_doc::next_item_id("cut", t.tracks.cutaways.iter().map(|x| x.id.as_str()));
                        t.tracks.cutaways.push(CutawayItem {
                            id,
                            timing: ItemTiming { out_start: Some(start), out_end: Some(((start + len) * 100.0).round() / 100.0), start_word: None, end_word: None },
                            media: CutawayMedia { source_id: None, asset: Some(r.entry.id.clone()), file: r.file.clone(), in_: 0.0, extra: Map::new() },
                            mode,
                            rect: (mode == CutawayMode::Pip).then(|| cr.rect.unwrap_or_else(|| default_box(cap_y, 0.44, 0.25))),
                            fit: Fit::Cover,
                            mute: true,
                            extra: tag_map(&tag, "cutaway"),
                        });
                    }
                }
            }
        }
    }
    // Rendered length: transitions overlap their neighbours.
    let out_len = dur - t.tracks.transitions.iter().map(|x| x.duration).sum::<f64>();
    if let Some(x) = &tpl.export {
        let mut v = serde_json::to_value(x).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
        if let Some(o) = v.as_object_mut() {
            o.insert(BY_TEMPLATE.into(), json!(tag));
        }
        t.tracks.extra.insert(EXPORT_KEY.into(), v);
        if let Some(m) = x.max_len.filter(|m| out_len > m + 0.05) {
            w.push(format!("clip is {out_len:.1}s, longer than this template's max_len {m:.0}s: trim it (`rustler edit trim {} ...` or `rustler edit delete-segment`)", t.id));
        }
        if let Some(m) = x.min_len.filter(|m| out_len < m - 0.05) {
            w.push(format!("clip is {out_len:.1}s, shorter than this template's min_len {m:.0}s"));
        }
    }
    if let Some(p) = &tpl.post_defaults {
        let mut v = serde_json::to_value(p).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
        if let Some(o) = v.as_object_mut() {
            o.insert(BY_TEMPLATE.into(), json!(tag));
        }
        t.tracks.extra.insert(POST_DEFAULTS_KEY.into(), v);
        if let Some(pl) = t.meta.post.as_mut().and_then(|p| p.get_mut("platforms")).and_then(Value::as_object_mut) {
            for copy in pl.values_mut() {
                merge_post_defaults(copy, p);
            }
        }
    }
    t.template_ref = Some(TemplateRef { id: tpl.name.clone(), version: Some(tpl.version), extra: Map::new() });
    Ok((t, w))
}

/// Result of applying a template to one clip.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct ClipResult {
    pub clip_id: ClipId,
    /// Revision before.
    pub from_revision: u64,
    /// Revision after (same as before on a dry run or when nothing changed).
    pub revision: u64,
    /// JSON-Patch operations sent to timeline.apply.
    pub ops: usize,
    pub changes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The JSON-Patch (dry run only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<Value>,
}

/// Applies `tpl` to `clips` through `timeline.apply`: every clip is validated first (dry run), then
/// saved, one revision each.
pub async fn apply_to(ctx: &Ctx, tpl: &Template, clips: &[ClipId], dry: bool) -> Result<Vec<ClipResult>> {
    const STAGE: &str = "template.apply";
    let store = ctx.store()?;
    let paths = check_refs(ctx, STAGE, tpl)?;
    let mut planned: Vec<(ClipId, u64, Vec<Value>, Vec<String>, Vec<String>)> = Vec::new();
    for id in clips {
        let (old, _) = timeline_doc::assemble(&store, id)?;
        let (new, warnings) = build(ctx, &store, tpl, &paths, &old)?;
        let a = serde_json::to_value(&old).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
        let b = serde_json::to_value(&new).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
        let patch = json_patch::diff(&a, &b);
        let ops: Vec<Value> = serde_json::to_value(&patch).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        // Validate through the same path (dry run) before anything is written.
        let check = timeline_cmds::apply(
            ctx.clone(),
            timeline_cmds::ApplyIn { clip_id: id.clone(), base_revision: old.revision, patch: Some(ops.clone()), document: None, dry_run: Some(true) },
        )
        .await
        .map_err(|e| {
            let mut e = with_stage(e, STAGE);
            e.message = format!("clip {id}: {}", e.message);
            e
        })?;
        planned.push((id.clone(), old.revision, ops, warnings, check.changes));
    }
    let mut out = Vec::new();
    for (id, rev, ops, mut warnings, changes) in planned {
        if dry || ops.is_empty() {
            if ops.is_empty() {
                warnings.push("already matches the template; nothing to change".into());
            }
            let n = ops.len();
            out.push(ClipResult { clip_id: id, from_revision: rev, revision: rev, ops: n, changes, warnings, patch: dry.then(|| Value::Array(ops)) });
            continue;
        }
        let n = ops.len();
        let r = timeline_cmds::apply(
            ctx.clone(),
            timeline_cmds::ApplyIn { clip_id: id.clone(), base_revision: rev, patch: Some(ops), document: None, dry_run: Some(false) },
        )
        .await
        .map_err(|e| with_stage(e, STAGE))?;
        warnings.extend(r.warnings);
        out.push(ClipResult { clip_id: id, from_revision: rev, revision: r.revision, ops: n, changes: r.changes, warnings, patch: None });
    }
    if !dry {
        log_event(ctx, &store, STAGE, json!({"template": format!("{}@{}", tpl.name, tpl.version), "clips": out.iter().map(|r| &r.clip_id).collect::<Vec<_>>()}))?;
    }
    Ok(out)
}

/// Resolves `spec`, merges `overrides` (RFC 7386 merge patch) and checks the result is complete
/// (fields valid, every referenced asset present). Used by template.apply and clip.create.
pub fn prepare(ctx: &Ctx, stage: &str, spec: &str, overrides: Option<&Value>) -> Result<Template> {
    let (_, t) = resolve(ctx, stage, spec)?;
    let t = match overrides {
        Some(o) if !o.is_null() => {
            let mut v = serde_json::to_value(&t).map_err(|e| RustlerError::new(ErrorCode::Internal, stage, e.to_string()))?;
            json_patch::merge(&mut v, o);
            let mut n = parse_template_value(stage, v).map_err(|e| verr(stage, format!("overrides: {}", e.message)))?;
            n.name = t.name.clone();
            n.version = t.version;
            n
        }
        _ => t,
    };
    validate_template(&t).map_err(|e| with_stage(e, stage))?;
    check_refs(ctx, stage, &t)?;
    Ok(t)
}

// ---------------------------------------------------------------------------------------------
// template.create / template.update
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateIn {
    /// Lowercase letters, digits, `-` and `_`.
    pub name: String,
    /// Capture the look of this clip.
    #[serde(default)]
    pub from_clip: Option<ClipId>,
    /// A whole template document (instead of from_clip); name and version are set for you.
    #[serde(default)]
    pub template: Option<Value>,
    #[serde(default)]
    pub description: Option<String>,
    /// Also keep the hook / label texts (default false: a template is a look, not content).
    #[serde(default)]
    pub include_text: Option<bool>,
    /// project (default when a project is open) | global.
    #[serde(default)]
    pub scope: Option<Scope>,
}

#[derive(Serialize, JsonSchema)]
pub struct SavedOut {
    pub name: String,
    pub version: u32,
    pub scope: String,
    pub file: PathBuf,
    pub template: Template,
    /// Files and assets the template needs (exported with it).
    pub refs: Vec<TplRef>,
    /// What was not captured from the clip and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

fn source_template(ctx: &Ctx, stage: &str, name: &str, from_clip: Option<&ClipId>, template: Option<Value>, include_text: bool) -> Result<(Template, Vec<String>)> {
    match (from_clip, template) {
        (Some(_), Some(_)) => Err(verr(stage, "give from_clip or template, not both")),
        (None, None) => Err(verr(stage, "nothing to save: give from_clip (capture a clip's look) or template (JSON)")
            .hint("e.g. `rustler template create my-look --from-clip clp_x`")),
        (Some(c), None) => {
            let store = ctx.store()?;
            let (doc, _) = timeline_doc::assemble(&store, c)?;
            capture(ctx, &store, &doc, name, include_text)
        }
        (None, Some(v)) => {
            let mut v = v;
            if let Some(o) = v.as_object_mut() {
                o.entry("name").or_insert(json!(name));
                o.entry("schema").or_insert(json!(TEMPLATE_SCHEMA));
            }
            let mut t = parse_template_value(stage, v)?;
            t.name = name.to_string();
            if t.created_by.is_empty() {
                t.created_by = "agent".into();
            }
            Ok((t, Vec::new()))
        }
    }
}

fn saved(t: Template, scope: &str, file: PathBuf, notes: Vec<String>, dry: bool) -> SavedOut {
    let spec = format!("{}@{}", t.name, t.version);
    SavedOut {
        name: t.name.clone(),
        version: t.version,
        scope: scope.into(),
        file,
        refs: t.refs(),
        template: t,
        notes,
        dry_run: dry.then_some(true),
        next: vec![
            format!("rustler template apply {spec} --clip-ids <clip_id> --dry-run"),
            format!("rustler template show {spec}"),
            format!("rustler template export {spec} --out <file.zip>"),
        ],
    }
}

async fn create(ctx: Ctx, i: CreateIn) -> Result<SavedOut> {
    const STAGE: &str = "template.create";
    let name = i.name.trim().to_string();
    if !valid_name(&name) {
        return Err(verr(STAGE, format!("name '{name}' must be 1-64 lowercase letters, digits, '-' or '_'")).hint("e.g. my-podcast-look"));
    }
    if let Some(s) = latest_in(&ctx, &name) {
        return Err(verr(STAGE, format!("template '{name}' already exists ({} v{})", s.scope, s.version))
            .hint(format!("save a new version with `rustler template update {name} ...`, or pick another name")));
    }
    let (mut t, notes) = source_template(&ctx, STAGE, &name, i.from_clip.as_ref(), i.template, i.include_text.unwrap_or(false))?;
    t.version = 1;
    if let Some(d) = i.description.filter(|d| !d.trim().is_empty()) {
        t.description = d;
    }
    validate_template(&t).map_err(|e| with_stage(e, STAGE))?;
    let scope = i.scope.unwrap_or_else(|| default_scope(&ctx));
    let dir = dir_of(&ctx, scope)?;
    let file = dir.join(format!("{name}@1.json"));
    if ctx.dry_run {
        return Ok(saved(t, scope_label(scope), file, notes, true));
    }
    let file = write_template(STAGE, &dir, &t)?;
    if let Ok(store) = ctx.store() {
        log_event(&ctx, &store, STAGE, json!({"template": name, "version": 1, "scope": scope_label(scope)}))?;
    }
    Ok(saved(t, scope_label(scope), file, notes, false))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateIn {
    pub name: String,
    /// Re-capture from this clip.
    #[serde(default)]
    pub from_clip: Option<ClipId>,
    /// A whole new template document.
    #[serde(default)]
    pub template: Option<Value>,
    /// RFC 7386 merge patch onto the latest version, e.g. {"captions": {"position_y": 0.65}} (null removes a field).
    #[serde(default)]
    pub patch: Option<Value>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub include_text: Option<bool>,
}

async fn update(ctx: Ctx, i: UpdateIn) -> Result<SavedOut> {
    const STAGE: &str = "template.update";
    let name = i.name.trim().to_string();
    let Some(latest) = latest_in(&ctx, &name) else {
        return Err(nf(STAGE, format!("no template called '{name}'")).hint("create it with `rustler template create`"));
    };
    let base = load(STAGE, &latest)?;
    let given = [i.from_clip.is_some(), i.template.is_some(), i.patch.is_some()].iter().filter(|b| **b).count();
    if given > 1 {
        return Err(verr(STAGE, "give one of from_clip, template or patch"));
    }
    let (mut t, notes) = if let Some(p) = &i.patch {
        let mut v = serde_json::to_value(&base).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
        json_patch::merge(&mut v, p);
        (parse_template_value(STAGE, v)?, Vec::new())
    } else if given == 0 {
        if i.description.is_none() {
            return Err(verr(STAGE, "nothing to change: give from_clip, template, patch or description"));
        }
        (base.clone(), Vec::new())
    } else {
        source_template(&ctx, STAGE, &name, i.from_clip.as_ref(), i.template, i.include_text.unwrap_or(false))?
    };
    t.name = name.clone();
    t.schema = TEMPLATE_SCHEMA.into();
    if let Some(d) = i.description.filter(|d| !d.trim().is_empty()) {
        t.description = d;
    } else if t.description.is_empty() {
        t.description = base.description.clone();
    }
    if latest.scope == "built-in" {
        t.created_by = "agent".into();
    }
    // The next free version across every scope, so name@N stays unambiguous.
    t.version = all_stored(&ctx).iter().filter(|s| s.name == name).map(|s| s.version).max().unwrap_or(0) + 1;
    validate_template(&t).map_err(|e| with_stage(e, STAGE))?;
    let scope = match latest.scope.as_str() {
        "global" => Scope::Global,
        "project" => Scope::Project,
        _ => default_scope(&ctx),
    };
    let dir = dir_of(&ctx, scope)?;
    let file = dir.join(format!("{name}@{}.json", t.version));
    if ctx.dry_run {
        return Ok(saved(t, scope_label(scope), file, notes, true));
    }
    let file = write_template(STAGE, &dir, &t)?;
    if let Ok(store) = ctx.store() {
        log_event(&ctx, &store, STAGE, json!({"template": name, "version": t.version, "scope": scope_label(scope)}))?;
    }
    Ok(saved(t, scope_label(scope), file, notes, false))
}

// ---------------------------------------------------------------------------------------------
// template.show / template.list / template.delete
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameIn {
    /// `name` (latest) or `name@version`.
    pub name: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ShowOut {
    pub name: String,
    pub version: u32,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Every version of this name in this scope.
    pub versions: Vec<u32>,
    pub template: Template,
    pub refs: Vec<TplRef>,
    /// Referenced assets/files that are missing here (apply would fail with not_found).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    pub next: Vec<String>,
}

async fn show(ctx: Ctx, i: NameIn) -> Result<ShowOut> {
    const STAGE: &str = "template.show";
    let (s, t) = resolve(&ctx, STAGE, &i.name)?;
    let versions = all_stored(&ctx).into_iter().filter(|x| x.name == s.name && x.scope == s.scope).map(|x| x.version).collect();
    let missing = match check_refs(&ctx, STAGE, &t) {
        Ok(_) => Vec::new(),
        Err(e) => vec![e.message],
    };
    let spec = format!("{}@{}", s.name, s.version);
    Ok(ShowOut {
        name: s.name.clone(),
        version: s.version,
        scope: s.scope.clone(),
        file: s.file.clone(),
        versions,
        refs: t.refs(),
        template: t,
        missing,
        next: vec![format!("rustler template apply {spec} --clip-ids <clip_id> --dry-run"), format!("rustler template diff {spec} <other>")],
    })
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ListIn {}

#[derive(Serialize, JsonSchema)]
pub struct ListRow {
    pub name: String,
    /// Latest version in this scope.
    pub version: u32,
    pub versions: Vec<u32>,
    /// project | global | built-in (project wins over global over built-in for a bare name).
    pub scope: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOut {
    pub count: usize,
    pub templates: Vec<ListRow>,
    pub next: Vec<String>,
}

async fn list(ctx: Ctx, _i: ListIn) -> Result<ListOut> {
    let all = all_stored(&ctx);
    let mut rows: Vec<ListRow> = Vec::new();
    for s in &all {
        if let Some(r) = rows.iter_mut().find(|r| r.name == s.name && r.scope == s.scope) {
            r.versions.push(s.version);
            continue;
        }
        let same: Vec<&Stored> = all.iter().filter(|x| x.name == s.name && x.scope == s.scope).collect();
        let latest = same.iter().max_by_key(|x| x.version).copied().unwrap_or(s);
        let (description, error) = match load("template.list", latest) {
            Ok(t) => (t.description, None),
            Err(e) => (String::new(), Some(e.message)),
        };
        rows.push(ListRow { name: s.name.clone(), version: latest.version, versions: vec![s.version], scope: s.scope.clone(), description, file: latest.file.clone(), error });
    }
    Ok(ListOut {
        count: rows.len(),
        templates: rows,
        next: vec!["rustler template show <name>".into(), "rustler template apply <name> --clip-ids <clip_id> --dry-run".into()],
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteIn {
    /// `name` (every version in its scope) or `name@version`.
    pub name: String,
}

#[derive(Serialize, JsonSchema)]
pub struct DeleteOut {
    pub deleted: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn delete(ctx: Ctx, i: DeleteIn) -> Result<DeleteOut> {
    const STAGE: &str = "template.delete";
    let (name, ver) = split_spec(STAGE, &i.name)?;
    let (s, _) = resolve(&ctx, STAGE, &i.name)?;
    if s.scope == "built-in" {
        return Err(verr(STAGE, format!("'{name}' is a built-in template and cannot be deleted")).hint("make your own with `rustler template update` (a new version shadows it) or `template create`"));
    }
    let files: Vec<PathBuf> = all_stored(&ctx)
        .into_iter()
        .filter(|x| x.name == name && x.scope == s.scope && ver.is_none_or(|v| v == x.version))
        .filter_map(|x| x.file)
        .collect();
    if ctx.dry_run {
        return Ok(DeleteOut { deleted: files, dry_run: Some(true) });
    }
    for f in &files {
        std::fs::remove_file(f).map_err(|e| io_err(STAGE, "delete", f, e))?;
    }
    if let Ok(store) = ctx.store() {
        log_event(&ctx, &store, STAGE, json!({"template": i.name, "files": files.len()}))?;
    }
    Ok(DeleteOut { deleted: files, dry_run: None })
}

// ---------------------------------------------------------------------------------------------
// template.apply
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyIn {
    /// `name` (latest) or `name@version`.
    pub name: String,
    /// Clips to style.
    #[serde(default)]
    pub clip_ids: Option<Vec<ClipId>>,
    /// Every clip of the project.
    #[serde(default)]
    pub all: Option<bool>,
    /// RFC 7386 merge patch onto the template for this run only, e.g. {"hook": {"text": "He quit"}}.
    #[serde(default)]
    pub overrides: Option<Value>,
    /// Validate and show the patch per clip without saving (same as --dry-run).
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct ApplyOut {
    pub template: String,
    pub version: u32,
    pub clips: Vec<ClipResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

async fn apply(ctx: Ctx, i: ApplyIn) -> Result<ApplyOut> {
    const STAGE: &str = "template.apply";
    let dry = ctx.dry_run || i.dry_run.unwrap_or(false);
    let store = ctx.store()?;
    let clips: Vec<ClipId> = match (i.clip_ids, i.all.unwrap_or(false)) {
        (Some(c), false) if !c.is_empty() => c,
        (None, true) => store.load_project()?.clips,
        (Some(c), true) if c.is_empty() => store.load_project()?.clips,
        (Some(_), true) => return Err(verr(STAGE, "give clip_ids or all, not both")),
        _ => return Err(verr(STAGE, "which clips? give clip_ids or all").hint("e.g. `rustler template apply podcast-viral --clip-ids clp_x`")),
    };
    if clips.is_empty() {
        return Err(nf(STAGE, "the project has no clips"));
    }
    let tpl = prepare(&ctx, STAGE, &i.name, i.overrides.as_ref())?;
    let results = apply_to(&ctx, &tpl, &clips, dry).await?;
    let next = match results.first() {
        Some(r) if dry => vec![format!("rustler template apply {}@{} --clip-ids {} (without --dry-run)", tpl.name, tpl.version, r.clip_id)],
        Some(r) => vec![format!("rustler render preview-frame {} --out-t 1", r.clip_id), format!("rustler history undo {}", r.clip_id)],
        None => Vec::new(),
    };
    Ok(ApplyOut { template: tpl.name.clone(), version: tpl.version, clips: results, dry_run: dry.then_some(true), next })
}

// ---------------------------------------------------------------------------------------------
// template.diff
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffIn {
    /// `name[@version]`.
    pub a: String,
    /// `name[@version]`.
    pub b: String,
}

#[derive(Serialize, JsonSchema)]
pub struct DiffRow {
    /// JSON pointer.
    pub path: String,
    /// add | remove | replace.
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct DiffOut {
    pub a: String,
    pub b: String,
    pub same: bool,
    pub changes: Vec<DiffRow>,
}

async fn diff(ctx: Ctx, i: DiffIn) -> Result<DiffOut> {
    const STAGE: &str = "template.diff";
    let (sa, ta) = resolve(&ctx, STAGE, &i.a)?;
    let (sb, tb) = resolve(&ctx, STAGE, &i.b)?;
    let va = serde_json::to_value(&ta).unwrap_or(Value::Null);
    let vb = serde_json::to_value(&tb).unwrap_or(Value::Null);
    let patch = serde_json::to_value(json_patch::diff(&va, &vb)).unwrap_or(Value::Null);
    let changes: Vec<DiffRow> = patch
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|op| {
            let path = op.get("path")?.as_str()?.to_string();
            if path == "/version" {
                return None;
            }
            Some(DiffRow {
                op: op.get("op")?.as_str()?.to_string(),
                a: va.pointer(&path).cloned(),
                b: vb.pointer(&path).cloned(),
                path,
            })
        })
        .collect();
    Ok(DiffOut { a: format!("{}@{} ({})", sa.name, sa.version, sa.scope), b: format!("{}@{} ({})", sb.name, sb.version, sb.scope), same: changes.is_empty(), changes })
}

// ---------------------------------------------------------------------------------------------
// template.export / template.import
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BundleAsset {
    pub entry: AssetEntry,
    /// Path of the file inside the zip.
    pub zip_path: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BundleFile {
    /// The path the template refers to.
    pub original: PathBuf,
    pub zip_path: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Bundle {
    pub schema: String,
    pub template: String,
    #[serde(default)]
    pub assets: Vec<BundleAsset>,
    #[serde(default)]
    pub files: Vec<BundleFile>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportIn {
    /// `name[@version]`.
    pub name: String,
    /// The zip to write.
    pub out: PathBuf,
}

#[derive(Serialize, JsonSchema)]
pub struct ExportOut {
    pub out: PathBuf,
    pub template: String,
    /// Asset ids bundled.
    pub assets: Vec<String>,
    /// Plain files bundled.
    pub files: Vec<PathBuf>,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into())
}

async fn export(ctx: Ctx, i: ExportIn) -> Result<ExportOut> {
    const STAGE: &str = "template.export";
    let (s, t) = resolve(&ctx, STAGE, &i.name)?;
    check_refs(&ctx, STAGE, &t)?;
    let project = ctx.project_dir.as_deref();
    let rows = asset_cmds::load_rows(&ctx.home, project);
    let mut bundle = Bundle { schema: BUNDLE_SCHEMA.into(), template: format!("{}@{}", t.name, t.version), assets: Vec::new(), files: Vec::new() };
    let mut payload: Vec<(String, PathBuf)> = Vec::new();
    for r in t.refs() {
        match (&r.asset, &r.file) {
            (Some(id), _) => {
                if bundle.assets.iter().any(|a| &a.entry.id == id) {
                    continue;
                }
                let Some(row) = rows.iter().find(|x| &x.entry.id == id) else { continue };
                let Some(file) = row.file.clone().filter(|f| f.is_file()) else { continue };
                let zip_path = format!("assets/{}/{}", row.entry.kind.dir(), file_name_of(&file));
                payload.push((zip_path.clone(), file));
                bundle.assets.push(BundleAsset { entry: row.entry.clone(), zip_path });
            }
            (None, Some(f)) => {
                if bundle.files.iter().any(|x| &x.original == f) {
                    continue;
                }
                let zip_path = format!("files/{}-{}", bundle.files.len() + 1, file_name_of(f));
                payload.push((zip_path.clone(), f.clone()));
                bundle.files.push(BundleFile { original: f.clone(), zip_path });
            }
            _ => {}
        }
    }
    let spec = format!("{}@{}", s.name, s.version);
    let out = std::path::absolute(&i.out).unwrap_or(i.out.clone());
    let assets: Vec<String> = bundle.assets.iter().map(|a| a.entry.id.clone()).collect();
    let files: Vec<PathBuf> = bundle.files.iter().map(|f| f.original.clone()).collect();
    let next = vec![format!("rustler template import {}", out.display())];
    if ctx.dry_run {
        return Ok(ExportOut { out, template: spec, assets, files, bytes: 0, dry_run: Some(true), next });
    }
    let tjson = serde_json::to_vec_pretty(&t).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
    let bjson = serde_json::to_vec_pretty(&bundle).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
    let out2 = out.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if let Some(parent) = out2.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(STAGE, "create", parent, e))?;
        }
        let tmp = out2.with_extension(format!("zip.tmp{}", std::process::id()));
        let zerr = |e: &dyn std::fmt::Display| RustlerError::new(ErrorCode::Internal, STAGE, format!("zip: {e}"));
        let f = std::fs::File::create(&tmp).map_err(|e| io_err(STAGE, "create", &tmp, e))?;
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("template.json", opts).map_err(|e| zerr(&e))?;
        z.write_all(&tjson).map_err(|e| zerr(&e))?;
        z.start_file("bundle.json", opts).map_err(|e| zerr(&e))?;
        z.write_all(&bjson).map_err(|e| zerr(&e))?;
        for (name, path) in &payload {
            z.start_file(name.as_str(), opts).map_err(|e| zerr(&e))?;
            let mut r = std::fs::File::open(path).map_err(|e| io_err(STAGE, "read", path, e))?;
            std::io::copy(&mut r, &mut z).map_err(|e| zerr(&e))?;
        }
        let mut w = z.finish().map_err(|e| zerr(&e))?;
        w.flush().map_err(|e| zerr(&e))?;
        drop(w);
        std::fs::rename(&tmp, &out2).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io_err(STAGE, "write", &out2, e)
        })
    })
    .await
    .map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("export task failed: {e}")))??;
    let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    if let Ok(store) = ctx.store() {
        log_event(&ctx, &store, STAGE, json!({"template": spec, "out": out, "assets": assets.len(), "files": files.len()}))?;
    }
    Ok(ExportOut { out, template: spec, assets, files, bytes, dry_run: None, next })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportIn {
    /// A zip written by template.export.
    pub file: PathBuf,
    /// project (default when a project is open) | global: where the template and its assets go.
    #[serde(default)]
    pub scope: Option<Scope>,
}

#[derive(Serialize, JsonSchema)]
pub struct ImportOut {
    pub name: String,
    pub version: u32,
    pub scope: String,
    pub file: PathBuf,
    /// Assets added to the library.
    pub assets_added: Vec<String>,
    /// Assets already in the library (kept).
    pub assets_present: Vec<String>,
    /// Plain files unpacked (the template now points at these).
    pub files: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

fn read_entry(z: &mut zip::ZipArchive<std::fs::File>, name: &str, archive: &Path) -> Result<Vec<u8>> {
    const STAGE: &str = "template.import";
    let mut f = z.by_name(name).map_err(|e| verr(STAGE, format!("{} has no {name}: {e}", archive.display())).hint("pass a zip written by `rustler template export`"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| io_err(STAGE, "read", archive, e))?;
    Ok(buf)
}

async fn import(ctx: Ctx, i: ImportIn) -> Result<ImportOut> {
    const STAGE: &str = "template.import";
    let archive = i.file.clone();
    if !archive.is_file() {
        return Err(nf(STAGE, format!("no such file: {}", archive.display())));
    }
    let f = std::fs::File::open(&archive).map_err(|e| io_err(STAGE, "open", &archive, e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| verr(STAGE, format!("{} is not a zip: {e}", archive.display())))?;
    let mut t: Template = serde_json::from_slice(&read_entry(&mut z, "template.json", &archive)?)
        .map_err(|e| verr(STAGE, format!("template.json is not a valid template: {e}")))?;
    let bundle: Bundle = serde_json::from_slice(&read_entry(&mut z, "bundle.json", &archive)?).map_err(|e| verr(STAGE, format!("bundle.json is invalid: {e}")))?;
    validate_template(&t).map_err(|e| with_stage(e, STAGE))?;
    let scope = i.scope.unwrap_or_else(|| default_scope(&ctx));
    let tdir = dir_of(&ctx, scope)?;
    let aroot = match scope {
        Scope::Global => assets_root(&ctx.home),
        Scope::Project => project_assets_root(ctx.project_dir.as_deref().unwrap_or(Path::new("."))),
    };
    let project = ctx.project_dir.as_deref();
    let rows = asset_cmds::load_rows(&ctx.home, project);
    let dry = ctx.dry_run;
    let mut warnings = Vec::new();
    let (mut added, mut present) = (Vec::new(), Vec::new());

    // Plain files are unpacked to templates/files/<name>@<version>/ and the template points there.
    let fdir_of = |t: &Template| tdir.join("files").join(format!("{}@{}", t.name, t.version));
    let remapped = |t: &Template| -> Template {
        let fdir = fdir_of(t);
        let remap: HashMap<&PathBuf, PathBuf> = bundle.files.iter().map(|f| (&f.original, fdir.join(f.zip_path.rsplit('/').next().unwrap_or("file")))).collect();
        let mut t = t.clone();
        let fix = |p: &mut Option<PathBuf>| {
            if let Some(n) = p.as_ref().and_then(|x| remap.get(x)) {
                *p = Some(n.clone());
            }
        };
        for im in t.images.iter_mut().flatten() {
            fix(&mut im.file);
        }
        if let Some(a) = t.audio.as_mut() {
            if let Some(m) = a.music.as_mut() {
                fix(&mut m.file);
            }
            for s in a.sfx.iter_mut().flatten() {
                fix(&mut s.file);
            }
        }
        t
    };
    // Version: keep it when free; identical content is "already imported"; else the next free one.
    let existing: Vec<Stored> = all_stored(&ctx).into_iter().filter(|s| s.name == t.name).collect();
    let mut identical = false;
    if let Some(s) = existing.iter().find(|s| s.version == t.version) {
        if load(STAGE, s).ok().as_ref() == Some(&remapped(&t)) {
            identical = true;
        } else {
            let v = existing.iter().map(|s| s.version).max().unwrap_or(0) + 1;
            warnings.push(format!("{}@{} already exists with other content; imported as version {v}", t.name, t.version));
            t.version = v;
        }
    }

    for a in &bundle.assets {
        if rows.iter().any(|r| r.entry.id == a.entry.id && r.exists) {
            let same = rows.iter().find(|r| r.entry.id == a.entry.id).is_some_and(|r| r.entry.sha256.is_empty() || a.entry.sha256.is_empty() || r.entry.sha256 == a.entry.sha256);
            if !same {
                warnings.push(format!("asset '{}' already exists with a different file; kept the existing one", a.entry.id));
            }
            present.push(a.entry.id.clone());
            continue;
        }
        added.push(a.entry.id.clone());
        if dry {
            continue;
        }
        let dir = aroot.join(a.entry.kind.dir());
        std::fs::create_dir_all(&dir).map_err(|e| io_err(STAGE, "create", &dir, e))?;
        let fname = a.zip_path.rsplit('/').next().unwrap_or("file").to_string();
        let mut dest = dir.join(&fname);
        let mut n = 2;
        while dest.exists() && a.entry.kind != AssetKind::CaptionStyle {
            let p = Path::new(&fname);
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = p.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
            dest = dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        let bytes = read_entry(&mut z, &a.zip_path, &archive)?;
        std::fs::write(&dest, &bytes).map_err(|e| io_err(STAGE, "write", &dest, e))?;
        let mut entry = a.entry.clone();
        entry.path = format!("{}/{}", a.entry.kind.dir(), file_name_of(&dest));
        update_manifest(&aroot, |m| {
            upsert(m, entry.clone());
            Ok(())
        })
        .map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
    }

    let mut files = Vec::new();
    let fdir = fdir_of(&t);
    for f in &bundle.files {
        let dest = fdir.join(f.zip_path.rsplit('/').next().unwrap_or("file"));
        if !dry && !identical {
            std::fs::create_dir_all(&fdir).map_err(|e| io_err(STAGE, "create", &fdir, e))?;
            let bytes = read_entry(&mut z, &f.zip_path, &archive)?;
            std::fs::write(&dest, &bytes).map_err(|e| io_err(STAGE, "write", &dest, e))?;
        }
        files.push(dest);
    }
    let t = remapped(&t);
    let file = tdir.join(format!("{}@{}.json", t.name, t.version));
    let spec = format!("{}@{}", t.name, t.version);
    let next = vec![format!("rustler template show {spec}"), format!("rustler template apply {spec} --clip-ids <clip_id> --dry-run")];
    if dry {
        return Ok(ImportOut { name: t.name, version: t.version, scope: scope_label(scope).into(), file, assets_added: added, assets_present: present, files, warnings, dry_run: Some(true), next });
    }
    let file = if identical {
        warnings.push(format!("{spec} is already installed with the same content"));
        existing.iter().find(|s| s.version == t.version).and_then(|s| s.file.clone()).unwrap_or(file)
    } else {
        write_template(STAGE, &tdir, &t)?
    };
    if let Ok(store) = ctx.store() {
        log_event(&ctx, &store, STAGE, json!({"template": spec, "assets_added": added, "files": files.len()}))?;
    }
    Ok(ImportOut { name: t.name, version: t.version, scope: scope_label(scope).into(), file, assets_added: added, assets_present: present, files, warnings, dry_run: None, next })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    let read = Annotations { read_only: true, ..write };
    reg.register(typed(
        "template.create",
        "Save a reusable look as a template (version 1): capture a clip's whole look (--from-clip: caption style/position/words per line, layout, hook title look, text looks, brand images, progress bar, loudness/denoise/music/sfx and effect patterns, transitions, canvas; text content only with --include-text) or write it as JSON (--template). Stored in templates/<name>@1.json (project) or <home>/templates/ (--scope global)",
        &["name"],
        write,
        vec![
            Example { description: "capture the look of a finished clip", cli: "rustler template create my-look --from-clip clp_x" },
            Example { description: "write a template by hand (PowerShell-safe)", cli: "rustler template create my-look --input-json @template.json" },
        ],
        create,
    ));
    reg.register(typed(
        "template.update",
        "Save a new version of a template: re-capture --from-clip, a whole --template, or a merge --patch onto the latest version (old versions stay; name@N picks one). Updating a built-in starter saves your own version that shadows it",
        &["name"],
        write,
        vec![Example { description: "move the captions up in a new version", cli: "rustler template update my-look --patch '{\"captions\":{\"position_y\":0.65}}'" }],
        update,
    ));
    reg.register(typed(
        "template.show",
        "One template (name = latest, name@N = that version) with its versions, the assets/files it needs and any that are missing here",
        &["name"],
        read,
        vec![Example { description: "see a built-in starter", cli: "rustler template show podcast-viral" }],
        show,
    ));
    reg.register(typed(
        "template.list",
        "Every template: project, global (<home>/templates) and the built-in starters (podcast-viral, streamer-split), with versions and descriptions",
        &[],
        read,
        vec![Example { description: "list templates", cli: "rustler template list" }],
        list,
    ));
    reg.register(typed(
        "template.delete",
        "Delete a template (every version in its scope) or one version (name@N); built-in starters cannot be deleted",
        &["name"],
        Annotations { destructive: true, ..write },
        vec![Example { description: "delete version 2", cli: "rustler template delete my-look@2" }],
        delete,
    ));
    reg.register(typed(
        "template.apply",
        "Style clips with a template: builds JSON-Patch ops for each clip's timeline and applies them through timeline.apply (validated; one revision per clip; history undo reverts it). Items the template adds are tagged by_template and replaced on re-apply; absent sections leave the clip alone. A referenced asset/font/file that is missing fails with not_found naming it. --overrides is a merge patch for this run; --dry-run shows the patch",
        &["name"],
        write,
        vec![
            Example { description: "preview the changes", cli: "rustler template apply podcast-viral --clip-ids clp_x --dry-run" },
            Example { description: "style every clip", cli: "rustler template apply my-look@2 --all" },
        ],
        apply,
    ));
    reg.register(typed(
        "template.diff",
        "What differs between two templates or versions (JSON paths with both values)",
        &["a", "b"],
        read,
        vec![Example { description: "compare two versions", cli: "rustler template diff my-look@1 my-look@2" }],
        diff,
    ));
    reg.register(typed(
        "template.export",
        "Write a template plus every asset (fonts, caption style, images, music, sfx) and file it references into one zip, for another machine or project",
        &["name"],
        write,
        vec![Example { description: "bundle a template", cli: "rustler template export my-look --out my-look.zip" }],
        export,
    ));
    reg.register(typed(
        "template.import",
        "Install a template zip from template.export: its assets go into the library (existing ids are kept), its plain files are unpacked next to the template, and the template is saved (a taken version becomes the next free one)",
        &["file"],
        write,
        vec![Example { description: "install a bundle into this project", cli: "rustler template import my-look.zip" }],
        import,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_parse_and_validate() {
        let ts = builtin_templates();
        assert_eq!(ts.len(), 2);
        for t in &ts {
            validate_template(t).unwrap();
            assert!(t.refs().is_empty(), "starters must not need assets: {:?}", t.refs());
        }
    }

    #[test]
    fn instances_cover_patterns() {
        assert_eq!(instances(PlaceAt::Every, Some(5.0), None, &[], 16.0), vec![5.0, 10.0, 15.0]);
        assert_eq!(instances(PlaceAt::Every, Some(5.0), None, &[], 15.2), vec![5.0, 10.0]);
        assert_eq!(instances(PlaceAt::Cuts, None, None, &[1.4, 3.4], 5.0), vec![1.4, 3.4]);
        assert!(instances(PlaceAt::Time, None, Some(9.0), &[], 5.0).is_empty());
        assert_eq!(parse_file_name(Path::new("a-b@12.json")), Some(("a-b".into(), 12)));
        assert_eq!(parse_file_name(Path::new("a-b.json")), None);
    }

    #[test]
    fn shot_class_from_smart_decisions() {
        // The reasons smart_cmds' shot classifier writes.
        assert_eq!(class_from_decision("center-crop for source 0.00-4.00 s", "one face in 90% of samples -> center-crop + smooth track"), Some((ShotClass::OneFace, false)));
        assert_eq!(class_from_decision("center-crop ...", "2 faces in 80% of samples -> center-crop + active speaker"), Some((ShotClass::MultiFace, false)));
        assert_eq!(class_from_decision("zoom-fit for source 1-2 s", "graphics (flat 70%, edges 1.2%), faces in 0% of samples; content box ..."), Some((ShotClass::Graphics, true)));
        assert_eq!(class_from_decision("zoom-fit for source", "faces in only 10% of samples; content box x"), Some((ShotClass::NoFace, true)));
        assert_eq!(class_from_decision("blur-fit", "something else"), None);
        let merged = "one face in 100% of samples -> center-crop + smooth track; then 2 faces in 90% of samples -> center-crop + active speaker";
        assert_eq!(class_from_decision("center-crop", merged), Some((ShotClass::MultiFace, false)));
        let s = ShotInfo { class: ShotClass::NoFace, face_x: None, zoom_fit: true };
        assert!(shot_matches(ShotWhen::Screen, &s) && shot_matches(ShotWhen::NoFace, &s) && !shot_matches(ShotWhen::Graphics, &s));
        let g = ShotInfo { class: ShotClass::Graphics, face_x: None, zoom_fit: false };
        assert!(shot_matches(ShotWhen::Screen, &g) && shot_matches(ShotWhen::NoFace, &g));
    }

    #[test]
    fn amount_ranges_match_media() {
        for k in [EffectKind::PunchIn, EffectKind::ZoomSlow, EffectKind::Shake, EffectKind::Flash] {
            let m: rustler_media::compile::EffectKind = serde_json::from_value(serde_json::to_value(k).unwrap()).unwrap();
            assert_eq!(k.amount_range(), m.amount_range());
        }
    }
}
