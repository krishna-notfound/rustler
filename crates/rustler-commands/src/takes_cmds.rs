//! Command: `edit.find_bad_takes` (f39) — read-only. Scans the words a clip plays (exact word
//! timings) for restarts and repeats and lists each with the exact `edit remove-words` command
//! that keeps the cleaner (later) take. Nothing is cut: the agent decides.
//!
//! What is found (deterministic, from the transcript words only):
//! - `stutter`: the same word said two or more times in a row ("I I think") — the earlier copies go.
//! - `restart`: a phrase of 2+ words said again at once ("I think I think the real thing").
//! - `false_start`: a phrase begun, abandoned after a few words, then restated ("the real thing
//!   is the real problem is ...") — the first attempt and the abandoned words go.
//! - `repeated_sentence`: a whole sentence (4+ words) said again as the next sentence.
//!
//! Word indices (`Word.i`) are stable across edits, so every command stays valid after another one
//! is applied. Sources with approximate (captions-based) word timings are skipped and named.

use crate::director_cmds::Fix;
use crate::highlights_cmds::tok;
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, Result, RustlerError, SourceId, TimeMap, WordTiming};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const STAGE: &str = "edit.find_bad_takes";
/// Findings below this confidence are not listed (default `min_confidence`).
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.4;
/// The restated copy must start within this many seconds of the first attempt.
const MAX_SPAN: f64 = 4.0;
/// At most this many words between the start of the first attempt and the restart.
const MAX_WORDS_BETWEEN: usize = 8;
/// A repeated sentence must start within this many seconds of the first one.
const MAX_SENTENCE_GAP: f64 = 12.0;
/// Words that are often doubled on purpose ("very very", "no no", "had had").
const DOUBLED_ON_PURPOSE: &[&str] = &[
    "very", "really", "no", "yes", "yeah", "so", "bye", "ha", "haha", "na", "okay", "ok", "right", "go", "oh", "had", "that", "bas",
    "nahi", "haan", "hmm",
];
/// Function words: a "phrase" made only of these ("of the", "in a") repeats naturally.
const FUNCTION_WORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "to", "and", "or", "but", "is", "it", "its", "that", "this", "for", "at", "with", "as", "be",
    "was", "are", "i", "you", "we", "they", "he", "she", "my", "your", "like", "so", "um", "uh", "ki", "ke", "ka", "hai", "toh",
];

fn verr(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, STAGE, msg)
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// A word the clip plays, in clip order.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipWord {
    /// `Word.i` in the source transcript.
    pub i: u32,
    pub text: String,
    /// Source seconds.
    pub start: f64,
    pub end: f64,
    pub source_id: SourceId,
    /// Edit-timeline seconds.
    pub out_start: f64,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct BadTake {
    /// stutter | restart | false_start | repeated_sentence
    pub kind: String,
    /// 0..1 (how sure the pattern is a mistake and not said on purpose).
    pub confidence: f64,
    pub source_id: SourceId,
    /// The words the command removes (source seconds).
    pub start: f64,
    pub end: f64,
    /// Where it plays in the clip (edit-timeline seconds).
    pub out_start: f64,
    pub out_end: f64,
    /// What goes.
    pub cut_text: String,
    /// The take that stays (it starts right after the cut).
    pub kept_text: String,
    pub word_indices: Vec<u32>,
    /// Seconds the clip gets shorter.
    pub removes_seconds: f64,
    /// Exact command that keeps the later take.
    pub command: String,
    pub why: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SkippedSource {
    pub source_id: SourceId,
    pub reason: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct FindBadTakesIn {
    pub clip_id: ClipId,
    /// List only findings at or above this confidence (0..1, default 0.4).
    pub min_confidence: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct FindBadTakesOut {
    pub clip_id: ClipId,
    pub revision: u64,
    /// Highest confidence first. Never applied: run the commands you agree with.
    pub findings: Vec<BadTake>,
    /// Words checked (sources with exact timings only).
    pub words_checked: usize,
    /// Sources not checked (no transcript, or approximate captions-based word timings).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped_sources: Vec<SkippedSource>,
    pub note: String,
    pub next: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Detection (pure)
// ---------------------------------------------------------------------------------------------

/// Confidence of a repeat said three or more times in a row (likely deliberate emphasis).
const EMPHASIS_CONF: f64 = 0.35;

struct Raw {
    kind: &'static str,
    conf: f64,
    /// Cut positions `a..b` (exclusive) into the word list; the kept take starts at `b`.
    a: usize,
    b: usize,
    /// Words of the kept take that match the cut (for the text shown).
    keep_len: usize,
    /// Said three or more times in a row: probably on purpose.
    emphasis: bool,
}

/// One detected restart or repeat (positions into the word list).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub kind: String,
    pub conf: f64,
    /// Cut `a..b` (exclusive); the kept take starts at `b`.
    pub a: usize,
    pub b: usize,
    /// Words of the kept take that match the cut.
    pub keep: usize,
    /// Said three or more times in a row: probably deliberate emphasis.
    pub emphasis: bool,
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', ']', '”', '’']).ends_with(['.', '?', '!', '…', '।'])
}

/// A break before position `b`: a pause, or the word before it ends with a dash, comma or ellipsis.
fn break_before(ws: &[ClipWord], b: usize) -> bool {
    if b == 0 {
        return false;
    }
    let prev = &ws[b - 1];
    ws[b].start - prev.end >= 0.25 || prev.text.trim_end().ends_with(['-', '—', '–', ',', '…']) || prev.text.ends_with("...")
}

/// Finds restarts and repeats in `ws` (one clip's words in play order). Returns findings at or
/// above `min_conf`, non-overlapping, highest confidence first. Cuts never span two sources.
pub fn detect(ws: &[ClipWord], min_conf: f64) -> Vec<Hit> {
    let t: Vec<String> = ws.iter().map(|w| tok(&w.text)).collect();
    let n = ws.len();
    let mut raw: Vec<Raw> = Vec::new();
    let same_src = |a: usize, b: usize| ws[a..=b.min(n - 1)].iter().all(|w| w.source_id == ws[a].source_id);

    let mut a = 0usize;
    while a < n {
        if t[a].is_empty() {
            a += 1;
            continue;
        }
        // Stutter: the same word again at once ("I I I think"); keep the last copy.
        let mut r = a + 1;
        while r < n && t[r] == t[a] && ws[r].start - ws[r - 1].end < 0.8 {
            r += 1;
        }
        if r - a >= 2 && same_src(a, r - 1) {
            let on_purpose = DOUBLED_ON_PURPOSE.contains(&t[a].as_str());
            raw.push(Raw { kind: "stutter", conf: if on_purpose { 0.3 } else { 0.75 }, a, b: r - 1, keep_len: 1, emphasis: on_purpose });
        }
        // Restart / false start: the longest phrase (2+ words) starting at `a` said again at `b`.
        let mut best: Option<(usize, usize)> = None; // (b, m)
        for b in a + 1..n.min(a + 1 + MAX_WORDS_BETWEEN) {
            if ws[b].start - ws[a].start > MAX_SPAN {
                break;
            }
            let m = (0..b - a).take_while(|&k| b + k < n && !t[a + k].is_empty() && t[a + k] == t[b + k]).count();
            if m >= 2 && best.is_none_or(|(_, bm)| m > bm) {
                best = Some((b, m));
            }
        }
        if let Some((b, m)) = best {
            if same_src(a, b) {
                let phrase_fn = t[a..a + m].iter().all(|x| FUNCTION_WORDS.contains(&x.as_str()));
                let len_bonus = 0.1 * (m.min(4) as f64 - 2.0);
                let brk = if break_before(ws, b) { 0.1 } else { 0.0 };
                if b == a + m {
                    let conf = 0.6 + len_bonus + brk - if phrase_fn && m == 2 { 0.1 } else { 0.0 };
                    // A whole sentence said twice is a repeated sentence, not a restart.
                    let kind = if m >= 4 && ends_sentence(&ws[b - 1].text) { "repeated_sentence" } else { "restart" };
                    raw.push(Raw { kind, conf, a, b, keep_len: m, emphasis: false });
                } else {
                    let conf = 0.45 + len_bonus + brk - if phrase_fn { 0.25 } else { 0.0 };
                    raw.push(Raw { kind: "false_start", conf, a, b, keep_len: m, emphasis: false });
                }
            }
        }
        a += 1;
    }

    // Repeated sentence: a sentence of 4+ words said again as the next sentence (or the start of it).
    let mut sentences: Vec<(usize, usize)> = Vec::new();
    let mut s0 = 0usize;
    for k in 0..n {
        if ends_sentence(&ws[k].text) || k + 1 == n {
            sentences.push((s0, k + 1));
            s0 = k + 1;
        }
    }
    for w in sentences.windows(2) {
        let ((a0, a1), (b0, b1)) = (w[0], w[1]);
        let ta: Vec<&String> = t[a0..a1].iter().filter(|x| !x.is_empty()).collect();
        let tb: Vec<&String> = t[b0..b1].iter().filter(|x| !x.is_empty()).collect();
        if ta.len() < 4 || tb.len() < ta.len() || ws[b0].start - ws[a0].start > MAX_SENTENCE_GAP || !same_src(a0, b0) {
            continue;
        }
        if tb[..ta.len()] == ta[..] {
            let conf = if tb.len() == ta.len() { 0.8 } else { 0.65 };
            raw.push(Raw { kind: "repeated_sentence", conf, a: a0, b: b0, keep_len: ta.len(), emphasis: false });
        }
    }

    // A phrase said three or more times in a row ("Listen to less advice. Listen to less advice.
    // Listen to less advice.") is usually deliberate emphasis, not a mistake.
    // Measured as the length of the periodic stretch (period = the cut) around the finding, so a
    // match that starts mid-phrase ("to less advice. Listen") is caught too.
    for r in raw.iter_mut().filter(|r| r.kind != "stutter" && r.b > r.a) {
        let p = r.b - r.a;
        let mut left = r.a;
        while left > 0 && t[left - 1] == t[left - 1 + p] {
            left -= 1;
        }
        let mut right = (r.b + r.keep_len).min(n);
        while right < n && right >= p && t[right] == t[right - p] {
            right += 1;
        }
        if (right - left) as f64 >= 2.5 * p as f64 {
            r.conf = r.conf.min(EMPHASIS_CONF);
            r.emphasis = true;
        }
    }
    raw.retain(|r| r.conf + 1e-9 >= min_conf && r.b > r.a);
    raw.sort_by(|x, y| y.conf.total_cmp(&x.conf).then((y.b - y.a).cmp(&(x.b - x.a))));
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut out = Vec::new();
    for r in raw {
        if taken.iter().any(|&(a, b)| r.a < b && a < r.b) {
            continue;
        }
        taken.push((r.a, r.b));
        out.push(Hit { kind: r.kind.to_string(), conf: (r.conf.clamp(0.0, 1.0) * 100.0).round() / 100.0, a: r.a, b: r.b, keep: r.keep_len, emphasis: r.emphasis });
    }
    out
}

/// The clip's words in play order (midpoint inside a segment), per the edit timeline.
pub fn clip_words(clip: &Clip, words_by_source: &HashMap<SourceId, Vec<rustler_core::Word>>) -> Vec<ClipWord> {
    let tm = TimeMap::from_segments(&clip.segments);
    let mut out = Vec::new();
    for seg in tm.segments() {
        let Some(words) = words_by_source.get(&seg.source_id) else { continue };
        for w in words {
            let mid = (w.start + w.end) / 2.0;
            if mid < seg.src_start || mid >= seg.src_end || w.text.trim().is_empty() {
                continue;
            }
            out.push(ClipWord {
                i: w.i,
                text: w.text.trim().to_string(),
                start: w.start,
                end: w.end,
                source_id: seg.source_id.clone(),
                out_start: seg.out_start + (w.start.max(seg.src_start) - seg.src_start),
            });
        }
    }
    out
}

fn join(ws: &[ClipWord]) -> String {
    ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// Findings for `ws` with their commands (`multi_source`: the clip uses several sources, so the
/// command names the source).
pub fn findings(clip_id: &ClipId, ws: &[ClipWord], min_conf: f64, multi_source: bool) -> Vec<BadTake> {
    detect(ws, min_conf)
        .into_iter()
        .map(|Hit { kind, conf, a, b, keep, emphasis }| {
            let cut = &ws[a..b];
            let idx: Vec<u32> = cut.iter().map(|w| w.i).collect();
            let list = idx.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            let src = if multi_source { format!(" --source-id {}", ws[a].source_id) } else { String::new() };
            let command = format!("rustler edit remove-words {clip_id} --word-indices '[{list}]'{src}");
            let kept = join(&ws[b..(b + keep + 3).min(ws.len())]);
            let last = &cut[cut.len() - 1];
            let mut why = match kind.as_str() {
                "stutter" => format!("\"{}\" is said {} times in a row; the last one stays", ws[b].text, b - a + 1),
                "restart" => format!("\"{}\" is said twice; the second take (\"{kept}\") stays", join(&ws[a..b])),
                "false_start" => format!("\"{}\" is begun, dropped and restated as \"{kept}\"; the restated take stays", join(cut)),
                _ => format!("the sentence \"{}\" is said again right after; the second take stays", join(cut)),
            };
            if emphasis {
                why.push_str(" (said three or more times in a row, or a word people double on purpose: probably deliberate emphasis, so low confidence)");
            }
            BadTake {
                kind,
                confidence: conf,
                source_id: ws[a].source_id.clone(),
                start: r2(cut[0].start),
                end: r2(last.end),
                out_start: r2(cut[0].out_start),
                out_end: r2(cut[0].out_start + (last.end - cut[0].start)),
                cut_text: join(cut),
                kept_text: kept,
                word_indices: idx,
                removes_seconds: r2(last.end - cut[0].start),
                command,
                why,
            }
        })
        .collect()
}

/// Bad takes of a stored clip: (findings, words checked, skipped sources). Sources without a
/// transcript or with approximate word timings are skipped.
pub fn for_clip(store: &ProjectStore, clip: &Clip, min_conf: f64) -> Result<(Vec<BadTake>, usize, Vec<SkippedSource>)> {
    let mut by_source: HashMap<SourceId, Vec<rustler_core::Word>> = HashMap::new();
    let mut skipped = Vec::new();
    for seg in &clip.segments {
        if by_source.contains_key(&seg.source_id) || skipped.iter().any(|s: &SkippedSource| s.source_id == seg.source_id) {
            continue;
        }
        match crate::clip_cmds::load_transcript(store, &seg.source_id)? {
            None => skipped.push(SkippedSource {
                source_id: seg.source_id.clone(),
                reason: format!("no transcript: run `rustler analyze transcribe {}`", seg.source_id),
            }),
            Some(t) if t.word_timing == WordTiming::Approximate => skipped.push(SkippedSource {
                source_id: seg.source_id.clone(),
                reason: format!(
                    "word timings are approximate (captions-based transcript), so a cut could clip a word; run `rustler analyze transcribe {}` for exact timings",
                    seg.source_id
                ),
            }),
            Some(t) => {
                by_source.insert(seg.source_id.clone(), t.words);
            }
        }
    }
    let ws = clip_words(clip, &by_source);
    let multi = {
        let mut s: Vec<&SourceId> = clip.segments.iter().map(|s| &s.source_id).collect();
        s.sort();
        s.dedup();
        s.len() > 1
    };
    Ok((findings(&clip.id, &ws, min_conf, multi), ws.len(), skipped))
}

/// Bad takes as retention fixes (action `bad_take`; never applied by `clips.auto`).
pub fn retention_fixes(store: &ProjectStore, clip: &Clip) -> Vec<Fix> {
    match for_clip(store, clip, DEFAULT_MIN_CONFIDENCE) {
        Ok((f, _, _)) => f
            .into_iter()
            .map(|b| Fix {
                action: "bad_take".into(),
                command: Some(b.command),
                why: format!("{} at {:.1} s (confidence {:.2}): {}; cut \"{}\" (-{:.2} s)", b.kind.replace('_', " "), b.out_start, b.confidence, b.why, b.cut_text, b.removes_seconds),
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

async fn find_bad_takes(ctx: Ctx, i: FindBadTakesIn) -> Result<FindBadTakesOut> {
    let min_conf = i.min_confidence.unwrap_or(DEFAULT_MIN_CONFIDENCE);
    if !(0.0..=1.0).contains(&min_conf) {
        return Err(verr(format!("min_confidence {min_conf} is out of range")).hint("use a number between 0 and 1 (default 0.4)"));
    }
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id).map_err(|e| {
        if e.code == ErrorCode::NotFound {
            RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} not found", i.clip_id)).hint("run `rustler clip list` to see clip ids")
        } else {
            e
        }
    })?;
    if clip.segments.is_empty() {
        return Err(verr(format!("clip {} has no segments", clip.id)));
    }
    let (findings, checked, skipped) = for_clip(&store, &clip, min_conf)?;
    let id = clip.id.as_str();
    let mut next = Vec::new();
    if let Some(f) = findings.first() {
        next.push(format!("read the moment before cutting: rustler see preview {id} --start {:.1} --end {:.1}", (f.out_start - 1.0).max(0.0), f.out_end + 2.0));
        next.push(f.command.clone());
        next.push(format!("undo a cut you do not like: rustler history undo {id}"));
        next.push(format!("rustler clip retention {id}"));
    } else if skipped.is_empty() {
        next.push(format!("no restarts or repeats found; tighten pauses instead if needed: rustler edit remove-pauses {id}"));
    }
    let note = if checked == 0 && !skipped.is_empty() {
        "nothing checked: every source of the clip was skipped (see skipped_sources)".to_string()
    } else {
        "Suggestions only, never applied: each command keeps the later (cleaner) take; word indices stay valid after other cuts. A repeat said on purpose (emphasis, a list) is not a mistake: read it first.".to_string()
    };
    Ok(FindBadTakesOut { clip_id: clip.id.clone(), revision: clip.revision, findings, words_checked: checked, skipped_sources: skipped, note, next })
}

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        STAGE,
        "Read-only: find restarts and repeats in a clip (stutters, phrases said twice, false starts, repeated sentences) from exact word timings; each with times, text, confidence and the `edit remove-words` command that keeps the cleaner take. Never cuts anything; skips captions-based (approximate) timings",
        &["clip_id"],
        Annotations { read_only: true, ..Default::default() },
        vec![
            Example { description: "list bad takes", cli: "rustler edit find-bad-takes clp_X" },
            Example { description: "only the surest ones", cli: "rustler edit find-bad-takes clp_X --min-confidence 0.7" },
        ],
        find_bad_takes,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(text: &str) -> Vec<ClipWord> {
        let src = SourceId::new();
        let mut t = 10.0;
        text.split_whitespace()
            .enumerate()
            .map(|(k, w)| {
                // "|" before a word = a 0.4 s pause before it.
                let (w, pause) = match w.strip_prefix('|') {
                    Some(r) => (r, 0.4),
                    None => (w, 0.05),
                };
                t += pause;
                let cw = ClipWord { i: 100 + k as u32, text: w.into(), start: t, end: t + 0.25, source_id: src.clone(), out_start: t - 10.0 };
                t += 0.25;
                cw
            })
            .collect()
    }

    fn kinds(text: &str) -> Vec<(String, String)> {
        let w = ws(text);
        detect(&w, DEFAULT_MIN_CONFIDENCE).into_iter().map(|h| (h.kind, join(&w[h.a..h.b]))).collect()
    }

    #[test]
    fn restart_keeps_the_second_take() {
        let w = ws("I think I think the real thing is focus.");
        let f = findings(&ClipId::new(), &w, 0.4, false);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].kind, "restart");
        assert_eq!(f[0].cut_text, "I think");
        assert_eq!(f[0].word_indices, vec![100, 101]);
        assert!(f[0].kept_text.starts_with("I think the real"));
        assert!(f[0].command.contains("edit remove-words") && f[0].command.contains("[100,101]"), "{}", f[0].command);
        assert!(f[0].confidence >= 0.5);
    }

    #[test]
    fn stutter_false_start_and_repeated_sentence() {
        assert_eq!(kinds("so I I I went home today."), vec![("stutter".to_string(), "I I".to_string())]);
        // Doubled on purpose: not listed at the default confidence.
        assert!(kinds("it was very very good indeed.").is_empty());
        let k = kinds("the real thing is, |the real problem is money.");
        assert_eq!(k, vec![("false_start".to_string(), "the real thing is,".to_string())]);
        let k = kinds("We should build it now. |We should build it now. Then ship.");
        assert_eq!(k[0], ("repeated_sentence".to_string(), "We should build it now.".to_string()));
    }

    #[test]
    fn clean_speech_and_function_word_repeats_are_not_flagged() {
        assert!(kinds("Most people thought it was too early and not good enough.").is_empty());
        // "of the" twice a few words apart is normal speech.
        assert!(kinds("one of the best of the year for us").is_empty());
        // A phrase repeated far apart is not a restart.
        let mut w = ws("I think this works and then a lot of other words come here before I think again");
        for (k, x) in w.iter_mut().enumerate() {
            x.start += k as f64 * 0.5;
        }
        assert!(detect(&w, 0.4).iter().all(|f| f.kind != "restart"));
    }

    #[test]
    fn a_line_said_three_times_is_emphasis_with_low_confidence() {
        // Trial 3, clip 3: "listen to less advice. Listen to less advice. Listen to less advice."
        let text = "my advice is listen to less advice. Listen to less advice. Listen to less advice. Do you agree?";
        assert!(kinds(text).is_empty(), "hidden at the default confidence");
        let w = ws(text);
        let h = detect(&w, 0.3);
        assert!(!h.is_empty() && h.iter().all(|x| x.emphasis && x.conf <= EMPHASIS_CONF + 1e-9), "{h:?}");
        let f = findings(&ClipId::new(), &w, 0.3, false);
        assert!(f[0].why.contains("deliberate emphasis"));
    }
}
