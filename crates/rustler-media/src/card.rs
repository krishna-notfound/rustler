//! Agent-designed graphics: a JSON [`CardSpec`] (canvas, background, rounded shapes, images, text
//! blocks) rendered to a PNG with tiny-skia (shapes, gradients, glyph outlines) and ab_glyph (font
//! metrics and outlines). Used for title cards, lower thirds, badges, CTA cards and logo plates.
//!
//! All coordinates and sizes are canvas pixels. Draw order: background, shapes, images, texts
//! (in list order inside each group). Text wraps by measured glyph advances; emoji are drawn from a
//! bitmap emoji font (Noto Color Emoji) when one is installed, else skipped with a warning.

use ab_glyph::{Font, FontArc, GlyphId, OutlineCurve};
use image::RgbaImage;
use rustler_core::{ErrorCode, RustlerError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tiny_skia::{
    FillRule, FilterQuality, GradientStop, IntSize, LineCap, LineJoin, LinearGradient, Mask, Paint, PathBuilder, Pixmap,
    PixmapPaint, Point, Shader, SpreadMode, Stroke, Transform,
};

type Result<T> = rustler_core::Result<T>;

const STAGE: &str = "asset.render_card";
const MAX_SIDE: u32 = 4096;

fn validation(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg)
}

// ---------------------------------------------------------------------------------------------
// Spec types
// ---------------------------------------------------------------------------------------------

/// A rectangle in canvas pixels (`x`,`y` = top-left).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RectPx {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Linear gradient: `angle_deg` 0 = left to right, 90 = top to bottom (default), 45 = top-left to bottom-right.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GradientSpec {
    pub from: String,
    pub to: String,
    pub angle_deg: Option<f32>,
}

/// A paint: a colour string (`#RRGGBB`, `#RRGGBBAA`, `#RGB`, `transparent`, `white`, `black`) or a gradient.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Fill {
    Color(String),
    Gradient(GradientSpec),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Background {
    Transparent,
    Solid { color: String },
    LinearGradient { from: String, to: String, angle_deg: Option<f32> },
}

impl Default for Background {
    fn default() -> Self {
        Background::Transparent
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrokeSpec {
    pub color: String,
    /// Line width in px (centred on the edge).
    pub width: f32,
}

/// Outline around text: `width` px visible outside the glyphs.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OutlineSpec {
    pub color: String,
    pub width: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShadowSpec {
    /// Default `#00000080`.
    pub color: Option<String>,
    #[serde(default)]
    pub dx: f32,
    #[serde(default)]
    pub dy: f32,
    /// Blur radius in px (0 = hard shadow).
    #[serde(default)]
    pub blur: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShapeSpec {
    pub rect: RectPx,
    /// Corner radius in px (clamped to half the shorter side; a square with a big radius is a circle).
    #[serde(default)]
    pub radius: f32,
    pub fill: Option<Fill>,
    pub stroke: Option<StrokeSpec>,
    pub shadow: Option<ShadowSpec>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// Whole image visible, centred (default).
    #[default]
    Contain,
    /// Fills the rect, cropping the overflow.
    Cover,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageSpec {
    /// An image asset id (`image-logo`, `asset:image-logo` also works) or a local file path.
    pub asset: String,
    pub rect: RectPx,
    #[serde(default)]
    pub fit: ImageFit,
    /// Rounded corners in px (use half the side for a circle).
    #[serde(default)]
    pub radius: f32,
    /// 0..1 (default 1).
    pub opacity: Option<f32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// 100..900 or a name: thin, light, regular, medium, semibold, bold, extrabold, black.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Weight {
    Number(u32),
    Name(String),
}

impl Weight {
    pub fn value(&self) -> Option<u32> {
        match self {
            Weight::Number(n) if (100..=900).contains(n) => Some(*n),
            Weight::Number(_) => None,
            Weight::Name(s) => match s.trim().to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
                "thin" => Some(100),
                "extralight" | "ultralight" => Some(200),
                "light" => Some(300),
                "regular" | "normal" | "book" => Some(400),
                "medium" => Some(500),
                "semibold" | "demibold" => Some(600),
                "bold" => Some(700),
                "extrabold" | "ultrabold" => Some(800),
                "black" | "heavy" => Some(900),
                _ => None,
            },
        }
    }
}

/// A rounded box hugging the text (measured from the real glyph ink), with `padding` px around it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextBox {
    pub color: String,
    #[serde(default)]
    pub radius: f32,
    #[serde(default)]
    pub padding: f32,
}

/// Animation hint stored with a text style (consumed by the layers that render it over video).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextAnim {
    /// pop | bounce | fade | typewriter | slide_up
    pub kind: String,
    pub duration_s: Option<f32>,
}

pub const ANIM_KINDS: [&str; 5] = ["pop", "bounce", "fade", "typewriter", "slide_up"];

/// A saved text look (`text.create_style`, asset kind `text_style`). `size` is in px of the canvas
/// the text is drawn on (cards: card px; 1080-wide video: px at 1080).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    pub name: String,
    /// Font asset id, file path or installed family name.
    pub font: String,
    pub size: f32,
    pub weight: Option<Weight>,
    pub color: String,
    pub outline: Option<OutlineSpec>,
    #[serde(rename = "box")]
    pub box_: Option<TextBox>,
    pub shadow: Option<ShadowSpec>,
    pub uppercase: Option<bool>,
    pub anim: Option<TextAnim>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextSpec {
    pub text: String,
    /// A saved text style (name or asset id); fields given here override it.
    pub style: Option<String>,
    /// Font asset id (`font-poppins-bold`), a .ttf/.otf path, or an installed family ("Anton", "Arial").
    /// Default: the Rustler default font.
    pub font: Option<String>,
    /// Font size in px (default 64).
    pub size: Option<f32>,
    /// Picks the closest installed file of the family; ignored for an asset id or a path.
    pub weight: Option<Weight>,
    /// Default `#FFFFFF`.
    pub color: Option<String>,
    pub outline: Option<OutlineSpec>,
    #[serde(rename = "box")]
    pub box_: Option<TextBox>,
    pub shadow: Option<ShadowSpec>,
    pub uppercase: Option<bool>,
    #[serde(default)]
    pub align: Align,
    #[serde(default)]
    pub valign: VAlign,
    /// The text is laid out inside this rect.
    pub rect: RectPx,
    /// Wrap at the rect width by measured glyph widths (default true).
    pub wrap: Option<bool>,
    /// Line height as a multiple of the size (default 1.2).
    pub line_height: Option<f32>,
    /// Shrink the size until the block fits the rect height (default false).
    #[serde(default)]
    pub fit: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CardSpec {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub shapes: Vec<ShapeSpec>,
    #[serde(default)]
    pub images: Vec<ImageSpec>,
    #[serde(default)]
    pub texts: Vec<TextSpec>,
}

// ---------------------------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------------------------

type AssetResolver<'a> = Box<dyn Fn(&str) -> Option<PathBuf> + 'a>;
type StyleResolver<'a> = Box<dyn Fn(&str) -> Option<TextStyle> + 'a>;

/// What the renderer may look up: installed fonts, the default font, asset ids and text styles.
pub struct CardEnv<'a> {
    /// Directories searched (recursively, 4 levels) for `.ttf`/`.otf` files when a font is a family name.
    pub font_dirs: Vec<PathBuf>,
    pub default_font: Option<PathBuf>,
    /// Maps an asset id (font or image) to its file.
    pub resolve_asset: AssetResolver<'a>,
    /// Maps a text style name or asset id to the style.
    pub resolve_style: StyleResolver<'a>,
}

impl<'a> CardEnv<'a> {
    pub fn new(font_dirs: Vec<PathBuf>, default_font: Option<PathBuf>) -> Self {
        CardEnv { font_dirs, default_font, resolve_asset: Box::new(|_| None), resolve_style: Box::new(|_| None) }
    }
}

/// The rendered card and what was skipped.
pub struct CardOutput {
    pub image: RgbaImage,
    pub warnings: Vec<String>,
}

impl CardOutput {
    pub fn save_png(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("create {}: {e}", dir.display())))?;
        }
        self.image.save_with_format(path, image::ImageFormat::Png).map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("write {}: {e}", path.display())))
    }
}

// ---------------------------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------------------------

/// `#RGB`, `#RRGGBB`, `#RRGGBBAA`, `transparent`, `white`, `black` -> RGBA.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let t = s.trim();
    match t.to_ascii_lowercase().as_str() {
        "transparent" | "none" => return Some([0, 0, 0, 0]),
        "white" => return Some([255, 255, 255, 255]),
        "black" => return Some([0, 0, 0, 255]),
        _ => {}
    }
    let h = t.strip_prefix('#')?;
    if !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    match h.len() {
        3 => {
            let n = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
            Some([n(0)?, n(1)?, n(2)?, 255])
        }
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

fn colour(label: &str, s: &str) -> Result<[u8; 4]> {
    parse_color(s).ok_or_else(|| validation(format!("{label} '{s}' is not a colour")).hint("use #RRGGBB, #RRGGBBAA, #RGB, transparent, white or black"))
}

fn sk_color(c: [u8; 4]) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

fn solid_paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(sk_color(c));
    p.anti_alias = true;
    p
}

fn gradient_paint(rect: &RectPx, from: [u8; 4], to: [u8; 4], angle_deg: f32) -> Paint<'static> {
    let a = angle_deg.to_radians();
    let (dx, dy) = (a.cos(), a.sin());
    let half = (rect.w * dx.abs() + rect.h * dy.abs()) / 2.0;
    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let stops = vec![GradientStop::new(0.0, sk_color(from)), GradientStop::new(1.0, sk_color(to))];
    let shader = LinearGradient::new(
        Point::from_xy(cx - dx * half, cy - dy * half),
        Point::from_xy(cx + dx * half, cy + dy * half),
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(sk_color(from)));
    let mut p = Paint::default();
    p.shader = shader;
    p.anti_alias = true;
    p
}

fn fill_paint(label: &str, fill: &Fill, rect: &RectPx) -> Result<Paint<'static>> {
    match fill {
        Fill::Color(c) => Ok(solid_paint(colour(label, c)?)),
        Fill::Gradient(g) => Ok(gradient_paint(rect, colour(&format!("{label}.from"), &g.from)?, colour(&format!("{label}.to"), &g.to)?, g.angle_deg.unwrap_or(90.0))),
    }
}

// ---------------------------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------------------------

fn check_rect(label: &str, r: &RectPx) -> Result<()> {
    if ![r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) || r.w <= 0.0 || r.h <= 0.0 {
        return Err(validation(format!("{label} must have finite numbers and w, h > 0 (canvas pixels)")));
    }
    Ok(())
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    if r < 0.01 {
        pb.move_to(x, y);
        pb.line_to(x + w, y);
        pb.line_to(x + w, y + h);
        pb.line_to(x, y + h);
        pb.close();
        return pb.finish();
    }
    let k = 0.552_284_75 * r;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Three box-blur passes (a close Gaussian) over premultiplied RGBA8.
fn blur_rgba(data: &mut [u8], w: usize, h: usize, blur: f32) {
    let r = (blur * 0.58).ceil() as usize;
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let win = (2 * r + 1) as u32;
    let mut tmp = vec![0u8; data.len()];
    for _ in 0..3 {
        // horizontal: data -> tmp
        for y in 0..h {
            for c in 0..4 {
                let mut pre = vec![0u32; w + 1];
                for x in 0..w {
                    pre[x + 1] = pre[x] + data[(y * w + x) * 4 + c] as u32;
                }
                for x in 0..w {
                    let hi = (x + r + 1).min(w);
                    let lo = x.saturating_sub(r);
                    tmp[(y * w + x) * 4 + c] = ((pre[hi] - pre[lo]) / win) as u8;
                }
            }
        }
        // vertical: tmp -> data
        for x in 0..w {
            for c in 0..4 {
                let mut pre = vec![0u32; h + 1];
                for y in 0..h {
                    pre[y + 1] = pre[y] + tmp[(y * w + x) * 4 + c] as u32;
                }
                for y in 0..h {
                    let hi = (y + r + 1).min(h);
                    let lo = y.saturating_sub(r);
                    data[(y * w + x) * 4 + c] = ((pre[hi] - pre[lo]) / win) as u8;
                }
            }
        }
    }
}

/// Draws a (blurred) shadow: `draw` paints the silhouette into a scratch layer using the given paint and
/// offset transform.
fn draw_shadow(pm: &mut Pixmap, label: &str, sh: &ShadowSpec, draw: impl Fn(&mut Pixmap, &Paint, Transform)) -> Result<()> {
    let c = colour(label, sh.color.as_deref().unwrap_or("#00000080"))?;
    let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return Ok(()) };
    draw(&mut layer, &solid_paint(c), Transform::from_translate(sh.dx, sh.dy));
    let (w, h) = (layer.width() as usize, layer.height() as usize);
    blur_rgba(layer.data_mut(), w, h, sh.blur.max(0.0));
    pm.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------------------------

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

fn find_font_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 4 {
                find_font_files(&p, depth + 1, out);
            }
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")) {
            out.push(p);
        }
    }
}

fn system_font_dirs() -> Vec<PathBuf> {
    [r"C:\Windows\Fonts", "/usr/share/fonts", "/Library/Fonts", "/System/Library/Fonts/Supplemental"].iter().map(PathBuf::from).filter(|p| p.is_dir()).collect()
}

const WEIGHT_WORDS: [(&str, u32); 14] = [
    ("extrabold", 800),
    ("ultrabold", 800),
    ("extralight", 200),
    ("semibold", 600),
    ("demibold", 600),
    ("regular", 400),
    ("medium", 500),
    ("black", 900),
    ("heavy", 900),
    ("light", 300),
    ("thin", 100),
    ("bold", 700),
    ("book", 400),
    ("italic", 400),
];

/// (weight guessed from the file stem, leftover characters after removing the family and weight words).
fn stem_weight(stem: &str, family: &str) -> (u32, usize) {
    let mut s = norm(stem).replacen(family, "", 1);
    let mut weight = 400;
    for (word, w) in WEIGHT_WORDS {
        if s.contains(word) {
            s = s.replacen(word, "", 1);
            if word != "italic" {
                weight = w;
            }
        }
    }
    // Windows-style suffixes (arialbd, ariali, arialbi).
    for (suf, w) in [("bd", Some(700)), ("bi", Some(700)), ("b", Some(700)), ("i", None), ("z", Some(700))] {
        if s == suf {
            if let Some(w) = w {
                weight = w;
            }
            s.clear();
        }
    }
    (weight, s.len())
}

fn family_names(files: &[PathBuf]) -> Vec<String> {
    let mut v: Vec<String> = files
        .iter()
        .filter_map(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .map(|s| s.split(['-', '[']).next().unwrap_or(&s).to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

struct Fonts {
    cache: HashMap<String, FontArc>,
}

impl Fonts {
    fn load_path(&mut self, label: &str, p: &Path) -> Result<FontArc> {
        let key = p.to_string_lossy().into_owned();
        if let Some(f) = self.cache.get(&key) {
            return Ok(f.clone());
        }
        let bytes = std::fs::read(p).map_err(|e| RustlerError::new(ErrorCode::NotFound, STAGE, format!("{label}: cannot read font {}: {e}", p.display())))?;
        let f = FontArc::try_from_vec(bytes).map_err(|e| validation(format!("{label}: {} is not a usable ttf/otf font: {e}", p.display())))?;
        self.cache.insert(key, f.clone());
        Ok(f)
    }

    fn resolve(&mut self, label: &str, env: &CardEnv, font: Option<&str>, weight: Option<u32>) -> Result<FontArc> {
        let Some(name) = font.map(str::trim).filter(|f| !f.is_empty()) else {
            return match &env.default_font {
                Some(p) => self.load_path(label, p),
                None => Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("{label}: no font given and no default font is installed"))
                    .hint("run `rustler setup` (installs fonts) or add a font: `rustler asset add <file.ttf> --kind font`")),
            };
        };
        if let Some(id) = name.strip_prefix("asset:") {
            return match (env.resolve_asset)(id.trim()) {
                Some(p) => self.load_path(label, &p),
                None => Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("{label}: no font asset '{}'", id.trim())).hint("`rustler asset list --kind font` shows the ids")),
            };
        }
        if let Some(p) = (env.resolve_asset)(name) {
            return self.load_path(label, &p);
        }
        let as_path = Path::new(name);
        if as_path.is_file() {
            return self.load_path(label, as_path);
        }
        // Family lookup among installed fonts.
        let want = norm(name);
        let mut files = Vec::new();
        for d in env.font_dirs.iter().cloned().chain(system_font_dirs()) {
            find_font_files(&d, 0, &mut files);
        }
        let wanted_weight = weight.unwrap_or(400);
        let best = files
            .iter()
            .filter(|f| f.file_stem().is_some_and(|s| norm(&s.to_string_lossy()).contains(&want)))
            .map(|f| {
                let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let (w, left) = stem_weight(&stem, &want);
                (w.abs_diff(wanted_weight) as usize + left * 30, f)
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, f)| f.clone());
        match best {
            Some(p) => self.load_path(label, &p),
            None => {
                let have = family_names(&files);
                Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("{label}: font '{name}' is not a font asset id, a file, or an installed family"))
                    .hint(format!(
                        "use a font asset id (`rustler asset list --kind font`), a .ttf path, or one of the installed families: {}",
                        if have.is_empty() { "(none found)".to_string() } else { have.into_iter().take(20).collect::<Vec<_>>().join(", ") }
                    )))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Emoji (bitmap fonts such as Noto Color Emoji)
// ---------------------------------------------------------------------------------------------

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B50 | 0x2B55 | 0x2B1B | 0x2B1C | 0x231A | 0x231B | 0x23E9..=0x23F3)
}

fn is_emoji_modifier(c: char) -> bool {
    matches!(c as u32, 0xFE0E | 0xFE0F | 0x200D | 0x1F3FB..=0x1F3FF)
}

struct EmojiFont {
    bytes: Vec<u8>,
}

struct EmojiBitmap {
    img: RgbaImage,
    /// Offsets (strike px) of the bitmap's left and bottom edge from the glyph origin / baseline.
    x: f32,
    y: f32,
    ppem: f32,
}

impl EmojiFont {
    fn find(env: &CardEnv) -> Option<EmojiFont> {
        let mut files = Vec::new();
        for d in env.font_dirs.iter().cloned().chain(system_font_dirs()) {
            find_font_files(&d, 0, &mut files);
        }
        files.retain(|f| f.file_stem().is_some_and(|s| norm(&s.to_string_lossy()).contains("emoji")));
        files.sort_by_key(|f| !norm(&f.to_string_lossy()).contains("notocoloremoji"));
        files.into_iter().find_map(|f| {
            let bytes = std::fs::read(&f).ok()?;
            let font = EmojiFont { bytes };
            // Only bitmap fonts (CBDT/sbix) are usable; outline-colour fonts (COLR) are not.
            font.bitmap('\u{1F600}', 64.0).map(|_| font)
        })
    }

    fn bitmap(&self, c: char, size: f32) -> Option<EmojiBitmap> {
        let face = ttf_parser::Face::parse(&self.bytes, 0).ok()?;
        let gid = face.glyph_index(c)?;
        let r = face.glyph_raster_image(gid, size.round().clamp(1.0, 4000.0) as u16)?;
        if r.format != ttf_parser::RasterImageFormat::PNG {
            return None;
        }
        let img = image::load_from_memory_with_format(r.data, image::ImageFormat::Png).ok()?.to_rgba8();
        Some(EmojiBitmap { img, x: r.x as f32, y: r.y as f32, ppem: r.pixels_per_em.max(1) as f32 })
    }
}

// ---------------------------------------------------------------------------------------------
// Text layout
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
enum Glyph {
    Space,
    Text(GlyphId),
    Emoji(char),
}

#[derive(Clone)]
struct LChar {
    g: Glyph,
    adv: f32,
}

struct Layout {
    lines: Vec<(Vec<LChar>, f32)>,
}

struct Measure<'a> {
    font: &'a FontArc,
    emoji: Option<&'a EmojiFont>,
    size: f32,
    scale: f32,
}

fn tokenize(m: &Measure, text: &str, warnings: &mut Vec<String>) -> Vec<Vec<Vec<LChar>>> {
    // paragraphs -> words -> chars
    let mut paragraphs = Vec::new();
    for para in text.split('\n') {
        let mut words: Vec<Vec<LChar>> = Vec::new();
        for w in para.split(' ') {
            let mut chars: Vec<LChar> = Vec::new();
            let mut prev: Option<GlyphId> = None;
            for c in w.chars() {
                if is_emoji_modifier(c) || c == '\r' || c == '\t' {
                    continue;
                }
                let gid = m.font.glyph_id(c);
                let prefer_emoji = is_emoji(c) && (c as u32 >= 0x1F000 || gid.0 == 0);
                if prefer_emoji {
                    match m.emoji.and_then(|e| e.bitmap(c, m.size)) {
                        Some(bm) => {
                            let adv = bm.img.width() as f32 * m.size / bm.ppem;
                            chars.push(LChar { g: Glyph::Emoji(c), adv });
                            prev = None;
                            continue;
                        }
                        None => {
                            let w = format!("emoji '{c}' skipped: no bitmap emoji font found (install Noto Color Emoji into ~/.rustler/fonts)");
                            if !warnings.contains(&w) {
                                warnings.push(w);
                            }
                            continue;
                        }
                    }
                }
                if gid.0 == 0 {
                    let w = format!("character '{c}' has no glyph in the chosen font and was skipped");
                    if !warnings.contains(&w) {
                        warnings.push(w);
                    }
                    continue;
                }
                let mut adv = m.font.h_advance_unscaled(gid) * m.scale;
                if let Some(p) = prev {
                    adv += m.font.kern_unscaled(p, gid) * m.scale;
                }
                prev = Some(gid);
                chars.push(LChar { g: Glyph::Text(gid), adv });
            }
            words.push(chars);
        }
        paragraphs.push(words);
    }
    paragraphs
}

fn word_width(w: &[LChar]) -> f32 {
    w.iter().map(|c| c.adv).sum()
}

fn layout_text(m: &Measure, text: &str, max_w: Option<f32>, warnings: &mut Vec<String>) -> Layout {
    let space = {
        let g = m.font.glyph_id(' ');
        m.font.h_advance_unscaled(g) * m.scale
    };
    let mut lines: Vec<(Vec<LChar>, f32)> = Vec::new();
    for words in tokenize(m, text, warnings) {
        let mut cur: Vec<LChar> = Vec::new();
        let mut cur_w = 0.0f32;
        for word in words {
            let ww = word_width(&word);
            let needs_space = !cur.is_empty();
            let add = if needs_space { space + ww } else { ww };
            match max_w {
                Some(mw) if !cur.is_empty() && cur_w + add > mw => {
                    lines.push((std::mem::take(&mut cur), cur_w));
                    cur_w = 0.0;
                }
                _ => {}
            }
            let parts: Vec<Vec<LChar>> = match max_w {
                // a single word wider than the line: break it by characters
                Some(mw) if ww > mw => {
                    let mut out: Vec<Vec<LChar>> = vec![Vec::new()];
                    let mut w = 0.0;
                    for ch in word {
                        if w + ch.adv > mw && !out.last().is_some_and(|l| l.is_empty()) {
                            out.push(Vec::new());
                            w = 0.0;
                        }
                        w += ch.adv;
                        out.last_mut().map(|l| l.push(ch));
                    }
                    out
                }
                _ => vec![word],
            };
            let n = parts.len();
            for (i, part) in parts.into_iter().enumerate() {
                let pw = word_width(&part);
                if i + 1 < n {
                    if !cur.is_empty() {
                        lines.push((std::mem::take(&mut cur), cur_w));
                        cur_w = 0.0;
                    }
                    lines.push((part, pw));
                } else {
                    if !cur.is_empty() {
                        cur.push(LChar { g: Glyph::Space, adv: space });
                        cur_w += space;
                    }
                    cur_w += pw;
                    cur.extend(part);
                }
            }
        }
        lines.push((cur, cur_w));
    }
    // Trailing empty line from a trailing newline is kept; fully empty text gives one empty line.
    Layout { lines }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

fn glyph_path(pb: &mut PathBuilder, font: &FontArc, gid: GlyphId, x: f32, baseline: f32, s: f32) {
    let Some(outline) = font.outline(gid) else { return };
    let map = |p: ab_glyph::Point| (x + p.x * s, baseline - p.y * s);
    let mut last: Option<(f32, f32)> = None;
    let mut open = false;
    for curve in &outline.curves {
        let start = match curve {
            OutlineCurve::Line(a, _) | OutlineCurve::Quad(a, _, _) | OutlineCurve::Cubic(a, _, _, _) => map(*a),
        };
        let continues = last.is_some_and(|l| (l.0 - start.0).abs() < 0.01 && (l.1 - start.1).abs() < 0.01);
        if !continues {
            if open {
                pb.close();
            }
            pb.move_to(start.0, start.1);
            open = true;
        }
        last = Some(match curve {
            OutlineCurve::Line(_, b) => {
                let b = map(*b);
                pb.line_to(b.0, b.1);
                b
            }
            OutlineCurve::Quad(_, b, c) => {
                let (b, c) = (map(*b), map(*c));
                pb.quad_to(b.0, b.1, c.0, c.1);
                c
            }
            OutlineCurve::Cubic(_, b, c, d) => {
                let (b, c, d) = (map(*b), map(*c), map(*d));
                pb.cubic_to(b.0, b.1, c.0, c.1, d.0, d.1);
                d
            }
        });
    }
    if open {
        pb.close();
    }
}

fn effective_text(i: usize, t: &TextSpec, env: &CardEnv) -> Result<TextSpec> {
    let Some(sname) = t.style.as_deref().map(str::trim).filter(|s| !s.is_empty()) else { return Ok(t.clone()) };
    let st = (env.resolve_style)(sname).ok_or_else(|| {
        RustlerError::new(ErrorCode::NotFound, STAGE, format!("texts[{i}].style: no text style '{sname}'")).hint("`rustler text styles` lists them; create one with `rustler text create-style`")
    })?;
    let mut e = t.clone();
    e.font = e.font.or(Some(st.font));
    e.size = e.size.or(Some(st.size));
    e.weight = e.weight.or(st.weight);
    e.color = e.color.or(Some(st.color));
    e.outline = e.outline.or(st.outline);
    e.box_ = e.box_.or(st.box_);
    e.shadow = e.shadow.or(st.shadow);
    e.uppercase = e.uppercase.or(st.uppercase);
    Ok(e)
}

fn fill_rounded(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, radius: f32, paint: &Paint, t: Transform) {
    if let Some(p) = rounded_rect(x, y, w, h, radius) {
        pm.fill_path(&p, paint, FillRule::Winding, t, None);
    }
}

fn draw_shape(pm: &mut Pixmap, i: usize, s: &ShapeSpec) -> Result<()> {
    let label = format!("shapes[{i}]");
    check_rect(&format!("{label}.rect"), &s.rect)?;
    let r = s.rect;
    if let Some(sh) = &s.shadow {
        draw_shadow(pm, &format!("{label}.shadow.color"), sh, |layer, paint, t| fill_rounded(layer, r.x, r.y, r.w, r.h, s.radius, paint, t))?;
    }
    if let Some(f) = &s.fill {
        let paint = fill_paint(&format!("{label}.fill"), f, &r)?;
        fill_rounded(pm, r.x, r.y, r.w, r.h, s.radius, &paint, Transform::identity());
    }
    if let Some(st) = &s.stroke {
        if st.width > 0.0 {
            let paint = solid_paint(colour(&format!("{label}.stroke.color"), &st.color)?);
            if let Some(p) = rounded_rect(r.x, r.y, r.w, r.h, s.radius) {
                let stroke = Stroke { width: st.width, line_join: LineJoin::Round, ..Stroke::default() };
                pm.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
            }
        }
    }
    Ok(())
}

fn premultiplied(img: &RgbaImage) -> Option<Pixmap> {
    let mut data = Vec::with_capacity(img.as_raw().len());
    for p in img.pixels() {
        let a = p[3] as u32;
        data.extend_from_slice(&[(p[0] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[2] as u32 * a / 255) as u8, p[3]]);
    }
    Pixmap::from_vec(data, IntSize::from_wh(img.width(), img.height())?)
}

fn draw_image(pm: &mut Pixmap, i: usize, im: &ImageSpec, env: &CardEnv) -> Result<()> {
    let label = format!("images[{i}]");
    check_rect(&format!("{label}.rect"), &im.rect)?;
    let key = im.asset.trim();
    let id = key.strip_prefix("asset:").unwrap_or(key).trim();
    let path = (env.resolve_asset)(id).or_else(|| Some(PathBuf::from(key)).filter(|p| p.is_file())).ok_or_else(|| {
        RustlerError::new(ErrorCode::NotFound, STAGE, format!("{label}.asset: no image asset or file '{key}'")).hint("`rustler asset list --kind image` shows the ids")
    })?;
    let src = image::open(&path).map_err(|e| RustlerError::new(ErrorCode::NotFound, STAGE, format!("{label}: cannot read image {}: {e}", path.display())))?.to_rgba8();
    let r = im.rect;
    let (rw, rh) = (r.w.round().max(1.0) as u32, r.h.round().max(1.0) as u32);
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    let k = match im.fit {
        ImageFit::Contain => (r.w / iw).min(r.h / ih),
        ImageFit::Cover => (r.w / iw).max(r.h / ih),
    };
    let (nw, nh) = ((iw * k).round().max(1.0) as u32, (ih * k).round().max(1.0) as u32);
    let scaled = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
    // Canvas-sized crop/placement inside the rect.
    let mut tile = RgbaImage::new(rw, rh);
    let ox = (rw as i64 - nw as i64) / 2;
    let oy = (rh as i64 - nh as i64) / 2;
    for (x, y, p) in scaled.enumerate_pixels() {
        let (tx, ty) = (x as i64 + ox, y as i64 + oy);
        if tx >= 0 && ty >= 0 && (tx as u32) < rw && (ty as u32) < rh {
            tile.put_pixel(tx as u32, ty as u32, *p);
        }
    }
    let Some(tile_pm) = premultiplied(&tile) else { return Ok(()) };
    let mut paint = PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
    paint.opacity = im.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
    let mask = if im.radius > 0.0 {
        let mut m = Mask::new(pm.width(), pm.height());
        if let (Some(m), Some(p)) = (m.as_mut(), rounded_rect(r.x, r.y, rw as f32, rh as f32, im.radius)) {
            m.fill_path(&p, FillRule::Winding, true, Transform::identity());
        }
        m
    } else {
        None
    };
    pm.draw_pixmap(r.x.round() as i32, r.y.round() as i32, tile_pm.as_ref(), &paint, Transform::identity(), mask.as_ref());
    Ok(())
}

struct TextCtx<'a> {
    fonts: Fonts,
    emoji: Option<Option<EmojiFont>>,
    env: &'a CardEnv<'a>,
    warnings: Vec<String>,
}

fn draw_text(pm: &mut Pixmap, i: usize, raw: &TextSpec, cx: &mut TextCtx) -> Result<()> {
    let label = format!("texts[{i}]");
    let t = effective_text(i, raw, cx.env)?;
    check_rect(&format!("{label}.rect"), &t.rect)?;
    let color = colour(&format!("{label}.color"), t.color.as_deref().unwrap_or("#FFFFFF"))?;
    let weight = t.weight.as_ref().map(|w| w.value().ok_or_else(|| validation(format!("{label}.weight must be 100..900 or a name like bold")))).transpose()?;
    let font = cx.fonts.resolve(&label, cx.env, t.font.as_deref(), weight)?;
    let mut size = t.size.unwrap_or(64.0);
    if !(size.is_finite() && size >= 1.0 && size <= 2000.0) {
        return Err(validation(format!("{label}.size must be 1..2000 px")));
    }
    let line_mult = t.line_height.unwrap_or(1.2).clamp(0.5, 4.0);
    let text = if t.uppercase.unwrap_or(false) { t.text.to_uppercase() } else { t.text.clone() };
    let wrap = t.wrap.unwrap_or(true);
    let upem = font.units_per_em().unwrap_or(1000.0);
    let rect = t.rect;
    if text.chars().any(|c| is_emoji(c) && !is_emoji_modifier(c)) && cx.emoji.is_none() {
        cx.emoji = Some(EmojiFont::find(cx.env));
    }
    let emoji = cx.emoji.as_ref().and_then(|e| e.as_ref());

    let (layout, lh) = loop {
        let m = Measure { font: &font, emoji, size, scale: size / upem };
        let mut local = Vec::new();
        let l = layout_text(&m, &text, wrap.then_some(rect.w), &mut local);
        let lh = size * line_mult;
        if !t.fit || size <= 8.0 || l.lines.len() as f32 * lh <= rect.h {
            for w in local {
                if !cx.warnings.contains(&w) {
                    cx.warnings.push(w);
                }
            }
            break (l, lh);
        }
        size *= 0.94;
    };
    let s = size / upem;
    let (asc, desc) = (font.ascent_unscaled() * s, font.descent_unscaled() * s);
    let block_h = layout.lines.len() as f32 * lh;
    let top = match t.valign {
        VAlign::Top => rect.y,
        VAlign::Middle => rect.y + (rect.h - block_h) / 2.0,
        VAlign::Bottom => rect.y + rect.h - block_h,
    };
    let max_line_w = layout.lines.iter().map(|l| l.1).fold(0.0f32, f32::max);
    if block_h > rect.h + 0.5 || max_line_w > rect.w + 0.5 {
        cx.warnings.push(format!("{label} overflows its rect ({:.0}x{:.0} needed, rect is {:.0}x{:.0}); shorten it, enlarge the rect or set fit: true", max_line_w, block_h, rect.w, rect.h));
    }

    // Place lines; collect glyph path, emoji draws and ink bounds.
    let mut pb = PathBuilder::new();
    let (mut ink_l, mut ink_r, mut ink_t, mut ink_b) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    let mut emoji_draws: Vec<(char, f32, f32)> = Vec::new();
    for (li, (chars, lw)) in layout.lines.iter().enumerate() {
        let x0 = match t.align {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.w - lw) / 2.0,
            Align::Right => rect.x + rect.w - lw,
        };
        let baseline = top + li as f32 * lh + (lh - (asc - desc)) / 2.0 + asc;
        let mut x = x0;
        let (mut line_t, mut line_b) = (f32::MAX, f32::MIN);
        for ch in chars {
            match &ch.g {
                Glyph::Text(gid) => {
                    glyph_path(&mut pb, &font, *gid, x, baseline, s);
                    if let Some(o) = font.outline(*gid) {
                        // ab_glyph stores outline bounds with y flipped: min.y = glyph top (y_max), max.y = bottom (y_min).
                        line_t = line_t.min(baseline - o.bounds.min.y * s);
                        line_b = line_b.max(baseline - o.bounds.max.y * s);
                    }
                }
                Glyph::Emoji(c) => {
                    emoji_draws.push((*c, x, baseline));
                    line_t = line_t.min(baseline - size * 0.95);
                    line_b = line_b.max(baseline + size * 0.2);
                }
                Glyph::Space => {}
            }
            x += ch.adv;
        }
        if line_t <= line_b {
            ink_t = ink_t.min(line_t);
            ink_b = ink_b.max(line_b);
            ink_l = ink_l.min(x0);
            ink_r = ink_r.max(x0 + lw);
        }
    }
    let path = pb.finish();

    // Box behind the text, hugging the measured ink.
    if let (Some(b), true) = (&t.box_, ink_l <= ink_r) {
        let bc = colour(&format!("{label}.box.color"), &b.color)?;
        let p = b.padding.max(0.0);
        let (bx, by, bw, bh) = (ink_l - p, ink_t - p, ink_r - ink_l + 2.0 * p, ink_b - ink_t + 2.0 * p);
        if let Some(sh) = &t.shadow {
            draw_shadow(pm, &format!("{label}.shadow.color"), sh, |layer, paint, tr| fill_rounded(layer, bx, by, bw, bh, b.radius, paint, tr))?;
        }
        fill_rounded(pm, bx, by, bw, bh, b.radius, &solid_paint(bc), Transform::identity());
    } else if let (Some(sh), Some(p)) = (&t.shadow, &path) {
        draw_shadow(pm, &format!("{label}.shadow.color"), sh, |layer, paint, tr| layer.fill_path(p, paint, FillRule::Winding, tr, None))?;
    }
    if let Some(p) = &path {
        if let Some(o) = &t.outline {
            if o.width > 0.0 {
                let oc = solid_paint(colour(&format!("{label}.outline.color"), &o.color)?);
                let stroke = Stroke { width: o.width * 2.0, line_join: LineJoin::Round, line_cap: LineCap::Round, ..Stroke::default() };
                pm.stroke_path(p, &oc, &stroke, Transform::identity(), None);
            }
        }
        pm.fill_path(p, &solid_paint(color), FillRule::Winding, Transform::identity(), None);
    }
    for (c, x, baseline) in emoji_draws {
        let Some(bm) = emoji.and_then(|e| e.bitmap(c, size)) else { continue };
        let k = size / bm.ppem;
        let Some(src) = premultiplied(&bm.img) else { continue };
        let (dx, dy) = (x + bm.x * k, baseline - (bm.y + bm.img.height() as f32) * k);
        let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
        pm.draw_pixmap(0, 0, src.as_ref(), &paint, Transform::from_row(k, 0.0, 0.0, k, dx, dy), None);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------------------------

/// Renders `spec` to an RGBA image.
pub fn render_card(spec: &CardSpec, env: &CardEnv) -> Result<CardOutput> {
    if spec.width < 8 || spec.height < 8 || spec.width > MAX_SIDE || spec.height > MAX_SIDE {
        return Err(validation(format!("width and height must be 8..={MAX_SIDE} px (got {}x{})", spec.width, spec.height)));
    }
    let mut pm = Pixmap::new(spec.width, spec.height).ok_or_else(|| validation("canvas too large"))?;
    let full = RectPx { x: 0.0, y: 0.0, w: spec.width as f32, h: spec.height as f32 };
    match &spec.background {
        Background::Transparent => {}
        Background::Solid { color } => pm.fill(sk_color(colour("background.color", color)?)),
        Background::LinearGradient { from, to, angle_deg } => {
            let paint = gradient_paint(&full, colour("background.from", from)?, colour("background.to", to)?, angle_deg.unwrap_or(90.0));
            fill_rounded(&mut pm, 0.0, 0.0, full.w, full.h, 0.0, &paint, Transform::identity());
        }
    }
    for (i, s) in spec.shapes.iter().enumerate() {
        draw_shape(&mut pm, i, s)?;
    }
    for (i, im) in spec.images.iter().enumerate() {
        draw_image(&mut pm, i, im, env)?;
    }
    let mut cx = TextCtx { fonts: Fonts { cache: HashMap::new() }, emoji: None, env, warnings: Vec::new() };
    for (i, t) in spec.texts.iter().enumerate() {
        draw_text(&mut pm, i, t, &mut cx)?;
    }
    let (w, h) = (pm.width(), pm.height());
    let image = RgbaImage::from_raw(w, h, pm.take_demultiplied()).ok_or_else(|| RustlerError::new(ErrorCode::Internal, STAGE, "pixel buffer size mismatch"))?;
    Ok(CardOutput { image, warnings: cx.warnings })
}
