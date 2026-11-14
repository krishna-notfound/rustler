//! Caption rendering to ASS subtitles (karaoke presets) and SRT export. Task 7 implements.

pub mod ass;
pub mod chunk;
pub mod presets;

pub use ass::{parse_hex_colour, write_ass, write_ass_colored, write_srt};
pub use chunk::{CaptionWord, chunk_words};
pub use presets::{PRESETS, PresetInfo, list_presets};
