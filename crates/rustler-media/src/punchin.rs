//! The one punch-in planner. Template punch-in patterns (`template.apply`, `clip create
//! --template`) and the retention `punch_in` fix (`clip retention`) both place punch-ins here, so
//! a clip gets one consistent rhythm instead of a fixed "every N s" grid plus extra fixes on top.
//!
//! Rules (all on the edit timeline, seconds):
//! - a visual change at least every `max_static_s` (just under the retention scorer's static gap),
//!   but never more than `max_changes_per_10s` changes in any 10 s window (cuts, text pops and
//!   existing effects count too; a punch-in is two changes: zoom in, zoom out);
//! - each punch-in starts just before a stressed word (content word, number, emphasis word,
//!   phrase end, lengthened word) when the transcript has one in reach;
//! - at least `min_cut_gap_s` from every cut / layout change / transition seam, and never across one;
//! - never inside `blocked` spans (graphic, blur-fit / zoom-fit, cutaway segments: a punch-in there
//!   crops the graphic's text) and never on top of an existing zoom;
//! - the amount varies between `amount_min` and `amount_max` (stronger words zoom more, and
//!   consecutive punch-ins never use the same amount);
//! - the zoom is centred on the face (`focus`) when the caller knows where the face is.
//!
//! Pure: no I/O. `rustler-commands` gathers the input from the timeline.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A transcript word on the edit timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct PunchWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Where the face is on the output canvas (0..1 fractions) during `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusSpan {
    pub start: f64,
    pub end: f64,
    pub x: f64,
    pub y: f64,
}

/// Everything the planner needs about one clip (edit-timeline seconds).
#[derive(Clone, Debug, Default)]
pub struct PunchInput {
    pub duration: f64,
    pub words: Vec<PunchWord>,
    /// Cuts: segment seams that change the picture (jump cut, layout change, other source) and
    /// transition seams.
    pub cuts: Vec<f64>,
    /// Spans where no punch-in may be (graphics / blur-fit / zoom-fit segments, cutaways,
    /// transition overlaps).
    pub blocked: Vec<(f64, f64)>,
    /// Existing zoom-type effects (punch-in, zoom-slow, shake): not stacked on.
    pub existing: Vec<(f64, f64)>,
    /// Other visual changes that count toward pacing (text pops, images, cutaway starts/ends).
    pub changes: Vec<f64>,
    /// Plan only inside this span (a retention dead zone); the whole clip when `None`.
    pub window: Option<(f64, f64)>,
    pub focus: Vec<FocusSpan>,
}

/// Pacing and look of planned punch-ins.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PunchConfig {
    /// Upper bound on visual changes (all kinds) in any 10 s window.
    pub max_changes_per_10s: f64,
    /// Longest stretch without a visual change the planner leaves (it fills longer ones).
    pub max_static_s: f64,
    /// Shortest gap between two changes the planner creates.
    pub min_spacing_s: f64,
    /// Distance kept from cuts, layout changes and transition seams.
    pub min_cut_gap_s: f64,
    /// Target length of one punch-in (snapped to word boundaries).
    pub len_s: f64,
    pub min_len_s: f64,
    pub max_len_s: f64,
    pub amount_min: f64,
    pub amount_max: f64,
}

impl Default for PunchConfig {
    fn default() -> Self {
        Self {
            // Rare and subtle: a tight shot held for a phrase (like a second camera), about one
            // every 6-8 s, never a zoom in/out pump every couple of seconds.
            max_changes_per_10s: 1.5,
            max_static_s: crate::retention::STATIC_GAP_S - 0.1,
            min_spacing_s: 3.0,
            min_cut_gap_s: 0.5,
            len_s: 5.0,
            min_len_s: 2.5,
            max_len_s: 8.0,
            amount_min: 1.06,
            amount_max: 1.12,
        }
    }
}

/// One planned punch-in.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct PlannedPunch {
    pub start: f64,
    pub end: f64,
    pub amount: f64,
    /// Zoom centre on the canvas (0..1) when a face position is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_y: Option<f64>,
    /// The word the zoom lands on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
}

/// The zoom starts this long before the stressed word so it lands on it.
const LEAD_S: f64 = 0.04;
/// Changes closer than this count once toward the 10 s cap.
const MERGE_S: f64 = 0.5;

fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').flat_map(char::to_lowercase).collect()
}

const FUNCTION: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "so", "of", "to", "in", "on", "at", "for", "by", "with", "as", "is", "are", "was",
    "were", "be", "been", "am", "it", "its", "it's", "i", "i'm", "you", "you're", "he", "she", "we", "they", "me", "him",
    "her", "us", "them", "my", "your", "his", "our", "their", "this", "that", "these", "those", "there", "here", "then",
    "than", "do", "does", "did", "have", "has", "had", "will", "would", "can", "could", "should", "just", "like", "um", "uh",
    "yeah", "okay", "if", "when", "what", "which", "who", "how", "because", "about", "from", "into", "up", "out", "know",
    "going", "gonna", "want", "kind", "sort", "thing", "things", "really", "very", "get", "got", "say", "said", "that's",
    "hai", "hain", "toh", "aur", "ki", "ka", "ke", "bhi", "kya", "matlab", "nahi",
];

const EMPHASIS: &[&str] = &[
    "never", "always", "every", "everyone", "everything", "nobody", "nothing", "no", "not", "don't", "can't", "won't",
    "best", "worst", "most", "least", "biggest", "only", "first", "last", "huge", "massive", "secret", "truth", "real",
    "actually", "exactly", "insane", "crazy", "literally", "must", "need", "stop", "wrong", "right", "free", "money",
    "million", "billion", "thousand", "hundred", "percent", "half", "double", "zero", "one", "two", "three", "four",
    "five", "six", "seven", "eight", "nine", "ten",
];

/// Stress score of word `i` (0 = function word .. ~4 = emphatic, lengthened, phrase-final).
pub fn stress(words: &[PunchWord], i: usize, median_per_char: f64) -> f64 {
    let w = &words[i];
    let n = norm(&w.text);
    if n.is_empty() {
        return 0.0;
    }
    let chars = n.chars().count();
    let mut s = if FUNCTION.contains(&n.as_str()) { 0.1 } else { 1.0 };
    s += 0.08 * chars.min(10) as f64;
    if EMPHASIS.contains(&n.as_str()) {
        s += 1.2;
    }
    if n.chars().any(|c| c.is_ascii_digit()) {
        s += 1.0;
    }
    let raw = w.text.trim_end_matches(['"', '\'', ')', ']', '”', '’']);
    if raw.ends_with(['!', '?']) {
        s += 0.6;
    } else if raw.ends_with(['.', ',', ';', ':']) {
        s += 0.4;
    }
    if chars >= 2 && w.text.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) {
        s += 0.3;
    }
    if let Some(next) = words.get(i + 1)
        && next.start - w.end >= 0.25
    {
        s += 0.5;
    }
    let len = w.end - w.start;
    if median_per_char > 0.0 && len > 0.0 && chars > 0 {
        let slow = len / chars as f64 / median_per_char - 1.0;
        s += 0.5 * slow.clamp(0.0, 1.0);
    }
    s
}

fn median_per_char(words: &[PunchWord]) -> f64 {
    let mut v: Vec<f64> = words
        .iter()
        .filter_map(|w| {
            let c = norm(&w.text).chars().count();
            (c > 0 && w.end > w.start).then(|| (w.end - w.start) / c as f64)
        })
        .collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn overlaps(a: (f64, f64), b: (f64, f64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// Zoom centre for a window around `t`: the face position, pulled in so a zoom of `amount`
/// never shows past the frame edge (the renderer clamps the same way).
fn focus_at(focus: &[FocusSpan], s: f64, e: f64, amount: f64) -> Option<(f64, f64)> {
    let mid = (s + e) / 2.0;
    let f = focus
        .iter()
        .filter(|f| f.start <= mid && mid < f.end)
        .min_by(|a, b| (a.end - a.start).total_cmp(&(b.end - b.start)))
        .or_else(|| focus.iter().filter(|f| overlaps((f.start, f.end), (s, e))).max_by(|a, b| a.start.total_cmp(&b.start)))?;
    let half = 0.5 / amount.max(1.0);
    let c = |v: f64| (v.clamp(half, 1.0 - half) * 1000.0).round() / 1000.0;
    Some((c(f.x), c(f.y)))
}

/// Plans punch-ins for `input` (see the module docs). Sorted by start; never overlapping.
pub fn plan(input: &PunchInput, cfg: &PunchConfig) -> Vec<PlannedPunch> {
    let dur = input.duration;
    if !(dur.is_finite() && dur > 0.0) {
        return Vec::new();
    }
    let mut cfg = sanitize(cfg);
    // Even spacing: the cap allows one change every `ideal` seconds; planned changes aim for it
    // (a burst of close changes would force a long static stretch after it) and never come
    // closer than 80% of it.
    let ideal = (10.0 / cfg.max_changes_per_10s).clamp(cfg.min_spacing_s, cfg.max_static_s);
    cfg.min_spacing_s = cfg.min_spacing_s.max(0.8 * ideal);
    cfg.min_len_s = cfg.min_len_s.max(0.8 * ideal).min(cfg.max_len_s);
    cfg.len_s = cfg.len_s.max(cfg.min_len_s);
    let (w0, w1) = input.window.map_or((0.0, dur), |(a, b)| (a.max(0.0), b.min(dur)));
    if w1 - w0 < cfg.min_len_s {
        return Vec::new();
    }
    // Hard edges: the clip ends, cuts. Soft changes: everything else that changes the picture.
    let mut hard: Vec<f64> = input.cuts.iter().copied().filter(|t| t.is_finite()).collect();
    hard.push(0.0);
    hard.push(dur);
    hard.sort_by(|a, b| a.total_cmp(b));
    let mut changes: Vec<f64> = hard.clone();
    changes.extend(input.changes.iter().copied().filter(|t| t.is_finite()));
    for &(s, e) in &input.existing {
        changes.push(s);
        changes.push(e);
    }
    changes.sort_by(|a, b| a.total_cmp(b));
    let mut no_go: Vec<(f64, f64)> = input.blocked.clone();
    no_go.extend(input.existing.iter().map(|&(s, e)| (s - cfg.min_spacing_s * 0.5, e + cfg.min_spacing_s * 0.5)));

    let mpc = median_per_char(&input.words);
    let scores: Vec<f64> = (0..input.words.len()).map(|i| stress(&input.words, i, mpc)).collect();
    let max_score = scores.iter().copied().fold(0.0_f64, f64::max).max(1e-6);

    // Is a punch-in [s, e] allowed (cuts, blocked spans, density)?
    let allowed = |s: f64, e: f64, changes: &[f64]| -> bool {
        if s < w0 - 1e-9 || e > w1 + 1e-9 || e - s < cfg.min_len_s - 1e-9 {
            return false;
        }
        if hard.iter().any(|&c| c > s - cfg.min_cut_gap_s && c < e + cfg.min_cut_gap_s) {
            return false;
        }
        if no_go.iter().any(|&b| overlaps(b, (s, e))) {
            return false;
        }
        if changes.iter().any(|&c| (c - s).abs() < cfg.min_spacing_s * 0.8 || (c - e).abs() < cfg.min_spacing_s * 0.8) {
            return false;
        }
        // Density: every 10 s window that holds s or e.
        // One punch-in is two changes (in and out), so a cap below 2 would forbid every punch-in.
        let cap = (cfg.max_changes_per_10s.ceil() as usize).max(2);
        for p in [s, e] {
            let mut starts: Vec<f64> = changes.iter().copied().filter(|c| *c >= p - 10.0 && *c <= p).collect();
            starts.push(p - 10.0);
            starts.push(s);
            for w in starts {
                // The clip's own start and end are bounds, not changes; changes closer than
                // MERGE_S read as one (a cut into a fragment and out again).
                let mut inside: Vec<f64> =
                    changes.iter().copied().filter(|c| *c >= w && *c < w + 10.0 && *c > 1e-9 && *c < dur - 1e-9).collect();
                inside.extend([s, e].into_iter().filter(|c| *c >= w && *c < w + 10.0));
                inside.sort_by(|a, b| a.total_cmp(b));
                let mut n = 0;
                let mut prev = f64::NEG_INFINITY;
                for c in inside {
                    if c - prev >= MERGE_S {
                        n += 1;
                        prev = c;
                    }
                }
                if n > cap {
                    return false;
                }
            }
        }
        true
    };

    let mut out: Vec<PlannedPunch> = Vec::new();
    let mut prev_amount: Option<f64> = None;
    let mut guard = 0;
    // Walk the timeline: from the last change, place the next punch-in before the static
    // stretch gets longer than max_static_s.
    let mut cursor = w0;
    while cursor < w1 && guard < 10_000 {
        guard += 1;
        let last = changes.iter().copied().filter(|c| *c <= cursor + 1e-9).fold(f64::NEG_INFINITY, f64::max).max(0.0);
        let next = changes.iter().copied().filter(|c| *c > cursor + 1e-9).fold(f64::INFINITY, f64::min).min(dur);
        // `cursor` runs ahead of `last` only after nothing fitted earlier in this stretch.
        let from = last.max(cursor);
        if next - last <= cfg.max_static_s || next - from < cfg.min_len_s {
            cursor = next;
            continue;
        }
        // Candidate starts in [last + spacing, from + max_static]: word starts (stressed first),
        // or the plain time when there is no transcript in reach.
        let lo = (last + cfg.min_spacing_s).max(cursor).max(w0);
        let hi = (from + cfg.max_static_s).min(next - cfg.min_len_s).min(w1 - cfg.min_len_s);
        let mut cands: Vec<(f64, f64, Option<usize>)> = Vec::new(); // (start, rank, word)
        for (i, w) in input.words.iter().enumerate() {
            let s = w.start - LEAD_S;
            if s >= lo && s <= hi {
                // Prefer stressed words near the even spacing from the last change.
                let off = ((s - (last + ideal)).abs() / ideal).min(1.0);
                let rank = scores[i] / max_score + 0.8 * (1.0 - off);
                cands.push((s, rank, Some(i)));
            }
        }
        if hi >= lo {
            let t = (last + ideal).clamp(lo, hi);
            cands.push((t, if input.words.is_empty() { 1.0 } else { 0.3 }, None));
        }
        cands.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut placed = None;
        for (s, _, wi) in cands {
            // End: a word end near the target length, inside the allowed range.
            let max_e = (s + cfg.max_len_s).min(s + cfg.max_static_s).min(next).min(w1);
            let target = s + cfg.len_s;
            // Word ends first (closest to the target length), then any time in the range.
            let lo_e = s + cfg.min_len_s;
            let mut ends: Vec<(f64, f64)> =
                input.words.iter().map(|w| w.end).filter(|e| *e >= lo_e && *e <= max_e).map(|e| (e, (e - target).abs())).collect();
            let mut t = lo_e;
            while t <= max_e + 1e-9 {
                ends.push((t, (t - target).abs() + 0.3));
                t += 0.1;
            }
            ends.sort_by(|a, b| a.1.total_cmp(&b.1));
            if let Some(e) = ends.into_iter().map(|x| x.0).find(|&e| allowed(s, e, &changes)) {
                placed = Some((s, e, wi));
                break;
            }
        }
        match placed {
            Some((s, e, wi)) => {
                let strength = wi.map_or(0.5, |i| (scores[i] / max_score).clamp(0.0, 1.0));
                // Vary: blend word strength with a fixed cycle so neighbours differ.
                const CYCLE: [f64; 6] = [0.15, 0.85, 0.35, 1.0, 0.0, 0.6];
                let u = 0.35 * strength + 0.65 * CYCLE[out.len() % CYCLE.len()];
                let mut amount = cfg.amount_min + (cfg.amount_max - cfg.amount_min) * u;
                let step = (0.25 * (cfg.amount_max - cfg.amount_min)).max(0.005);
                if let Some(p) = prev_amount
                    && (amount - p).abs() < step
                {
                    // Too close to the previous zoom: step away from it (inside the range).
                    let up = p + step;
                    let down = p - step;
                    amount = if (amount >= p && up <= cfg.amount_max) || down < cfg.amount_min { up } else { down };
                }
                let amount = r2(amount.clamp(cfg.amount_min, cfg.amount_max));
                prev_amount = Some(amount);
                let (s, e) = (r2(s.max(0.0)), r2(e.min(dur)));
                let focus = focus_at(&input.focus, s, e, amount);
                out.push(PlannedPunch {
                    start: s,
                    end: e,
                    amount,
                    focus_x: focus.map(|f| f.0),
                    focus_y: focus.map(|f| f.1),
                    word: wi.map(|i| input.words[i].text.clone()),
                });
                changes.push(s);
                changes.push(e);
                changes.sort_by(|a, b| a.total_cmp(b));
                cursor = e;
            }
            None => {
                // Nothing fits here (cut too close, blocked span, density): try later in the stretch.
                cursor = (from + cfg.max_static_s * 0.5).min(next);
            }
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

fn sanitize(c: &PunchConfig) -> PunchConfig {
    let d = PunchConfig::default();
    let pos = |v: f64, dv: f64| if v.is_finite() && v > 0.0 { v } else { dv };
    let mut o = PunchConfig {
        max_changes_per_10s: pos(c.max_changes_per_10s, d.max_changes_per_10s).clamp(1.0, 20.0),
        max_static_s: pos(c.max_static_s, d.max_static_s).clamp(1.0, 30.0),
        min_spacing_s: pos(c.min_spacing_s, d.min_spacing_s).clamp(0.2, 10.0),
        min_cut_gap_s: if c.min_cut_gap_s.is_finite() && c.min_cut_gap_s >= 0.0 { c.min_cut_gap_s.min(5.0) } else { d.min_cut_gap_s },
        len_s: pos(c.len_s, d.len_s).clamp(0.3, 10.0),
        min_len_s: pos(c.min_len_s, d.min_len_s).clamp(0.2, 10.0),
        max_len_s: pos(c.max_len_s, d.max_len_s).clamp(0.3, 30.0),
        amount_min: pos(c.amount_min, d.amount_min).clamp(1.01, 2.0),
        amount_max: pos(c.amount_max, d.amount_max).clamp(1.01, 2.0),
    };
    if o.amount_max < o.amount_min {
        std::mem::swap(&mut o.amount_min, &mut o.amount_max);
    }
    if o.max_len_s < o.min_len_s {
        o.max_len_s = o.min_len_s;
    }
    o.len_s = o.len_s.clamp(o.min_len_s, o.max_len_s);
    o
}

/// Visual changes per 10 s of `changes` over `duration` (for reports and tests).
pub fn per_10s(changes: usize, duration: f64) -> f64 {
    if duration > 0.0 { changes as f64 * 10.0 / duration } else { 0.0 }
}

/// Longest stretch without any of `changes` in `[0, duration]`.
pub fn longest_static(changes: &[f64], duration: f64) -> f64 {
    let mut v: Vec<f64> = changes.iter().copied().filter(|t| *t > 0.0 && *t < duration).collect();
    v.push(0.0);
    v.push(duration);
    v.sort_by(|a, b| a.total_cmp(b));
    v.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize, step: f64) -> Vec<PunchWord> {
        (0..n)
            .map(|i| PunchWord {
                text: if i % 5 == 3 { "NEVER".into() } else { "the".into() },
                start: i as f64 * step,
                end: i as f64 * step + step * 0.8,
            })
            .collect()
    }

    fn all_changes(input: &PunchInput, p: &[PlannedPunch]) -> Vec<f64> {
        let mut c = input.cuts.clone();
        c.extend(input.changes.iter());
        for x in p {
            c.push(x.start);
            c.push(x.end);
        }
        c
    }

    #[test]
    fn paces_a_talking_head_without_cuts() {
        let input = PunchInput { duration: 51.0, words: words(170, 0.3), ..Default::default() };
        let p = plan(&input, &PunchConfig::default());
        let ch = all_changes(&input, &p);
        let rate = per_10s(ch.len(), 51.0);
        // Rare: about one held punch-in every ~10 s (in + out = 2 changes), never a zoom pump.
        assert!((1.0..=2.0).contains(&rate), "rate {rate}: {p:?}");
        // No stretch is left much longer than a held punch-in's spacing (the tail can stay plain:
        // a punch-in there would sit on the clip's end).
        assert!(longest_static(&ch, 51.0) <= 10.0, "{p:?}");
        // Held for a phrase, subtle, varied; neighbours differ.
        assert!(p.iter().all(|x| x.end - x.start >= 2.5), "{p:?}");
        assert!(p.iter().all(|x| (1.06..=1.12).contains(&x.amount)), "{p:?}");
        assert!(p.windows(2).all(|w| (w[0].amount - w[1].amount).abs() >= 0.01), "{p:?}");
        // Stressed words are 1 in 5 here and the even-spacing window is 0.8 s wide: they are
        // picked more often than chance.
        let on = p.iter().filter(|x| x.word.as_deref() == Some("NEVER")).count();
        assert!(on * 4 >= p.len(), "{on} of {}", p.len());
    }

    #[test]
    fn keeps_away_from_cuts_and_blocked_spans() {
        let input = PunchInput {
            duration: 30.0,
            words: words(100, 0.3),
            cuts: vec![5.0, 12.0],
            blocked: vec![(5.0, 12.0)],
            ..Default::default()
        };
        let p = plan(&input, &PunchConfig::default());
        assert!(!p.is_empty());
        for x in &p {
            assert!(!(x.start < 12.0 && x.end > 5.0), "{x:?} in the graphic");
            for c in [5.0, 12.0] {
                assert!(x.start >= c + 0.5 || x.end <= c - 0.5, "{x:?} too close to cut {c}");
            }
        }
    }

    #[test]
    fn does_not_stack_and_respects_a_window() {
        let input = PunchInput {
            duration: 30.0,
            words: words(100, 0.3),
            existing: vec![(4.0, 5.5), (10.0, 11.5)],
            window: Some((12.0, 26.0)),
            ..Default::default()
        };
        let p = plan(&input, &PunchConfig::default());
        assert!(!p.is_empty());
        assert!(p.iter().all(|x| x.start >= 12.0 && x.end <= 26.0), "{p:?}");
        assert!(p.iter().all(|x| !overlaps((x.start, x.end), (10.0, 11.5))));
    }

    #[test]
    fn dense_cuts_need_no_punch_ins() {
        let cuts: Vec<f64> = (1..25).map(|k| k as f64 * 2.0).collect();
        let input = PunchInput { duration: 50.0, words: words(160, 0.3), cuts, ..Default::default() };
        assert!(plan(&input, &PunchConfig::default()).is_empty());
    }

    #[test]
    fn focus_follows_the_face_and_stays_inside() {
        let input = PunchInput {
            duration: 20.0,
            words: words(66, 0.3),
            focus: vec![FocusSpan { start: 0.0, end: 20.0, x: 0.05, y: 0.4 }],
            ..Default::default()
        };
        let p = plan(&input, &PunchConfig::default());
        assert!(!p.is_empty());
        for x in &p {
            let half = 0.5 / x.amount;
            assert!((x.focus_x.unwrap() - half).abs() < 0.002, "{x:?}");
            // y is pulled in the same way (0.4 is above the highest centre a zoom allows).
            assert!((x.focus_y.unwrap() - 0.4_f64.max(half)).abs() < 0.002, "{x:?}");
        }
    }

    #[test]
    fn stress_prefers_content_and_emphasis() {
        let w = vec![
            PunchWord { text: "the".into(), start: 0.0, end: 0.1 },
            PunchWord { text: "communication".into(), start: 0.1, end: 0.6 },
            PunchWord { text: "never.".into(), start: 0.6, end: 0.9 },
            PunchWord { text: "and".into(), start: 1.3, end: 1.4 },
        ];
        let m = median_per_char(&w);
        assert!(stress(&w, 0, m) < stress(&w, 1, m));
        assert!(stress(&w, 1, m) < stress(&w, 2, m));
        assert!(stress(&w, 3, m) < 1.0);
    }
}
