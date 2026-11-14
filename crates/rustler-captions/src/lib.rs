//! Caption rendering to ASS subtitles (karaoke presets) and SRT export. Task 7 implements.

pub mod ass;
pub mod chunk;
pub mod presets;

pub use ass::{write_ass, write_srt};
pub use chunk::{CaptionWord, chunk_words};
pub use presets::{PRESETS, PresetInfo, list_presets};
