//! Retention map of an output clip (director layer, design §5.9).
//!
//! [`analyze`] is pure: it takes what the clip shows and says on the **rendered** timeline
//! (visual-change events, motion spans, spoken words, silences, loudness, text overlays) and
//! returns a per-second timeline, the derived metrics (first-frame hook, speech start delay,
//! dead zones, ending type, pacing, caption density) and a 0-100 retention score with its
//! per-factor breakdown ([`SCORE_FORMULA`]). [`frame_scores`] is the one measurement on a
//! render: ffmpeg's scene score for every frame (spikes = visible changes, per-second mean =
//! motion). Rustler measures and advises; the agent decides.

use crate::proc::{self, argv};
use rustler_core::LoudnessPoint;
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// A span longer than this without any visual change is a static dead zone (seconds). A talking
/// head with live captions holds attention on its own; the old 2.5 s pushed the tools into
/// zooming in and out every couple of seconds, which viewers find distracting.
pub const STATIC_GAP_S: f64 = 6.0;
/// Silence at least this long is a dead zone (seconds).
pub const SILENCE_MIN_S: f64 = 0.8;
/// A text overlay that appears before this time counts as a first-frame hook.
pub const HOOK_TEXT_BEFORE_S: f64 = 0.3;
/// Speech that starts before this time counts as a first-frame hook.
pub const FAST_SPEECH_S: f64 = 0.5;
/// ffmpeg scene score (on 240 px wide frames) above which a frame is a visible change. Measured
/// on the trial renders: camera cuts 0.27-0.52, jump cuts in a talking head 0.13-0.20, gestures
/// and caption changes below 0.10, so 0.12 catches jump cuts that 0.25 would miss.
pub const SCENE_SPIKE: f64 = 0.12;
/// Mean scene score over the first second above which the opening counts as moving.
pub const MOTION_FIRST_SECOND: f64 = 0.02;
/// Visual changes closer than this are one change.
pub const MERGE_S: f64 = 0.25;
/// Pacing target: visual changes per 10 s (about one every 6-7 s: a cut, a reframe or a held
/// punch-in, not constant zooming).
pub const PACE_TARGET_PER_10S: f64 = 1.5;

/// How the retention score is computed (also printed by `clip.retention`).
pub const SCORE_FORMULA: &str = "retention_score = hook (25) + pacing (25) + visual_dead (15) + silence (10) + captions (10) + ending (10) + energy (5). \
hook: text overlay on screen before 0.3 s = 10, speech start delay <=0.3 s = 10 / <=0.5 s = 8 / <=1 s = 4 / <=2 s = 1, motion in the first second = 5 (capped at 25). \
pacing: 25 x min(1, visual changes per 10 s / 1.5). \
visual_dead: 15 x (1 - min(1, 2 x share of the clip inside static dead zones > 6 s)). \
silence: 10 x (1 - min(1, seconds of silence >= 0.8 s / (10% of the clip, at least 1 s))). \
captions: words per second 2.0-4.5 = 10, below scales down linearly, above loses 5 per extra word/s; captions off with speech = 0; no speech = 5. \
ending: loopable 10, cta 8, sentence_final 6, hard_stop 0, no_speech 5. \
energy: 5 x (1 - min(1, 3 x share of seconds more than 8 LU below the median loudness)); no loudness data = 3.";

/// A moment the picture visibly changes (rendered timeline).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct VisualChange {
    pub t: f64,
    /// cut | jump_cut | layout_change | transition | scene_cut | effect | effect_end | text | image | cutaway | cutaway_end | frame_change
    pub kind: String,
    pub detail: String,
}

/// Text with a time range (rendered timeline): a spoken word, an overlay, a motion span.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct TimedText {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

/// Everything [`analyze`] looks at, on the rendered timeline.
#[derive(Clone, Debug, Default)]
pub struct RetentionInput {
    pub duration: f64,
    pub changes: Vec<VisualChange>,
    /// Ranges with continuous camera motion (zoom-slow, shake): never static.
    pub motion_spans: Vec<TimedText>,
    /// Spoken words (caption words), sorted by start.
    pub words: Vec<TimedText>,
    /// Measured silences; `None` = no audio data (word gaps are used instead).
    pub silences: Option<Vec<Span>>,
    /// 1 s loudness buckets; `None` = no audio data.
    pub loudness: Option<Vec<LoudnessPoint>>,
    /// Mean frame-difference score per second of a render; `None` = not measured.
    pub motion: Option<Vec<f64>>,
    /// Text overlays (hook titles, labels, text layers).
    pub overlays: Vec<TimedText>,
    pub captions_enabled: bool,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SecondView {
    /// Start of the second.
    pub t: f64,
    pub visual_changes: u32,
    /// Seconds since the last visual change at the end of this second (0 inside motion).
    pub since_visual_change: f64,
    /// Share of the second with speech (0..1).
    pub speech: f64,
    /// Share of the second inside a silence dead zone (0..1).
    pub silence: f64,
    /// Words starting in this second (caption density).
    pub words: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness_lufs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion: Option<f64>,
    /// Inside a dead zone.
    pub dead: bool,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct HookCheck {
    /// True when at least one reason holds.
    pub present: bool,
    /// text_overlay | speech | motion
    pub reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_overlay: Option<String>,
    pub motion_first_second: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_line: Option<String>,
}

#[derive(Serialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeadKind {
    /// > 2.5 s without a visual change.
    Static,
    /// >= 0.8 s of silence.
    Silence,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct DeadZone {
    pub kind: DeadKind,
    pub start: f64,
    pub end: f64,
    pub length: f64,
    /// Words spoken inside the zone (empty when silent).
    pub said: String,
}

#[derive(Serialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndingKind {
    /// Ends on a full sentence that connects back to the opening (replays flow).
    Loopable,
    /// Ends on a call to action (follow, comment, part 2, ...).
    Cta,
    /// Ends on a full sentence.
    SentenceFinal,
    /// Ends mid-sentence.
    HardStop,
    NoSpeech,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Ending {
    pub kind: EndingKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Pacing {
    pub visual_changes: u32,
    pub per_10s: f64,
    pub longest_static_s: f64,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Factor {
    pub name: String,
    pub points: f64,
    pub max: f64,
    pub detail: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct RetentionAnalysis {
    pub duration: f64,
    pub retention_score: u32,
    pub breakdown: Vec<Factor>,
    pub first_frame_hook: HookCheck,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_start_delay: Option<f64>,
    pub dead_zones: Vec<DeadZone>,
    pub ending: Ending,
    pub pacing: Pacing,
    /// Words per second over the clip; `None` without speech.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption_density: Option<f64>,
    pub silence_total_s: f64,
    /// Every visual change event, sorted (several events can fall on one merged change).
    pub changes: Vec<VisualChange>,
    /// Merged change times used for pacing and dead zones.
    pub change_times: Vec<f64>,
    pub timeline: Vec<SecondView>,
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0 + 0.0
}

/// Overlap of `[a, b]` with `[c, d]`.
fn overlap(a: f64, b: f64, c: f64, d: f64) -> f64 {
    (b.min(d) - a.max(c)).max(0.0)
}

/// `[a, b]` minus the spans (sorted pieces, each > 0).
fn subtract(a: f64, b: f64, spans: &[Span]) -> Vec<Span> {
    let mut pieces = vec![Span { start: a, end: b }];
    for s in spans {
        let mut next = Vec::new();
        for p in pieces {
            if s.end <= p.start || s.start >= p.end {
                next.push(p);
                continue;
            }
            if s.start > p.start {
                next.push(Span { start: p.start, end: s.start });
            }
            if s.end < p.end {
                next.push(Span { start: s.end, end: p.end });
            }
        }
        pieces = next;
    }
    pieces.retain(|p| p.end - p.start > 1e-9);
    pieces
}

/// Sorted change times (> 0.05 s, < duration - 0.05 s), events within [`MERGE_S`] merged.
pub fn merge_change_times(changes: &[VisualChange], duration: f64) -> Vec<f64> {
    let mut ts: Vec<f64> = changes.iter().map(|c| c.t).filter(|t| t.is_finite() && *t > 0.05 && *t < duration - 0.05).collect();
    ts.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for t in ts {
        if out.last().is_none_or(|l| t - l > MERGE_S) {
            out.push(t);
        }
    }
    out
}

/// Text of a word without trailing quotes/brackets, for punctuation tests.
fn bare_end(w: &str) -> &str {
    w.trim().trim_end_matches(['"', '\'', ')', ']', '”', '’', '»'])
}

/// The word ends a sentence.
pub fn is_terminal(word: &str) -> bool {
    bare_end(word).ends_with(['.', '?', '!', '…', '।', '॥'])
}

fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

const STOP: &[&str] = &[
    "that", "this", "with", "have", "what", "your", "they", "them", "then", "there", "their", "were", "will", "would",
    "about", "just", "like", "from", "into", "because", "when", "where", "which", "some", "been", "very", "really",
    "know", "yeah", "okay", "it's", "that's", "dont", "don't", "going", "gonna", "want", "also", "only", "more",
    "than", "even", "much", "here", "these", "those", "thing", "things", "well", "make", "made", "kind", "sort",
    "hain", "nahi", "matlab", "kya", "toh", "bhi", "aur", "ki", "hai",
];

fn content_words(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in line.split_whitespace() {
        let n = norm(w);
        if n.chars().count() >= 4 && !STOP.contains(&n.as_str()) && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

const CONNECTORS: &[&str] = &["and", "but", "so", "because", "which", "that", "thats", "or", "aur", "toh", "kyunki"];

const CTA: &[&str] = &[
    "follow for", "follow me", "follow us", "follow karo", "subscribe", "comment below", "comment karo", "drop a comment",
    "let me know in the comments", "link in bio", "link in the bio", "part 2", "part two", "full video", "full episode",
    "full podcast", "save this", "share this", "share karo", "like and", "hit like", "watch till the end",
];

/// Sentences of the word list (text, start, end), split after terminal punctuation.
fn sentences(words: &[TimedText]) -> Vec<TimedText> {
    let mut out = Vec::new();
    let mut cur: Vec<&TimedText> = Vec::new();
    for w in words {
        cur.push(w);
        if is_terminal(&w.text) {
            out.push(join(&cur));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(join(&cur));
    }
    out
}

fn join(ws: &[&TimedText]) -> TimedText {
    TimedText {
        text: ws.iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" "),
        start: ws.first().map_or(0.0, |w| w.start),
        end: ws.last().map_or(0.0, |w| w.end),
    }
}

/// Ending type of the clip from its words and the overlays near the end.
pub fn ending(words: &[TimedText], overlays: &[TimedText], duration: f64) -> Ending {
    let Some(last) = words.last() else {
        return Ending { kind: EndingKind::NoSpeech, last_line: None, evidence: vec!["no words in the clip".into()] };
    };
    let lines = sentences(words);
    let last_line = lines.last().map(|l| l.text.clone()).unwrap_or_default();
    let mut evidence = Vec::new();
    let lower = format!(" {} ", last_line.to_lowercase());
    let late_overlays: Vec<&TimedText> = overlays.iter().filter(|o| o.end >= duration - 3.0).collect();
    let cta_hit = CTA.iter().find(|p| lower.contains(*p)).map(|p| format!("last line says '{p}'")).or_else(|| {
        late_overlays.iter().find_map(|o| {
            let t = o.text.to_lowercase();
            CTA.iter().find(|p| t.contains(*p)).map(|p| format!("overlay '{}' near the end says '{p}'", o.text))
        })
    });
    let terminal = is_terminal(&last.text);
    if !terminal {
        evidence.push(format!("last word '{}' has no sentence-final punctuation", last.text.trim()));
    }
    if let Some(c) = cta_hit {
        evidence.push(c);
        return Ending { kind: EndingKind::Cta, last_line: Some(last_line), evidence };
    }
    if !terminal {
        return Ending { kind: EndingKind::HardStop, last_line: Some(last_line), evidence };
    }
    let tail = duration - last.end;
    evidence.push(format!("ends on a full sentence ({tail:.2} s after the last word)"));
    if lines.len() >= 2 {
        let first = &lines[0].text;
        let shared: Vec<String> = {
            let fw = content_words(first);
            content_words(&last_line).into_iter().filter(|w| fw.contains(w)).collect()
        };
        let opener = first.split_whitespace().next().map(norm).unwrap_or_default();
        if !shared.is_empty() {
            evidence.push(format!("last line repeats the opening's words: {}", shared.join(", ")));
            return Ending { kind: EndingKind::Loopable, last_line: Some(last_line), evidence };
        }
        if CONNECTORS.contains(&opener.as_str()) {
            evidence.push(format!("the opening starts with '{opener}', so the end can flow into it on replay"));
            return Ending { kind: EndingKind::Loopable, last_line: Some(last_line), evidence };
        }
    }
    Ending { kind: EndingKind::SentenceFinal, last_line: Some(last_line), evidence }
}

/// Retention map + score (pure).
pub fn analyze(input: &RetentionInput) -> RetentionAnalysis {
    let d = if input.duration.is_finite() { input.duration.max(0.0) } else { 0.0 };
    let mut changes: Vec<VisualChange> =
        input.changes.iter().filter(|c| c.t.is_finite() && c.t >= 0.0 && c.t <= d).cloned().collect();
    changes.sort_by(|a, b| a.t.total_cmp(&b.t));
    let change_times = merge_change_times(&changes, d);
    let motion: Vec<Span> = input
        .motion_spans
        .iter()
        .filter(|m| m.end > m.start)
        .map(|m| Span { start: m.start.max(0.0), end: m.end.min(d) })
        .collect();
    let mut words: Vec<TimedText> = input.words.iter().filter(|w| w.end > w.start && w.start < d).cloned().collect();
    words.sort_by(|a, b| a.start.total_cmp(&b.start));

    // Static dead zones: gaps between changes, minus motion spans.
    let mut bounds = vec![0.0];
    bounds.extend(change_times.iter().copied());
    bounds.push(d);
    let mut dead: Vec<DeadZone> = Vec::new();
    let mut longest_static: f64 = 0.0;
    let said = |a: f64, b: f64| -> String {
        words.iter().filter(|w| (w.start + w.end) / 2.0 >= a && (w.start + w.end) / 2.0 < b).map(|w| w.text.trim()).collect::<Vec<_>>().join(" ")
    };
    for w in bounds.windows(2) {
        for p in subtract(w[0], w[1], &motion) {
            let len = p.end - p.start;
            longest_static = longest_static.max(len);
            if len > STATIC_GAP_S {
                dead.push(DeadZone { kind: DeadKind::Static, start: r2(p.start), end: r2(p.end), length: r2(len), said: said(p.start, p.end) });
            }
        }
    }
    // Silence dead zones: measured silences, else word gaps.
    let silence_spans: Vec<Span> = match &input.silences {
        Some(s) => s
            .iter()
            .map(|s| Span { start: s.start.max(0.0), end: s.end.min(d) })
            .filter(|s| s.end - s.start >= SILENCE_MIN_S - 1e-9)
            .collect(),
        None if !words.is_empty() => {
            let mut gaps = Vec::new();
            let mut prev = 0.0;
            for w in &words {
                if w.start - prev >= SILENCE_MIN_S {
                    gaps.push(Span { start: prev, end: w.start });
                }
                prev = prev.max(w.end);
            }
            if d - prev >= SILENCE_MIN_S {
                gaps.push(Span { start: prev, end: d });
            }
            gaps
        }
        None => Vec::new(),
    };
    let silence_total: f64 = silence_spans.iter().map(|s| s.end - s.start).sum();
    for s in &silence_spans {
        dead.push(DeadZone { kind: DeadKind::Silence, start: r2(s.start), end: r2(s.end), length: r2(s.end - s.start), said: String::new() });
    }
    dead.sort_by(|a, b| a.start.total_cmp(&b.start));
    let static_secs: f64 = dead.iter().filter(|z| z.kind == DeadKind::Static).map(|z| z.length).sum();

    // Hook.
    let speech_start = words.first().map(|w| w.start);
    let hook_overlay = input.overlays.iter().find(|o| o.start < HOOK_TEXT_BEFORE_S && o.end - o.start >= 0.5 && !o.text.trim().is_empty());
    let motion_first = change_times.iter().any(|t| *t <= 1.0)
        || motion.iter().any(|m| m.start < 1.0)
        || input.motion.as_ref().and_then(|m| m.first()).is_some_and(|m| *m >= MOTION_FIRST_SECOND);
    let mut reasons = Vec::new();
    if hook_overlay.is_some() {
        reasons.push("text_overlay".to_string());
    }
    if speech_start.is_some_and(|s| s < FAST_SPEECH_S) {
        reasons.push("speech".to_string());
    }
    if motion_first {
        reasons.push("motion".to_string());
    }
    let first_line = sentences(&words).first().map(|l| l.text.clone());
    let hook = HookCheck {
        present: !reasons.is_empty(),
        reasons,
        text_overlay: hook_overlay.map(|o| o.text.clone()),
        motion_first_second: motion_first,
        first_line,
    };

    let end = ending(&words, &input.overlays, d);
    let per_10s = if d > 0.0 { change_times.len() as f64 * 10.0 / d } else { 0.0 };
    let density = (!words.is_empty() && d > 0.0).then(|| words.len() as f64 / d);

    // Score.
    let mut breakdown = Vec::new();
    let text_pts = if hook.text_overlay.is_some() { 10.0 } else { 0.0 };
    let speech_pts = match speech_start {
        Some(s) if s <= 0.3 => 10.0,
        Some(s) if s <= 0.5 => 8.0,
        Some(s) if s <= 1.0 => 4.0,
        Some(s) if s <= 2.0 => 1.0,
        _ => 0.0,
    };
    let motion_pts = if motion_first { 5.0 } else { 0.0 };
    breakdown.push(Factor {
        name: "hook".into(),
        points: f64::min(25.0, text_pts + speech_pts + motion_pts),
        max: 25.0,
        detail: format!(
            "text overlay at 0: {}; speech starts at {}; motion in the first second: {}",
            if text_pts > 0.0 { "yes" } else { "no" },
            speech_start.map_or("never".into(), |s| format!("{s:.2} s")),
            if motion_first { "yes" } else { "no" }
        ),
    });
    breakdown.push(Factor {
        name: "pacing".into(),
        points: 25.0 * (per_10s / PACE_TARGET_PER_10S).min(1.0),
        max: 25.0,
        detail: format!("{} visual changes, {per_10s:.1} per 10 s (target {PACE_TARGET_PER_10S}); longest static stretch {longest_static:.1} s", change_times.len()),
    });
    let static_frac = if d > 0.0 { static_secs / d } else { 0.0 };
    breakdown.push(Factor {
        name: "visual_dead".into(),
        points: 15.0 * (1.0 - (2.0 * static_frac).min(1.0)),
        max: 15.0,
        detail: format!("{static_secs:.1} s ({:.0}%) inside static dead zones (> {STATIC_GAP_S} s without a visual change)", static_frac * 100.0),
    });
    let silence_budget = (0.1 * d).max(1.0);
    breakdown.push(Factor {
        name: "silence".into(),
        points: 10.0 * (1.0 - (silence_total / silence_budget).min(1.0)),
        max: 10.0,
        detail: if input.silences.is_none() && words.is_empty() {
            "no audio data and no words: silence not measured".into()
        } else {
            format!(
                "{silence_total:.1} s of silence >= {SILENCE_MIN_S} s ({})",
                if input.silences.is_some() { "measured audio" } else { "word gaps; no audio data" }
            )
        },
    });
    let (cap_pts, cap_detail) = match density {
        None => (5.0, "no speech".to_string()),
        Some(_) if !input.captions_enabled => (0.0, "captions are off but the clip has speech (most viewers watch muted)".to_string()),
        Some(dn) if dn < 2.0 => (10.0 * dn / 2.0, format!("{dn:.1} words/s (sparse; 2.0-4.5 reads best)")),
        Some(dn) if dn <= 4.5 => (10.0, format!("{dn:.1} words/s")),
        Some(dn) => ((10.0 - (dn - 4.5) * 5.0).max(0.0), format!("{dn:.1} words/s (too dense to read)")),
    };
    breakdown.push(Factor { name: "captions".into(), points: cap_pts, max: 10.0, detail: cap_detail });
    let end_pts = match end.kind {
        EndingKind::Loopable => 10.0,
        EndingKind::Cta => 8.0,
        EndingKind::SentenceFinal => 6.0,
        EndingKind::HardStop => 0.0,
        EndingKind::NoSpeech => 5.0,
    };
    breakdown.push(Factor { name: "ending".into(), points: end_pts, max: 10.0, detail: format!("{:?}", end.kind).to_lowercase() });
    let (energy_pts, energy_detail) = match input.loudness.as_deref().filter(|l| !l.is_empty()) {
        None => (3.0, "no loudness data (neutral)".to_string()),
        Some(curve) => {
            let mut audible: Vec<f64> = curve.iter().map(|p| p.lufs).filter(|l| *l > -70.0).collect();
            audible.sort_by(f64::total_cmp);
            if audible.is_empty() {
                (0.0, "the clip is silent".to_string())
            } else {
                let median = audible[audible.len() / 2];
                let dips = curve.iter().filter(|p| p.lufs > -70.0 && p.lufs < median - 8.0).count();
                let frac = dips as f64 / curve.len() as f64;
                (5.0 * (1.0 - (3.0 * frac).min(1.0)), format!("{dips} of {} seconds dip more than 8 LU below the median ({median:.1} LUFS)", curve.len()))
            }
        }
    };
    breakdown.push(Factor { name: "energy".into(), points: energy_pts, max: 5.0, detail: energy_detail });
    for f in &mut breakdown {
        f.points = r1(f.points);
    }
    let total: f64 = breakdown.iter().map(|f| f.points).sum();
    let score = total.round().clamp(0.0, 100.0) as u32;

    // Per-second timeline.
    let n = d.ceil() as usize;
    let mut timeline = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (i as f64, ((i + 1) as f64).min(d));
        let span = (b - a).max(1e-9);
        let in_motion = motion.iter().any(|m| m.start <= b && m.end >= b);
        let last = change_times.iter().rev().find(|t| **t <= b).copied().unwrap_or(0.0);
        let speech: f64 = words.iter().map(|w| overlap(a, b, w.start, w.end)).sum::<f64>().min(span);
        let sil: f64 = silence_spans.iter().map(|s| overlap(a, b, s.start, s.end)).sum::<f64>().min(span);
        let loud = input.loudness.as_ref().and_then(|c| c.iter().find(|p| p.t >= a - 0.5 && p.t < a + 0.5)).map(|p| r1(p.lufs));
        timeline.push(SecondView {
            t: a,
            visual_changes: change_times.iter().filter(|t| **t >= a && **t < b).count() as u32,
            since_visual_change: if in_motion { 0.0 } else { r2(b - last) },
            speech: r2(speech / span),
            silence: r2(sil / span),
            words: words.iter().filter(|w| w.start >= a && w.start < b).count() as u32,
            loudness_lufs: loud,
            motion: input.motion.as_ref().and_then(|m| m.get(i)).map(|m| (m * 1000.0).round() / 1000.0),
            dead: dead.iter().any(|z| overlap(a, b, z.start, z.end) > 0.05),
        });
    }

    RetentionAnalysis {
        duration: r2(d),
        retention_score: score,
        breakdown,
        first_frame_hook: hook,
        speech_start_delay: speech_start.map(r2),
        dead_zones: dead,
        ending: end,
        pacing: Pacing { visual_changes: change_times.len() as u32, per_10s: r2(per_10s), longest_static_s: r2(longest_static) },
        caption_density: density.map(r2),
        silence_total_s: r2(silence_total),
        changes,
        change_times: change_times.iter().map(|t| r2(*t)).collect(),
        timeline,
    }
}

// ---------------------------------------------------------------------------------------------
// Measurement on a render
// ---------------------------------------------------------------------------------------------

/// Frame-difference measurement of a video.
#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct FrameScores {
    /// `(t, score)` of frames whose scene score exceeds the threshold, spikes within 0.2 s merged
    /// (the highest kept).
    pub spikes: Vec<(f64, f64)>,
    /// Mean scene score per second (index = second).
    pub per_second: Vec<f64>,
}

/// Parses `metadata=print` lines (`pts_time:` then `lavfi.scene_score=`) into `(t, score)`.
#[derive(Default)]
struct ScoreParser {
    last_t: Option<f64>,
    samples: Vec<(f64, f64)>,
}

impl ScoreParser {
    fn feed(&mut self, line: &str) {
        if let Some(i) = line.find("pts_time:") {
            self.last_t = line[i + 9..].split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(i) = line.find("lavfi.scene_score=") {
            if let (Some(t), Ok(s)) = (self.last_t.take(), line[i + 18..].trim().parse::<f64>()) {
                self.samples.push((t, s));
            }
        }
    }
}

/// Spikes and per-second means from `(t, score)` samples.
pub fn summarize_scores(samples: &[(f64, f64)], threshold: f64) -> FrameScores {
    let mut spikes: Vec<(f64, f64)> = Vec::new();
    for &(t, s) in samples {
        if s <= threshold {
            continue;
        }
        match spikes.last_mut() {
            Some(last) if t - last.0 <= 0.2 => {
                if s > last.1 {
                    *last = (t, s);
                }
            }
            _ => spikes.push((t, s)),
        }
    }
    let n = samples.iter().map(|(t, _)| t.floor().max(0.0) as usize + 1).max().unwrap_or(0);
    let mut sum = vec![0.0; n];
    let mut cnt = vec![0u32; n];
    for &(t, s) in samples {
        let i = t.floor().max(0.0) as usize;
        sum[i] += s;
        cnt[i] += 1;
    }
    let per_second = sum.iter().zip(&cnt).map(|(s, c)| if *c > 0 { s / f64::from(*c) } else { 0.0 }).collect();
    FrameScores { spikes, per_second }
}

/// ffmpeg scene score of every frame of `media` (downscaled to 240 px wide), summarized.
pub async fn frame_scores(paths: &ToolPaths, media: &Path, threshold: f64, cancel: &CancellationToken) -> rustler_core::Result<FrameScores> {
    let mut args = argv!["-hide_banner", "-loglevel", "info", "-nostdin", "-nostats", "-i"];
    args.push(media.as_os_str().to_owned());
    args.extend(argv!["-map", "0:v:0", "-an", "-vf", "scale=240:-2,select='gte(scene,0)',metadata=print", "-f", "null", "-"]);
    let mut parser = ScoreParser::default();
    proc::run_streaming(&paths.ffmpeg, &args, None, cancel, |line| parser.feed(line)).await.map_err(|mut e| {
        e.stage = "clip.retention".into();
        e
    })?;
    Ok(summarize_scores(&parser.samples, threshold))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start: f64, end: f64) -> TimedText {
        TimedText { text: text.into(), start, end }
    }

    fn change(t: f64) -> VisualChange {
        VisualChange { t, kind: "cut".into(), detail: String::new() }
    }

    fn words_every(n: usize, from: f64, step: f64) -> Vec<TimedText> {
        (0..n).map(|i| w(if i + 1 == n { "end." } else { "word" }, from + i as f64 * step, from + i as f64 * step + step * 0.8)).collect()
    }

    #[test]
    fn static_dead_zone_and_motion_span() {
        let input = RetentionInput { duration: 10.0, changes: vec![change(2.0)], words: words_every(30, 0.1, 0.33), captions_enabled: true, ..Default::default() };
        let a = analyze(&input);
        let st: Vec<&DeadZone> = a.dead_zones.iter().filter(|z| z.kind == DeadKind::Static).collect();
        assert_eq!(st.len(), 1, "{:?}", a.dead_zones);
        assert_eq!((st[0].start, st[0].end), (2.0, 10.0));
        assert!(!st[0].said.is_empty());
        // A zoom-slow over 4..8 leaves 2..4 and 8..10 (both <= 2.5 s): no static zone.
        let mut with_motion = input.clone();
        with_motion.motion_spans.push(w("zoom-slow", 4.0, 8.0));
        let b = analyze(&with_motion);
        assert!(b.dead_zones.iter().all(|z| z.kind != DeadKind::Static), "{:?}", b.dead_zones);
        assert!(b.retention_score > a.retention_score);
        assert_eq!(b.timeline[5].since_visual_change, 0.0);
        assert!(a.timeline[9].dead);
    }

    #[test]
    fn merge_and_pacing() {
        let cs = vec![change(1.0), change(1.1), change(3.0), change(0.0), change(9.99)];
        assert_eq!(merge_change_times(&cs, 10.0), vec![1.0, 3.0]);
    }

    #[test]
    fn hook_reasons() {
        let base = RetentionInput { duration: 6.0, words: words_every(10, 1.2, 0.4), captions_enabled: true, ..Default::default() };
        let a = analyze(&base);
        assert!(!a.first_frame_hook.present);
        assert_eq!(a.speech_start_delay, Some(1.2));
        let mut t = base.clone();
        t.overlays.push(w("He quit for this", 0.0, 3.0));
        let b = analyze(&t);
        assert_eq!(b.first_frame_hook.reasons, vec!["text_overlay"]);
        assert_eq!(b.breakdown[0].points - a.breakdown[0].points, 10.0);
        let mut m = base.clone();
        m.motion = Some(vec![0.05, 0.0]);
        assert_eq!(analyze(&m).first_frame_hook.reasons, vec!["motion"]);
        let mut s = base;
        s.words = words_every(10, 0.2, 0.4);
        assert_eq!(analyze(&s).first_frame_hook.reasons, vec!["speech"]);
    }

    #[test]
    fn silence_from_audio_or_word_gaps() {
        let mut i = RetentionInput { duration: 8.0, words: vec![w("Hi", 0.1, 0.5), w("there.", 2.0, 2.5)], captions_enabled: true, ..Default::default() };
        let a = analyze(&i);
        let sil: Vec<(f64, f64)> = a.dead_zones.iter().filter(|z| z.kind == DeadKind::Silence).map(|z| (z.start, z.end)).collect();
        assert_eq!(sil, vec![(0.5, 2.0), (2.5, 8.0)]);
        i.silences = Some(vec![Span { start: 3.0, end: 3.5 }, Span { start: 4.0, end: 5.0 }]);
        let b = analyze(&i);
        let sil: Vec<(f64, f64)> = b.dead_zones.iter().filter(|z| z.kind == DeadKind::Silence).map(|z| (z.start, z.end)).collect();
        assert_eq!(sil, vec![(4.0, 5.0)]);
    }

    #[test]
    fn endings() {
        let o: Vec<TimedText> = Vec::new();
        let hard = [w("We", 0.0, 0.2), w("never", 0.3, 0.5), w("knew", 0.6, 1.0)];
        assert_eq!(ending(&hard, &o, 1.0).kind, EndingKind::HardStop);
        let fin = [w("Money", 0.0, 0.3), w("matters.", 0.3, 0.6), w("Nothing", 0.7, 1.0), w("else.", 1.0, 1.4)];
        assert_eq!(ending(&fin, &o, 1.6).kind, EndingKind::SentenceFinal);
        let lp = [w("Discipline", 0.0, 0.3), w("wins.", 0.3, 0.6), w("Always", 0.7, 1.0), w("discipline.", 1.0, 1.4)];
        assert_eq!(ending(&lp, &o, 1.6).kind, EndingKind::Loopable);
        let cta = [w("Big", 0.0, 0.3), w("news.", 0.3, 0.6), w("Follow", 0.7, 1.0), w("for", 1.0, 1.1), w("more.", 1.1, 1.4)];
        assert_eq!(ending(&cta, &o, 1.6).kind, EndingKind::Cta);
        let ov = vec![w("Part 2 tomorrow", 0.5, 1.6)];
        assert_eq!(ending(&fin, &ov, 1.6).kind, EndingKind::Cta);
        assert_eq!(ending(&[], &o, 1.0).kind, EndingKind::NoSpeech);
    }

    #[test]
    fn parses_metadata_print_and_summarizes() {
        let mut p = ScoreParser::default();
        for l in [
            "[Parsed_metadata_2 @ 0000] frame:0    pts:0       pts_time:0",
            "[Parsed_metadata_2 @ 0000] lavfi.scene_score=0.000000",
            "[Parsed_metadata_2 @ 0000] frame:1    pts:512     pts_time:0.5",
            "[Parsed_metadata_2 @ 0000] lavfi.scene_score=0.400000",
            "[Parsed_metadata_2 @ 0000] frame:2    pts:600     pts_time:0.6",
            "[Parsed_metadata_2 @ 0000] lavfi.scene_score=0.600000",
            "[Parsed_metadata_2 @ 0000] frame:3    pts:1100    pts_time:1.5",
            "[Parsed_metadata_2 @ 0000] lavfi.scene_score=0.010000",
        ] {
            p.feed(l);
        }
        assert_eq!(p.samples.len(), 4);
        let s = summarize_scores(&p.samples, SCENE_SPIKE);
        assert_eq!(s.spikes, vec![(0.6, 0.6)]);
        assert_eq!(s.per_second.len(), 2);
        assert!((s.per_second[0] - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn score_is_bounded_and_explained() {
        let a = analyze(&RetentionInput { duration: 0.0, ..Default::default() });
        assert!(a.retention_score <= 100);
        assert_eq!(a.breakdown.len(), 7);
        assert_eq!(a.breakdown.iter().map(|f| f.max).sum::<f64>(), 100.0);
    }
}
