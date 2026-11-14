//! Caption rendering to ASS subtitles (karaoke presets). Task 7 implements.

pub mod ass;
pub mod chunk;
pub mod presets;

pub use ass::write_ass;
pub use chunk::{CaptionWord, chunk_words};
pub use presets::PRESETS;
