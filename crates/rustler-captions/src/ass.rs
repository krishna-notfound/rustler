//! ASS (Advanced SubStation Alpha) writer. (Task 7)

use crate::chunk::CaptionWord;
use rustler_core::{Canvas, CaptionTrack};

/// Full `.ass` file contents; `Validation` for an unknown `track.preset`.
pub fn write_ass(
    words: &[CaptionWord],
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
) -> rustler_core::Result<String> {
    let _ = (words, track, canvas, font_family);
    todo!("Task 7: write_ass")
}
