//! Grouping caption words into on-screen chunks. (Task 7)

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Already in output time and already in the target script.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct CaptionWord {
    pub text: String,
    pub out_start: f64,
    pub out_end: f64,
}

/// New chunk at `max_words`, a gap > `max_gap` (typically 0.6 s), or sentence-final punctuation.
pub fn chunk_words(words: &[CaptionWord], max_words: u32, max_gap: f64) -> Vec<Vec<CaptionWord>> {
    let _ = (words, max_words, max_gap);
    todo!("Task 7: chunk_words")
}
