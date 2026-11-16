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

/// A token that continues the word before it: whisper splits some hyphenated words into two
/// tokens ("chop" + "-up,"), and a section cut mid-word starts on such a tail.
pub fn is_continuation(text: &str) -> bool {
    let mut c = text.trim().chars();
    matches!(c.next(), Some('-' | '–' | '—')) && c.next().is_some_and(char::is_alphabetic)
}

/// Fixes word fragments before chunking: a continuation token ([`is_continuation`]) is joined
/// onto the word before it when they are at most `join_gap` seconds apart and that word does not
/// end a sentence ("chop" + "-up," -> "chop-up,", so a chunk never starts with "-UP,"); a
/// fragment at the very start (nothing to join it to: the clip starts mid-word) is dropped; any
/// other loose fragment loses its leading dash.
pub fn join_fragments(words: Vec<CaptionWord>, join_gap: f64) -> Vec<CaptionWord> {
    let mut out: Vec<CaptionWord> = Vec::with_capacity(words.len());
    for w in words {
        if !is_continuation(&w.text) {
            out.push(w);
            continue;
        }
        let tail = w.text.trim().trim_start_matches(['-', '–', '—']).to_string();
        match out.last_mut() {
            None => {} // leading fragment of a word cut by the clip start: dropped
            Some(prev) if w.out_start - prev.out_end <= join_gap && !ends_sentence(&prev.text) => {
                prev.text = format!("{}-{tail}", prev.text.trim_end_matches(['-', '–', '—']));
                prev.out_end = prev.out_end.max(w.out_end);
            }
            Some(_) => out.push(CaptionWord { text: tail, ..w }),
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cw(text: &str, s: f64, e: f64) -> CaptionWord {
        CaptionWord { text: text.into(), out_start: s, out_end: e }
    }

    #[test]
    fn hyphen_tail_joins_the_word_before_it() {
        // Trial 3: whisper wrote "chop" + "-up,", and a chunk started with "-UP, BUT MOST".
        let w = vec![cw("launch", 0.0, 0.2), cw("a", 0.2, 0.3), cw("chop", 0.3, 0.45), cw("-up,", 0.45, 0.55), cw("but", 0.7, 0.8), cw("most", 0.8, 1.0)];
        let j = join_fragments(w, 0.3);
        let texts: Vec<&str> = j.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["launch", "a", "chop-up,", "but", "most"]);
        assert!((j[2].out_end - 0.55).abs() < 1e-9);
        let chunks = chunk_words(&j, 3, 0.6);
        assert!(chunks.iter().all(|c| !c[0].text.starts_with('-')), "{chunks:?}");
    }

    #[test]
    fn leading_fragment_is_dropped_and_loose_dashes_stripped() {
        let j = join_fragments(vec![cw("-up,", 0.0, 0.1), cw("but", 0.2, 0.3), cw("most", 0.3, 0.5)], 0.3);
        assert_eq!(j.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["but", "most"]);
        // After a sentence end the fragment stays, without its dash.
        let j = join_fragments(vec![cw("done.", 0.0, 0.2), cw("-up", 0.25, 0.4)], 0.3);
        assert_eq!(j[1].text, "up");
        // A plain dash marker, a negative number or a whole hyphenated word is not a fragment.
        assert!(!is_continuation("-") && !is_continuation("—") && !is_continuation("-5") && !is_continuation("well-known"));
        assert!(is_continuation("-up,") && is_continuation("–like"));
    }
}
