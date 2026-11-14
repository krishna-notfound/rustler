//! Caption style presets (Task 7): a data table, no per-preset code beyond [`ActiveStyle`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Names of the built-in presets (same order as [`PRESET_TABLE`]).
pub const PRESETS: &[&str] = &["bold-karaoke", "clean-white", "boxed-highlight", "hormozi-pop"];

/// How the currently spoken word is emphasised.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActiveStyle {
    /// Active word gets another colour (same layer, no size change).
    Recolor,
    /// Active word gets an opaque coloured box behind it (box on layer 0, text on layer 1).
    Box,
    /// Active word gets another colour plus a short scale pop on its own layer (layer 1).
    Pop,
}

/// One preset. Colours are `0xAABBGGRR` (ASS order: alpha, blue, green, red; alpha 0 = opaque).
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub font_family: &'static str,
    /// Font size = round(canvas.width * size_factor).
    pub size_factor: f64,
    pub primary: u32,
    pub outline_colour: u32,
    /// Shadow (`BackColour`) colour.
    pub back_colour: u32,
    pub outline: f64,
    pub shadow: f64,
    /// `\blur` edge softening applied to every event (0 = none).
    pub blur: f64,
    pub bold: bool,
    pub uppercase: bool,
    /// Active-word colour (for [`ActiveStyle::Box`]: the box colour).
    pub active_colour: u32,
    /// Box padding in px (only used by [`ActiveStyle::Box`]).
    pub box_padding: f64,
    pub active: ActiveStyle,
}

pub const PRESET_TABLE: &[Preset] = &[
    Preset {
        name: "bold-karaoke",
        description: "Anton, uppercase white text with a thick black outline; the spoken word turns yellow.",
        font_family: "Anton",
        size_factor: 0.072,
        primary: 0x00FF_FFFF,
        outline_colour: 0x0000_0000,
        back_colour: 0x0000_0000,
        outline: 6.0,
        shadow: 0.0,
        blur: 0.0,
        bold: false,
        uppercase: true,
        active_colour: 0x0000_E0FF, // #FFE000
        box_padding: 0.0,
        active: ActiveStyle::Recolor,
    },
    Preset {
        name: "clean-white",
        description: "Montserrat ExtraBold white text, soft black outline and shadow; the spoken word turns gold.",
        font_family: "Montserrat ExtraBold",
        size_factor: 0.060,
        primary: 0x00FF_FFFF,
        outline_colour: 0x0000_0000,
        back_colour: 0x8000_0000,
        outline: 4.0,
        shadow: 2.0,
        blur: 1.5,
        bold: false,
        uppercase: false,
        active_colour: 0x0000_D7FF, // #FFD700
        box_padding: 0.0,
        active: ActiveStyle::Recolor,
    },
    Preset {
        name: "boxed-highlight",
        description: "Montserrat ExtraBold white text; the spoken word sits on an opaque violet box.",
        font_family: "Montserrat ExtraBold",
        size_factor: 0.058,
        primary: 0x00FF_FFFF,
        outline_colour: 0x0000_0000,
        back_colour: 0x0000_0000,
        outline: 3.0,
        shadow: 0.0,
        blur: 0.0,
        bold: false,
        uppercase: false,
        active_colour: 0x00FF_2C7B, // box #7B2CFF
        box_padding: 12.0,
        active: ActiveStyle::Box,
    },
    Preset {
        name: "hormozi-pop",
        description: "Anton, uppercase white text; the spoken word turns green with a brief scale pop.",
        font_family: "Anton",
        size_factor: 0.076,
        primary: 0x00FF_FFFF,
        outline_colour: 0x0000_0000,
        back_colour: 0x0000_0000,
        outline: 6.0,
        shadow: 0.0,
        blur: 0.0,
        bold: false,
        uppercase: true,
        active_colour: 0x005E_FF2B, // #2BFF5E
        box_padding: 0.0,
        active: ActiveStyle::Pop,
    },
];

/// Look a preset up by name.
pub fn find_preset(name: &str) -> Option<&'static Preset> {
    PRESET_TABLE.iter().find(|p| p.name == name)
}

/// Public description of a preset (for `caption.list_styles`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct PresetInfo {
    pub name: String,
    pub description: String,
    pub font_family: String,
}

/// All presets, in table order.
pub fn list_presets() -> Vec<PresetInfo> {
    PRESET_TABLE
        .iter()
        .map(|p| PresetInfo {
            name: p.name.to_string(),
            description: p.description.to_string(),
            font_family: p.font_family.to_string(),
        })
        .collect()
}
