//! Custom caption presets: JSON files saved by the asset library (`caption.create_style`) and
//! loaded into [`Preset`]s that `write_ass_with` accepts next to the built-in table.

use crate::presets::{ActiveStyle, Preset, PresetInfo, find_preset, list_presets};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the active word is emphasised, as stored in a custom style file.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ActiveKind {
    Recolor,
    Box,
    Pop,
    Static,
}

impl From<ActiveKind> for ActiveStyle {
    fn from(k: ActiveKind) -> Self {
        match k {
            ActiveKind::Recolor => ActiveStyle::Recolor,
            ActiveKind::Box => ActiveStyle::Box,
            ActiveKind::Pop => ActiveStyle::Pop,
            ActiveKind::Static => ActiveStyle::Static,
        }
    }
}

impl From<ActiveStyle> for ActiveKind {
    fn from(k: ActiveStyle) -> Self {
        match k {
            ActiveStyle::Recolor => ActiveKind::Recolor,
            ActiveStyle::Box => ActiveKind::Box,
            ActiveStyle::Pop => ActiveKind::Pop,
            ActiveStyle::Static => ActiveKind::Static,
        }
    }
}

/// A fully resolved custom caption style as stored on disk (`caption_styles/<name>.json`).
/// Colours are `#RRGGBB` (opaque) or `#RRGGBBAA` (CSS order, `AA` = opacity).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct CustomStyle {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Built-in preset this style started from (informational).
    #[serde(default)]
    pub base: Option<String>,
    pub font_family: String,
    pub size_factor: f64,
    pub primary: String,
    pub outline_colour: String,
    pub back_colour: String,
    pub outline: f64,
    pub shadow: f64,
    #[serde(default)]
    pub blur: f64,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub uppercase: bool,
    pub active_kind: ActiveKind,
    /// Active-word colour (the box colour for `active_kind = box`).
    pub active_colour: String,
    #[serde(default)]
    pub box_padding: f64,
    /// Size factor for words that carry their own colour (keywords); 0 = off.
    #[serde(default)]
    pub keyword_scale: f64,
}

/// `#RRGGBB` / `#RRGGBBAA` (leading `#` optional) to ASS `0xAABBGGRR`.
pub fn parse_hex_colour(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if !(h.len() == 6 || h.len() == 8) || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let (rgb, opacity) = if h.len() == 8 { (v >> 8, v & 0xFF) } else { (v, 0xFF) };
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    Some(((0xFF - opacity) << 24) | (b << 16) | (g << 8) | r)
}

/// ASS `0xAABBGGRR` to `#RRGGBB` (opaque) or `#RRGGBBAA`.
pub fn hex_colour(c: u32) -> String {
    let (a, b, g, r) = ((c >> 24) & 0xFF, (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    if a == 0 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{:02X}", 0xFF - a) }
}

/// Process-wide interning so custom presets can carry `&'static str` like the built-in table.
/// The set is tiny (one entry per distinct style name / font family / description).
fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static POOL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut pool = POOL.get_or_init(|| Mutex::new(HashSet::new())).lock().unwrap_or_else(|p| p.into_inner());
    if let Some(v) = pool.get(s) {
        return v;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    pool.insert(leaked);
    leaked
}

impl CustomStyle {
    /// Resolves into a [`Preset`]; `Err` names the first invalid field.
    pub fn to_preset(&self) -> Result<Preset, String> {
        let col = |label: &str, v: &str| {
            parse_hex_colour(v).ok_or_else(|| format!("{label} '{v}' is not #RRGGBB or #RRGGBBAA"))
        };
        if self.name.trim().is_empty() {
            return Err("name is empty".into());
        }
        if self.font_family.trim().is_empty() {
            return Err("font_family is empty".into());
        }
        if !(0.02..=0.15).contains(&self.size_factor) {
            return Err(format!("size_factor {} must be between 0.02 and 0.15", self.size_factor));
        }
        for (label, v, max) in [
            ("outline", self.outline, 20.0),
            ("shadow", self.shadow, 20.0),
            ("blur", self.blur, 10.0),
            ("box_padding", self.box_padding, 60.0),
            ("keyword_scale", self.keyword_scale, 3.0),
        ] {
            if !(0.0..=max).contains(&v) {
                return Err(format!("{label} {v} must be between 0 and {max}"));
            }
        }
        let desc = if self.description.is_empty() { "Custom caption style." } else { &self.description };
        Ok(Preset {
            name: intern(self.name.trim()),
            description: intern(desc),
            font_family: intern(self.font_family.trim()),
            size_factor: self.size_factor,
            primary: col("primary", &self.primary)?,
            outline_colour: col("outline_colour", &self.outline_colour)?,
            back_colour: col("back_colour", &self.back_colour)?,
            outline: self.outline,
            shadow: self.shadow,
            blur: self.blur,
            bold: self.bold,
            uppercase: self.uppercase,
            active_colour: col("active_colour", &self.active_colour)?,
            box_padding: self.box_padding,
            active: self.active_kind.into(),
            keyword_scale: self.keyword_scale,
        })
    }

    /// The style a built-in preset corresponds to (starting point for custom styles).
    pub fn from_preset(p: &Preset) -> Self {
        Self {
            name: p.name.to_string(),
            description: p.description.to_string(),
            base: Some(p.name.to_string()),
            font_family: p.font_family.to_string(),
            size_factor: p.size_factor,
            primary: hex_colour(p.primary),
            outline_colour: hex_colour(p.outline_colour),
            back_colour: hex_colour(p.back_colour),
            outline: p.outline,
            shadow: p.shadow,
            blur: p.blur,
            bold: p.bold,
            uppercase: p.uppercase,
            active_kind: p.active.into(),
            active_colour: hex_colour(p.active_colour),
            box_padding: p.box_padding,
            keyword_scale: p.keyword_scale,
        }
    }
}

/// Loads every `*.json` custom style in `dir` (sorted by file name). Files that do not parse, are
/// invalid or reuse a built-in name are skipped. A missing directory gives an empty list.
pub fn load_custom_presets(dir: &std::path::Path) -> Vec<Preset> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| serde_json::from_slice::<CustomStyle>(&std::fs::read(p).ok()?).ok())
        .filter_map(|s| s.to_preset().ok())
        .filter(|p| find_preset(p.name).is_none())
        .collect()
}

/// Built-in first, then the first custom preset called `name` (callers list project styles before
/// global ones so the project wins).
pub fn find_preset_in<'a>(name: &str, custom: &'a [Preset]) -> Option<&'a Preset> {
    find_preset(name).or_else(|| custom.iter().find(|p| p.name == name))
}

/// Built-in presets followed by `custom` (duplicate names dropped).
pub fn list_presets_with(custom: &[Preset]) -> Vec<PresetInfo> {
    let mut out = list_presets();
    for p in custom {
        if !out.iter().any(|i| i.name == p.name) {
            out.push(PresetInfo {
                name: p.name.to_string(),
                description: p.description.to_string(),
                font_family: p.font_family.to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_round_trip() {
        assert_eq!(parse_hex_colour("#FFE000"), Some(0x0000_E0FF));
        assert_eq!(hex_colour(0x0000_E0FF), "#FFE000");
        // 50 % black shadow: ASS alpha 0x80
        assert_eq!(parse_hex_colour("#00000080"), Some(0x7F00_0000));
        assert_eq!(hex_colour(0x8000_0000), "#0000007F");
        assert_eq!(parse_hex_colour("nope"), None);
    }

    #[test]
    fn builtin_round_trips_through_custom_style() {
        for p in crate::presets::PRESET_TABLE {
            let back = CustomStyle::from_preset(p).to_preset().unwrap();
            assert_eq!(back.primary, p.primary, "{}", p.name);
            assert_eq!(back.active_colour, p.active_colour);
            assert_eq!(back.active, p.active);
        }
    }

    #[test]
    fn loads_valid_skips_invalid_and_builtin_names() {
        let d = tempfile::tempdir().unwrap();
        let mut ok = CustomStyle::from_preset(&crate::presets::PRESET_TABLE[0]);
        ok.name = "my-style".into();
        std::fs::write(d.path().join("a.json"), serde_json::to_vec(&ok).unwrap()).unwrap();
        ok.name = "bold-karaoke".into();
        std::fs::write(d.path().join("b.json"), serde_json::to_vec(&ok).unwrap()).unwrap();
        std::fs::write(d.path().join("c.json"), b"{not json").unwrap();
        let got = load_custom_presets(d.path());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "my-style");
        assert!(load_custom_presets(&d.path().join("missing")).is_empty());
    }
}
