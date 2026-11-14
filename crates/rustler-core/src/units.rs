//! Sentence/clause units over transcript words, and quote matching.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::model::{Unit, UnitKind, Word};

/// Pause between words (seconds) that closes a sentence even without punctuation.
const PAUSE_SPLIT: f64 = 0.7;
/// Sentences longer than this many words are split into clauses.
const MAX_SENTENCE_WORDS: usize = 25;

const SENTENCE_END: &[char] = &['.', '?', '!', '।', '॥', '…'];
const CLAUSE_END: &[char] = &[',', ';', ':'];
/// Closing quotes/brackets that may follow terminal punctuation (`stop."`).
const TRAILING: &[char] = &['"', '\'', '”', '’', ')', ']', '}', '»'];

fn last_significant(text: &str) -> Option<char> {
    text.trim_end_matches(|c: char| TRAILING.contains(&c) || c.is_whitespace()).chars().next_back()
}

fn ends_sentence(w: &Word) -> bool {
    last_significant(&w.text).is_some_and(|c| SENTENCE_END.contains(&c))
}

fn ends_clause(w: &Word) -> bool {
    last_significant(&w.text).is_some_and(|c| CLAUSE_END.contains(&c))
}

/// Sentence ends on `. ? ! । …` at word end or a gap >= 0.7 s to the next word;
/// sentences > 25 words split into Clause units at the last `, ; :` before word 25
/// (else a hard split at 25). `text` = words joined by single spaces.
///
/// `boundary_strength` describes the boundary after the unit: 1.0 punctuation or end of
/// transcript, 0.8 pause only, 0.5 comma-style clause split, 0.3 hard split.
/// `word_start`/`word_end` are the `Word::i` values of the first/last word.
pub fn build_units(words: &[Word]) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    let mut start = 0usize;
    for k in 0..words.len() {
        let punct = ends_sentence(&words[k]);
        let last = k + 1 == words.len();
        let pause = !last && words[k + 1].start - words[k].end >= PAUSE_SPLIT - 1e-9;
        if punct || pause || last {
            let strength = if punct || last { 1.0 } else { 0.8 };
            push_sentence(words, start, k, strength, &mut units);
            start = k + 1;
        }
    }
    units
}

/// Emits `words[a..=b]` as one Sentence unit, or as Clause units when it is too long.
fn push_sentence(words: &[Word], a: usize, b: usize, strength: f32, units: &mut Vec<Unit>) {
    if b - a + 1 <= MAX_SENTENCE_WORDS {
        push_unit(words, a, b, UnitKind::Sentence, strength, units);
        return;
    }
    let mut rem = a;
    while b - rem + 1 > MAX_SENTENCE_WORDS {
        let window_end = rem + MAX_SENTENCE_WORDS - 1;
        match (rem..=window_end).rev().find(|&p| ends_clause(&words[p])) {
            Some(p) => {
                push_unit(words, rem, p, UnitKind::Clause, 0.5, units);
                rem = p + 1;
            }
            None => {
                push_unit(words, rem, window_end, UnitKind::Clause, 0.3, units);
                rem = window_end + 1;
            }
        }
    }
    push_unit(words, rem, b, UnitKind::Clause, strength, units);
}

fn push_unit(words: &[Word], a: usize, b: usize, kind: UnitKind, strength: f32, units: &mut Vec<Unit>) {
    let text = words[a..=b].iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" ");
    units.push(Unit {
        id: units.len() as u32,
        kind,
        word_start: words[a].i,
        word_end: words[b].i,
        start: words[a].start,
        end: words[b].end,
        text,
        boundary_strength: strength,
    });
}

/// Lowercase, drop punctuation (keeping letters, digits and Devanagari signs), collapse spaces.
fn normalize(s: &str) -> String {
    let kept: String = s
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| {
            let devanagari_sign = ('\u{0900}'..='\u{097F}').contains(&c) && c != '।' && c != '॥';
            let combining = ('\u{0300}'..='\u{036F}').contains(&c);
            if c.is_alphanumeric() || devanagari_sign || combining { c } else { ' ' }
        })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalized (lowercase, strip punctuation, collapse spaces) prefix match of the words
/// starting at `from_word` against `quote`; true when `strsim::normalized_levenshtein >= 0.8`.
/// `from_word` is a `Word::i` value (equal to the slice index for contiguous transcripts).
pub fn find_quote(words: &[Word], from_word: u32, quote: &str) -> bool {
    let q = normalize(quote);
    let n = q.split(' ').filter(|t| !t.is_empty()).count();
    if n == 0 {
        return false;
    }
    let pos = match words.get(from_word as usize) {
        Some(w) if w.i == from_word => from_word as usize,
        _ => match words.iter().position(|w| w.i == from_word) {
            Some(p) => p,
            None => return false,
        },
    };
    let mut tokens: Vec<String> = Vec::new();
    for w in &words[pos..] {
        for t in normalize(&w.text).split(' ').filter(|t| !t.is_empty()) {
            tokens.push(t.to_string());
        }
        if tokens.len() >= n {
            break;
        }
    }
    if tokens.is_empty() {
        return false;
    }
    tokens.truncate(n);
    strsim::normalized_levenshtein(&tokens.join(" "), &q) >= 0.8
}
