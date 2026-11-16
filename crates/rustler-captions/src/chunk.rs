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

fn ends_sentence(text: &str) -> bool {
    let t = text.trim_end_matches(['"', '\'', ')', ']', '”', '’']);
    t.ends_with(['.', '?', '!', '…', '。', '।', '？', '！'])
}

/// A token with nothing to read: YouTube caption speaker-change markers (`>>`) and dashes come
/// through as separate words with their own timing and would otherwise be burned in as `>> >>`.
fn is_marker(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || t.chars().all(|c| c.is_ascii_punctuation())
}

/// New chunk at `max_words`, a gap > `max_gap` (typically 0.6 s), or sentence-final punctuation.
/// Marker tokens with no readable characters (`>>`, `-`) are left out.
pub fn chunk_words(words: &[CaptionWord], max_words: u32, max_gap: f64) -> Vec<Vec<CaptionWord>> {
    let max_words = max_words.max(1) as usize;
    let mut out: Vec<Vec<CaptionWord>> = Vec::new();
    let mut cur: Vec<CaptionWord> = Vec::new();
    for w in words.iter().filter(|w| !is_marker(&w.text)) {
        if let Some(prev) = cur.last() {
            let split = cur.len() >= max_words
                || w.out_start - prev.out_end > max_gap
                || ends_sentence(&prev.text);
            if split {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.push(w.clone());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
