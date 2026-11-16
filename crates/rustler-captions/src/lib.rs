//! Caption rendering to ASS subtitles (karaoke presets) and SRT export. Task 7 implements.

pub mod ass;
pub mod chunk;
pub mod custom;
pub mod presets;

pub use ass::{
    WordPaint, parse_hex_colour, write_ass, write_ass_colored, write_ass_colored_with, write_ass_painted_with, write_ass_with,
    write_srt,
};
pub use custom::{ActiveKind, CustomStyle, find_preset_in, list_presets_with, load_custom_presets};
pub use chunk::{CaptionWord, chunk_words};
pub use presets::{PRESETS, Preset, PresetInfo, list_presets};
