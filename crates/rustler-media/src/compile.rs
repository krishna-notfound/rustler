//! Filtergraph compiler: Clip + sources -> one ffmpeg invocation. (Task 9)
//!
//! The graph itself is a pure function of the inputs ([`build_graph`]); [`compile_with`] also
//! stages the side files the graph names by bare relative path (`graph.txt`, `captions.ass`,
//! `fonts/`, `crop_*.cmd`) into `workdir`, because ffmpeg runs with cwd = workdir (no Windows
//! drive-colon escaping inside the filter script; autoclip trick, MIT).
//!
//! Render-graph discipline adapted from bridgeclip `layout_renderer.py` (MIT): frames are allocated
//! on the cumulative output timeline so rounding never accumulates at cuts, timestamps are rebuilt
//! after concat/loudnorm, and NaNs from loudnorm on digital silence are scrubbed before AAC.
//! Music ducking from opensource-clipping `audio_bgm.py` (MIT) with `amix normalize=0` and fades.

use crate::encoders::EncoderChoice;
use crate::layers::{Cutaway, ImageLayer, LayerCtx, LayerInput, MaskLayer, VChain};
use rustler_core::{
    Background, Canvas, Clip, ErrorCode, Fit, MediaProbe, Rect, Region, RustlerError, SegmentId,
    SourceId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const STAGE: &str = "render.compile";
const SR: u64 = 48_000;
/// Segment end may exceed the probed duration by this much (container rounding).
const END_TOLERANCE: f64 = 0.05;
pub(crate) const AUDIO_FORMAT: &str = "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo";
const FINITE_AUDIO: &str = "aeval='if(isnan(val(ch))+isinf(val(ch)),0,val(ch))':c=same";
pub const GRAPH_FILE: &str = "graph.txt";
pub const PREVIEW_GRAPH_FILE: &str = "graph_preview.txt";
pub const ASS_FILE: &str = "captions.ass";
/// Hook titles / text overlays, burned after the captions in the same graph.
pub const OVERLAYS_ASS_FILE: &str = "overlays.ass";
/// Audio-only graph of the loudness measurement pass (two-pass loudnorm).
pub const LOUDNESS_GRAPH_FILE: &str = "graph_loudness.txt";
/// Audio-only graph of the noise-floor pass (denoise).
pub const NOISE_GRAPH_FILE: &str = "graph_noise.txt";
pub const FONTS_DIR: &str = "fonts";
/// Font family of text overlays (`~/.rustler/fonts/Poppins/Poppins-ExtraBold.ttf`).
pub const OVERLAY_FONT: &str = "Poppins ExtraBold";

#[derive(Clone, Debug, PartialEq)]
pub struct SourceMedia {
    pub id: SourceId,
    pub path: PathBuf,
    pub probe: MediaProbe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledRender {
    /// Full ffmpeg argv (without the program).
    pub args: Vec<OsString>,
    /// Contents of `workdir/graph.txt` (single-pass loudnorm; [`crate::render::render`] swaps in
    /// the measured filter when `loudness` is set and the measurement succeeds).
    pub filter_script: String,
    pub workdir: PathBuf,
    /// Two-pass loudnorm: the measurement pass to run before the final encode.
    pub loudness: Option<LoudnessPass>,
    /// Denoise: the noise-floor pass to run first (before the loudness pass).
    pub noise: Option<NoisePass>,
}

/// Audio-only pass that measures the dialogue noise floor for `afftdn`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoisePass {
    /// ffmpeg argv (without the program); cwd = workdir; reads `graph_noise.txt`.
    pub args: Vec<OsString>,
    /// The denoise filter text (default floor) inside `filter_script` and `graph_loudness.txt`.
    pub filter: String,
    pub denoise: Denoise,
}

/// First pass of two-pass loudnorm: an audio-only run of the same audio graph into `-f null`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoudnessPass {
    /// ffmpeg argv (without the program); cwd = workdir; reads `graph_loudness.txt`.
    pub args: Vec<OsString>,
    /// The single-pass `loudnorm=I=..:TP=..:LRA=..` text inside `filter_script`, replaced by
    /// [`measured_loudnorm`] for the final pass.
    pub filter: String,
}

/// Values printed by `loudnorm=...:print_format=json` (the first pass).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct LoudnessMeasurement {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

/// Parses the last JSON block loudnorm printed to stderr. `None` unless all five values are
/// present and finite (digital silence prints `-inf`).
pub fn parse_loudnorm_json(stderr: &str) -> Option<LoudnessMeasurement> {
    let start = stderr.rfind("\"input_i\"")?;
    let open = stderr[..start].rfind('{')?;
    let close = open + stderr[open..].find('}')?;
    let v: serde_json::Value = serde_json::from_str(&stderr[open..=close]).ok()?;
    let f = |k: &str| -> Option<f64> {
        let x = match &v[k] {
            serde_json::Value::String(s) => s.trim().parse::<f64>().ok()?,
            serde_json::Value::Number(n) => n.as_f64()?,
            _ => return None,
        };
        x.is_finite().then_some(x)
    };
    Some(LoudnessMeasurement {
        input_i: f("input_i")?,
        input_tp: f("input_tp")?,
        input_lra: f("input_lra")?,
        input_thresh: f("input_thresh")?,
        target_offset: f("target_offset")?,
    })
}

/// Second-pass filter: `base` (`loudnorm=I=..:TP=..:LRA=..`) plus the measured values and
/// `linear=true` (ffmpeg drops to dynamic mode by itself when linear gain would break TP/LRA;
/// the measured offset still lands the integrated loudness on target).
pub fn measured_loudnorm(base: &str, m: &LoudnessMeasurement) -> String {
    format!(
        "{base}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
        num(m.input_i),
        num(m.input_tp),
        num(m.input_lra),
        num(m.input_thresh),
        num(m.target_offset)
    )
}

/// Background music under the whole clip.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct MusicBed {
    pub path: PathBuf,
    /// Gain applied to the music before mixing (default -18 dB).
    pub volume_db: f64,
    /// Sidechain-duck the music under speech (default true).
    pub duck: bool,
    /// Seconds (default 0.5).
    pub fade_in: f64,
    /// Seconds (default 1.0).
    pub fade_out: f64,
}

impl MusicBed {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            volume_db: -18.0,
            duck: true,
            fade_in: 0.5,
            fade_out: 1.0,
        }
    }
}

/// One crop-path keyframe: at segment-relative time `t` (seconds from the segment start) the
/// crop's left edge is `x` (normalized 0..1 of the source display width).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct CropKey {
    pub t: f64,
    pub x: f64,
}

/// Look of a text overlay.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayStyle {
    /// Big hook title near the top (default y 0.12, boxed).
    #[default]
    HookTop,
    /// Smaller label under the captions (default y 0.80, boxed).
    LowerThird,
    /// Outlined white text, no box (default y 0.50).
    Plain,
    /// Viral hook: two bold uppercase lines, ~0.10 of the width, thick outline, pops in.
    HookBig,
    /// Viral hook: black text on an opaque yellow box, slides in from above.
    HookBoxYellow,
    /// Viral hook: yellow text with a thick black outline, typed out letter by letter.
    HookOutline,
    /// Viral hook: left-aligned dark box, "POV:" in yellow, slides in from the left.
    HookPov,
    /// Viral hook: ends in a big red "?", zooms in.
    HookQuestion,
}

/// Entry animation of a text overlay.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayAnim {
    /// Static (appears with a short fade).
    None,
    /// Scales 70% -> 112% -> 100% in a quarter of a second.
    Pop,
    /// Starts 140% large and settles to 100%.
    ZoomIn,
    /// Slides in (from above; left-aligned styles from the left).
    Slide,
    /// Letters appear one by one (about 1 s for a 25 character hook).
    Typewriter,
}

/// Colours and flags that make up a style's look (ASS `&HBBGGRR` colours).
#[derive(Clone, Copy, Debug)]
struct OverlayLook {
    /// Text colour on the box.
    text_on_box: &'static str,
    /// Box colour.
    box_colour: &'static str,
    /// Text colour without a box (drawn with a black outline).
    text_plain: &'static str,
    /// Accent colour of "POV:" / "?" ([`OverlayStyle::HookPov`], [`OverlayStyle::HookQuestion`]).
    accent: &'static str,
    /// Left-aligned block instead of centred.
    left: bool,
    /// Outline thickness of the unboxed text as a fraction of the font size.
    outline_factor: f64,
    default_anim: OverlayAnim,
}

/// Common minimum font size (fraction of the canvas width) of every hook preset; longer text wraps to
/// two lines (and only then shrinks) so it stays inside the safe area.
pub const HOOK_MIN_SIZE_FACTOR: f64 = 0.10;

impl OverlayStyle {
    pub fn default_position_y(self) -> f64 {
        match self {
            Self::HookTop => 0.12,
            Self::LowerThird => 0.80,
            Self::Plain => 0.50,
            Self::HookBig => 0.17,
            Self::HookBoxYellow => 0.14,
            Self::HookOutline => 0.16,
            Self::HookPov => 0.15,
            Self::HookQuestion => 0.15,
        }
    }
    /// Font size = canvas width * factor.
    pub fn default_font_size_factor(self) -> f64 {
        match self {
            Self::HookTop => 0.085,
            Self::LowerThird => 0.05,
            Self::Plain => 0.065,
            Self::HookBig => HOOK_MIN_SIZE_FACTOR,
            Self::HookBoxYellow => HOOK_MIN_SIZE_FACTOR,
            Self::HookOutline => HOOK_MIN_SIZE_FACTOR,
            Self::HookPov => HOOK_MIN_SIZE_FACTOR,
            Self::HookQuestion => HOOK_MIN_SIZE_FACTOR,
        }
    }
    pub fn default_box(self) -> bool {
        !matches!(self, Self::Plain | Self::HookBig | Self::HookOutline)
    }
    /// The five viral hook presets, in `overlay.hook_styles` order.
    pub const HOOKS: [OverlayStyle; 5] =
        [Self::HookBig, Self::HookBoxYellow, Self::HookOutline, Self::HookPov, Self::HookQuestion];
    /// True for the viral hook presets.
    pub fn is_hook_preset(self) -> bool {
        Self::HOOKS.contains(&self)
    }
    /// Kebab-case name (what `--style` takes).
    pub fn name(self) -> &'static str {
        match self {
            Self::HookTop => "hook-top",
            Self::LowerThird => "lower-third",
            Self::Plain => "plain",
            Self::HookBig => "hook-big",
            Self::HookBoxYellow => "hook-box-yellow",
            Self::HookOutline => "hook-outline",
            Self::HookPov => "hook-pov",
            Self::HookQuestion => "hook-question",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::HookTop => "Big white hook on a dark box near the top (default).",
            Self::LowerThird => "Small label on a dark box under the captions.",
            Self::Plain => "White outlined text without a box, centre of the frame.",
            Self::HookBig => "Two bold uppercase lines (10% of the width), white with a thick black outline, pops in.",
            Self::HookBoxYellow => "Black text on an opaque yellow box, slides in from above.",
            Self::HookOutline => "Yellow text with a thick black outline, typed out letter by letter.",
            Self::HookPov => "Left-aligned dark box; \"POV:\" (added when missing) in yellow; slides in from the left.",
            Self::HookQuestion => "Dark box; the closing \"?\" (added when missing) is big and red; zooms in.",
        }
    }
    /// Entry animation when the overlay does not name one.
    pub fn default_anim(self) -> OverlayAnim {
        self.look().default_anim
    }

    fn look(self) -> OverlayLook {
        const WHITE: &str = "&H00FFFFFF";
        const DARK: &str = "&H00141414";
        const YELLOW: &str = "&H0000E0FF"; // #FFE000
        const RED: &str = "&H004D4DFF"; // #FF4D4D
        let base = OverlayLook {
            text_on_box: WHITE,
            box_colour: DARK,
            text_plain: WHITE,
            accent: WHITE,
            left: false,
            outline_factor: 0.07,
            default_anim: OverlayAnim::None,
        };
        match self {
            Self::HookTop | Self::LowerThird | Self::Plain => base,
            Self::HookBig => OverlayLook { outline_factor: 0.12, default_anim: OverlayAnim::Pop, ..base },
            Self::HookBoxYellow => OverlayLook {
                text_on_box: DARK,
                box_colour: YELLOW,
                text_plain: YELLOW,
                default_anim: OverlayAnim::Slide,
                ..base
            },
            Self::HookOutline => OverlayLook {
                text_plain: YELLOW,
                outline_factor: 0.13,
                default_anim: OverlayAnim::Typewriter,
                ..base
            },
            Self::HookPov => OverlayLook { accent: YELLOW, left: true, default_anim: OverlayAnim::Slide, ..base },
            Self::HookQuestion => OverlayLook { accent: RED, default_anim: OverlayAnim::ZoomIn, ..base },
        }
    }
}

fn default_overlay_y() -> f64 {
    OverlayStyle::HookTop.default_position_y()
}
fn default_overlay_size() -> f64 {
    OverlayStyle::HookTop.default_font_size_factor()
}
fn default_true() -> bool {
    true
}

/// A text overlay (hook title, label) on the output timeline.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct TextOverlay {
    pub id: String,
    pub text: String,
    /// Output time (seconds) the overlay appears.
    pub start: f64,
    /// Output time (seconds) the overlay disappears.
    pub end: f64,
    #[serde(default)]
    pub style: OverlayStyle,
    /// Vertical centre of the text block, 0 = top .. 1 = bottom of the canvas.
    #[serde(default = "default_overlay_y")]
    pub position_y: f64,
    /// Font size as a fraction of the canvas width (hook-top 0.085).
    #[serde(default = "default_overlay_size")]
    pub font_size_factor: f64,
    /// Opaque dark box behind the text (white text either way).
    #[serde(rename = "box", default = "default_true")]
    pub boxed: bool,
    /// Entry animation; `None` = the style's default (hook presets animate, the others do not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<OverlayAnim>,
    /// Id of the sound effect (`sfx_<n>` in the clip's `sfx`) added with the hook; removed with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx_id: Option<String>,
}

impl TextOverlay {
    /// Overlay with the style's default position, size and box.
    pub fn new(id: impl Into<String>, text: impl Into<String>, start: f64, end: f64, style: OverlayStyle) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            start,
            end,
            style,
            position_y: style.default_position_y(),
            font_size_factor: style.default_font_size_factor(),
            boxed: style.default_box(),
            anim: None,
            sfx_id: None,
        }
    }

    /// The entry animation in effect.
    pub fn effective_anim(&self) -> OverlayAnim {
        self.anim.unwrap_or_else(|| self.style.default_anim())
    }
}

/// Data carried alongside the frozen Clip model (5-hour scope additions).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct CompileExtras {
    #[serde(default)]
    pub music: Option<MusicBed>,
    /// Crop path per segment; it drives the crop `x` of that segment's first region
    /// (`layout.regions[0]`). Width/height stay fixed (sendcmd changes only `x`).
    #[serde(default)]
    pub crop_paths: HashMap<SegmentId, Vec<CropKey>>,
    /// Measure loudness in a fast audio-only pass first, then normalize with the measured values
    /// (default true). Only used when the clip has loudnorm on.
    #[serde(default = "default_true")]
    pub two_pass_loudness: bool,
    /// Text overlays (hook titles) burned on top of the captions.
    #[serde(default)]
    pub overlays: Vec<TextOverlay>,
    /// Speech denoise (`highpass` 80 Hz + `afftdn`) on the joined dialogue, before the mix.
    #[serde(default)]
    pub denoise: Denoise,
    /// Per-segment gain (dB) applied to that segment's audio before the join.
    #[serde(default)]
    pub segment_gain_db: HashMap<SegmentId, f64>,
    /// Whole-dialogue gain (dB) after denoise, before the mix and loudnorm.
    #[serde(default)]
    pub speech_gain_db: f64,
    /// Sound effects placed on the edit timeline.
    #[serde(default)]
    pub sfx: Vec<SfxCue>,
    /// Transitions between consecutive segments (`xfade` + `acrossfade`).
    #[serde(default)]
    pub transitions: Vec<Transition>,
    /// Camera effects on the composed canvas (before captions).
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// Progress bar across the whole rendered clip (on top of everything).
    #[serde(default)]
    pub progress_bar: Option<ProgressBar>,
    /// Image layers (logos, stickers, GIFs). Timing must be resolved (`out_start`/`out_end`;
    /// [`crate::layers::resolve_timing`]); edit timeline like the overlays.
    #[serde(default)]
    pub images: Vec<ImageLayer>,
    /// Masks over the source picture (blur / box / pixelate), resolved timing.
    #[serde(default)]
    pub masks: Vec<MaskLayer>,
    /// Cutaways (full-frame or picture-in-picture), resolved timing and `media.file` set.
    #[serde(default)]
    pub cutaways: Vec<Cutaway>,
    /// Playback speed per segment (0.5..2.0; missing = 1). Edit-timeline times inside the
    /// segment are compressed / stretched on the rendered timeline ([`TransitionMap`]).
    #[serde(default)]
    pub segment_speed: HashMap<SegmentId, f64>,
    /// Freeze frames on the edit timeline; each lengthens the rendered clip by its duration.
    #[serde(default)]
    pub freezes: Vec<Freeze>,
}

impl Default for CompileExtras {
    fn default() -> Self {
        Self {
            music: None,
            crop_paths: HashMap::new(),
            two_pass_loudness: true,
            overlays: Vec::new(),
            denoise: Denoise::Off,
            segment_gain_db: HashMap::new(),
            speech_gain_db: 0.0,
            sfx: Vec::new(),
            transitions: Vec::new(),
            effects: Vec::new(),
            progress_bar: None,
            images: Vec::new(),
            masks: Vec::new(),
            cutaways: Vec::new(),
            segment_speed: HashMap::new(),
            freezes: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Audio post, transitions, effects, progress bar (B1)
// ---------------------------------------------------------------------------------------------
//
// Time convention: every time stored in the extras (overlays, sfx `at`, effect start/end) is on
// the *edit timeline* (the clip's `TimeMap`, what `clip show` prints). Transitions overlap
// consecutive segments, so the rendered file is shorter; [`TransitionMap`] maps edit time to
// rendered time. The cut point of a transition sits in the middle of the overlap: material
// before a boundary keeps its time but is clamped to end at the overlap midpoint, material after
// it moves earlier by the overlap and is clamped to start at the midpoint. Captions, overlays,
// sfx and effects all go through the same map, so they stay in sync with the picture.

/// Speech denoise strength.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Denoise {
    #[default]
    Off,
    Light,
    Medium,
    Strong,
}

/// Noise floor assumed when the measurement pass cannot run (dBFS RMS).
pub const DEFAULT_NOISE_FLOOR_DB: f64 = -50.0;

impl Denoise {
    /// (afftdn `nr` dB, `nf` margin above the measured noise floor dB).
    fn params(self) -> Option<(f64, f64)> {
        match self {
            Self::Off => None,
            Self::Light => Some((12.0, 4.0)),
            Self::Medium => Some((20.0, 8.0)),
            Self::Strong => Some((30.0, 12.0)),
        }
    }

    /// `highpass=f=80,afftdn=nr=..:nf=..` for a dialogue whose noise floor is `noise_floor_db`
    /// (dBFS RMS of the quiet parts); `None` when off. afftdn only removes noise up to its `nf`,
    /// so `nf` sits a strength-dependent margin above the measured floor. Measured with this
    /// FFmpeg build: white noise under a tone drops 12 / 20 / 30 dB (light / medium / strong),
    /// pink noise under real speech (jfk.wav) 8 / 12 / 19 dB; the speech level moves < 0.1 dB.
    pub fn filter_for(self, noise_floor_db: f64) -> Option<String> {
        let (nr, margin) = self.params()?;
        let floor = if noise_floor_db.is_finite() { noise_floor_db } else { DEFAULT_NOISE_FLOOR_DB };
        let nf = (floor + margin).clamp(-80.0, -20.0);
        Some(format!("highpass=f=80,afftdn=nr={}:nf={}", num(nr), num((nf * 10.0).round() / 10.0)))
    }

    /// The filter at the default floor (what the graph holds until the noise pass measured it).
    pub fn filter(self) -> Option<String> {
        self.filter_for(DEFAULT_NOISE_FLOOR_DB)
    }
}

/// Noise floor (dBFS) from the noise pass log: the 10th percentile of the 50 ms RMS windows
/// (digital silence below -90 dB ignored), but never closer than 12 dB to the median window,
/// so audio without pauses (continuous speech, a tone) is not mistaken for noise.
/// `None` with fewer than 5 usable windows.
pub fn noise_floor_from_log(log: &str) -> Option<f64> {
    let key = "lavfi.astats.Overall.RMS_level=";
    let mut v: Vec<f64> = log
        .lines()
        .filter_map(|l| l.find(key).map(|i| &l[i + key.len()..]))
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .filter(|x| x.is_finite() && *x > -90.0)
        .collect();
    if v.len() < 5 {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 10].min(v[v.len() / 2] - 12.0))
}

/// A sound effect, already resolved to an edit-timeline time.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct SfxCue {
    pub id: String,
    pub path: PathBuf,
    /// Edit-timeline time (seconds) where the effect starts.
    pub at: f64,
    /// Gain (dB) before mixing (default -6).
    pub volume_db: f64,
}

/// Transition between two consecutive segments.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "lowercase")]
pub enum TransitionKind {
    /// Hard cut (no overlap).
    Cut,
    /// Dip through black (`xfade=fadeblack`).
    Fade,
    /// Cross-dissolve: the two shots blend (`xfade=fade`).
    Dissolve,
    Wipeleft,
    Wiperight,
    Slideup,
    Zoomin,
    /// Quick white flash at the cut (`xfade=fadewhite`; default 0.1 s).
    Flash,
    /// Quick black flash at the cut (`xfade=fadeblack`; default 0.1 s).
    Flashblack,
    /// Fast whip: both shots smear horizontally into the cut (`xfade=hblur`; default 0.2 s).
    Whip,
}

impl TransitionKind {
    /// `xfade` transition name; `None` for a hard cut.
    pub fn xfade_name(self) -> Option<&'static str> {
        match self {
            Self::Cut => None,
            Self::Fade | Self::Flashblack => Some("fadeblack"),
            Self::Dissolve => Some("fade"),
            Self::Wipeleft => Some("wipeleft"),
            Self::Wiperight => Some("wiperight"),
            Self::Slideup => Some("slideup"),
            Self::Zoomin => Some("zoomin"),
            Self::Flash => Some("fadewhite"),
            Self::Whip => Some("hblur"),
        }
    }

    /// Overlap in seconds when `transition.set` is not given a duration.
    pub fn default_duration(self) -> f64 {
        match self {
            Self::Flash | Self::Flashblack => 0.1,
            Self::Whip => 0.2,
            _ => 0.3,
        }
    }
}

/// A transition after segment `after_segment_id` (into the next segment).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Transition {
    pub after_segment_id: SegmentId,
    pub kind: TransitionKind,
    /// Overlap in seconds (rounded to whole frames; default 0.3).
    pub duration: f64,
}

/// Camera effect on the composed canvas.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    /// Instant zoom to `amount` (default 1.15) for the range.
    PunchIn,
    /// Linear zoom 1.0 -> `amount` (default 1.15) over the range.
    ZoomSlow,
    /// Small sinusoidal shake; `amount` = amplitude as a fraction of the frame (default 0.02).
    Shake,
    /// White flash fading out over the range (`amount` is not used).
    Flash,
}

impl EffectKind {
    pub fn default_amount(self) -> f64 {
        match self {
            Self::PunchIn | Self::ZoomSlow => 1.15,
            Self::Shake => 0.02,
            Self::Flash => 1.0,
        }
    }
    /// Allowed `amount` range (inclusive).
    pub fn amount_range(self) -> (f64, f64) {
        match self {
            Self::PunchIn | Self::ZoomSlow => (1.01, 2.0),
            Self::Shake => (0.002, 0.08),
            Self::Flash => (0.0, 1.0),
        }
    }
}

/// An effect over an edit-timeline range.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Effect {
    pub id: String,
    pub kind: EffectKind,
    pub start: f64,
    pub end: f64,
    /// Zoom factor (punch-in, zoom-slow) or shake amplitude; default by kind.
    #[serde(default)]
    pub amount: Option<f64>,
    /// Zoom centre for punch-in / zoom-slow as a canvas fraction (0..1, e.g. the face); the
    /// frame centre when absent. Pulled in so the zoomed window never leaves the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_y: Option<f64>,
    /// Other stored fields (e.g. `by_template`, `template_role`), kept on read-modify-write.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Effect {
    pub fn amount_or_default(&self) -> f64 {
        self.amount.unwrap_or_else(|| self.kind.default_amount())
    }

    /// Source-quad offset (ox, oy) that centres a zoom of `z` on the focus point, clamped so the
    /// window stays inside the frame.
    pub fn focus_offset(&self, z: f64) -> (f64, f64) {
        let lim = (0.5 - 0.5 / z.max(1.0)).max(0.0);
        let off = |f: Option<f64>| f.filter(|v| v.is_finite()).map_or(0.0, |v| (v - 0.5).clamp(-lim, lim));
        (off(self.focus_x), off(self.focus_y))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
}

fn default_bar_color() -> String {
    "#FFE000".into()
}
fn default_bar_height() -> u32 {
    8
}

/// A bar that grows from 0 to the full canvas width over the rendered clip.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct ProgressBar {
    /// `#RRGGBB` (default `#FFE000`).
    #[serde(default = "default_bar_color")]
    pub color: String,
    /// Pixels (default 8; rounded up to even).
    #[serde(default = "default_bar_height")]
    pub height: u32,
    #[serde(default)]
    pub position: BarPosition,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self { color: default_bar_color(), height: default_bar_height(), position: BarPosition::Top }
    }
}

/// Slowest / fastest segment speed (`setpts` + `atempo`).
pub const MIN_SPEED: f64 = 0.5;
pub const MAX_SPEED: f64 = 2.0;
/// Longest freeze frame (seconds).
pub const MAX_FREEZE: f64 = 10.0;

/// A freeze frame: at edit-timeline time `out_t` the picture holds for `duration` seconds
/// (the audio is silent meanwhile), then the clip continues. A freeze on a segment boundary
/// holds the first frame of the next segment; one at the very end holds the last frame.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Freeze {
    pub id: String,
    /// Edit-timeline seconds (like overlays and effects).
    pub out_t: f64,
    /// Seconds the frame holds (0 < duration <= 10).
    pub duration: f64,
}

/// `#RRGGBB` / `RRGGBB` -> ffmpeg `0xRRGGBB`; `None` when malformed.
pub fn ffmpeg_color(hex: &str) -> Option<String> {
    let h = hex.trim().trim_start_matches('#');
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("0x{}", h.to_ascii_uppercase()))
}

/// Edit timeline -> rendered timeline under the clip's segment speeds, freeze frames and
/// transitions (see the time convention above). Built from the clip's segments and fps;
/// transitions are rounded to whole frames.
///
/// Two steps: the *retime* (speed compresses / stretches a segment's span, a freeze inserts a
/// hold) gives the "retimed" timeline, then transitions overlap the retimed segments. A time
/// exactly on a freeze maps to the start of the hold, so an item starting there shows during
/// the freeze. With no speed and no freeze the retime is the identity.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionMap {
    /// Retimed segment boundaries `0 = B0 < B1 < .. < Bn` (freezes inside a segment count to it).
    bounds: Vec<f64>,
    /// Edit-timeline segment boundaries `0 = b0 < b1 < .. < bn = edit duration`.
    edit_bounds: Vec<f64>,
    /// Speed per segment (1 = normal).
    speeds: Vec<f64>,
    /// (segment index, edit offset inside the segment, duration), ordered by time.
    freezes: Vec<(usize, f64, f64)>,
    /// Per boundary k (between segment k and k+1): kind and overlap in frames.
    boundary: Vec<Option<(TransitionKind, u64)>>,
    fps: f64,
}

impl TransitionMap {
    /// Transitions whose `after_segment_id` is not followed by another segment (or is unknown)
    /// are ignored; `cut` and zero-frame transitions are hard cuts.
    pub fn new(clip: &Clip, transitions: &[Transition]) -> Self {
        Self::with_retime(clip, transitions, &HashMap::new(), &[])
    }

    /// Like [`TransitionMap::new`] plus segment speeds and freeze frames. Speeds outside
    /// 0.5..2.0 (or not finite) count as 1; freezes outside the clip or with a duration outside
    /// 0..10 s are ignored (`render.compile` rejects them).
    pub fn with_retime(
        clip: &Clip,
        transitions: &[Transition],
        speeds: &HashMap<SegmentId, f64>,
        freezes: &[Freeze],
    ) -> Self {
        let fps = clip.canvas.fps;
        let n = clip.segments.len();
        let mut edit_bounds = vec![0.0];
        let mut cum = 0.0;
        for s in &clip.segments {
            cum += s.end - s.start;
            edit_bounds.push(cum);
        }
        let speeds: Vec<f64> = clip
            .segments
            .iter()
            .map(|s| speeds.get(&s.id).copied().filter(|f| f.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(f)).unwrap_or(1.0))
            .collect();
        let mut fz: Vec<(usize, f64, f64)> = Vec::new();
        if n > 0 {
            for f in freezes {
                if !(f.out_t.is_finite() && f.duration.is_finite() && f.duration > 0.0 && f.duration <= MAX_FREEZE) {
                    continue;
                }
                if f.out_t < -1e-9 || f.out_t > cum + 1e-6 {
                    continue;
                }
                let t = f.out_t.clamp(0.0, cum);
                // The segment that starts at or holds `t` (a boundary belongs to the next one).
                let k = edit_bounds[1..n].partition_point(|b| *b <= t);
                fz.push((k, (t - edit_bounds[k]).clamp(0.0, edit_bounds[k + 1] - edit_bounds[k]), f.duration));
            }
        }
        fz.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal));
        let mut bounds = vec![0.0];
        let mut r = 0.0;
        for k in 0..n {
            r += (edit_bounds[k + 1] - edit_bounds[k]) / speeds[k];
            r += fz.iter().filter(|f| f.0 == k).map(|f| f.2).sum::<f64>();
            bounds.push(r);
        }
        let boundary = (0..n.saturating_sub(1))
            .map(|k| {
                let id = &clip.segments[k].id;
                transitions.iter().rev().find(|t| &t.after_segment_id == id).and_then(|t| {
                    t.kind.xfade_name()?;
                    if !(t.duration.is_finite() && t.duration > 0.0 && fps.is_finite() && fps > 0.0) {
                        return None;
                    }
                    let frames = (t.duration * fps).round().max(1.0) as u64;
                    Some((t.kind, frames))
                })
            })
            .collect();
        Self { bounds, edit_bounds, speeds, freezes: fz, boundary, fps }
    }

    /// Kind and overlap (frames) at boundary `k` (after segment k).
    pub fn at_boundary(&self, k: usize) -> Option<(TransitionKind, u64)> {
        self.boundary.get(k).copied().flatten()
    }

    pub fn is_identity(&self) -> bool {
        self.boundary.iter().all(Option::is_none) && !self.is_retimed()
    }

    /// True when a segment speed or a freeze changes the timing.
    pub fn is_retimed(&self) -> bool {
        !self.freezes.is_empty() || self.speeds.iter().any(|s| (s - 1.0).abs() > 1e-9)
    }

    fn overlap(&self, k: usize) -> f64 {
        self.at_boundary(k).map_or(0.0, |(_, f)| f as f64 / self.fps)
    }

    /// Seconds removed by all transitions.
    pub fn total_overlap(&self) -> f64 {
        (0..self.boundary.len()).map(|k| self.overlap(k)).sum()
    }

    /// Edit-timeline duration.
    pub fn edit_duration(&self) -> f64 {
        *self.edit_bounds.last().unwrap_or(&0.0)
    }

    /// Retimed duration (speeds and freezes applied, before transitions).
    pub fn retimed_duration(&self) -> f64 {
        *self.bounds.last().unwrap_or(&0.0)
    }

    /// Retimed segment boundaries (`n + 1` values).
    pub fn retimed_bounds(&self) -> &[f64] {
        &self.bounds
    }

    /// Speed of segment `k` (1 = normal).
    pub fn speed(&self, k: usize) -> f64 {
        self.speeds.get(k).copied().unwrap_or(1.0)
    }

    /// Freezes of segment `k`: (edit offset from the segment start, duration), in time order.
    pub fn segment_freezes(&self, k: usize) -> Vec<(f64, f64)> {
        self.freezes.iter().filter(|f| f.0 == k).map(|f| (f.1, f.2)).collect()
    }

    /// Rendered duration (retimed duration minus the transition overlaps).
    pub fn duration(&self) -> f64 {
        (self.retimed_duration() - self.total_overlap()).max(0.0)
    }

    /// Retimed time of edit time `t`; a time on a freeze maps to the start of the hold.
    pub fn retime(&self, t: f64) -> f64 {
        if !self.is_retimed() || !t.is_finite() || t <= 0.0 {
            return t;
        }
        let end = self.edit_duration();
        if t >= end {
            return self.retimed_duration() + (t - end);
        }
        let n = self.edit_bounds.len() - 1;
        let k = self.edit_bounds[1..n].partition_point(|b| *b <= t);
        let off = t - self.edit_bounds[k];
        let held: f64 = self.freezes.iter().filter(|f| f.0 == k && f.1 < off - 1e-9).map(|f| f.2).sum();
        self.bounds[k] + off / self.speeds[k] + held
    }

    /// Rendered time of edit-timeline time `t` (continuous and non-decreasing except at
    /// freezes, where it jumps forward by the hold).
    pub fn map_time(&self, t: f64) -> f64 {
        let n = self.bounds.len().saturating_sub(1);
        if n == 0 || !t.is_finite() || t <= 0.0 || self.is_identity() {
            return t;
        }
        let r = self.retime(t);
        if self.boundary.iter().all(Option::is_none) {
            return r;
        }
        if r >= self.retimed_duration() {
            return r - self.total_overlap();
        }
        let k = self.bounds[1..n].partition_point(|b| *b <= r);
        let shift: f64 = (0..k).map(|j| self.overlap(j)).sum();
        let lo = self.bounds[k] - shift + if k > 0 { self.overlap(k - 1) / 2.0 } else { 0.0 };
        let hi = self.bounds[k + 1] - shift - if k + 1 < n { self.overlap(k) / 2.0 } else { 0.0 };
        (r - shift).clamp(lo, hi.max(lo))
    }

    /// Rendered interval of `[s, e]`; `None` when it collapses (it lay inside the hidden half
    /// of a transition).
    pub fn map_interval(&self, s: f64, e: f64) -> Option<(f64, f64)> {
        let (a, b) = (self.map_time(s), self.map_time(e));
        (b - a > 0.001).then_some((a, b))
    }
}

/// Rendered duration of `clip` with these extras (edit duration with segment speeds and freeze
/// frames applied, minus transition overlaps). QC and `render.run` compare the file against
/// this, not the bare TimeMap duration.
pub fn effective_duration(clip: &Clip, extras: &CompileExtras) -> f64 {
    TransitionMap::with_retime(clip, &extras.transitions, &extras.segment_speed, &extras.freezes).duration()
}

/// `perspective` corner expressions: source quad = centre zoom `z` shifted by (ox, oy), all
/// as expression strings in W/H.
fn quad(z: &str, ox: &str, oy: &str) -> String {
    let lo = format!("(0.5-0.5/({z}))");
    let hi = format!("(0.5+0.5/({z}))");
    let x = |e: &str| format!("'W*({e}+{ox})'");
    let y = |e: &str| format!("'H*({e}+{oy})'");
    format!(
        "x0={}:y0={}:x1={}:y1={}:x2={}:y2={}:x3={}:y3={}",
        x(&lo),
        y(&lo),
        x(&hi),
        y(&lo),
        x(&lo),
        y(&hi),
        x(&hi),
        y(&hi)
    )
}

/// The filter for one effect over rendered frames `[f0, f1)`; `in - 1` is the output frame index
/// inside `perspective` (it counts every frame, also those the timeline disables).
fn effect_filter(e: &Effect, f0: u64, f1: u64, fps: f64) -> String {
    let a = e.amount_or_default();
    let last = f1.saturating_sub(1).max(f0);
    let enable = format!("enable='between(n,{f0},{last})'");
    match e.kind {
        EffectKind::PunchIn => {
            let (ox, oy) = e.focus_offset(a);
            format!("perspective={}:interpolation=linear:{enable}", quad(&num6(a), &off_expr(ox), &off_expr(oy)))
        }
        EffectKind::ZoomSlow => {
            let span = (f1 - f0).max(1);
            let p = format!("clip((in-1-{f0})/{span},0,1)");
            let z = format!("1+{}*{p}", num6(a - 1.0));
            // The centre drifts towards the focus as the zoom grows (always inside the frame).
            let (ox, oy) = e.focus_offset(a);
            let off = |o: f64| if o == 0.0 { "0".to_string() } else { format!("{}*{p}", off_expr(o)) };
            format!("perspective={}:interpolation=linear:eval=frame:{enable}", quad(&z, &off(ox), &off(oy)))
        }
        EffectKind::Shake => {
            // Zoom in just enough that the offsets never show the frame edge.
            let z = 1.0 + 2.0 * a;
            let amp = 0.9 * (1.0 - 1.0 / z) / 2.0;
            let wx = 2.0 * std::f64::consts::PI * 11.0 / fps;
            let wy = 2.0 * std::f64::consts::PI * 7.0 / fps;
            let ox = format!("{}*sin({}*(in-1))", num6(amp), num6(wx));
            let oy = format!("{}*sin({}*(in-1)+1.3)", num6(amp), num6(wy));
            format!("perspective={}:interpolation=linear:eval=frame:{enable}", quad(&num6(z), &ox, &oy))
        }
        EffectKind::Flash => {
            let d = (f1 - f0).max(1);
            format!(
                "fade=t=in:st={}:d={}:color=white:{enable}",
                num6(f0 as f64 / fps),
                num6(d as f64 / fps)
            )
        }
    }
}

/// A quad offset as an expression term (`0`, or a parenthesised signed number).
fn off_expr(o: f64) -> String {
    if o == 0.0 { "0".to_string() } else { format!("({})", num6(o)) }
}

/// Six-decimal number with trailing zeros trimmed (filter arguments that must be frame-exact).
pub(crate) fn num6(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

/// Per-segment frame and sample counts on the cumulative grid of the retimed timeline
/// (segment speeds and freezes applied; the plain edit timeline when there are none).
fn segment_lengths(tmap: &TransitionMap) -> (Vec<u64>, Vec<u64>) {
    let fps = tmap.fps;
    let b = tmap.retimed_bounds();
    let (mut fr, mut sa) = (Vec::new(), Vec::new());
    for k in 0..b.len().saturating_sub(1) {
        fr.push((frame_at(b[k + 1], fps) - frame_at(b[k], fps)).max(1));
        sa.push(sample_at(b[k + 1]) - sample_at(b[k]));
    }
    (fr, sa)
}

/// Joins `[v{i}]`/`[a{i}]` into `[vcat]`/`[acat]` (audio only when `video` is false): segments
/// between transitions are concatenated, the groups are chained with `xfade` + `acrossfade`
/// at frame-exact offsets.
fn join_segments(
    tmap: &TransitionMap,
    frames: &[u64],
    fps: f64,
    video: bool,
) -> Vec<String> {
    let n = frames.len();
    let mut parts = Vec::new();
    if tmap.is_identity() {
        if video {
            let cat_in: String = (0..n).map(|i| format!("[v{i}][a{i}]")).collect();
            parts.push(format!("{cat_in}concat=n={n}:v=1:a=1[vcat][acat]"));
        } else {
            let cat: String = (0..n).map(|i| format!("[a{i}]")).collect();
            parts.push(format!("{cat}concat=n={n}:v=0:a=1[acat]"));
        }
        return parts;
    }
    let tb = tb_str(fps);
    // Groups of segments separated by transitions.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for k in 0..n {
        if k + 1 == n || tmap.at_boundary(k).is_some() {
            groups.push((start, k + 1));
            start = k + 1;
        }
    }
    let mut gframes = Vec::new();
    for (g, &(a, b)) in groups.iter().enumerate() {
        let (vin, ain) = if b - a == 1 {
            (format!("[v{a}]"), format!("[a{a}]"))
        } else if video {
            let cat_in: String = (a..b).map(|i| format!("[v{i}][a{i}]")).collect();
            parts.push(format!("{cat_in}concat=n={}:v=1:a=1[g{g}vc][g{g}ac]", b - a));
            (format!("[g{g}vc]"), format!("[g{g}ac]"))
        } else {
            let cat: String = (a..b).map(|i| format!("[a{i}]")).collect();
            parts.push(format!("{cat}concat=n={}:v=0:a=1[g{g}ac]", b - a));
            (String::new(), format!("[g{g}ac]"))
        };
        if video {
            parts.push(format!("{vin}settb={tb},setpts=N[g{g}v]"));
        }
        parts.push(format!("{ain}asetpts=N/SR/TB[g{g}a]"));
        gframes.push(frames[a..b].iter().sum::<u64>());
    }
    let mut acc = gframes[0];
    let (mut accv, mut acca) = ("[g0v]".to_string(), "[g0a]".to_string());
    for g in 1..groups.len() {
        let (kind, df) = tmap.at_boundary(groups[g].0 - 1).expect("group split at a transition");
        let off = acc.saturating_sub(df);
        let last = g + 1 == groups.len();
        let (vout, aout) =
            if last { ("[vcat]".to_string(), "[acat]".to_string()) } else { (format!("[x{g}v]"), format!("[x{g}a]")) };
        if video {
            parts.push(format!(
                "{accv}[g{g}v]xfade=transition={}:duration={}:offset={}{vout}",
                kind.xfade_name().unwrap_or("fade"),
                num6(df as f64 / fps),
                num6(off as f64 / fps)
            ));
        }
        let ns = sample_at(df as f64 / fps).max(1);
        parts.push(format!("{acca}[g{g}a]acrossfade=ns={ns}:c1=tri:c2=tri{aout}"));
        acc = off + gframes[g];
        accv = vout;
        acca = aout;
    }
    parts
}

pub fn compile(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    ass_file: Option<&Path>,
    fonts_dir: &Path,
    enc: &EncoderChoice,
    out: &Path,
    workdir: &Path,
) -> rustler_core::Result<CompiledRender> {
    compile_with(
        clip,
        sources,
        ass_file,
        fonts_dir,
        enc,
        out,
        workdir,
        &CompileExtras::default(),
    )
}

/// Output duration of `clip` (sum of segment durations).
pub fn output_duration(clip: &Clip) -> f64 {
    clip.segments.iter().map(|s| s.end - s.start).sum()
}

#[allow(clippy::too_many_arguments)]
pub fn compile_with(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    ass_file: Option<&Path>,
    fonts_dir: &Path,
    enc: &EncoderChoice,
    out: &Path,
    workdir: &Path,
    extras: &CompileExtras,
) -> rustler_core::Result<CompiledRender> {
    let plan = build_graph(clip, sources, ass_file.is_some(), extras)?;

    std::fs::create_dir_all(workdir).map_err(|e| io_err("create workdir", workdir, e))?;
    write_file(&workdir.join(GRAPH_FILE), &plan.graph)?;
    write_file(
        &workdir.join(PREVIEW_GRAPH_FILE),
        &format!("{};\n[aout]anullsink", plan.graph),
    )?;
    for (name, body) in &plan.cmd_files {
        write_file(&workdir.join(name), body)?;
    }
    if let Some(ass) = ass_file {
        stage_ass(ass, workdir)?;
    }
    if let Some(ov) = &plan.overlays_ass {
        write_file(&workdir.join(OVERLAYS_ASS_FILE), ov)?;
    }
    if ass_file.is_some() || plan.overlays_ass.is_some() {
        stage_fonts(fonts_dir, workdir)?;
    }

    let head = ["-hide_banner", "-nostdin", "-y", "-loglevel", "error"];
    let mut inputs: Vec<OsString> = Vec::new();
    for seg in &clip.segments {
        let src = &sources[&seg.source_id];
        let end = seg.end.min(src.probe.duration.max(seg.start + 0.001));
        inputs.push("-ss".into());
        inputs.push(secs(seg.start).into());
        inputs.push("-to".into());
        inputs.push(secs(end).into());
        inputs.push("-i".into());
        inputs.push(abs(&src.path).into_os_string());
    }
    let seg_inputs = inputs.clone();
    if let Some(m) = &extras.music {
        inputs.extend(["-stream_loop", "-1", "-i"].iter().map(OsString::from));
        inputs.push(abs(&m.path).into_os_string());
    }
    for p in &plan.sfx_inputs {
        inputs.push("-i".into());
        inputs.push(abs(p).into_os_string());
    }
    for li in &plan.layer_inputs {
        inputs.extend(li.opts.iter().map(OsString::from));
        inputs.push("-i".into());
        inputs.push(abs(&li.path).into_os_string());
    }

    let loudness = match (&plan.loudness_graph, &plan.loudnorm_filter) {
        (Some(g), Some(filter)) => {
            write_file(&workdir.join(LOUDNESS_GRAPH_FILE), g)?;
            // loudnorm prints its JSON at info level; -nostats keeps the log small.
            let mut a: Vec<OsString> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"]
                .iter()
                .map(OsString::from)
                .collect();
            a.extend(inputs.iter().cloned());
            a.extend(
                ["-/filter_complex", LOUDNESS_GRAPH_FILE, "-map", "[aout]", "-f", "null", "-"]
                    .iter()
                    .map(OsString::from),
            );
            Some(LoudnessPass {
                args: a,
                filter: filter.clone(),
            })
        }
        _ => {
            let _ = std::fs::remove_file(workdir.join(LOUDNESS_GRAPH_FILE));
            None
        }
    };

    let noise = match (&plan.noise_graph, &plan.denoise_filter) {
        (Some(g), Some(filter)) => {
            write_file(&workdir.join(NOISE_GRAPH_FILE), g)?;
            // ametadata prints at info level.
            let mut a: Vec<OsString> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"]
                .iter()
                .map(OsString::from)
                .collect();
            a.extend(seg_inputs);
            a.extend(
                ["-/filter_complex", NOISE_GRAPH_FILE, "-map", "[aout]", "-f", "null", "-"]
                    .iter()
                    .map(OsString::from),
            );
            Some(NoisePass { args: a, filter: filter.clone(), denoise: extras.denoise })
        }
        _ => {
            let _ = std::fs::remove_file(workdir.join(NOISE_GRAPH_FILE));
            None
        }
    };

    let mut args: Vec<OsString> = head.iter().map(OsString::from).collect();
    args.extend(inputs);
    args.extend(
        [
            "-/filter_complex",
            GRAPH_FILE,
            "-map",
            "[vout]",
            "-map",
            "[aout]",
        ]
        .iter()
        .map(OsString::from),
    );
    args.extend(enc.args.iter().map(OsString::from));
    args.extend(
        ["-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-r"]
            .iter()
            .map(OsString::from),
    );
    args.push(fps_str(clip.canvas.fps).into());
    args.extend(["-movflags", "+faststart"].iter().map(OsString::from));
    args.push(abs(out).into_os_string());

    Ok(CompiledRender {
        args,
        filter_script: plan.graph,
        workdir: workdir.to_path_buf(),
        loudness,
        noise,
    })
}

/// The pure part of the compiler: the filter script plus the sendcmd files it references.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphPlan {
    pub graph: String,
    /// (bare file name in workdir, contents)
    pub cmd_files: Vec<(String, String)>,
    /// Contents of `overlays.ass` when the clip has overlays inside its duration.
    pub overlays_ass: Option<String>,
    /// Audio-only measurement graph (`graph_loudness.txt`) for two-pass loudnorm.
    pub loudness_graph: Option<String>,
    /// The single-pass loudnorm filter text inside `graph` (when loudnorm is on).
    pub loudnorm_filter: Option<String>,
    /// Sound-effect files, in input order after the segments (and the music bed).
    pub sfx_inputs: Vec<PathBuf>,
    /// Rendered duration (edit duration minus transition overlaps), seconds.
    pub duration: f64,
    /// Audio-only graph that measures the dialogue noise floor (`graph_noise.txt`; denoise on).
    pub noise_graph: Option<String>,
    /// The denoise filter text inside `graph` (default floor), replaced after the noise pass.
    pub denoise_filter: Option<String>,
    /// Inputs of the image / cutaway layers, in input order after the sound effects.
    pub layer_inputs: Vec<LayerInput>,
}

pub fn build_graph(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    captions: bool,
    extras: &CompileExtras,
) -> rustler_core::Result<GraphPlan> {
    validate(clip, sources, extras)?;
    let canvas = clip.canvas;
    let cw = even(canvas.width as f64);
    let ch = even(canvas.height as f64);
    let fps = fps_str(canvas.fps);
    let mut parts: Vec<String> = Vec::new();
    let mut cmd_files = Vec::new();
    let mut seg_audio: Vec<String> = Vec::new();
    let tmap = TransitionMap::with_retime(clip, &extras.transitions, &extras.segment_speed, &extras.freezes);
    let (seg_frames, seg_samples) = segment_lengths(&tmap);

    for (i, seg) in clip.segments.iter().enumerate() {
        let src = &sources[&seg.source_id];
        let (frames, samples) = (seg_frames[i], seg_samples[i]);
        let speed = tmap.speed(i);
        // Freeze frames of this segment: (frame index in the moving picture, held frames,
        // sample offset, held samples). The picture holds via `loop`, the audio gets silence.
        let holds = segment_holds(&tmap.segment_freezes(i), speed, canvas.fps, frames);
        let held_frames: u64 = holds.iter().map(|h| h.1).sum();
        let moving_frames = frames.saturating_sub(held_frames).max(1);

        // Crop keys are segment-relative source seconds; a sped-up segment reaches them sooner.
        let crop_keys: Option<Vec<CropKey>> = extras
            .crop_paths
            .get(&seg.id)
            .filter(|k| !k.is_empty() && !seg.layout.regions.is_empty())
            .map(|k| k.iter().map(|c| CropKey { t: c.t / speed, x: c.x }).collect());
        let mut tail = String::new();
        // Later holds first, so the earlier frame indices stay valid.
        for (f, n, _, _) in holds.iter().rev() {
            let _ = write!(tail, ",loop=loop={n}:size=1:start={f}");
        }
        if !holds.is_empty() {
            tail.push_str(",setpts=N/FRAME_RATE/TB");
        }
        segment_video(
            &mut parts,
            &mut cmd_files,
            i,
            seg.layout.regions.as_slice(),
            seg.layout.background,
            &src.probe,
            &canvas,
            (cw, ch),
            &fps,
            moving_frames,
            crop_keys.as_deref(),
            speed,
            &tail,
        );

        let gain = extras
            .segment_gain_db
            .get(&seg.id)
            .filter(|g| g.is_finite() && **g != 0.0)
            .map(|g| format!(",volume={}dB", num(*g)))
            .unwrap_or_default();
        let tempo = if (speed - 1.0).abs() > 1e-9 { format!(",atempo={}", num6(speed)) } else { String::new() };
        let achain = if src.probe.has_audio && !holds.is_empty() {
            let held: u64 = holds.iter().map(|h| h.3).sum();
            let moving = samples.saturating_sub(held);
            audio_with_holds(
                i,
                &format!(
                    "[{i}:a:0]aresample=48000:async=1:first_pts=0{tempo},{AUDIO_FORMAT}{gain},apad,\
                     asetpts=N/SR/TB,atrim=end_sample={moving},asetpts=N/SR/TB"
                ),
                moving,
                &holds,
            )
        } else if src.probe.has_audio {
            format!(
                "[{i}:a:0]aresample=48000:async=1:first_pts=0{tempo},{AUDIO_FORMAT}{gain},apad,\
                 asetpts=N/SR/TB,atrim=end_sample={samples},asetpts=N/SR/TB[a{i}]"
            )
        } else {
            format!(
                "anullsrc=r=48000:cl=stereo,{AUDIO_FORMAT},atrim=end_sample={samples},\
                 asetpts=N/SR/TB[a{i}]"
            )
        };
        seg_audio.push(achain.clone());
        parts.push(achain);
    }

    let n = clip.segments.len();
    parts.extend(join_segments(&tmap, &seg_frames, canvas.fps, true));

    // Rendered length: the overlaps of the transitions are gone.
    let overlap_frames: u64 = (0..n.saturating_sub(1)).filter_map(|k| tmap.at_boundary(k)).map(|(_, f)| f).sum();
    let overlap_samples: u64 = (0..n.saturating_sub(1))
        .filter_map(|k| tmap.at_boundary(k))
        .map(|(_, f)| sample_at(f as f64 / canvas.fps).max(1))
        .sum();
    let total_frames = seg_frames.iter().sum::<u64>().saturating_sub(overlap_frames).max(1);
    let total_samples = seg_samples.iter().sum::<u64>().saturating_sub(overlap_samples);
    let duration = total_samples as f64 / SR as f64;

    // Overlays live on the edit timeline: map them through the transitions.
    let mapped_overlays: Vec<TextOverlay> = extras
        .overlays
        .iter()
        .filter_map(|o| {
            let (s, e) = tmap.map_interval(o.start, o.end)?;
            Some(TextOverlay { start: s, end: e, ..o.clone() })
        })
        .collect();
    let overlays_ass = overlays_ass(&mapped_overlays, &canvas, duration);

    // Sound effects: input index after the segments and the music bed.
    let base = n + usize::from(extras.music.is_some());
    let mut sfx_inputs = Vec::new();
    let mut sfx_chains = Vec::new();
    for cue in &extras.sfx {
        let at = tmap.map_time(cue.at);
        if !(at.is_finite() && at >= 0.0 && at < duration - 0.01) {
            continue;
        }
        let idx = base + sfx_inputs.len();
        sfx_chains.push(format!(
            "[{idx}:a:0]aresample=48000,{AUDIO_FORMAT},volume={}dB,adelay=delays={}S:all=1,apad,\
             atrim=end_sample={total_samples},asetpts=N/SR/TB",
            num(cue.volume_db),
            sample_at(at)
        ));
        sfx_inputs.push(cue.path.clone());
    }

    // Layers read extra inputs after the sound effects.
    let mut lx = LayerCtx {
        tmap: &tmap,
        fps: canvas.fps,
        cw,
        ch,
        total_frames,
        total_samples,
        base_input: base + sfx_inputs.len(),
        inputs: Vec::new(),
        audio_chains: Vec::new(),
    };

    // Concat runs on a microsecond time base; rebuild exact frame timestamps.
    // Z-order: video -> full cutaways -> masks -> effects -> PiP cutaways -> images (below the
    // captions, by z) -> captions -> text overlays -> images above the captions -> progress bar.
    let mut vc = VChain::new(format!("[vcat]settb={},setpts=N", tb_str(canvas.fps)));
    lx.full_cutaways(&mut parts, &mut vc, &extras.cutaways);
    lx.masks(&mut parts, &mut vc, &extras.masks);
    for e in &extras.effects {
        let Some((s, t)) = tmap.map_interval(e.start, e.end) else { continue };
        let (f0, f1) = (frame_at(s, canvas.fps), frame_at(t, canvas.fps).min(total_frames));
        if f1 > f0 {
            vc.push(&effect_filter(e, f0, f1, canvas.fps));
        }
    }
    lx.pip_cutaways(&mut parts, &mut vc, &extras.cutaways);
    lx.images(&mut parts, &mut vc, &extras.images, false);
    if captions {
        vc.push(&format!("ass={ASS_FILE}:fontsdir={FONTS_DIR}"));
    }
    if overlays_ass.is_some() {
        vc.push(&format!("ass={OVERLAYS_ASS_FILE}:fontsdir={FONTS_DIR}"));
    }
    lx.images(&mut parts, &mut vc, &extras.images, true);
    let LayerCtx { inputs: layer_inputs, audio_chains: layer_audio, .. } = lx;
    sfx_chains.extend(layer_audio);
    let mut vpost = vc.into_open();
    match &extras.progress_bar {
        None => {
            vpost.push_str(",format=yuv420p[vout]");
            parts.push(vpost);
        }
        Some(pb) => {
            // A canvas-wide bar slides in from the left: at output frame n it shows
            // (n+1)/N of the width, so the last frame is full.
            let h = (pb.height.max(2) + pb.height % 2).min(ch);
            let y = match pb.position {
                BarPosition::Top => 0,
                BarPosition::Bottom => ch - h,
            };
            let color = ffmpeg_color(&pb.color).unwrap_or_else(|| "0xFFE000".into());
            vpost.push_str("[vbar]");
            parts.push(vpost);
            parts.push(format!(
                "color=c={color}:s={cw}x{h}:r={fps},trim=end_frame={total_frames},format=yuv420p[pbar]"
            ));
            parts.push(format!(
                "[vbar][pbar]overlay=x='W*(n+1)/{total_frames}-W':y={y}:eval=frame,format=yuv420p[vout]"
            ));
        }
    }

    let loudnorm_filter = clip.audio.loudnorm.then(|| {
        format!(
            "loudnorm=I={}:TP={}:LRA={}",
            num(clip.audio.target_lufs),
            num(clip.audio.true_peak),
            num(clip.audio.lra)
        )
    });
    parts.extend(audio_tail(extras, n, &sfx_chains, total_samples, loudnorm_filter.as_deref()));

    // Denoise: measure the noise floor of the joined dialogue first (50 ms RMS windows).
    let denoise_filter = extras.denoise.filter();
    let noise_graph = denoise_filter.as_ref().map(|_| {
        let mut g = seg_audio.clone();
        g.extend(join_segments(&tmap, &seg_frames, canvas.fps, false));
        g.push(
            "[acat]highpass=f=80,asetnsamples=n=2400:p=0,astats=metadata=1:reset=1:measure_perchannel=none:\
             measure_overall=RMS_level,ametadata=mode=print:key=lavfi.astats.Overall.RMS_level[aout]"
                .to_string(),
        );
        g.join(";\n")
    });

    // Two-pass loudnorm: the same audio graph, audio-only, with loudnorm printing JSON.
    let loudness_graph = match (&loudnorm_filter, extras.two_pass_loudness) {
        (Some(f), true) => {
            let mut g = seg_audio;
            g.extend(join_segments(&tmap, &seg_frames, canvas.fps, false));
            let measure = format!("{f}:print_format=json");
            g.extend(audio_tail(extras, n, &sfx_chains, total_samples, Some(&measure)));
            Some(g.join(";\n"))
        }
        _ => None,
    };

    Ok(GraphPlan {
        graph: parts.join(";\n"),
        cmd_files,
        overlays_ass,
        loudness_graph,
        loudnorm_filter,
        sfx_inputs,
        duration,
        noise_graph,
        denoise_filter,
        layer_inputs,
    })
}

/// `[acat]` (+ denoise/gain, sound effects, music bed) -> loudnorm -> `[aout]`.
/// `sfx` are the effect chains without their output label.
fn audio_tail(
    extras: &CompileExtras,
    k: usize,
    sfx: &[String],
    total_samples: u64,
    loudnorm: Option<&str>,
) -> Vec<String> {
    let mut parts = Vec::new();
    let mut apost = String::new();
    if let Some(ln) = loudnorm {
        let _ = write!(apost, "{ln},aresample=48000,{FINITE_AUDIO},");
    }
    // loudnorm's buffered flush can leave a PTS jump; rebuild the sample clock after it.
    let _ = write!(apost, "{AUDIO_FORMAT},asetpts=N/SR/TB[aout]");

    // Dialogue clean-up before anything is mixed in.
    let mut pre: Vec<String> = Vec::new();
    if let Some(f) = extras.denoise.filter() {
        pre.push(f);
    }
    if extras.speech_gain_db.is_finite() && extras.speech_gain_db != 0.0 {
        pre.push(format!("volume={}dB", num(extras.speech_gain_db)));
    }
    let speech = if pre.is_empty() {
        "[acat]".to_string()
    } else {
        parts.push(format!("[acat]{}[speech0]", pre.join(",")));
        "[speech0]".to_string()
    };
    let mut fx_labels = String::new();
    for (j, chain) in sfx.iter().enumerate() {
        parts.push(format!("{chain}[sfx{j}]"));
        let _ = write!(fx_labels, "[sfx{j}]");
    }
    let mix = |inputs: usize| format!("amix=inputs={inputs}:duration=first:dropout_transition=0:normalize=0");

    match &extras.music {
        None if sfx.is_empty() => parts.push(format!("{speech}{apost}")),
        None => parts.push(format!("{speech}{fx_labels}{},{apost}", mix(1 + sfx.len()))),
        Some(m) => {
            let total = total_samples as f64 / SR as f64;
            let fi = m.fade_in.clamp(0.0, total / 2.0);
            let fo = m.fade_out.clamp(0.0, total / 2.0);
            let mut mus = format!(
                "[{k}:a:0]aresample=48000,{AUDIO_FORMAT},volume={}dB,\
                 atrim=end_sample={total_samples},asetpts=N/SR/TB",
                num(m.volume_db)
            );
            if fi > 0.0 {
                let _ = write!(mus, ",afade=t=in:st=0:d={}", num(fi));
            }
            if fo > 0.0 {
                let _ = write!(mus, ",afade=t=out:st={}:d={}", num(total - fo), num(fo));
            }
            mus.push_str("[mus]");
            parts.push(mus);
            let mix = mix(2 + sfx.len());
            if m.duck {
                // The sidechain keys on speech only, never on the sound effects.
                parts.push(format!("{speech}asplit=2[speech][sc]"));
                parts.push(
                    "[mus][sc]sidechaincompress=threshold=0.08:ratio=5:attack=100:release=1000[duck]"
                        .to_string(),
                );
                parts.push(format!("[speech][duck]{fx_labels}{mix},{apost}"));
            } else {
                parts.push(format!("{speech}[mus]{fx_labels}{mix},{apost}"));
            }
        }
    }
    parts
}

// ---------------------------------------------------------------------------------------------
// Text overlays (hook titles)
// ---------------------------------------------------------------------------------------------

/// Text overlays never get wider than this fraction of the canvas (box included).
const OVERLAY_MAX_WIDTH: f64 = 0.86;
/// Overlays shrink down to this fraction of their font size before giving up on 2 lines.
const OVERLAY_MIN_SCALE: f64 = 0.6;
pub const OVERLAY_MAX_LINES: usize = 2;
pub const OVERLAY_MAX_CHARS: usize = 120;

/// Where an overlay lands on the canvas (pixels) after wrapping.
#[derive(Serialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct OverlayLayout {
    pub lines: Vec<String>,
    pub font_size: u32,
    /// Box padding (pixels around each line; 0 without a box).
    pub padding: u32,
    pub center_y: f64,
    pub top: f64,
    pub bottom: f64,
    /// Widest line (estimated, pixels, box included).
    pub width: f64,
    /// False when the text needed more than 2 lines even at the smallest size; shorten it.
    pub fits: bool,
}

/// Overlay text with ASS override characters removed and whitespace collapsed.
pub fn clean_overlay_text(text: &str) -> String {
    let s: String = text
        .chars()
        .filter(|c| !matches!(c, '{' | '}' | '\\') && !c.is_control())
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text as shown: cleaned, then shaped by the style (hook-big is uppercase, hook-pov starts
/// with "POV:", hook-question ends with "?").
pub fn display_text(ov: &TextOverlay) -> String {
    let t = clean_overlay_text(&ov.text);
    if t.is_empty() {
        return t;
    }
    match ov.style {
        OverlayStyle::HookBig => t.to_uppercase(),
        OverlayStyle::HookPov => {
            let first = t.split(' ').next().unwrap_or("");
            if first.to_uppercase().starts_with("POV") {
                format!("{}{}", &t[..first.len()].to_uppercase(), &t[first.len()..])
            } else {
                format!("POV: {t}")
            }
        }
        OverlayStyle::HookQuestion if !t.ends_with('?') => format!("{t}?"),
        _ => t,
    }
}

/// Advance width of `c` in units of the ASS font size (Poppins ExtraBold; ASS font size is the
/// line height, roughly 1.4 em for Poppins). An approximation on the wide side.
fn char_width(c: char) -> f64 {
    match c {
        ' ' => 0.20,
        'i' | 'l' | 'j' | 'I' | '|' | '.' | ',' | ':' | ';' | '!' | '\'' => 0.22,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '"' | '-' => 0.30,
        'm' | 'w' => 0.66,
        'M' | 'W' => 0.74,
        'a'..='z' => 0.46,
        'A'..='Z' => 0.55,
        '0'..='9' => 0.48,
        '?' | '&' | '#' | '%' | '@' => 0.55,
        c if c.is_ascii() => 0.40,
        _ => 0.62, // emoji, Devanagari, ...: wide
    }
}

/// libass draws Poppins ~0.77x as wide as the table above (measured on a rendered frame:
/// "build this in 30 days" at size 67 = 390 px); 0.8 keeps a small safety margin.
const WIDTH_CALIBRATION: f64 = 0.8;

fn text_width(s: &str, size: f64) -> f64 {
    s.chars().map(char_width).sum::<f64>() * size * WIDTH_CALIBRATION
}

/// Wraps `words` into at most 2 balanced lines that fit `max_w` at `size`; `None` when they
/// do not fit.
fn wrap_two(words: &[&str], size: f64, max_w: f64) -> Option<Vec<String>> {
    let one = words.join(" ");
    if text_width(&one, size) <= max_w {
        return Some(vec![one]);
    }
    let mut best: Option<(f64, Vec<String>)> = None;
    for k in 1..words.len() {
        let (a, b) = (words[..k].join(" "), words[k..].join(" "));
        let w = text_width(&a, size).max(text_width(&b, size));
        if w <= max_w && best.as_ref().is_none_or(|(bw, _)| w < *bw) {
            best = Some((w, vec![a, b]));
        }
    }
    best.map(|(_, l)| l)
}

/// Wraps and sizes an overlay: max 2 lines; the font shrinks (down to 60%) until the text fits
/// 86% of the canvas width; the block is kept inside the canvas.
pub fn overlay_layout(ov: &TextOverlay, canvas: &Canvas) -> OverlayLayout {
    let (cw, ch) = (canvas.width as f64, canvas.height as f64);
    let text = display_text(ov);
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    let base = (cw * ov.font_size_factor).round().max(8.0);
    let pad_of = |size: f64| if ov.boxed { (size * 0.28).round() } else { 0.0 };
    let mut chosen: Option<(f64, Vec<String>)> = None;
    let mut scale = 1.0;
    while scale >= OVERLAY_MIN_SCALE - 1e-9 {
        let size = (base * scale).round();
        let max_w = cw * OVERLAY_MAX_WIDTH - 2.0 * pad_of(size);
        if let Some(lines) = wrap_two(&words, size, max_w) {
            chosen = Some((size, lines));
            break;
        }
        scale -= 0.05;
    }
    let fits = chosen.is_some();
    let (size, lines) = chosen.unwrap_or_else(|| {
        // Smallest size, greedy first line, the rest on line 2 (will overflow: warn the caller).
        let size = (base * OVERLAY_MIN_SCALE).round();
        let max_w = cw * OVERLAY_MAX_WIDTH - 2.0 * pad_of(size);
        let mut k = 1;
        while k < words.len() && text_width(&words[..k + 1].join(" "), size) <= max_w {
            k += 1;
        }
        let k = k.min(words.len());
        let mut l = vec![words[..k].join(" ")];
        if k < words.len() {
            l.push(words[k..].join(" "));
        }
        (size, l)
    });
    // hook-big is always two lines (three or more words): split a one-line text where it balances.
    let lines = if ov.style == OverlayStyle::HookBig && lines.len() == 1 && words.len() >= 3 {
        let best = (1..words.len())
            .map(|k| (words[..k].join(" "), words[k..].join(" ")))
            .min_by(|a, b| {
                let w = |p: &(String, String)| text_width(&p.0, size).max(text_width(&p.1, size));
                w(a).total_cmp(&w(b))
            })
            .expect("at least two split points");
        vec![best.0, best.1]
    } else {
        lines
    };
    let pad = pad_of(size);
    let height = lines.len() as f64 * size + 2.0 * pad;
    let half = height / 2.0;
    let center_y = (ov.position_y * ch).clamp(half.min(ch / 2.0), (ch - half).max(ch / 2.0));
    let width = lines
        .iter()
        .map(|l| text_width(l, size))
        .fold(0.0, f64::max)
        + 2.0 * pad;
    OverlayLayout {
        lines,
        font_size: size as u32,
        padding: pad as u32,
        center_y,
        top: center_y - half,
        bottom: center_y + half,
        width,
        fits,
    }
}

/// ASS time `H:MM:SS.cc`.
fn ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        cs / 360_000,
        cs / 6000 % 60,
        cs / 100 % 60,
        cs % 100
    )
}

/// `overlays.ass` for the overlays that are visible inside `[0, duration]` (ends clamped to
/// the duration); `None` when there are none. One style per overlay (its own size), centred
/// with `\an5\pos`, white text; `box` = BorderStyle 3 (opaque dark box padded around each line).
pub fn overlays_ass(overlays: &[TextOverlay], canvas: &Canvas, duration: f64) -> Option<String> {
    let visible: Vec<&TextOverlay> = overlays
        .iter()
        .filter(|o| {
            o.start.is_finite()
                && o.end.is_finite()
                && o.start < duration - 0.01
                && o.end > o.start
                && !clean_overlay_text(&o.text).is_empty()
        })
        .collect();
    if visible.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("[Script Info]\nScriptType: v4.00+\n");
    let _ = writeln!(out, "PlayResX: {}\nPlayResY: {}", canvas.width, canvas.height);
    // WrapStyle 2: no automatic wrapping; lines come from overlay_layout.
    out.push_str("WrapStyle: 2\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\n\n");
    out.push_str(
        "[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, \
         OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, \
         Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
    );
    let layouts: Vec<OverlayLayout> = visible.iter().map(|o| overlay_layout(o, canvas)).collect();
    for (i, (o, l)) in visible.iter().zip(&layouts).enumerate() {
        let look = o.style.look();
        let (border_style, fill, outline_colour, outline, shadow) = if o.boxed {
            // BorderStyle 3: the "outline" is an opaque box of OutlineColour, `Outline` px of padding.
            (3, look.text_on_box, look.box_colour, l.padding as f64, 0.0)
        } else {
            let thick = (l.font_size as f64 * look.outline_factor).round().max(2.0);
            (1, look.text_plain, "&H00000000", thick, if o.style == OverlayStyle::HookBig { 3.0 } else { 2.0 })
        };
        let _ = writeln!(
            out,
            "Style: ov{i},{OVERLAY_FONT},{},{fill},{fill},{outline_colour},&H80000000,0,0,0,0,\
             100,100,0,0,{border_style},{},{},5,0,0,0,1",
            l.font_size,
            num(outline),
            num(shadow)
        );
    }
    out.push_str("\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
    for (i, (o, l)) in visible.iter().zip(&layouts).enumerate() {
        let look = o.style.look();
        let start = o.start.max(0.0);
        let end = o.end.min(duration);
        // No fade-in at t=0: the first frame (thumbnail) shows the hook fully.
        let fade_in = if start > 0.05 { 150 } else { 0 };
        let fade_out = if end < duration - 0.05 { 150 } else { 0 };
        let anim = o.effective_anim();
        let (an, x) = if look.left {
            (4, (canvas.width as f64 * (1.0 - OVERLAY_MAX_WIDTH) / 2.0 + l.padding as f64).round())
        } else {
            (5, (canvas.width as f64 / 2.0).round())
        };
        let y = l.center_y.round();
        let (place, motion) = match anim {
            OverlayAnim::Slide => {
                let (x0, y0) = if look.left { (x - canvas.width as f64 * 0.6, y) } else { (x, y - 200.0) };
                (format!("\\move({},{},{},{},0,260)", num(x0.round()), num(y0.round()), num(x), num(y)), String::new())
            }
            OverlayAnim::Pop => (
                format!("\\pos({},{})", num(x), num(y)),
                "\\fscx70\\fscy70\\t(0,140,\\fscx112\\fscy112)\\t(140,260,\\fscx100\\fscy100)".to_string(),
            ),
            OverlayAnim::ZoomIn => {
                (format!("\\pos({},{})", num(x), num(y)), "\\fscx140\\fscy140\\t(0,300,\\fscx100\\fscy100)".to_string())
            }
            OverlayAnim::None | OverlayAnim::Typewriter => (format!("\\pos({},{})", num(x), num(y)), String::new()),
        };
        let fill = if o.boxed { look.text_on_box } else { look.text_plain };
        let body = overlay_body(o, &l.lines, fill, anim);
        let _ = writeln!(
            out,
            "Dialogue: {i},{},{},ov{i},,0,0,0,,{{\\an{an}{place}\\fad({fade_in},{fade_out}){motion}}}{body}",
            ass_time(start),
            ass_time(end),
        );
    }
    Some(out)
}

/// ASS text of an overlay's lines: accent runs ("POV:" yellow, the closing "?" big and red) and,
/// for the typewriter, a per-character reveal. Plain styles give the lines joined with `\N`.
fn overlay_body(o: &TextOverlay, lines: &[String], fill: &str, anim: OverlayAnim) -> String {
    let look = o.style.look();
    // (char, accent) per line.
    let mut glyphs: Vec<Vec<(char, bool)>> = lines.iter().map(|l| l.chars().map(|c| (c, false)).collect()).collect();
    match o.style {
        OverlayStyle::HookPov => {
            if let Some(first) = glyphs.first_mut() {
                let head: String = first.iter().take(3).map(|(c, _)| *c).collect();
                if head == "POV" {
                    for g in first.iter_mut().take_while(|(c, _)| *c != ' ') {
                        g.1 = true;
                    }
                }
            }
        }
        OverlayStyle::HookQuestion => {
            if let Some(last) = glyphs.last_mut() {
                for g in last.iter_mut().rev().take_while(|(c, _)| *c == '?') {
                    g.1 = true;
                }
            }
        }
        _ => {}
    }
    let has_accent = glyphs.iter().flatten().any(|(_, a)| *a);
    // A bigger "?" would stretch the box of the line; only unboxed text scales it.
    let accent_on = if o.style == OverlayStyle::HookQuestion && !o.boxed {
        format!("\\1c{}&\\fscx135\\fscy135", look.accent)
    } else {
        format!("\\1c{}&", look.accent)
    };
    let accent_off = format!("\\1c{fill}&\\fscx100\\fscy100");
    let typed = anim == OverlayAnim::Typewriter;
    let letters = glyphs.iter().flatten().filter(|(c, _)| *c != ' ').count().max(1);
    let per_ms = (1200.0 / letters as f64).min(45.0);
    let (hide, show) = if o.boxed {
        ("\\1a&HFF&", "\\1a&H00&")
    } else {
        ("\\1a&HFF&\\3a&HFF&\\4a&HFF&", "\\1a&H00&\\3a&H00&\\4a&H80&")
    };
    let mut out = String::new();
    let (mut n, mut cur) = (0usize, false);
    for (li, line) in glyphs.iter().enumerate() {
        if li > 0 {
            out.push_str("\\N");
        }
        for &(c, accent) in line {
            let mut tags = String::new();
            if has_accent && accent != cur {
                tags.push_str(if accent { &accent_on } else { &accent_off });
                cur = accent;
            }
            if typed && c != ' ' {
                let t0 = (n as f64 * per_ms).round() as i64;
                tags.push_str(&format!("{hide}\\t({t0},{},{show})", t0 + 1));
                n += 1;
            }
            if !tags.is_empty() {
                out.push_str(&format!("{{{tags}}}"));
            }
            out.push(c);
        }
    }
    out
}

struct Px {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Source rect (normalized) -> even pixel crop inside `sw`x`sh`.
fn src_px(r: &Rect, sw: u32, sh: u32) -> Px {
    let (fw, fh) = (even(sw as f64).max(2), even(sh as f64).max(2));
    let w = even(r.w * sw as f64).clamp(2, fw);
    let h = even(r.h * sh as f64).clamp(2, fh);
    let x = even(r.x * sw as f64).min(even((fw - w) as f64));
    let y = even(r.y * sh as f64).min(even((fh - h) as f64));
    Px { x, y, w, h }
}

fn dst_px(r: &Rect, cw: u32, ch: u32) -> Px {
    let w = even(r.w * cw as f64).clamp(2, cw);
    let h = even(r.h * ch as f64).clamp(2, ch);
    Px {
        x: even(r.x * cw as f64).min(cw - 2),
        y: even(r.y * ch as f64).min(ch - 2),
        w,
        h,
    }
}

#[allow(clippy::too_many_arguments)]
fn segment_video(
    parts: &mut Vec<String>,
    cmd_files: &mut Vec<(String, String)>,
    i: usize,
    regions: &[Region],
    background: Background,
    probe: &MediaProbe,
    canvas: &Canvas,
    (cw, ch): (u32, u32),
    fps: &str,
    frames: u64,
    crop_keys: Option<&[CropKey]>,
    speed: f64,
    tail: &str,
) {
    // Regions drawn back to front; the crop path follows regions[0] wherever it sorts.
    let mut order: Vec<usize> = (0..regions.len()).collect();
    order.sort_by_key(|&j| regions[j].z);
    let full = |r: &Region| {
        let d = dst_px(&r.dst, cw, ch);
        r.fit == Fit::Cover && d.x == 0 && d.y == 0 && d.w == cw && d.h == ch
    };
    // A full-canvas Cover region at the back hides any background: skip compositing it.
    let base_is_region = order.first().is_some_and(|&j| full(&regions[j]));
    let blur = background == Background::BlurFill && !base_is_region;
    let uses = order.len() + usize::from(blur);

    // A segment speed rescales the timestamps before the frame-rate conversion.
    let rate = if (speed - 1.0).abs() > 1e-9 { format!("setpts=PTS/{},", num6(speed)) } else { String::new() };
    let prep = format!(
        "[{i}:v:0]{rate}fps={fps}:start_time=0:round=near,tpad=stop_mode=clone:stop_duration=1,\
         trim=end_frame={frames},setpts=PTS-STARTPTS"
    );
    let inputs: Vec<String> = (0..uses.max(1)).map(|k| format!("[s{i}in{k}]")).collect();
    if uses <= 1 {
        parts.push(format!("{prep}{}", inputs[0]));
    } else {
        parts.push(format!("{prep},split={uses}{}", inputs.concat()));
    }
    let mut next_in = inputs.iter();

    let mut base: Option<String> = None;
    if !base_is_region {
        if blur {
            let (qw, qh) = (even(cw as f64 / 4.0).max(2), even(ch as f64 / 4.0).max(2));
            parts.push(format!(
                "{}scale={qw}:{qh}:force_original_aspect_ratio=increase,crop={qw}:{qh},\
                 gblur=sigma=10,lutyuv=y=val*0.6,scale={cw}:{ch},setsar=1[s{i}bg]",
                next_in.next().unwrap()
            ));
        } else {
            if order.is_empty() {
                // No regions at all: consume the (unused) input so the graph has no dangling pad.
                parts.push(format!("{}nullsink", next_in.next().unwrap()));
            }
            parts.push(format!(
                "color=c=black:s={cw}x{ch}:r={fps},trim=end_frame={frames},setsar=1[s{i}bg]"
            ));
        }
        base = Some(format!("[s{i}bg]"));
    }

    for (pos, &j) in order.iter().enumerate() {
        let r = &regions[j];
        let s = src_px(&r.src, probe.width, probe.height);
        let d = dst_px(&r.dst, cw, ch);
        let label = next_in.next().unwrap();
        let mut chain = String::new();
        match crop_keys.filter(|_| j == 0) {
            Some(keys) => {
                // Named crop driven by sendcmd: only x changes, w/h stay fixed for the shot.
                let name = format!("crop_s{i}r{j}.cmd");
                let target = format!("crop@s{i}r{j}");
                let (x0, body) = sendcmd_body(keys, &target, probe.width, s.w, canvas.fps, frames);
                if body.is_empty() {
                    // The path never moves (one key, or a short still segment): ffmpeg's sendcmd
                    // rejects an empty command file, so use a plain crop at the path's position.
                    let _ = write!(chain, "crop={}:{}:{x0}:{}", s.w, s.h, s.y);
                } else {
                    cmd_files.push((name.clone(), body));
                    let _ = write!(
                        chain,
                        "sendcmd=f={name},{target}={}:{}:{x0}:{}",
                        s.w, s.h, s.y
                    );
                }
            }
            None => {
                let _ = write!(chain, "crop={}:{}:{}:{}", s.w, s.h, s.x, s.y);
            }
        }
        match r.fit {
            Fit::Cover => {
                let _ = write!(
                    chain,
                    ",scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},setsar=1",
                    d.w, d.h, d.w, d.h
                );
            }
            Fit::Contain => {
                let _ = write!(
                    chain,
                    ",scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1",
                    d.w, d.h
                );
            }
        }
        match &base {
            None => {
                // Full-canvas Cover region becomes the base.
                parts.push(format!("{label}{chain}[s{i}c{pos}]"));
            }
            Some(b) => {
                parts.push(format!("{label}{chain}[s{i}r{pos}]"));
                let (ox, oy) = match r.fit {
                    Fit::Cover => (d.x.to_string(), d.y.to_string()),
                    Fit::Contain => (
                        format!("{}+({}-overlay_w)/2", d.x, d.w),
                        format!("{}+({}-overlay_h)/2", d.y, d.h),
                    ),
                };
                parts.push(format!("{b}[s{i}r{pos}]overlay=x={ox}:y={oy}[s{i}c{pos}]"));
            }
        }
        base = Some(format!("[s{i}c{pos}]"));
    }
    let last = base.expect("segment has a base");
    parts.push(format!("{last}setsar=1,format=yuv420p{tail}[v{i}]"));
}

/// Freeze frames of one segment as (frame index in the moving picture, held frames, sample
/// offset in the moving audio, held samples), in time order. `freezes` holds (edit offset from
/// the segment start, seconds); `frames` is the segment's total frame count (holds included).
fn segment_holds(freezes: &[(f64, f64)], speed: f64, fps: f64, frames: u64) -> Vec<(u64, u64, u64, u64)> {
    let held: u64 = freezes.iter().map(|(_, d)| frame_at(*d, fps).max(1)).sum();
    let moving = frames.saturating_sub(held).max(1);
    freezes
        .iter()
        .map(|(off, d)| {
            let n = frame_at(*d, fps).max(1);
            let f = frame_at(off / speed, fps).min(moving - 1);
            (f, n, sample_at(f as f64 / fps), sample_at(n as f64 / fps))
        })
        .collect()
}

/// `chain` (the moving audio of segment `i`, `moving` samples, no output label) cut at the
/// holds, with silence of each hold's length in between -> `[a{i}]`.
fn audio_with_holds(i: usize, chain: &str, moving: u64, holds: &[(u64, u64, u64, u64)]) -> String {
    let mut held: Vec<(u64, u64)> = holds.iter().map(|h| (h.2.min(moving), h.3)).collect();
    held.sort_unstable();
    // Pieces of moving audio between the cut points; silence k follows piece k.
    let mut pieces: Vec<(u64, u64)> = Vec::new();
    let mut prev = 0;
    for (c, _) in &held {
        pieces.push((prev, *c));
        prev = *c;
    }
    pieces.push((prev, moving));
    let live: Vec<usize> = (0..pieces.len()).filter(|&k| pieces[k].1 > pieces[k].0).collect();
    let mut out = Vec::new();
    match live.len() {
        0 => out.push(format!("{chain},anullsink")),
        1 => {
            let (a, b) = pieces[live[0]];
            out.push(format!("{chain},atrim=start_sample={a}:end_sample={b},asetpts=N/SR/TB[a{i}m{}]", live[0]));
        }
        n => {
            let labels: String = live.iter().map(|k| format!("[a{i}s{k}]")).collect();
            out.push(format!("{chain},asplit={n}{labels}"));
            for &k in &live {
                let (a, b) = pieces[k];
                out.push(format!("[a{i}s{k}]atrim=start_sample={a}:end_sample={b},asetpts=N/SR/TB[a{i}m{k}]"));
            }
        }
    }
    let mut order: Vec<String> = Vec::new();
    for k in 0..pieces.len() {
        if live.contains(&k) {
            order.push(format!("[a{i}m{k}]"));
        }
        if let Some(&(_, z)) = held.get(k) {
            out.push(format!(
                "anullsrc=r=48000:cl=stereo,{AUDIO_FORMAT},atrim=end_sample={z},asetpts=N/SR/TB[a{i}z{k}]"
            ));
            order.push(format!("[a{i}z{k}]"));
        }
    }
    out.push(format!("{}concat=n={}:v=0:a=1,asetpts=N/SR/TB[a{i}]", order.concat(), order.len()));
    out.join(";\n")
}

/// Interpolates the keys at every output frame and writes a sendcmd line only when the even
/// pixel x changes. Times sit half a frame before the frame they apply to, so 4-decimal rounding
/// can never push a change onto the following frame. Returns (initial x, file body).
fn sendcmd_body(
    keys: &[CropKey],
    target: &str,
    src_w: u32,
    crop_w: u32,
    fps: f64,
    frames: u64,
) -> (u32, String) {
    let mut keys: Vec<CropKey> = keys
        .iter()
        .copied()
        .filter(|k| k.t.is_finite() && k.x.is_finite())
        .collect();
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    let max_x = even((even(src_w as f64).saturating_sub(crop_w)) as f64);
    let at = |t: f64| -> u32 {
        let x = if keys.is_empty() {
            0.0
        } else if t <= keys[0].t {
            keys[0].x
        } else if t >= keys[keys.len() - 1].t {
            keys[keys.len() - 1].x
        } else {
            let k = keys.partition_point(|k| k.t <= t);
            let (a, b) = (keys[k - 1], keys[k]);
            let span = b.t - a.t;
            if span <= 0.0 {
                b.x
            } else {
                a.x + (b.x - a.x) * (t - a.t) / span
            }
        };
        even((x * src_w as f64).max(0.0)).min(max_x)
    };
    let x0 = at(0.0);
    let mut body = String::new();
    let mut prev = x0;
    for f in 1..frames {
        let x = at(f as f64 / fps);
        if x != prev {
            let t = (f as f64 - 0.5) / fps;
            let _ = writeln!(body, "{t:.4} {target} x {x};");
            prev = x;
        }
    }
    (x0, body)
}

fn validate(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    extras: &CompileExtras,
) -> rustler_core::Result<()> {
    let v = |m: String| RustlerError::new(ErrorCode::Validation, STAGE, m);
    if clip.segments.is_empty() {
        return Err(
            v("clip has no segments".into()).hint("add one with `rustler clip add-segment`")
        );
    }
    let c = clip.canvas;
    if c.width < 2 || c.height < 2 || !(c.fps.is_finite() && c.fps > 0.0) {
        return Err(v(format!(
            "invalid canvas {}x{} @ {} fps",
            c.width, c.height, c.fps
        )));
    }
    for seg in &clip.segments {
        let src = sources.get(&seg.source_id).ok_or_else(|| {
            v(format!(
                "segment {} references unknown source {}",
                seg.id, seg.source_id
            ))
        })?;
        if !src.probe.has_video || src.probe.width < 2 || src.probe.height < 2 {
            return Err(v(format!("source {} has no video", seg.source_id)));
        }
        if !(seg.start.is_finite() && seg.end.is_finite()) || seg.end <= seg.start {
            return Err(v(format!(
                "segment {}: end ({}) must be after start ({})",
                seg.id, seg.end, seg.start
            )));
        }
        if seg.start < 0.0 || seg.end > src.probe.duration + END_TOLERANCE {
            return Err(v(format!(
                "segment {} [{:.3}, {:.3}] is outside source {} duration {:.3}",
                seg.id, seg.start, seg.end, seg.source_id, src.probe.duration
            )));
        }
        for r in &seg.layout.regions {
            for (name, rect) in [("src", &r.src), ("dst", &r.dst)] {
                let ok = [rect.x, rect.y, rect.w, rect.h]
                    .iter()
                    .all(|x| x.is_finite())
                    && rect.w > 0.0
                    && rect.h > 0.0;
                if !ok {
                    return Err(v(format!(
                        "segment {} region {}: {name} rect must have positive w/h",
                        seg.id, r.id
                    )));
                }
            }
        }
    }
    if let Some(m) = &extras.music
        && !(m.volume_db.is_finite() && m.fade_in >= 0.0 && m.fade_out >= 0.0)
    {
        return Err(v("music: volume_db must be finite and fades >= 0".into()));
    }
    for o in &extras.overlays {
        let ok = o.start.is_finite()
            && o.end.is_finite()
            && o.start >= 0.0
            && o.end > o.start
            && (0.0..=1.0).contains(&o.position_y)
            && o.font_size_factor.is_finite()
            && (0.01..=0.2).contains(&o.font_size_factor);
        if !ok {
            return Err(v(format!(
                "overlay {}: needs 0 <= start < end, position_y in 0..1 and font_size_factor in 0.01..0.2",
                o.id
            ))
            .hint("fix it with `rustler overlay edit` or remove it with `rustler overlay remove`"));
        }
    }
    for (id, g) in &extras.segment_gain_db {
        if !(g.is_finite() && (-60.0..=24.0).contains(g)) {
            return Err(v(format!("segment {id}: volume {g} dB must be between -60 and 24")));
        }
    }
    if !(extras.speech_gain_db.is_finite() && (-60.0..=24.0).contains(&extras.speech_gain_db)) {
        return Err(v("clip volume must be between -60 and 24 dB".into()));
    }
    for s in &extras.sfx {
        if !(s.at.is_finite() && s.at >= 0.0 && s.volume_db.is_finite() && (-60.0..=12.0).contains(&s.volume_db)) {
            return Err(v(format!("sfx {}: needs at >= 0 and volume_db in -60..12", s.id))
                .hint("fix it with `rustler audio remove-sfx` and add it again"));
        }
    }
    for e in &extras.effects {
        let (lo, hi) = e.kind.amount_range();
        let a = e.amount_or_default();
        let ok = e.start.is_finite() && e.end.is_finite() && e.start >= 0.0 && e.end > e.start && a.is_finite();
        if !ok || !(lo..=hi).contains(&a) {
            return Err(v(format!("effect {}: needs 0 <= start < end and amount in {lo}..{hi}", e.id))
                .hint("remove it with `rustler effect remove` and add it again"));
        }
    }
    if let Some(pb) = &extras.progress_bar
        && (ffmpeg_color(&pb.color).is_none() || !(2..=200).contains(&pb.height))
    {
        return Err(v(format!("progress bar: color must be #RRGGBB and height 2..200 (got {} / {})", pb.color, pb.height))
            .hint("set it again with `rustler overlay progress-bar <clip_id> --enabled true`"));
    }
    for i in &extras.images {
        crate::layers::check_image(i).map_err(|m| {
            v(format!("image {}: {m}", i.id)).hint("fix it with `rustler image edit` or remove it with `rustler image remove`")
        })?;
    }
    for m in &extras.masks {
        crate::layers::check_mask(m).map_err(|e| {
            v(format!("mask {}: {e}", m.id)).hint("fix it with `rustler mask edit` or remove it with `rustler mask remove`")
        })?;
    }
    for c in &extras.cutaways {
        crate::layers::check_cutaway(c).map_err(|e| {
            v(format!("cutaway {}: {e}", c.id)).hint("fix it with `rustler cutaway edit` or remove it with `rustler cutaway remove`")
        })?;
        if c.media.file.is_none() {
            return Err(v(format!("cutaway {}: media file is not resolved", c.id)));
        }
    }
    for (id, f) in &extras.segment_speed {
        if !(f.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(f)) {
            return Err(v(format!("segment {id}: speed {f} must be between {MIN_SPEED} and {MAX_SPEED}"))
                .hint("set it again with `rustler edit speed <clip_id> --segment-id <id> --factor 1.25`"));
        }
    }
    let edit = output_duration(clip);
    for f in &extras.freezes {
        let ok = f.out_t.is_finite() && f.out_t >= 0.0 && f.out_t <= edit + END_TOLERANCE;
        if !ok || !(f.duration.is_finite() && f.duration > 0.0 && f.duration <= MAX_FREEZE) {
            return Err(v(format!(
                "freeze {}: needs 0 <= out_t <= {edit:.2} (edit duration) and 0 < duration <= {MAX_FREEZE}",
                f.id
            ))
            .hint("fix it with `rustler timeline apply` or remove it from tracks.freezes"));
        }
    }
    check_transitions_on(clip, &TransitionMap::with_retime(clip, &extras.transitions, &extras.segment_speed, &extras.freezes))
}

/// Transitions take frames from both neighbours: every segment must be longer than the
/// transitions on its two sides (Validation otherwise).
pub fn check_transitions(clip: &Clip, transitions: &[Transition]) -> rustler_core::Result<()> {
    check_transitions_on(clip, &TransitionMap::new(clip, transitions))
}

/// [`check_transitions`] on a map that may also carry segment speeds and freezes.
pub fn check_transitions_on(clip: &Clip, tmap: &TransitionMap) -> rustler_core::Result<()> {
    let (frames, _) = segment_lengths(tmap);
    for (k, seg) in clip.segments.iter().enumerate() {
        let tin = if k > 0 { tmap.at_boundary(k - 1).map_or(0, |(_, f)| f) } else { 0 };
        let tout = tmap.at_boundary(k).map_or(0, |(_, f)| f);
        if tin + tout > 0 && tin + tout >= frames[k] {
            return Err(RustlerError::new(
                ErrorCode::Validation,
                STAGE,
                format!(
                    "segment {} is {} frames long but its transitions overlap {} frames",
                    seg.id,
                    frames[k],
                    tin + tout
                ),
            )
            .hint("shorten the transition with `rustler transition set ... --duration 0.2` or remove it with `rustler transition clear`"));
        }
    }
    Ok(())
}

fn stage_ass(ass: &Path, workdir: &Path) -> rustler_core::Result<()> {
    let dest = workdir.join(ASS_FILE);
    if same_file(ass, &dest) {
        return Ok(());
    }
    std::fs::copy(ass, &dest).map_err(|e| io_err("copy captions", ass, e))?;
    Ok(())
}

fn stage_fonts(fonts_dir: &Path, workdir: &Path) -> rustler_core::Result<()> {
    let dest = workdir.join(FONTS_DIR);
    if same_file(fonts_dir, &dest) {
        return Ok(());
    }
    std::fs::create_dir_all(&dest).map_err(|e| io_err("create fonts dir", &dest, e))?;
    let Ok(rd) = std::fs::read_dir(fonts_dir) else {
        return Ok(()); // no fonts: libass falls back to system fonts
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() {
            let to = dest.join(entry.file_name());
            if !to.exists() {
                std::fs::copy(&p, &to).map_err(|e| io_err("copy font", &p, e))?;
            }
        }
    }
    Ok(())
}

/// ffmpeg runs with cwd = workdir, so every argv path must be absolute.
fn abs(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn write_file(p: &Path, body: &str) -> rustler_core::Result<()> {
    std::fs::write(p, body).map_err(|e| io_err("write", p, e))
}

fn io_err(what: &str, p: &Path, e: std::io::Error) -> RustlerError {
    RustlerError::new(
        ErrorCode::Internal,
        STAGE,
        format!("{what} {}: {e}", p.display()),
    )
}

/// Rounds down to an even integer.
fn even(v: f64) -> u32 {
    let v = if v.is_finite() && v > 0.0 {
        v.floor() as u64
    } else {
        0
    };
    (v - v % 2) as u32
}

pub(crate) fn frame_at(t: f64, fps: f64) -> u64 {
    (t * fps).round() as u64
}

pub(crate) fn sample_at(t: f64) -> u64 {
    (t * SR as f64).round() as u64
}

/// Seconds for argv: millisecond precision.
pub(crate) fn secs(t: f64) -> String {
    format!("{t:.3}")
}

/// Compact number: up to 3 decimals, trailing zeros trimmed (-14.0 -> "-14").
pub(crate) fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

const NTSC: &[(u32, u32)] = &[(24000, 1001), (30000, 1001), (60000, 1001)];

/// Frame rate as an ffmpeg rational string ("30", "30000/1001", "25.5").
pub fn fps_str(fps: f64) -> String {
    for &(n, d) in NTSC {
        if (fps - n as f64 / d as f64).abs() < 0.005 {
            return format!("{n}/{d}");
        }
    }
    if (fps - fps.round()).abs() < 1e-6 {
        format!("{}", fps.round() as u64)
    } else {
        num(fps)
    }
}

pub(crate) fn tb_str(fps: f64) -> String {
    let f = fps_str(fps);
    match f.split_once('/') {
        Some((n, d)) => format!("{d}/{n}"),
        None => format!("1/{f}"),
    }
}
