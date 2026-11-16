//! Hook-title and cold-open suggestions for `clips.auto` (f39). Deterministic, from the words
//! the clip plays: 2-3 short hook lines (<= 7 words, plain text) from the opening question and the
//! strongest lines, each with the exact `overlay add-text ... --style hook-big` command, and an
//! `edit cold-open` suggestion when the clip opens on a question. Suggestions only: `clips.auto`
//! never adds text or re-orders a clip (the agent writes the hook and decides).

use crate::highlights_cmds::{EMPHASIS_WORDS, tok, weak_opener_len};

/// Longest suggested hook line (words).
pub const MAX_HOOK_WORDS: usize = 7;
/// The opening sentence must start this close to the clip start (seconds) to count as the opener.
const OPENING_WITHIN: f64 = 3.0;
/// A cold-open line lasts between these many seconds.
const COLD_MIN: f64 = 2.0;
const COLD_MAX: f64 = 9.0;

/// A word the clip plays (source seconds), in play order.
#[derive(Clone, Debug)]
pub struct SWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// One suggestion (`action`: hook_title | cold_open).
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub action: String,
    pub command: String,
    pub why: String,
}

const FUNCTION_WORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "to", "and", "or", "but", "is", "it", "its", "that", "this", "for", "at", "with", "as", "be",
    "was", "are", "were", "am", "i", "we", "they", "he", "she", "my", "our", "their", "like", "so", "um", "uh", "just", "very",
    "have", "has", "had", "do", "does", "did", "there", "then", "than", "if", "would", "could", "can", "will", "about", "what",
    "which", "who", "how", "not", "from", "by", "me", "him", "her", "them", "us", "kind", "sort", "mean", "know", "think",
];
const BAD_END: &[&str] = &["a", "an", "the", "of", "in", "on", "to", "and", "or", "but", "is", "that", "for", "at", "with", "as", "my", "your", "our", "their", "be", "was", "are", "i", "we", "if", "than", "from", "by", "like", "so"];
const BAD_START: &[&str] = &["and", "but", "or", "of", "to", "that", "which", "because", "than", "as", "if", "then"];
const MID_START: &[&str] = &[
    "was", "were", "is", "are", "am", "be", "been", "could", "would", "should", "can", "will", "has", "have", "had", "did", "does", "do",
    "probably", "maybe", "just", "really", "also",
];
/// Shortest hook line taken from a line other than the opening question (words).
const MIN_LINE_WORDS: usize = 4;
const NEEDS_CONTEXT: &[&str] = &["he", "she", "they", "it", "that", "this", "those", "these", "him", "her"];
const YOU: &[&str] = &["you", "your", "youre", "yourself", "aap", "aapko", "tum"];
const NUMBER_WORDS: &[&str] = &["one", "two", "three", "five", "ten", "hundred", "thousand", "million", "billion"];

fn is_question(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', ']', '”', '’']).ends_with('?')
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', ']', '”', '’']).ends_with(['.', '?', '!', '…', '।'])
}

/// Sentences as index ranges into `ws` (split on end punctuation or a pause >= 0.8 s).
fn sentences(ws: &[SWord]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut s0 = 0usize;
    for k in 0..ws.len() {
        let gap_after = ws.get(k + 1).map_or(0.0, |n| n.start - ws[k].end);
        if ends_sentence(&ws[k].text) || gap_after >= 0.8 || k + 1 == ws.len() {
            if k + 1 > s0 {
                out.push((s0, k + 1));
            }
            s0 = k + 1;
        }
    }
    out
}

fn word_points(t: &str) -> f64 {
    let mut p = 0.0;
    if EMPHASIS_WORDS.contains(&t) {
        p += 2.0;
    }
    if t.chars().any(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&t) {
        p += 2.0;
    }
    if YOU.contains(&t) {
        p += 1.0;
    }
    if t.chars().count() >= 5 && !FUNCTION_WORDS.contains(&t) {
        p += 0.7;
    }
    p
}

/// How strong a sentence is as a stand-alone line (emphasis, numbers, "you", content, question).
fn line_score(ws: &[SWord]) -> f64 {
    let toks: Vec<String> = ws.iter().map(|w| tok(&w.text)).filter(|t| !t.is_empty()).collect();
    if toks.len() < 3 {
        return -1.0;
    }
    let mut s: f64 = toks.iter().map(|t| word_points(t)).sum::<f64>() / (toks.len() as f64).sqrt();
    if ws.last().is_some_and(|w| is_question(&w.text)) {
        s += 1.0;
    }
    let skip = weak_opener_len(&toks);
    if toks.get(skip).is_some_and(|t| NEEDS_CONTEXT.contains(&t.as_str())) {
        s -= 1.5;
    }
    s
}

fn clean_word(w: &str) -> String {
    w.trim_matches(|c: char| c == '"' || c == '“' || c == '”').to_string()
}

/// A hook line of at most [`MAX_HOOK_WORDS`] words from a sentence: weak openers dropped, the
/// best-scoring span kept (a question keeps its end), trailing punctuation removed except '?'.
pub fn shorten(ws: &[SWord]) -> Option<String> {
    // Readable words, a stutter said once ("you you wrote" -> "you wrote").
    let mut words: Vec<&SWord> = Vec::new();
    for w in ws.iter().filter(|w| !tok(&w.text).is_empty()) {
        if words.last().is_some_and(|p| tok(&p.text) == tok(&w.text)) {
            words.pop();
        }
        words.push(w);
    }
    let toks: Vec<String> = words.iter().map(|w| tok(&w.text)).collect();
    let skip = weak_opener_len(&toks);
    let (words, toks) = (&words[skip..], &toks[skip..]);
    if words.len() < 3 {
        return None;
    }
    let question = words.last().is_some_and(|w| is_question(&w.text));
    let (a, b) = if words.len() <= MAX_HOOK_WORDS {
        (0, words.len())
    } else {
        let mut best: Option<(f64, usize, usize)> = None;
        for len in (4..=MAX_HOOK_WORDS).rev() {
            for a in 0..=words.len() - len {
                let b = a + len;
                let mut s: f64 = toks[a..b].iter().map(|t| word_points(t)).sum();
                s += 0.15 * len as f64;
                if BAD_START.contains(&toks[a].as_str()) {
                    s -= 2.0;
                }
                // Starting mid-sentence on a verb ("could probably ...", "was maybe ...") reads broken.
                if a > 0 && MID_START.contains(&toks[a].as_str()) {
                    s -= 2.0;
                }
                if BAD_END.contains(&toks[b - 1].as_str()) && b != words.len() {
                    s -= 2.0;
                }
                // Mid-sentence punctuation inside the span breaks it in two.
                if words[a..b - 1].iter().any(|w| w.text.ends_with(['.', ',', ';', ':', '?', '!'])) {
                    s -= 1.0;
                }
                if question && b == words.len() {
                    s += 2.5;
                }
                if best.is_none_or(|(bs, _, _)| s > bs + 1e-9) {
                    best = Some((s, a, b));
                }
            }
        }
        let (_, a, b) = best?;
        (a, b)
    };
    let mut parts: Vec<String> = words[a..b].iter().map(|w| clean_word(&w.text)).collect();
    let keep_q = question && b == words.len();
    if let Some(last) = parts.last_mut() {
        let t = last.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
        *last = if keep_q { format!("{t}?") } else { t };
    }
    if let Some(first) = parts.first_mut() {
        let mut c = first.chars();
        if let Some(f) = c.next() {
            *first = f.to_uppercase().collect::<String>() + c.as_str();
        }
    }
    let line = parts.join(" ").replace('"', "");
    (!line.trim().is_empty()).then_some(line)
}

/// Hook-title and cold-open suggestions for one clip. `ws`: the words the clip plays (one source,
/// source seconds); `clip_ref`: the clip id (or `<clip_id>` in a plan); `src_offset`: subtracted
/// from source seconds in the cold-open command (a URL section starts later than the original
/// video); `source_flag`: e.g. ` --source-id src_x` when the words are not from the clip's first source.
pub fn suggest(ws: &[SWord], clip_ref: &str, src_offset: f64, source_flag: &str) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    if ws.is_empty() {
        return out;
    }
    let sents = sentences(ws);
    let clip_start = ws[0].start;
    let opener = sents.first().copied().filter(|&(a, b)| ws[a].start - clip_start <= OPENING_WITHIN && is_question(&ws[b - 1].text));
    let mut lines: Vec<(String, String)> = Vec::new();
    let push = |lines: &mut Vec<(String, String)>, line: String, from: String| {
        if !lines.iter().any(|(l, _)| l.eq_ignore_ascii_case(&line)) {
            lines.push((line, from));
        }
    };
    if let Some((a, b)) = opener {
        if let Some(l) = shorten(&ws[a..b]) {
            push(&mut lines, l, "the opening question".into());
        }
    }
    let mut ranked: Vec<(f64, usize, usize)> = sents.iter().filter(|s| Some(**s) != opener).map(|&(a, b)| (line_score(&ws[a..b]), a, b)).filter(|s| s.0 > 0.0).collect();
    ranked.sort_by(|x, y| y.0.total_cmp(&x.0));
    for &(_, a, b) in &ranked {
        if lines.len() >= 3 {
            break;
        }
        if let Some(l) = shorten(&ws[a..b]).filter(|l| l.split_whitespace().count() >= MIN_LINE_WORDS) {
            let said: String = ws[a..b].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            push(&mut lines, l, format!("the line at {:.1} s of the clip (\"{said}\")", ws[a].start - clip_start));
        }
    }
    for (line, from) in &lines {
        out.push(Suggestion {
            action: "hook_title".into(),
            command: format!("rustler overlay add-text {clip_ref} \"{line}\" --style hook-big"),
            why: format!(
                "suggested hook title ({} words, plain text) from {from}; rewrite it in your words if it does not land: clips auto never adds text",
                line.split_whitespace().count()
            ),
        });
    }
    // Cold open: the clip opens on a question, so lead with the strongest answer line.
    if let Some((qa, qb)) = opener {
        let best = ranked
            .iter()
            .filter(|&&(_, a, b)| a >= qb && ws[b - 1].end - ws[a].start >= COLD_MIN && ws[b - 1].end - ws[a].start <= COLD_MAX && b - a >= 5)
            .find(|&&(_, a, b)| {
                let toks: Vec<String> = ws[a..b].iter().map(|w| tok(&w.text)).filter(|t| !t.is_empty()).collect();
                !toks.get(weak_opener_len(&toks)).is_some_and(|t| NEEDS_CONTEXT.contains(&t.as_str()))
            });
        if let Some(&(_, a, b)) = best {
            let prev_end = if a > 0 { ws[a - 1].end } else { f64::MIN };
            let next_start = ws.get(b).map_or(f64::MAX, |w| w.start);
            let s = (ws[a].start - 0.08).max(prev_end).max(0.0) - src_offset;
            let e = (ws[b - 1].end + 0.15).min(next_start) - src_offset;
            let line: String = ws[a..b].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            let q: String = ws[qa..qb].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            out.push(Suggestion {
                action: "cold_open".into(),
                command: format!("rustler edit cold-open {clip_ref} --start {:.2} --end {:.2}{source_flag}", s.max(0.0), e),
                why: format!(
                    "the clip opens on a question (\"{q}\"): open on the strongest answer line instead (\"{line}\", {:.1} s) and let the question follow; clips auto never re-orders a clip",
                    e - s
                ),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(text: &str) -> Vec<SWord> {
        let mut t = 100.0;
        text.split_whitespace()
            .map(|w| {
                let x = SWord { text: w.into(), start: t, end: t + 0.3 };
                t += 0.35;
                x
            })
            .collect()
    }

    #[test]
    fn long_question_keeps_its_end_and_seven_words() {
        let l = shorten(&ws("So, what's the worst advice you've gotten as a founder?")).unwrap();
        assert!(l.split_whitespace().count() <= MAX_HOOK_WORDS, "{l}");
        assert!(l.ends_with("founder?"), "{l}");
        assert!(l.chars().next().unwrap().is_uppercase());
        // A short line is kept whole, weak opener dropped, trailing period removed.
        assert_eq!(shorten(&ws("Well, listen to less advice.")).unwrap(), "Listen to less advice");
        assert!(shorten(&ws("Yeah.")).is_none());
        // Trial 3: a stutter is said once, and a span never starts mid-sentence on a verb.
        let l = shorten(&ws("Do you know this takes me to the point where you you wrote an essay?")).unwrap();
        assert!(!l.to_lowercase().contains("you you"), "{l}");
        let l = shorten(&ws("I think YC could probably distill still five to ten very important lessons.")).unwrap();
        assert!(!l.starts_with("Could") && !l.starts_with("Probably"), "{l}");
    }

    #[test]
    fn opening_question_gives_hooks_and_a_cold_open() {
        let w = ws("What's the worst advice you've gotten as a founder? I mean, almost all the advice is terrible. \
                    Listen to less advice and trust your own instinct always. The biggest mistake is copying everyone else.");
        let s = suggest(&w, "clp_x", 90.0, "");
        let hooks: Vec<&Suggestion> = s.iter().filter(|x| x.action == "hook_title").collect();
        assert!((2..=3).contains(&hooks.len()), "{s:?}");
        for h in &hooks {
            assert!(h.command.starts_with("rustler overlay add-text clp_x \"") && h.command.ends_with("--style hook-big"), "{}", h.command);
            let line = h.command.split('"').nth(1).unwrap();
            assert!(line.split_whitespace().count() <= MAX_HOOK_WORDS, "{line}");
        }
        assert!(hooks[0].why.contains("opening question"));
        let cold = s.iter().find(|x| x.action == "cold_open").expect("cold open");
        assert!(cold.command.starts_with("rustler edit cold-open clp_x --start "), "{}", cold.command);
        // Source seconds minus the section offset (words start at 100 s, offset 90 s).
        let start: f64 = cold.command.split_whitespace().nth(5).unwrap().parse().unwrap();
        assert!(start > 10.0 && start < 30.0, "{start}");
    }

    #[test]
    fn no_cold_open_without_an_opening_question() {
        let w = ws("Almost all the advice is terrible. Listen to less advice and trust your own instinct always.");
        let s = suggest(&w, "clp_x", 0.0, "");
        assert!(s.iter().all(|x| x.action != "cold_open"));
        assert!(!s.is_empty());
    }
}
