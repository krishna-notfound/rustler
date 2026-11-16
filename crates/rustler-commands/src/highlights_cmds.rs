//! Commands: `highlights.signals`, `highlights.candidates`, `transcript.read_window` (feature f20).
//!
//! Deterministic evidence for the driving agent; no AI calls. Everything is computed from files that
//! `analyze quick` already wrote under `analysis/<source_id>/`:
//! `transcript.json` + `units.json`, `audio_events.json` (loudness curve, silences),
//! `scenes.json`, and optionally `source_info.json` (a saved `source.info` result with a `heatmap`).
//! A missing optional file makes its fields `null` and adds a warning naming the command to run.
//!
//! The agent decides which candidates become clips; scores are a way to read less, not a verdict.

use crate::project_cmds::{mmss, validation};
use crate::transcript_cmds::{analysis_rel, load_transcript, load_units, unit_line};
use rustler_core::{AudioEvents, ProjectStore, Scenes, SourceId, SourceOrigin, Unit, Word};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

// ---------------------------------------------------------------------------------------
// Word lists (data, EN + romanized Hinglish + a few Devanagari)
// ---------------------------------------------------------------------------------------

/// Words that signal emphasis, a claim or a hook when they appear in speech.
pub const EMPHASIS_WORDS: &[&str] = &[
    "never", "secret", "secrets", "crazy", "insane", "biggest", "mistake", "mistakes", "truth", "actually", "worst", "best",
    "shocking", "nobody", "everyone", "always", "literally", "unbelievable", "wrong", "million", "billion", "free", "stop",
    "warning", "dangerous", "exposed", "scam", "lie", "lies", "hidden", "impossible", "obsessed",
    // Hinglish (romanized)
    "bhai", "sach", "sachi", "pagal", "kasam", "jhooth", "galat", "bakwas", "kamaal", "zabardast", "dhoka", "raaz", "sabse",
    "kabhi", "lakh", "crore", "khatarnak",
    // Devanagari spellings of the strongest ones
    "भाई", "सच", "पागल", "कसम", "गलत", "झूठ", "कभी",
];

/// Single words that are weak openers (filler, connectors, acknowledgements).
pub const WEAK_SINGLE: &[&str] = &[
    // English
    "so", "and", "but", "um", "uh", "umm", "uhh", "like", "yeah", "yep", "okay", "ok", "right", "well", "anyway", "anyways",
    "basically", "hmm", "erm", "then",
    // Hinglish
    "toh", "matlab", "haan", "han", "na", "accha", "acha", "dekho", "yaar", "bas", "aur", "phir", "woh", "wo", "arre",
    "ki", "hm",
];

/// Multi-word weak openers (checked before single words).
pub const WEAK_PHRASES: &[&[&str]] = &[
    &["the", "thing", "is"],
    &["i", "mean"],
    &["you", "know"],
    &["so", "yeah"],
    &["and", "then"],
    &["matlab", "ki"],
    &["toh", "basically"],
    &["you", "see"],
    &["kind", "of"],
    &["sort", "of"],
];

/// Words that begin a sentence by pointing at something said earlier (needs context).
const PRONOUN_STARTS: &[&str] = &["he", "she", "they", "it", "that", "this", "those", "these", "him", "her", "woh", "wo", "usne", "unhone", "yeh", "ye"];

/// Lowercase letters/digits only (`"Great!"` -> `"great"`).
pub fn tok(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Normalized tokens of a text.
pub fn tokens_of(text: &str) -> Vec<String> {
    text.split_whitespace().map(tok).filter(|t| !t.is_empty()).collect()
}

/// How many leading tokens form a weak opener (0 = none). Consumes up to 3 consecutive weak
/// phrases/words (`"so yeah basically ..."` -> 3). `"right now"` is not an opener.
pub fn weak_opener_len(tokens: &[String]) -> usize {
    let mut pos = 0usize;
    for _ in 0..3 {
        let rest = &tokens[pos.min(tokens.len())..];
        if rest.is_empty() {
            break;
        }
        if let Some(p) = WEAK_PHRASES.iter().filter(|p| rest.len() >= p.len() && p.iter().zip(rest).all(|(a, b)| a == b)).max_by_key(|p| p.len()) {
            pos += p.len();
            continue;
        }
        let first = rest[0].as_str();
        if first == "right" && rest.get(1).is_some_and(|n| n == "now") {
            break;
        }
        if WEAK_SINGLE.contains(&first) {
            pos += 1;
            continue;
        }
        break;
    }
    pos
}

// ---------------------------------------------------------------------------------------
// Loaded analysis
// ---------------------------------------------------------------------------------------

/// Everything the feature extraction reads, loaded once per command call.
pub struct Data {
    pub words: Vec<Word>,
    pub units: Vec<Unit>,
    /// Loudness curve `(t, lufs)` sorted by t, near-silence (<= -70) dropped. None = no audio_events.json.
    pub curve: Option<Vec<(f64, f64)>>,
    pub loud_median: Option<f64>,
    /// Scene start times (cuts). None = no scenes.json.
    pub scene_starts: Option<Vec<f64>>,
    /// `(start, end, value)` in the *original* video's time. None = no saved heatmap.
    pub heat: Option<Vec<(f64, f64, f64)>>,
    /// Added to source time to get original-video time (url sources imported with `--sections`).
    pub heat_offset: f64,
    pub duration: f64,
    /// Median words/minute over `window`-second windows that contain speech.
    pub wpm_median: Option<f64>,
}

fn read_opt<T: serde::de::DeserializeOwned>(store: &ProjectStore, rel: &Path) -> rustler_core::Result<Option<T>> {
    if store.resolve(rel).is_file() { Ok(Some(store.read_json(rel)?)) } else { Ok(None) }
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

fn round(x: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (x * m).round() / m
}

fn parse_heat(v: &Value) -> Option<Vec<(f64, f64, f64)>> {
    let arr = v.get("heatmap")?.as_array()?;
    let f = |x: &Value, a: &str, b: &str| x.get(a).and_then(Value::as_f64).or_else(|| x.get(b).and_then(Value::as_f64));
    let out: Vec<(f64, f64, f64)> = arr
        .iter()
        .filter_map(|h| Some((f(h, "start", "start_time")?, f(h, "end", "end_time")?, h.get("value")?.as_f64()?)))
        .collect();
    if out.is_empty() { None } else { Some(out) }
}

/// Loads transcript (optional only when `need_transcript` is false), audio events, scenes and heatmap.
pub fn load_data(store: &ProjectStore, id: &str, window: f64, warnings: &mut Vec<String>, need_transcript: bool) -> rustler_core::Result<Data> {
    let dir = analysis_rel(id)?;
    let has_transcript = store.resolve(&dir.join("transcript.json")).is_file();
    let (words, units) = if has_transcript || need_transcript {
        let t = load_transcript(store, id)?;
        (t.words, load_units(store, id)?)
    } else {
        warnings.push(format!("no transcript: word-based fields are null; run `rustler analyze transcribe {id}` (or `rustler analyze quick {id}`)"));
        (Vec::new(), Vec::new())
    };

    let audio: Option<AudioEvents> = read_opt(store, &dir.join("audio_events.json"))?;
    let (curve, loud_median) = match audio {
        Some(a) => {
            let mut c: Vec<(f64, f64)> = a.curve.iter().filter(|p| p.lufs.is_finite() && p.lufs > -70.0).map(|p| (p.t, p.lufs)).collect();
            c.sort_by(|x, y| x.0.total_cmp(&y.0));
            if c.is_empty() {
                warnings.push(format!("audio_events.json has no loudness curve: loudness fields are null; rerun `rustler analyze audio {id} --force`"));
                (None, None)
            } else {
                let mut l: Vec<f64> = c.iter().map(|p| p.1).collect();
                let m = median(&mut l);
                (Some(c), m)
            }
        }
        None => {
            warnings.push(format!("no audio_events.json: loudness fields are null; run `rustler analyze audio {id}` (or `rustler analyze quick {id}`)"));
            (None, None)
        }
    };

    let scenes: Option<Scenes> = read_opt(store, &dir.join("scenes.json"))?;
    let scene_starts = match scenes {
        Some(s) => Some(s.scenes.iter().map(|x| x.start).filter(|t| *t > 0.05).collect::<Vec<_>>()),
        None => {
            warnings.push(format!("no scenes.json: scene_cuts is null; run `rustler analyze scenes {id}` (or `rustler analyze quick {id}`)"));
            None
        }
    };

    let source = id.parse::<SourceId>().ok().and_then(|sid| store.load_source(&sid).ok());
    let mut heat = None;
    let mut heat_offset = 0.0;
    let info: Option<Value> = read_opt(store, &dir.join("source_info.json"))?;
    match (&info, &source) {
        (Some(v), _) => {
            heat = parse_heat(v);
            if heat.is_none() {
                warnings.push("source_info.json has no `heatmap` windows: heatmap is null (the video has no most-replayed data)".to_string());
            }
        }
        (None, Some(s)) if matches!(s.origin, SourceOrigin::Url { .. }) => warnings.push(format!(
            "heatmap is null: this source came from a URL but its `rustler source info <url>` result was not saved; save that JSON as analysis/{id}/source_info.json (key `heatmap`) to enable it"
        )),
        _ => {}
    }
    if let Some(SourceOrigin::Url { original_offset, .. }) = source.as_ref().map(|s| &s.origin) {
        heat_offset = *original_offset;
    }

    let mut duration = words.last().map_or(0.0, |w| w.end);
    if let Some(s) = &source {
        duration = duration.max(s.probe.duration);
    }
    if let Some(c) = &curve {
        duration = duration.max(c.last().map_or(0.0, |p| p.0 + 1.0));
    }

    let mut d = Data { words, units, curve, loud_median, scene_starts, heat, heat_offset, duration, wpm_median: None };
    let mut rates = Vec::new();
    let mut t = 0.0;
    while t < d.duration {
        let e = (t + window).min(d.duration);
        if e - t >= 5.0 {
            let f = features(&d, t, e);
            if f.words >= 3 {
                rates.push(f.words as f64 / (e - t) * 60.0);
            }
        }
        t += window;
    }
    d.wpm_median = median(&mut rates);
    Ok(d)
}

// ---------------------------------------------------------------------------------------
// Features
// ---------------------------------------------------------------------------------------

/// Deterministic signals for one time range. `None` = the needed analysis file is missing.
#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct Features {
    pub words: usize,
    /// Words per minute over the range (words / range length).
    pub wpm: Option<f64>,
    /// `wpm` divided by the median wpm of the whole source.
    pub wpm_vs_median: Option<f64>,
    /// Loudest 1 s bucket in the range (LUFS).
    pub loudness_peak_lufs: Option<f64>,
    /// Loudest bucket minus the source's median loudness (LU).
    pub loudness_peak_lu: Option<f64>,
    pub questions: u32,
    pub exclamations: u32,
    /// Distinct emphasis words spoken (see the EN + Hinglish list).
    pub emphasis_words: Vec<String>,
    /// Total emphasis-word occurrences.
    pub emphasis_count: u32,
    /// Seconds of silence between the previous word and the first word in the range.
    pub pause_before: Option<f64>,
    /// Scene cuts inside the range.
    pub scene_cuts: Option<u32>,
    /// Mean "most replayed" heatmap value (0..1) over the range.
    pub heatmap: Option<f64>,
}

/// Index range `[lo, hi)` of the words whose midpoint lies in `[start, end)`.
fn word_range(words: &[Word], start: f64, end: f64) -> (usize, usize) {
    let lo = words.partition_point(|w| w.end < start);
    let mut first = None;
    let mut last = lo;
    for (k, w) in words.iter().enumerate().skip(lo) {
        if w.start >= end {
            break;
        }
        let mid = (w.start + w.end) / 2.0;
        if mid >= start && mid < end {
            first.get_or_insert(k);
            last = k + 1;
        }
    }
    (first.unwrap_or(lo), if first.is_some() { last } else { lo })
}

/// Computes every signal for `[start, end)`.
pub fn features(d: &Data, start: f64, end: f64) -> Features {
    let len = (end - start).max(1e-6);
    let (lo, hi) = word_range(&d.words, start, end);
    let ws = &d.words[lo..hi];
    let mut f = Features { words: ws.len(), ..Default::default() };
    if !ws.is_empty() {
        let wpm = ws.len() as f64 / len * 60.0;
        f.wpm = Some(round(wpm, 1));
        f.wpm_vs_median = d.wpm_median.filter(|m| *m > 0.0).map(|m| round(wpm / m, 2));
        let prev_end = if lo > 0 { d.words[lo - 1].end } else { 0.0 };
        f.pause_before = Some(round((ws[0].start - prev_end).max(0.0), 2));
    }
    let mut emph: Vec<String> = Vec::new();
    for w in ws {
        f.questions += w.text.chars().filter(|c| *c == '?' || *c == '？').count() as u32;
        f.exclamations += w.text.chars().filter(|c| *c == '!').count() as u32;
        let t = tok(&w.text);
        if EMPHASIS_WORDS.contains(&t.as_str()) {
            f.emphasis_count += 1;
            if !emph.contains(&t) {
                emph.push(t);
            }
        }
    }
    f.emphasis_words = emph;
    if let (Some(c), Some(m)) = (&d.curve, d.loud_median) {
        let a = c.partition_point(|p| p.0 < start - 0.999);
        let peak = c[a..].iter().take_while(|p| p.0 < end).map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        if peak.is_finite() {
            f.loudness_peak_lufs = Some(round(peak, 1));
            f.loudness_peak_lu = Some(round(peak - m, 1));
        }
    }
    if let Some(s) = &d.scene_starts {
        f.scene_cuts = Some(s.iter().filter(|t| **t > start && **t <= end).count() as u32);
    }
    if let Some(h) = &d.heat {
        let (a, b) = (start + d.heat_offset, end + d.heat_offset);
        let (mut num, mut den) = (0.0, 0.0);
        for (hs, he, v) in h {
            let o = (he.min(b) - hs.max(a)).max(0.0);
            num += o * v;
            den += o;
        }
        if den > 0.0 {
            f.heatmap = Some(round(num / den, 3));
        }
    }
    f
}

// ---------------------------------------------------------------------------------------
// highlights.signals
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SignalsIn {
    pub source_id: String,
    /// Window length in seconds. Default 30.
    pub window: Option<f64>,
    /// Skip this many windows. Default 0.
    pub offset: Option<usize>,
    /// Max windows returned. Default 200.
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SignalRow {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub mmss: String,
    /// First/last unit whose midpoint falls in the window (null when nothing is spoken).
    pub unit_start: Option<u32>,
    pub unit_end: Option<u32>,
    #[serde(flatten)]
    pub signals: Features,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Baselines {
    /// Median loudness (LUFS) of the 1 s buckets.
    pub loudness_median_lufs: Option<f64>,
    /// Median words/minute over windows containing speech.
    pub wpm_median: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Availability {
    pub transcript: bool,
    pub audio_events: bool,
    pub scenes: bool,
    pub heatmap: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SignalsOut {
    pub source_id: String,
    pub window: f64,
    pub duration: f64,
    pub total_windows: usize,
    pub offset: usize,
    pub returned: usize,
    pub next_offset: Option<usize>,
    pub baselines: Baselines,
    pub available: Availability,
    pub rows: Vec<SignalRow>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn availability(d: &Data) -> Availability {
    Availability { transcript: !d.words.is_empty(), audio_events: d.curve.is_some(), scenes: d.scene_starts.is_some(), heatmap: d.heat.is_some() }
}

fn units_in(units: &[Unit], start: f64, end: f64) -> (Option<u32>, Option<u32>) {
    let mut it = units.iter().filter(|u| {
        let mid = (u.start + u.end) / 2.0;
        mid >= start && mid < end
    });
    let first = it.next().map(|u| u.id);
    let last = it.last().map(|u| u.id).or(first);
    (first, last)
}

pub async fn signals(ctx: Ctx, input: SignalsIn) -> rustler_core::Result<SignalsOut> {
    let store = ctx.store()?;
    let window = input.window.unwrap_or(30.0);
    if !window.is_finite() || window < 5.0 {
        return Err(validation("highlights.signals", format!("window {window} is too small"), "use --window 30 (seconds, at least 5)"));
    }
    let limit = input.limit.unwrap_or(200).max(1);
    let offset = input.offset.unwrap_or(0);
    let mut warnings = Vec::new();
    let d = load_data(&store, &input.source_id, window, &mut warnings, false)?;
    if d.duration <= 0.0 {
        return Err(validation(
            "highlights.signals",
            format!("source {} has no transcript, audio analysis or duration to window", input.source_id),
            format!("run `rustler analyze quick {}` first", input.source_id),
        ));
    }
    let total = (d.duration / window).ceil() as usize;
    let rows: Vec<SignalRow> = (offset..total.min(offset + limit))
        .map(|k| {
            let (s, e) = (k as f64 * window, ((k + 1) as f64 * window).min(d.duration));
            let (us, ue) = units_in(&d.units, s, e);
            SignalRow { index: k, start: s, end: e, mmss: mmss(s), unit_start: us, unit_end: ue, signals: features(&d, s, e) }
        })
        .collect();
    let end_idx = offset + rows.len();
    let next_offset = (end_idx < total).then_some(end_idx);
    let mut next = Vec::new();
    if let Some(n) = next_offset {
        next.push(format!("more windows: rerun with --offset {n}"));
    }
    next.push(format!("rustler highlights candidates {}   (sentence-aligned windows scored from these signals)", input.source_id));
    Ok(SignalsOut {
        source_id: input.source_id,
        window,
        duration: round(d.duration, 2),
        total_windows: total,
        offset,
        returned: rows.len(),
        next_offset,
        baselines: Baselines { loudness_median_lufs: d.loud_median.map(|m| round(m, 1)), wpm_median: d.wpm_median.map(|m| round(m, 1)) },
        available: availability(&d),
        rows,
        warnings,
        next,
    })
}

// ---------------------------------------------------------------------------------------
// highlights.candidates
// ---------------------------------------------------------------------------------------

/// Score formula, shown verbatim in the command output (`scoring.formula`).
pub const SCORE_FORMULA: &str = "score = 100 * sum(weight_i * value_i) / sum(weight_i over features that could be measured); value_i in 0..1. \
Features and weights: hook 25 (opening unit: base 0.2, +0.4 question, +0.3 emphasis word, +0.2 digit, +0.2 pause >= 0.8 s before it, -0.3 starts with a pronoun that needs context), \
energy 20 (loudest second above the median loudness: value = LU/8, capped at 1), \
language 14 (emphasis words + questions + 0.5*exclamations per 30 s: value = rate/3, capped at 1), \
completeness 12 (0.5 * boundary strength of the last unit + 0.5 * strength of the boundary before the first unit), \
heatmap 9 (mean most-replayed value, null for non-URL sources), \
pace 8 (wpm vs the source median: 0 at 0.8x, 1 at 1.3x), \
length_fit 8 (1 inside the platform's sweet spot, falling to 0 at 15 s outside it), \
scenes 4 (scene cuts per 10 s, capped at 1). \
Intro trailer: when the source opens with a montage of lines said again later (see intro_trailer), windows overlapping it are not ranked (include_intro: true keeps them) \
and a window holding the original of a trailer line gets +5 points (trailer_line: the channel's editors picked that line). \
Windows that start with a weak opener (so/and/um/like/toh/matlab/...) are never generated. \
Selection: best window per start unit, then greedy by score with a diversity penalty of up to 15 points for nearness to an already chosen window; windows may not overlap in time.";

#[derive(Deserialize, JsonSchema, Debug)]
pub struct CandidatesIn {
    pub source_id: String,
    /// How many candidates to return. Default 10.
    pub count: Option<usize>,
    /// Shortest window in seconds. Default 15.
    pub min_len: Option<f64>,
    /// Longest window in seconds. Default 60.
    pub max_len: Option<f64>,
    /// `shorts` (default; sweet spot 20-45 s), `reels` (15-45 s) or `tiktok` (25-60 s): only affects the length_fit score.
    pub platform: Option<String>,
    /// Also rank windows inside the source's intro trailer (default false: they are skipped and reported in `intro_trailer`).
    pub include_intro: Option<bool>,
}

/// The source's opening montage and what ranking did with it.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct IntroReport {
    #[serde(flatten)]
    pub trailer: crate::intro_trailer::IntroTrailer,
    /// True when windows overlapping it were left out (false with include_intro: true).
    pub excluded: bool,
    /// Start units whose best window overlapped the trailer and was left out.
    pub skipped_windows: usize,
    /// What happened, in words.
    pub note: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Part {
    pub feature: String,
    /// 0..1, null when the analysis needed for it is missing (the weight is then ignored).
    pub value: Option<f64>,
    pub weight: f64,
    pub points: Option<f64>,
    pub note: String,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Candidate {
    pub rank: usize,
    pub start_unit: u32,
    pub end_unit: u32,
    /// Unit-aligned times (clip.create pads them slightly).
    pub start: f64,
    pub end: f64,
    pub mmss: String,
    pub duration: f64,
    pub score: f64,
    /// Points subtracted for being near an already chosen candidate.
    pub diversity_penalty: f64,
    pub breakdown: Vec<Part>,
    pub reasons: Vec<String>,
    pub opening_text: String,
    pub closing_text: String,
    pub signals: Features,
    /// Ready to run; change `--name`.
    pub create_cmd: String,
    /// Read the whole window before deciding.
    pub read_cmd: String,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Scoring {
    pub formula: String,
    pub platform: String,
    pub sweet_spot_s: (f64, f64),
    pub baselines: Baselines,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct CandidatesOut {
    pub source_id: String,
    pub count_requested: usize,
    pub returned: usize,
    /// Distinct start units that produced at least one valid window.
    pub windows_considered: usize,
    pub skipped_weak_openers: usize,
    pub scoring: Scoring,
    pub available: Availability,
    pub candidates: Vec<Candidate>,
    /// The source's intro trailer (a montage of lines said again later), when one was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro_trailer: Option<IntroReport>,
    pub warnings: Vec<String>,
    pub note: String,
    pub next: Vec<String>,
}

fn sweet_spot(platform: &str) -> Option<(f64, f64)> {
    match platform {
        "shorts" => Some((20.0, 45.0)),
        "reels" => Some((15.0, 45.0)),
        "tiktok" => Some((25.0, 60.0)),
        _ => None,
    }
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

struct Scored {
    i: usize,
    j: usize,
    score: f64,
    parts: Vec<Part>,
    feats: Features,
}

fn ends_terminal(text: &str) -> bool {
    text.trim_end_matches(|c: char| "\"'”’)]}»".contains(c) || c.is_whitespace())
        .chars()
        .next_back()
        .is_some_and(|c| ".?!।॥…".contains(c))
}

/// Scores units `i..=j` (see [`SCORE_FORMULA`]).
fn score_window(d: &Data, i: usize, j: usize, sweet: (f64, f64)) -> Scored {
    let (first, last) = (&d.units[i], &d.units[j]);
    let (start, end) = (first.start, last.end);
    let dur = end - start;
    let f = features(d, start, end);
    let toks = tokens_of(&first.text);
    let first_unit_f = features(d, first.start, first.end);

    // hook
    let mut hook = 0.2;
    let mut hook_note: Vec<String> = Vec::new();
    if first.text.contains('?') {
        hook += 0.4;
        hook_note.push("opens with a question".into());
    }
    if first_unit_f.emphasis_count > 0 {
        hook += 0.3;
        hook_note.push(format!("emphasis word in the first line ({})", first_unit_f.emphasis_words.join(", ")));
    }
    if first.text.chars().any(|c| c.is_ascii_digit()) {
        hook += 0.2;
        hook_note.push("number in the first line".into());
    }
    if f.pause_before.is_some_and(|p| p >= 0.8) {
        hook += 0.2;
        hook_note.push(format!("{:.1} s pause before it", f.pause_before.unwrap_or(0.0)));
    }
    if toks.first().is_some_and(|t| PRONOUN_STARTS.contains(&t.as_str())) {
        hook -= 0.3;
        hook_note.push(format!("starts with '{}' (may need earlier context)", toks[0]));
    }
    let hook = clamp01(hook);

    let energy = f.loudness_peak_lu.map(|lu| clamp01(lu / 8.0));
    let rate = (f.emphasis_count as f64 + f.questions as f64 + 0.5 * f.exclamations as f64) * 30.0 / dur.max(1.0);
    let language = clamp01(rate / 3.0);
    let prev_strength = if i == 0 { 1.0 } else { d.units[i - 1].boundary_strength as f64 };
    let mut end_strength = last.boundary_strength as f64;
    if !ends_terminal(&last.text) && end_strength > 0.8 {
        end_strength = 0.8;
    }
    let completeness = 0.5 * end_strength + 0.5 * prev_strength;
    let heat = f.heatmap.map(clamp01);
    let pace = f.wpm_vs_median.map(|r| clamp01((r - 0.8) / 0.5));
    let dist = if dur < sweet.0 { sweet.0 - dur } else if dur > sweet.1 { dur - sweet.1 } else { 0.0 };
    let length_fit = clamp01(1.0 - dist / 15.0);
    let scenes = f.scene_cuts.map(|c| clamp01(c as f64 * 10.0 / dur.max(1.0)));

    let spec: [(&str, f64, Option<f64>, String); 8] = [
        ("hook", 25.0, Some(hook), if hook_note.is_empty() { "plain opening line".into() } else { hook_note.join("; ") }),
        ("energy", 20.0, energy, f.loudness_peak_lu.map_or("no audio_events.json".into(), |lu| format!("loudest second {lu:+.1} LU vs median"))),
        ("language", 14.0, Some(language), format!("{} emphasis, {} questions, {} exclamations", f.emphasis_count, f.questions, f.exclamations)),
        ("completeness", 12.0, Some(completeness), format!("end boundary {end_strength:.1}, start boundary {prev_strength:.1}")),
        ("heatmap", 9.0, heat, f.heatmap.map_or("no saved source info heatmap".into(), |h| format!("most-replayed value {h:.2}"))),
        ("pace", 8.0, pace, f.wpm.map_or("no words".into(), |w| format!("{w:.0} wpm ({}x median)", f.wpm_vs_median.map_or("?".into(), |r| format!("{r:.2}"))))),
        ("length_fit", 8.0, Some(length_fit), format!("{dur:.0} s vs sweet spot {:.0}-{:.0} s", sweet.0, sweet.1)),
        ("scenes", 4.0, scenes, f.scene_cuts.map_or("no scenes.json".into(), |c| format!("{c} scene cuts"))),
    ];
    let wsum: f64 = spec.iter().filter(|s| s.2.is_some()).map(|s| s.1).sum();
    let mut total = 0.0;
    let parts: Vec<Part> = spec
        .into_iter()
        .map(|(name, weight, value, note)| {
            let points = value.map(|v| v * weight * 100.0 / wsum);
            total += points.unwrap_or(0.0);
            Part { feature: name.into(), value: value.map(|v| round(v, 3)), weight, points: points.map(|p| round(p, 1)), note }
        })
        .collect();
    Scored { i, j, score: round(total, 1), parts, feats: f }
}

/// Points a window gets for holding the original of a line the source's intro trailer features.
pub const TRAILER_BONUS: f64 = 5.0;

fn reasons_for(s: &Scored, units: &[Unit]) -> Vec<String> {
    let f = &s.feats;
    let mut r = Vec::new();
    if let Some(p) = s.parts.iter().find(|p| p.feature == "trailer_line") {
        r.push(p.note.clone());
    }
    let note = |name: &str| s.parts.iter().find(|p| p.feature == name).map(|p| p.note.clone()).unwrap_or_default();
    let val = |name: &str| s.parts.iter().find(|p| p.feature == name).and_then(|p| p.value);
    if val("hook").is_some_and(|v| v >= 0.6) {
        r.push(format!("strong opening: {}", note("hook")));
    } else if note("hook").contains("context") {
        r.push(format!("check the opening: {}", note("hook")));
    }
    if let Some(lu) = f.loudness_peak_lu.filter(|l| *l >= 4.0) {
        r.push(format!("loudness peak {lu:+.1} LU above the source median"));
    }
    if let Some(ratio) = f.wpm_vs_median.filter(|r| *r >= 1.15) {
        r.push(format!("fast delivery: {:.0} wpm ({ratio:.2}x median)", f.wpm.unwrap_or(0.0)));
    }
    if !f.emphasis_words.is_empty() {
        r.push(format!("emphasis words: {}", f.emphasis_words.join(", ")));
    }
    if f.questions > 0 {
        r.push(format!("{} question(s)", f.questions));
    }
    if f.exclamations > 0 {
        r.push(format!("{} exclamation(s)", f.exclamations));
    }
    if let Some(h) = f.heatmap.filter(|h| *h >= 0.6) {
        r.push(format!("most-replayed heatmap value {h:.2}"));
    }
    if f.scene_cuts.is_some_and(|c| c >= 2) {
        r.push(format!("{} scene cuts (edited, busy footage)", f.scene_cuts.unwrap_or(0)));
    }
    if units[s.j].boundary_strength >= 0.99 && ends_terminal(&units[s.j].text) {
        r.push("ends on a complete sentence".into());
    }
    if r.is_empty() {
        r.push("no strong signal: chosen mostly for sentence-aligned structure; read it before deciding".into());
    }
    r
}

pub async fn candidates(ctx: Ctx, input: CandidatesIn) -> rustler_core::Result<CandidatesOut> {
    const STAGE: &str = "highlights.candidates";
    let store = ctx.store()?;
    let count = input.count.unwrap_or(10);
    let (min_len, max_len) = (input.min_len.unwrap_or(15.0), input.max_len.unwrap_or(60.0));
    let platform = input.platform.clone().unwrap_or_else(|| "shorts".into()).to_ascii_lowercase();
    if count == 0 || count > 100 {
        return Err(validation(STAGE, format!("count {count} is out of range"), "use --count 1..100 (default 10)"));
    }
    if !(min_len.is_finite() && max_len.is_finite()) || min_len <= 0.0 || max_len <= min_len {
        return Err(validation(STAGE, format!("need 0 < min_len < max_len, got {min_len} and {max_len}"), "defaults are --min-len 15 --max-len 60"));
    }
    let sweet = sweet_spot(&platform)
        .ok_or_else(|| validation(STAGE, format!("unknown platform `{platform}`"), "use shorts, reels or tiktok"))?;
    let mut warnings = Vec::new();
    let d = load_data(&store, &input.source_id, 30.0, &mut warnings, true)?;
    if d.units.is_empty() {
        return Err(validation(
            STAGE,
            format!("source {} has no speech units", input.source_id),
            "no transcript text to window: use `rustler highlights signals` for audio-only signals and `rustler see tiles` to look",
        ));
    }

    // 1. best window per start unit
    let n = d.units.len();
    let mut skipped_weak = 0usize;
    let mut best_per_start: Vec<Scored> = Vec::new();
    for i in 0..n {
        let toks = tokens_of(&d.units[i].text);
        if weak_opener_len(&toks) > 0 {
            skipped_weak += 1;
            continue;
        }
        let mut best: Option<Scored> = None;
        for j in i..n {
            if j > i && d.units[j].start - d.units[j - 1].end > 4.0 {
                break; // a long gap is not continuous speech
            }
            let dur = d.units[j].end - d.units[i].start;
            if dur > max_len {
                break;
            }
            if dur < min_len || d.units[j].boundary_strength < 0.8 {
                continue;
            }
            let s = score_window(&d, i, j, sweet);
            if best.as_ref().is_none_or(|b| s.score > b.score) {
                best = Some(s);
            }
        }
        best_per_start.extend(best);
    }
    let considered = best_per_start.len();

    // 1b. intro trailer: skip windows in the opening montage, reward the originals of its lines.
    let include_intro = input.include_intro.unwrap_or(false);
    let mut intro = crate::intro_trailer::detect(&d.words, d.duration, d.scene_starts.as_deref()).map(|t| IntroReport {
        trailer: t,
        excluded: !include_intro,
        skipped_windows: 0,
        note: String::new(),
    });
    if let Some(rep) = intro.as_mut() {
        let (ts, te) = (rep.trailer.start, rep.trailer.end);
        let before = best_per_start.len();
        if !include_intro {
            best_per_start.retain(|c| d.units[c.j].end <= ts + 1e-9 || d.units[c.i].start >= te - 1e-9);
        }
        rep.skipped_windows = before - best_per_start.len();
        let mut boosted = 0usize;
        for c in &mut best_per_start {
            let (cs, ce) = (d.units[c.i].start, d.units[c.j].end);
            if cs < te {
                continue;
            }
            if let Some(r) = crate::intro_trailer::original_inside(&rep.trailer, cs, ce) {
                c.score = round((c.score + TRAILER_BONUS).min(100.0), 1);
                c.parts.push(Part {
                    feature: "trailer_line".into(),
                    value: Some(1.0),
                    weight: 0.0,
                    points: Some(TRAILER_BONUS),
                    note: format!("the source's intro trailer features this line at {} (\"{}\")", mmss(r.early_start), r.text),
                });
                boosted += 1;
            }
        }
        rep.note = if include_intro {
            format!(
                "intro trailer {}-{} kept in the ranking (include_intro); {boosted} windows hold the original of a trailer line (+{TRAILER_BONUS} points)",
                mmss(ts),
                mmss(te)
            )
        } else {
            format!(
                "intro trailer {}-{} skipped: {} windows overlapping it were not ranked (the same lines play later, uncut); {boosted} windows hold the original of a trailer line (+{TRAILER_BONUS} points); pass include_intro: true to rank the trailer too",
                mmss(ts),
                mmss(te),
                rep.skipped_windows
            )
        };
        warnings.push(rep.note.clone());
    }

    // 2. greedy selection: no overlap, diversity penalty for proximity
    let spread = (d.duration / (2.0 * count as f64)).max(30.0);
    let mut pool = best_per_start;
    let mut chosen: Vec<(Scored, f64)> = Vec::new();
    while chosen.len() < count && !pool.is_empty() {
        let mut best: Option<(usize, f64, f64)> = None;
        for (k, c) in pool.iter().enumerate() {
            let (cs, ce) = (d.units[c.i].start, d.units[c.j].end);
            let mut prox = 0.0f64;
            for (o, _) in &chosen {
                let (os, oe) = (d.units[o.i].start, d.units[o.j].end);
                let gap = (cs - oe).max(os - ce);
                prox = prox.max(clamp01(1.0 - gap.max(0.0) / spread));
            }
            let penalty = 15.0 * prox;
            let adj = c.score - penalty;
            if best.is_none_or(|b| adj > b.1) {
                best = Some((k, adj, penalty));
            }
        }
        let Some((k, _, penalty)) = best else { break };
        let pick = pool.swap_remove(k);
        let (ps, pe) = (d.units[pick.i].start, d.units[pick.j].end);
        pool.retain(|c| d.units[c.j].end <= ps + 1e-9 || d.units[c.i].start >= pe - 1e-9);
        chosen.push((pick, penalty));
    }

    let candidates: Vec<Candidate> = chosen
        .into_iter()
        .enumerate()
        .map(|(rank, (s, penalty))| {
            let (a, b) = (&d.units[s.i], &d.units[s.j]);
            let expect: String = a.text.split_whitespace().take(6).collect::<Vec<_>>().join(" ").replace('"', "");
            Candidate {
                rank: rank + 1,
                start_unit: a.id,
                end_unit: b.id,
                start: round(a.start, 2),
                end: round(b.end, 2),
                mmss: mmss(a.start),
                duration: round(b.end - a.start, 1),
                score: s.score,
                diversity_penalty: round(penalty, 1),
                reasons: reasons_for(&s, &d.units),
                opening_text: a.text.clone(),
                closing_text: b.text.clone(),
                create_cmd: format!(
                    "rustler clip create --name \"cand-{}\" --source-id {} --start-unit {} --end-unit {} --expect-text \"{}\"",
                    rank + 1,
                    input.source_id,
                    a.id,
                    b.id,
                    expect
                ),
                read_cmd: format!("rustler transcript read-window {} --start {:.1} --end {:.1}", input.source_id, a.start, b.end),
                breakdown: s.parts,
                signals: s.feats,
            }
        })
        .collect();

    let mut next = Vec::new();
    if let Some(c) = candidates.first() {
        next.push(format!("read it: {}", c.read_cmd));
        next.push(format!("look at it: `rustler see tiles {} --around \"{}\"`", input.source_id, c.opening_text.split_whitespace().take(5).collect::<Vec<_>>().join(" ").replace('"', "")));
        next.push("then judge it with the rubric in `rustler guide show finding-hooks` and create the clip with its create_cmd".into());
    } else {
        next.push("no window fits: lower --min-len, raise --max-len, or read the transcript with `rustler transcript get`".into());
    }
    Ok(CandidatesOut {
        source_id: input.source_id,
        count_requested: count,
        returned: candidates.len(),
        windows_considered: considered,
        skipped_weak_openers: skipped_weak,
        scoring: Scoring {
            formula: SCORE_FORMULA.to_string(),
            platform,
            sweet_spot_s: sweet,
            baselines: Baselines { loudness_median_lufs: d.loud_median.map(|m| round(m, 1)), wpm_median: d.wpm_median.map(|m| round(m, 1)) },
        },
        available: availability(&d),
        candidates,
        intro_trailer: intro,
        warnings,
        note: "Candidates are evidence, not decisions: scores come from deterministic signals only and cannot judge meaning, humour or whether the clip stands alone. Read each window and apply the finding-hooks rubric.".to_string(),
        next,
    })
}

// ---------------------------------------------------------------------------------------
// transcript.read_window
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct ReadWindowIn {
    pub source_id: String,
    /// Source seconds.
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ReadWindowOut {
    pub source_id: String,
    pub start: f64,
    pub end: f64,
    pub first_unit: Option<u32>,
    pub last_unit: Option<u32>,
    pub units: usize,
    /// Readable text with unit markers: `[#12] sentence [#13] sentence`.
    pub text: String,
    /// One line per unit: `#12 [00:47:05.2–00:47:09.8] text`.
    pub lines: Vec<String>,
    pub truncated: bool,
    pub next: Vec<String>,
}

pub async fn read_window(ctx: Ctx, input: ReadWindowIn) -> rustler_core::Result<ReadWindowOut> {
    const STAGE: &str = "transcript.read_window";
    const MAX_UNITS: usize = 300;
    let store = ctx.store()?;
    if !(input.start.is_finite() && input.end.is_finite()) || input.end <= input.start {
        return Err(validation(STAGE, format!("end ({}) must be greater than start ({})", input.end, input.start), "pass seconds in source time, e.g. --start 2820 --end 2850"));
    }
    load_transcript(&store, &input.source_id)?;
    let units = load_units(&store, &input.source_id)?;
    let sel: Vec<&Unit> = units.iter().filter(|u| u.end > input.start && u.start < input.end).collect();
    let truncated = sel.len() > MAX_UNITS;
    let page: Vec<&Unit> = sel.iter().take(MAX_UNITS).copied().collect();
    let text = page.iter().map(|u| format!("[#{}] {}", u.id, u.text)).collect::<Vec<_>>().join(" ");
    let mut next = Vec::new();
    if let (Some(a), Some(b)) = (page.first(), page.last()) {
        next.push(format!(
            "rustler clip create --name <name> --source-id {} --start-unit {} --end-unit {} --expect-text \"{}\"",
            input.source_id,
            a.id,
            b.id,
            a.text.split_whitespace().take(6).collect::<Vec<_>>().join(" ").replace('"', "")
        ));
    } else {
        next.push("no speech in this window: look with `rustler see tiles` instead".into());
    }
    if truncated {
        next.push(format!("window has {} units; only the first {MAX_UNITS} are shown: narrow --start/--end", sel.len()));
    }
    Ok(ReadWindowOut {
        source_id: input.source_id,
        start: input.start,
        end: input.end,
        first_unit: page.first().map(|u| u.id),
        last_unit: page.last().map(|u| u.id),
        units: page.len(),
        text,
        lines: page.iter().map(|u| unit_line(u)).collect(),
        truncated,
        next,
    })
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let ro = Annotations { read_only: true, ..Default::default() };
    reg.register(typed(
        "highlights.signals",
        "Per-window deterministic signals (loudness peak, speech rate, questions/exclamations, emphasis words, pause before, scene cuts, heatmap); missing analyses give null + a warning",
        &["source_id"],
        ro,
        vec![
            Example { description: "30 s windows over a source", cli: "rustler highlights signals src_01k..." },
            Example { description: "10 s windows, second page", cli: "rustler highlights signals src_01k... --window 10 --offset 200" },
        ],
        signals,
    ));
    reg.register(typed(
        "highlights.candidates",
        "Sentence-aligned candidate clip windows scored from deterministic signals, with unit ids, reasons and a ready clip.create line; evidence for you to judge, not a decision",
        &["source_id"],
        ro,
        vec![
            Example { description: "top 10 candidates for Shorts", cli: "rustler highlights candidates src_01k..." },
            Example { description: "5 longer candidates for TikTok", cli: "rustler highlights candidates src_01k... --count 5 --min-len 25 --max-len 75 --platform tiktok" },
        ],
        candidates,
    ));
    reg.register(typed(
        "transcript.read_window",
        "Readable text of a time window with unit ids (for reviewing a candidate before cutting)",
        &["source_id", "start", "end"],
        ro,
        vec![Example { description: "read 30 s", cli: "rustler transcript read-window src_01k... 2820 2850" }],
        read_window,
    ));
}
