//! Sentence/clause units over transcript words, and quote matching.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::model::{Unit, Word};

/// Sentence ends on `. ? ! । …` at word end or a gap >= 0.7 s to the next word;
/// sentences > 25 words split into Clause units at the last `, ; :` before word 25
/// (else a hard split at 25). `text` = words joined by single spaces.
pub fn build_units(words: &[Word]) -> Vec<Unit> {
    let _ = words;
    todo!("Task 3: build_units")
}

/// Normalized (lowercase, strip punctuation, collapse spaces) prefix match of the words
/// starting at `from_word` against `quote`; true when `strsim::normalized_levenshtein >= 0.8`.
pub fn find_quote(words: &[Word], from_word: u32, quote: &str) -> bool {
    let _ = (words, from_word, quote);
    todo!("Task 3: find_quote")
}
