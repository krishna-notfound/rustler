//! Caption rendering to ASS subtitles (karaoke presets) and SRT export. Task 7 implements.

pub mod ass;
pub mod chunk;
pub mod custom;
pub mod emoji;
pub mod presets;

pub use ass::{
    PositionSpan, WordLooks, WordPaint, parse_hex_colour, position_at, write_ass, write_ass_colored, write_ass_colored_with,
    write_ass_painted_with, write_ass_placed, write_ass_with, write_srt,
};
pub use custom::{ActiveKind, CustomStyle, find_preset_in, list_presets_with, load_custom_presets};
pub use chunk::{CaptionWord, chunk_words, chunk_words_split, is_continuation, join_fragments};
pub use presets::{PRESETS, Preset, PresetInfo, list_presets};
