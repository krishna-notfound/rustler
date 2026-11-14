//! Commands: transcript.get, transcript.search (Task 10a).
//!
//! Also the shared transcript storage helpers other command modules use:
//! `analysis/<id>/transcript.json` (a `Transcript`) and `analysis/<id>/units.json`
//! (a bare JSON array of `Unit`). `<id>` is a `src_...` id, or the `url_<sha8>` id of a
//! pending transcript written by `source.fetch_transcript`.

use crate::project_cmds::{hmsd, mmss, validation};
use rustler_core::units::build_units;
use rustler_core::{ErrorCode, ProjectStore, RustlerError, Transcript, Unit};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

// ---------------------------------------------------------------------------------------
// storage helpers (shared)
// ---------------------------------------------------------------------------------------

/// Relative path of an analysis folder; rejects ids that could escape it.
pub fn analysis_rel(id: &str) -> rustler_core::Result<PathBuf> {
    let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(validation(
            "transcript",
            format!("invalid source id `{id}`"),
            "ids look like `src_01k...` (see `rustler source list`) or `url_ab12cd34` (from `rustler source fetch-transcript`)",
        ));
    }
    Ok(Path::new("analysis").join(id))
}

/// Loads `analysis/<id>/transcript.json` or a Validation error naming the next command.
pub fn load_transcript(store: &ProjectStore, id: &str) -> rustler_core::Result<Transcript> {
    let rel = analysis_rel(id)?.join("transcript.json");
    if store.resolve(&rel).is_file() {
        return store.read_json(&rel);
    }
    let src_exists = store.root.join("sources").join(id).join("source.json").is_file();
    Err(if src_exists {
        validation(
            "transcript",
            format!("source {id} has no transcript yet"),
            format!("run `rustler analyze transcribe {id}` (or `rustler analyze quick {id}`); for clip.create you may also pass --start/--end in seconds"),
        )
    } else if id.starts_with("url_") {
        validation(
            "transcript",
            format!("no pending transcript {id} in this project"),
            "run `rustler source fetch-transcript <url>` first; it prints the pending id",
        )
    } else {
        RustlerError::new(ErrorCode::NotFound, "transcript", format!("no source or transcript `{id}` in this project"))
            .hint("run `rustler source list` to see source ids")
    })
}

/// Loads `units.json`; rebuilt from the transcript when missing.
pub fn load_units(store: &ProjectStore, id: &str) -> rustler_core::Result<Vec<Unit>> {
    let rel = analysis_rel(id)?.join("units.json");
    if store.resolve(&rel).is_file() {
        return store.read_json(&rel);
    }
    Ok(build_units(&load_transcript(store, id)?.words))
}

/// Writes `transcript.json` and `units.json`; returns the units.
pub fn save_transcript(store: &ProjectStore, id: &str, t: &Transcript) -> rustler_core::Result<Vec<Unit>> {
    let dir = analysis_rel(id)?;
    let units = build_units(&t.words);
    store.write_json(&dir.join("transcript.json"), t)?;
    store.write_json(&dir.join("units.json"), &units)?;
    Ok(units)
}

/// `#12 [47:05] text` (compact; used by previews).
pub fn unit_preview_line(u: &Unit) -> String {
    format!("#{} [{}] {}", u.id, mmss(u.start), u.text)
}

/// `#12 [00:47:05.2–00:47:09.8] text`.
pub fn unit_line(u: &Unit) -> String {
    format!("#{} [{}–{}] {}", u.id, hmsd(u.start), hmsd(u.end), u.text)
}

/// First `n` units as preview lines.
pub fn units_preview(units: &[Unit], n: usize) -> Vec<String> {
    units.iter().take(n).map(unit_preview_line).collect()
}

/// Unit overlapping `[t0, t1]` the most; falls back to the nearest unit within `slack` seconds.
pub fn unit_at(units: &[Unit], t0: f64, t1: f64, slack: f64) -> Option<&Unit> {
    let overlap = |u: &Unit| (u.end.min(t1) - u.start.max(t0)).max(0.0);
    let best = units.iter().filter(|u| u.end >= t0 - slack && u.start <= t1 + slack);
    best.max_by(|a, b| {
        let (oa, ob) = (overlap(a), overlap(b));
        let da = if oa > 0.0 { 0.0 } else { (a.start - t1).max(t0 - a.end) };
        let db = if ob > 0.0 { 0.0 } else { (b.start - t1).max(t0 - b.end) };
        oa.partial_cmp(&ob).unwrap().then(db.partial_cmp(&da).unwrap())
    })
}

// ---------------------------------------------------------------------------------------
// transcript.get
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct GetIn {
    /// `src_...` id, or `url_<sha8>` from `source.fetch_transcript`.
    pub source_id: String,
    /// Only units overlapping `[start, end)` (source seconds).
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// Skip this many units (after the time filter). Default 0.
    pub offset: Option<usize>,
    /// Max units to return. Default 200.
    pub limit: Option<usize>,
    /// `lines` (default; `"#12 [00:47:05.2–00:47:09.8] text"`) or `json` (full Unit objects).
    pub format: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct GetOut {
    pub source_id: String,
    pub language: String,
    pub engine: String,
    /// `exact` (whisper) or `approximate` (YouTube captions: confirm cut points with `see tiles`).
    pub word_timing: String,
    pub total_units: usize,
    pub offset: usize,
    pub returned: usize,
    pub next_offset: Option<usize>,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<Vec<Unit>>,
    pub next: Vec<String>,
}

pub async fn get(ctx: Ctx, input: GetIn) -> rustler_core::Result<GetOut> {
    let store = ctx.store()?;
    let format = input.format.as_deref().unwrap_or("lines").to_ascii_lowercase();
    if format != "lines" && format != "json" {
        return Err(validation(
            "transcript.get",
            format!("unknown format `{format}`"),
            "use --format lines (default) or --format json",
        ));
    }
    let limit = input.limit.unwrap_or(200);
    if limit == 0 {
        return Err(validation("transcript.get", "limit must be at least 1", "omit --limit for the default of 200"));
    }
    if let (Some(s), Some(e)) = (input.start, input.end) {
        if e <= s {
            return Err(validation("transcript.get", format!("end ({e}) must be greater than start ({s})"), "pass seconds in source time, e.g. --start 2820 --end 2970"));
        }
    }
    let t = load_transcript(&store, &input.source_id)?;
    let units = load_units(&store, &input.source_id)?;
    let (lo, hi) = (input.start.unwrap_or(f64::NEG_INFINITY), input.end.unwrap_or(f64::INFINITY));
    let filtered: Vec<&Unit> = units.iter().filter(|u| u.end > lo && u.start < hi).collect();
    let offset = input.offset.unwrap_or(0);
    let page: Vec<&Unit> = filtered.iter().skip(offset).take(limit).copied().collect();
    let end_idx = offset + page.len();
    let next_offset = (end_idx < filtered.len()).then_some(end_idx);

    let mut next = Vec::new();
    if let Some(n) = next_offset {
        next.push(format!("more units: rerun with --offset {n}"));
    }
    if units.is_empty() {
        next.push("transcript has no speech: use `rustler see tiles` and pass --start/--end (seconds) to `clip create`".to_string());
    } else {
        next.push(format!(
            "pick unit ids, then `rustler clip create --source-id {} --start-unit N --end-unit M --expect-text \"first words\"`",
            input.source_id
        ));
    }
    Ok(GetOut {
        source_id: input.source_id,
        language: t.language,
        engine: t.engine,
        word_timing: format!("{:?}", t.word_timing).to_lowercase(),
        total_units: filtered.len(),
        offset,
        returned: page.len(),
        next_offset,
        lines: (format == "lines").then(|| page.iter().map(|u| unit_line(u)).collect()),
        units: (format == "json").then(|| page.iter().map(|u| (*u).clone()).collect()),
        format,
        next,
    })
}

// ---------------------------------------------------------------------------------------
// transcript.search
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SearchIn {
    pub source_id: String,
    pub query: String,
    /// Max matches. Default 20.
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct NeighbourUnit {
    pub unit_id: u32,
    pub text: String,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct Match {
    pub unit_id: u32,
    pub start: f64,
    pub end: f64,
    pub mmss: String,
    pub text: String,
    /// `exact` (normalized substring) or `fuzzy` (similarity >= 0.8).
    pub kind: String,
    pub score: f64,
    pub before: Option<NeighbourUnit>,
    pub after: Option<NeighbourUnit>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SearchOut {
    pub source_id: String,
    pub query: String,
    pub total_matches: usize,
    pub matches: Vec<Match>,
    pub next: Vec<String>,
}

/// Lowercase, accent-stripped, punctuation-free, single-spaced.
pub fn normalize(s: &str) -> String {
    let stripped: String = s
        .nfd()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .flat_map(|c| c.to_lowercase())
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Similarity (0..1) of `q` against the best same-length window of `text`'s words.
fn fuzzy_score(q: &str, text: &str) -> f64 {
    let qwords = q.split(' ').count();
    let words: Vec<&str> = text.split(' ').collect();
    let mut best = 0.0f64;
    for w in [qwords.saturating_sub(1).max(1), qwords, qwords + 1] {
        if w > words.len() {
            continue;
        }
        for win in words.windows(w) {
            best = best.max(strsim::normalized_levenshtein(q, &win.join(" ")));
        }
    }
    best
}

/// Pure search over units: exact normalized-substring matches in time order, then fuzzy
/// (>= 0.8) matches by score. Returns every match; callers truncate.
pub fn search_units(units: &[Unit], query: &str) -> Vec<(usize, &'static str, f64)> {
    let q = normalize(query);
    if q.is_empty() {
        return Vec::new();
    }
    let mut exact = Vec::new();
    let mut fuzzy = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let n = normalize(&u.text);
        if n.contains(&q) {
            exact.push((i, "exact", 1.0));
        } else if q.chars().count() >= 4 {
            let s = fuzzy_score(&q, &n);
            if s >= 0.8 {
                fuzzy.push((i, "fuzzy", s));
            }
        }
    }
    fuzzy.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap().then(a.0.cmp(&b.0)));
    exact.extend(fuzzy);
    exact
}

pub async fn search(ctx: Ctx, input: SearchIn) -> rustler_core::Result<SearchOut> {
    let store = ctx.store()?;
    if normalize(&input.query).is_empty() {
        return Err(validation("transcript.search", "query is empty", "pass words to look for: --query \"fellow americans\""));
    }
    let limit = input.limit.unwrap_or(20).max(1);
    load_transcript(&store, &input.source_id)?;
    let units = load_units(&store, &input.source_id)?;
    let hits = search_units(&units, &input.query);
    let total = hits.len();
    let nb = |i: Option<usize>| i.and_then(|i| units.get(i)).map(|u| NeighbourUnit { unit_id: u.id, text: u.text.clone() });
    let matches: Vec<Match> = hits
        .into_iter()
        .take(limit)
        .map(|(i, kind, score)| {
            let u = &units[i];
            Match {
                unit_id: u.id,
                start: u.start,
                end: u.end,
                mmss: mmss(u.start),
                text: u.text.clone(),
                kind: kind.to_string(),
                score: (score * 1000.0).round() / 1000.0,
                before: nb(i.checked_sub(1)),
                after: nb(Some(i + 1)),
            }
        })
        .collect();
    let mut next = Vec::new();
    if let Some(m) = matches.first() {
        next.push(format!(
            "look at it: `rustler see tiles {} --around \"{}\"`; read around it: `rustler transcript get {} --start {:.0} --end {:.0}`",
            input.source_id,
            input.query,
            input.source_id,
            (m.start - 30.0).max(0.0),
            m.end + 30.0
        ));
    } else {
        next.push("no match: try fewer or differently spelled words (Hinglish is romanized, e.g. `nahi`), or page through with `rustler transcript get`".to_string());
    }
    if total > matches.len() {
        next.push(format!("{} more matches not shown: raise --limit", total - matches.len()));
    }
    Ok(SearchOut { source_id: input.source_id, query: input.query, total_matches: total, matches, next })
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let ro = Annotations { read_only: true, ..Default::default() };
    reg.register(typed(
        "transcript.get",
        "Read the transcript as numbered units (sentences/clauses); pick unit ids for clips. Paged: --offset/--limit; --format lines|json",
        &["source_id"],
        ro,
        vec![
            Example { description: "First page of units", cli: "rustler transcript get src_01k..." },
            Example { description: "Only a time window", cli: "rustler transcript get src_01k... --start 2820 --end 2970" },
        ],
        get,
    ));
    reg.register(typed(
        "transcript.search",
        "Find units containing a phrase (case/accent-insensitive, then fuzzy); returns neighbouring units for context",
        &["source_id", "query"],
        ro,
        vec![Example { description: "Find a quote", cli: "rustler transcript search src_01k... \"fellow americans\"" }],
        search,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_core::UnitKind;

    fn unit(id: u32, start: f64, end: f64, text: &str) -> Unit {
        Unit { id, kind: UnitKind::Sentence, word_start: 0, word_end: 0, start, end, text: text.into(), boundary_strength: 1.0 }
    }

    #[test]
    fn normalize_strips_accents_and_punct() {
        assert_eq!(normalize("  Café, au  LAIT!! "), "cafe au lait");
        assert_eq!(normalize("Hello—World"), "hello world");
    }

    #[test]
    fn exact_then_fuzzy() {
        let units = vec![
            unit(0, 0.0, 2.0, "And so my fellow Americans,"),
            unit(1, 2.0, 4.0, "ask not what your country"),
            unit(2, 4.0, 6.0, "my fellow american friends"),
        ];
        let r = search_units(&units, "fellow americans");
        assert_eq!(r[0].0, 0);
        assert_eq!(r[0].1, "exact");
        assert!(r.iter().any(|m| m.0 == 2 && m.1 == "fuzzy"), "got {r:?}");
        assert!(search_units(&units, "zebra crossing").is_empty());
    }

    #[test]
    fn unit_at_prefers_overlap() {
        let units = vec![unit(0, 0.0, 5.0, "a"), unit(1, 5.0, 10.0, "b")];
        assert_eq!(unit_at(&units, 6.0, 7.0, 2.0).unwrap().id, 1);
        assert_eq!(unit_at(&units, 11.0, 12.0, 2.0).unwrap().id, 1);
        assert!(unit_at(&units, 30.0, 31.0, 2.0).is_none());
    }
}
