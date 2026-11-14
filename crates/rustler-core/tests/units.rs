//! Task 3 units / find_quote tests.

use rustler_core::model::*;
use rustler_core::units::{build_units, find_quote};

/// Words 0.3 s long with 0.1 s gaps, unless `gaps` overrides the gap after word i.
fn words(texts: &[&str], gaps: &[(usize, f64)]) -> Vec<Word> {
    let mut t = 0.0;
    texts
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let w = Word {
                i: i as u32,
                text: s.to_string(),
                start: t,
                end: t + 0.3,
                conf: None,
                speaker: None,
            };
            let gap = gaps.iter().find(|(k, _)| *k == i).map(|(_, g)| *g).unwrap_or(0.1);
            t += 0.3 + gap;
            w
        })
        .collect()
}

#[test]
fn two_sentences() {
    let w = words(&["hello", "there.", "how", "are", "you?"], &[]);
    let u = build_units(&w);
    assert_eq!(u.len(), 2);
    assert_eq!(u[0].id, 0);
    assert_eq!(u[0].kind, UnitKind::Sentence);
    assert_eq!((u[0].word_start, u[0].word_end), (0, 1));
    assert_eq!(u[0].boundary_strength, 1.0);
    assert_eq!(u[0].text, "hello there.");
    assert_eq!((u[0].start, u[0].end), (w[0].start, w[1].end));
    assert_eq!(u[1].id, 1);
    assert_eq!((u[1].word_start, u[1].word_end), (2, 4));
    assert_eq!(u[1].text, "how are you?");
    assert_eq!(u[1].boundary_strength, 1.0);
}

#[test]
fn pause_splits_sentence() {
    let w = words(&["so", "we", "went", "home", "and", "slept"], &[(3, 0.8)]);
    let u = build_units(&w);
    assert_eq!(u.len(), 2);
    assert_eq!(u[0].word_end, 3);
    assert_eq!(u[0].boundary_strength, 0.8);
    assert_eq!(u[1].word_start, 4);
    // A short gap does not split.
    let w = words(&["so", "we", "went", "home"], &[(1, 0.5)]);
    assert_eq!(build_units(&w).len(), 1);
}

#[test]
fn long_sentence_splits_at_comma() {
    let mut texts: Vec<String> = (0..30).map(|i| format!("w{i}")).collect();
    texts[14] = "w14,".into();
    texts[29] = "w29.".into();
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let u = build_units(&words(&refs, &[]));
    assert_eq!(u.len(), 2);
    assert!(u.iter().all(|x| x.kind == UnitKind::Clause));
    assert_eq!((u[0].word_start, u[0].word_end), (0, 14));
    assert_eq!(u[0].boundary_strength, 0.5);
    assert_eq!((u[1].word_start, u[1].word_end), (15, 29));
    assert_eq!(u[1].boundary_strength, 1.0);
}

#[test]
fn long_sentence_without_comma_hard_splits_at_25() {
    let texts: Vec<String> = (0..30).map(|i| format!("w{i}")).collect();
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let u = build_units(&words(&refs, &[]));
    // One 30-word run (end of transcript closes it): 25 + 5.
    assert_eq!(u.len(), 2);
    assert_eq!((u[0].word_start, u[0].word_end), (0, 24));
    assert_eq!((u[1].word_start, u[1].word_end), (25, 29));
    assert!(u.iter().all(|x| x.kind == UnitKind::Clause));
}

#[test]
fn exactly_25_words_stays_sentence() {
    let texts: Vec<String> = (0..25).map(|i| format!("w{i}")).collect();
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let u = build_units(&words(&refs, &[]));
    assert_eq!(u.len(), 1);
    assert_eq!(u[0].kind, UnitKind::Sentence);
}

#[test]
fn devanagari_danda_ends_sentence() {
    let w = words(&["मैं", "घर", "जा", "रहा", "हूँ।", "तुम", "कहाँ", "हो"], &[]);
    let u = build_units(&w);
    assert_eq!(u.len(), 2);
    assert_eq!(u[0].word_end, 4);
    assert_eq!(u[0].boundary_strength, 1.0);
    assert_eq!(u[1].word_start, 5);
}

#[test]
fn ellipsis_and_trailing_quote_end_sentence() {
    let w = words(&["wait…", "no", "he said \"stop.\"", "ok"], &[]);
    let u = build_units(&w);
    assert_eq!(u.len(), 3);
    assert_eq!(u[0].word_end, 0);
    assert_eq!(u[1].word_end, 2);
}

#[test]
fn empty_is_empty() {
    assert!(build_units(&[]).is_empty());
}

fn kennedy() -> Vec<Word> {
    words(
        &["And", "so", "my", "fellow", "Americans,", "ask", "not", "what", "your", "country", "can", "do", "for", "you"],
        &[],
    )
}

#[test]
fn find_quote_matches_from_index() {
    let w = kennedy();
    assert!(find_quote(&w, 1, "So my fellow Americans"));
    assert!(find_quote(&w, 1, "so, my fellow americans!"));
    assert!(!find_quote(&w, 5, "So my fellow Americans"));
    assert!(find_quote(&w, 5, "ask not what your country"));
}

#[test]
fn find_quote_edge_cases() {
    let w = kennedy();
    assert!(!find_quote(&w, 99, "anything"));
    assert!(!find_quote(&w, 0, ""));
    assert!(!find_quote(&w, 0, "  ...  "));
    // Fuzzy: one dropped letter still matches (>= 0.8).
    assert!(find_quote(&w, 1, "so my fellow americns"));
}
