//! Intro-trailer detection (f39): long-form podcasts often open with a 1-2 minute montage of
//! their best lines (plus title cards) before the episode starts. Ranking a moment inside that
//! montage gives a clip cut from the trailer copy (burned-in titles, jump cuts, no context)
//! instead of the full original later in the episode.
//!
//! Detection is cheap and deterministic: every 5-word phrase spoken in the first
//! [`INTRO_SCAN`] seconds is looked up later in the transcript. A *repeat* is a run of at least
//! [`MIN_RUN_GRAMS`] consecutive phrases that all reappear at the same later offset (a whole
//! line said again, not a common phrase like "you know I was like"). When the repeats cover
//! enough speech, the cluster of repeats that starts the source is the trailer. Scene-cut
//! density in that range is reported as evidence (it is not needed to detect it).

use crate::highlights_cmds::tok;
use rustler_core::Word;
use schemars::JsonSchema;
use serde::Serialize;

/// Seconds at the start of a source searched for a trailer.
pub const INTRO_SCAN: f64 = 180.0;
/// Shortest source (seconds) that is checked: a short source has no intro montage.
pub const MIN_SOURCE: f64 = 360.0;
/// Phrase length (tokens) looked up later in the transcript.
const N: usize = 5;
/// The later copy must start at least this many seconds after the early one.
const MIN_LATER_GAP: f64 = 90.0;
/// Consecutive matching phrases (same offset) needed for a repeat (5 + 2 = 7 words in a row).
const MIN_RUN_GRAMS: usize = 3;
/// Offsets of consecutive phrases agree within this many seconds (caption timing jitter).
const OFFSET_TOL: f64 = 6.0;
/// Repeats further apart than this are separate clusters.
const CLUSTER_GAP: f64 = 30.0;
/// The first cluster must start within this many seconds of the source start.
const CLUSTER_START: f64 = 60.0;
/// A trailer needs this many seconds of repeated speech or at least `MIN_RUNS` repeats.
const MIN_REPEATED_S: f64 = 15.0;
const MIN_RUNS: usize = 3;

/// One line of the trailer found again later in the source.
#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Repeat {
    /// The trailer copy (source seconds).
    pub early_start: f64,
    pub early_end: f64,
    /// The original, later in the episode (source seconds).
    pub later_start: f64,
    pub later_end: f64,
    /// The first words of the repeated line.
    pub text: String,
}

/// An intro montage at the start of the source.
#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct IntroTrailer {
    /// Source seconds of the montage (windows overlapping it are not ranked).
    pub start: f64,
    pub end: f64,
    /// Seconds of trailer speech found again later.
    pub repeated_s: f64,
    pub repeats: Vec<Repeat>,
    /// Scene cuts per minute inside the montage and in the rest of the source (when scenes.json exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cuts_per_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cuts_per_min_rest: Option<f64>,
    /// How it was found, in words.
    pub evidence: String,
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Detects the intro trailer of a source from its words (and scene starts, as evidence only).
/// `None` for short sources, sources without a repeated opening, or too little repeated speech.
pub fn detect(words: &[Word], duration: f64, scene_starts: Option<&[f64]>) -> Option<IntroTrailer> {
    if duration < MIN_SOURCE {
        return None;
    }
    // Positions (into `words`) of words with a readable token.
    let idx: Vec<usize> = (0..words.len()).filter(|&k| !tok(&words[k].text).is_empty()).collect();
    if idx.len() < N + MIN_RUN_GRAMS {
        return None;
    }
    let toks: Vec<String> = idx.iter().map(|&k| tok(&words[k].text)).collect();
    let key = |a: usize| toks[a..a + N].join(" ");
    let mut grams: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for a in 0..=toks.len() - N {
        grams.entry(key(a)).or_default().push(a);
    }
    let t = |a: usize| words[idx[a]].start;
    let te = |a: usize| words[idx[a]].end;

    // Runs of consecutive early phrases whose later copies keep the same offset.
    struct Run {
        a0: usize,
        a1: usize,
        b0: usize,
        b1: usize,
        offset: f64,
        grams: usize,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut cur: Option<Run> = None;
    for a in 0..=toks.len() - N {
        if t(a) >= INTRO_SCAN {
            break;
        }
        let later: Vec<usize> = grams[&key(a)].iter().copied().filter(|&b| t(b) >= t(a) + MIN_LATER_GAP).collect();
        // Prefer the copy that continues the current run.
        let cont = cur.as_ref().and_then(|r| later.iter().copied().find(|&b| b > r.b1 && b <= r.b1 + 3 && (t(b) - t(a) - r.offset).abs() <= OFFSET_TOL));
        match (cont, cur.as_mut()) {
            (Some(b), Some(r)) if a <= r.a1 + 3 => {
                r.a1 = a;
                r.b1 = b;
                r.grams += 1;
            }
            _ => {
                if let Some(r) = cur.take() {
                    runs.push(r);
                }
                // The later copy whose following words agree longest (a common phrase can occur
                // many times; the real original keeps matching).
                let agree = |b: usize| (0..8).take_while(|&k| b + k + N <= toks.len() && a + k + N <= toks.len() && toks[a + k..a + k + N] == toks[b + k..b + k + N]).count();
                if let Some(b) = later.iter().copied().max_by_key(|&b| (agree(b), std::cmp::Reverse(b))) {
                    cur = Some(Run { a0: a, a1: a, b0: b, b1: b, offset: t(b) - t(a), grams: 1 });
                }
            }
        }
    }
    runs.extend(cur);
    let repeats: Vec<Repeat> = runs
        .into_iter()
        .filter(|r| r.grams >= MIN_RUN_GRAMS)
        .map(|r| Repeat {
            early_start: r2(t(r.a0)),
            early_end: r2(te(r.a1 + N - 1)),
            later_start: r2(t(r.b0)),
            later_end: r2(te(r.b1 + N - 1)),
            text: idx[r.a0..(r.a0 + 12).min(r.a1 + N)].iter().map(|&k| words[k].text.as_str()).collect::<Vec<_>>().join(" "),
        })
        .collect();
    // The first cluster of repeats, starting near the beginning of the source.
    let first = repeats.first()?;
    if first.early_start > CLUSTER_START {
        return None;
    }
    let mut cluster: Vec<Repeat> = vec![first.clone()];
    for r in &repeats[1..] {
        if r.early_start - cluster.last().map_or(0.0, |c| c.early_end) > CLUSTER_GAP {
            break;
        }
        cluster.push(r.clone());
    }
    let repeated_s: f64 = cluster.iter().map(|r| r.early_end - r.early_start).sum();
    if repeated_s < MIN_REPEATED_S && cluster.len() < MIN_RUNS {
        return None;
    }
    let start = if first.early_start <= 20.0 { 0.0 } else { first.early_start };
    let end = cluster.last().map_or(0.0, |r| r.early_end);
    // Evidence: scene cuts per minute in the montage vs the rest.
    let (cpm, cpm_rest) = match scene_starts {
        Some(s) if end - start > 1.0 && duration - end > 1.0 => {
            let inside = s.iter().filter(|&&x| x > start && x < end).count() as f64;
            let rest = s.iter().filter(|&&x| x >= end).count() as f64;
            (Some(r2(inside * 60.0 / (end - start))), Some(r2(rest * 60.0 / (duration - end))))
        }
        _ => (None, None),
    };
    let mut evidence = format!(
        "{} lines ({repeated_s:.0} s of speech) in the first {:.0} s are said again later in the source",
        cluster.len(),
        end
    );
    if let (Some(a), Some(b)) = (cpm, cpm_rest) {
        evidence.push_str(&format!("; {a:.1} scene cuts/min there vs {b:.1} in the rest"));
    }
    Some(IntroTrailer { start: r2(start), end: r2(end), repeated_s: r2(repeated_s), repeats: cluster, cuts_per_min: cpm, cuts_per_min_rest: cpm_rest, evidence })
}

/// The repeat whose later (original) copy lies mostly inside `start..end`, if any.
pub fn original_inside<'a>(t: &'a IntroTrailer, start: f64, end: f64) -> Option<&'a Repeat> {
    t.repeats.iter().find(|r| {
        let ov = (r.later_end.min(end) - r.later_start.max(start)).max(0.0);
        ov >= 0.5 * (r.later_end - r.later_start)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_from(spans: &[(f64, &str)]) -> Vec<Word> {
        let mut out = Vec::new();
        for (t0, text) in spans {
            for (j, w) in text.split_whitespace().enumerate() {
                let s = t0 + j as f64 * 0.3;
                out.push(Word { i: out.len() as u32, text: w.into(), start: s, end: s + 0.25, conf: None, speaker: None });
            }
        }
        out
    }

    const L1: &str = "Was there any point ever in your life where nobody else was convinced but just you?";
    const L2: &str = "Launching the product was kind of an example of that and most people thought it was too early.";
    const L3: &str = "There's no way that anyone was going to build super intelligence and not have a crazy life.";

    #[test]
    fn finds_the_opening_montage_and_the_originals() {
        let words = words_from(&[
            (2.0, L1),
            (8.0, L2),
            (15.0, L3),
            (25.0, "Before we start today's episode please subscribe to the channel."),
            (40.0, "Welcome to the show my guest today runs a large company in the valley."),
            (300.0, "filler talk about something else entirely for a while now."),
            (900.0, L2),
            (1500.0, L1),
            (2400.0, L3),
        ]);
        let tr = detect(&words, 3600.0, Some(&[1.0, 4.0, 7.0, 9.0, 12.0, 14.0, 600.0, 1200.0])).expect("trailer");
        assert_eq!(tr.start, 0.0);
        assert!(tr.end > 15.0 && tr.end < 25.0, "{tr:?}");
        assert_eq!(tr.repeats.len(), 3);
        assert!((tr.repeats[1].later_start - 900.0).abs() < 1e-6);
        assert!(tr.cuts_per_min.unwrap() > tr.cuts_per_min_rest.unwrap());
        assert!(original_inside(&tr, 1490.0, 1520.0).is_some());
        assert!(original_inside(&tr, 300.0, 330.0).is_none());
    }

    #[test]
    fn common_phrases_and_short_sources_are_not_a_trailer() {
        // Only a common 5-word phrase repeats ("you know i was like"): no run of 7 words.
        let words = words_from(&[
            (2.0, "honestly you know I was like amazed by it"),
            (400.0, "and then you know I was like done with it"),
        ]);
        assert!(detect(&words, 3600.0, None).is_none());
        // A real repeat, but the source is too short to have an intro montage.
        let words = words_from(&[(2.0, L1), (8.0, L2), (15.0, L3), (200.0, L1), (250.0, L2), (300.0, L3)]);
        assert!(detect(&words, 320.0, None).is_none());
        // Repeats that start late are not an intro.
        let words = words_from(&[(100.0, L1), (110.0, L2), (120.0, L3), (1000.0, L1), (1100.0, L2), (1200.0, L3)]);
        assert!(detect(&words, 3600.0, None).is_none());
    }
}
