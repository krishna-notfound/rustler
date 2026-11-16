//! Command: `edit.refine` (feature f19): deterministic cut repair for a clip's segment boundaries.
//!
//! * start: skips weak openers (so, and, um, like, toh, matlab, ... at the head of the clip), then places
//!   the cut in the silence trough just before the first spoken word;
//! * end: makes the clip end on a sentence-final word (extends to the next one, or pulls back to the
//!   previous one, whichever is closer and within `max_shift`), then puts the tail in the silence after it.
//!
//! `max_shift` limits how far a boundary may move to reach a *word* boundary (and how far a pure silence
//! alignment may move); the padding placed into the silence next to the chosen word adds at most 0.25 s.
//! Silences come from `analysis/<source>/audio_events.json`; without it, word gaps are used.
//! Saves like the other edit commands: revision-checked, history snapshot, event, `--dry-run` previews.

use crate::highlights_cmds::{tok, weak_opener_len};
use chrono::Utc;
use rustler_core::{AudioEvents, Clip, ClipId, ErrorCode, Event, ProjectStore, Result, RustlerError, SegmentId, SourceId, Transcript, Word};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

/// Pause between words that closes a sentence (matches `rustler_core::units`).
const PAUSE_SPLIT: f64 = 0.7;
/// Lead-in placed before the first word, at most (inside the available silence).
const MAX_LEAD: f64 = 0.15;
/// Tail placed after the last word, at most (inside the available silence).
const MAX_TAIL: f64 = 0.25;
/// Pure alignment moves smaller than this are not worth an edit.
const MIN_MOVE: f64 = 0.03;
/// Shortest segment a refinement may leave.
const MIN_LEN: f64 = 0.3;

fn verr(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, "edit.refine", msg)
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct RefineIn {
    pub clip_id: ClipId,
    /// Refine both ends of this segment. Default: the clip's outer boundaries (start of the first segment,
    /// end of the last one); inner cut points are left alone because they are deliberate.
    #[serde(default)]
    pub segment_id: Option<SegmentId>,
    /// Largest move (seconds) of a boundary to reach a word boundary or silence. Default 1.5.
    #[serde(default)]
    pub max_shift: Option<f64>,
    /// Fail with RevisionConflict unless the clip is at this revision.
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

#[derive(Serialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SegmentRefine {
    pub segment_id: SegmentId,
    pub source_id: SourceId,
    /// `start+end`, `start` or `end`.
    pub scope: String,
    pub before: Range,
    pub after: Range,
    pub changed: bool,
    pub opening_before: String,
    pub opening_after: String,
    pub closing_before: String,
    pub closing_after: String,
    pub reasons: Vec<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct RefineOut {
    pub clip_id: ClipId,
    pub dry_run: bool,
    /// Revision of the clip after the command (unchanged on dry-run or when nothing changed).
    pub revision: u64,
    pub changed: bool,
    pub max_shift: f64,
    pub segments: Vec<SegmentRefine>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Terminal punctuation at the end of a word (`stop."` counts).
fn ends_terminal(text: &str) -> bool {
    text.trim_end_matches(|c: char| "\"'”’)]}»".contains(c) || c.is_whitespace())
        .chars()
        .next_back()
        .is_some_and(|c| ".?!।॥…".contains(c))
}

fn sentence_final(words: &[Word], m: usize) -> bool {
    ends_terminal(&words[m].text) || m + 1 == words.len() || words[m + 1].start - words[m].end >= PAUSE_SPLIT - 1e-9
}

/// Index of the last word of the sentence containing word `k`.
fn sentence_end(words: &[Word], k: usize) -> usize {
    (k..words.len()).find(|&m| sentence_final(words, m)).unwrap_or(words.len().saturating_sub(1))
}

/// Longest silence overlapping `[lo, hi]`, clipped to it; falls back to `[lo, hi]` itself (the word gap).
fn trough(silences: &[(f64, f64)], lo: f64, hi: f64) -> (f64, f64, bool) {
    let best = silences
        .iter()
        .filter(|(a, b)| *b > lo + 0.01 && *a < hi - 0.01)
        .map(|(a, b)| (a.max(lo), b.min(hi)))
        .filter(|(a, b)| b - a >= 0.05)
        .max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)));
    match best {
        Some((a, b)) => (a, b, true),
        None => (lo, hi.max(lo), false),
    }
}

fn words_text(words: &[Word], from: usize, n: usize) -> String {
    words[from.min(words.len())..].iter().take(n).map(|w| w.text.trim()).collect::<Vec<_>>().join(" ")
}

/// First word still playing 20 ms after `t`.
fn first_playing(words: &[Word], t: f64) -> Option<usize> {
    words.iter().position(|w| w.end > t + 0.02)
}

/// Pure start refinement. Returns the new start and appends reasons.
pub fn refine_start(words: &[Word], silences: &[(f64, f64)], start: f64, max_shift: f64, reasons: &mut Vec<String>) -> f64 {
    let Some(mut k) = first_playing(words, start) else {
        reasons.push("start: no spoken words after the start; unchanged".into());
        return start;
    };
    let mut word_moved = false;

    let send = sentence_end(words, k);
    let tokens: Vec<String> = words[k..=send].iter().map(|w| tok(&w.text)).collect();
    let n_open = weak_opener_len(&tokens);
    if n_open > 0 {
        let opener = words_text(words, k, n_open);
        let new_k = k + n_open;
        if new_k > send || send - new_k + 1 < 2 {
            reasons.push(format!("start: opener '{opener}' kept: skipping it would leave under 2 words of that sentence"));
        } else if words[new_k].start - start > max_shift {
            reasons.push(format!(
                "start: opener '{opener}' kept: skipping it moves the start {:.2} s, over max_shift {max_shift:.2}",
                words[new_k].start - start
            ));
        } else {
            reasons.push(format!("start: skipped weak opener '{opener}' ({:.2} s)", words[new_k].start - words[k].start));
            k = new_k;
            word_moved = true;
        }
    }

    let lo = if k > 0 { words[k - 1].end } else { 0.0 };
    let hi = words[k].start;
    let (ra, rb, in_silence) = trough(silences, lo.min(hi), hi);
    let lead = MAX_LEAD.min((rb - ra) / 2.0);
    let target = (rb - lead).max(ra);
    let new = if word_moved {
        target
    } else {
        let d = (target - start).clamp(-max_shift, max_shift);
        if (target - start).abs() > max_shift + 1e-9 {
            reasons.push(format!("start: only moved {d:+.2} s of the {:+.2} s to the silence before the first word (max_shift {max_shift:.2})", target - start));
        }
        if d.abs() < MIN_MOVE { start } else { start + d }
    };
    if (new - start).abs() > 1e-6 {
        reasons.push(format!(
            "start: {} before '{}' ({:+.2} s)",
            if in_silence { "aligned to the silence trough" } else { "aligned to the word gap" },
            words_text(words, k, 2),
            new - start
        ));
    }
    new
}

/// Pure end refinement. `min_end` is the earliest acceptable end (keeps the segment from collapsing);
/// `duration` caps the end (0 = unknown).
pub fn refine_end(words: &[Word], silences: &[(f64, f64)], end: f64, max_shift: f64, min_end: f64, duration: f64, reasons: &mut Vec<String>) -> f64 {
    let Some(kk) = words.iter().rposition(|w| w.start < end - 0.02) else {
        reasons.push("end: no spoken words before the end; unchanged".into());
        return end;
    };
    let straddle = words[kk].end > end + 0.02;
    let mut m = kk;
    let mut word_moved = false;

    if straddle || !sentence_final(words, kk) {
        let fwd = (kk..words.len()).find(|&i| sentence_final(words, i));
        let back = (0..kk).rev().find(|&i| sentence_final(words, i));
        let df = fwd.map(|i| words[i].end - end).filter(|d| *d <= max_shift + 1e-9);
        let db = back.map(|i| end - words[i].end).filter(|d| *d <= max_shift + 1e-9 && words[back.unwrap_or(0)].end >= min_end);
        let choice = match (df, db) {
            (Some(f), Some(b)) if b < f - 1e-9 => back.map(|i| (i, "pulled back")),
            (Some(_), _) => fwd.map(|i| (i, "extended")),
            (None, Some(_)) => back.map(|i| (i, "pulled back")),
            (None, None) => None,
        };
        match choice {
            Some((i, how)) => {
                reasons.push(format!(
                    "end: {how} to the sentence end '{}' ({:+.2} s)",
                    words_text(words, i.saturating_sub(2), 3.min(i + 1)),
                    words[i].end - end
                ));
                m = i;
                word_moved = true;
            }
            None => {
                reasons.push(format!(
                    "end: does not land on a sentence end and none is within max_shift {max_shift:.2} s (last words '{}')",
                    words_text(words, kk.saturating_sub(2), 3.min(kk + 1))
                ));
                if straddle && words[kk].end - end <= max_shift + 1e-9 {
                    reasons.push(format!("end: extended to finish the word '{}' ({:+.2} s)", words[kk].text.trim(), words[kk].end - end));
                    word_moved = true;
                } else {
                    return end;
                }
            }
        }
    }

    let lo = words[m].end;
    let hi = words.get(m + 1).map(|w| w.start).unwrap_or_else(|| if duration > 0.0 { duration.max(lo) } else { lo + 0.5 });
    let (ra, rb, in_silence) = trough(silences, lo, hi);
    let tail = MAX_TAIL.min((rb - ra) / 2.0);
    let mut target = ra + tail;
    if duration > 0.0 {
        target = target.min(duration);
    }
    let new = if word_moved {
        target
    } else {
        let d = (target - end).clamp(-max_shift, max_shift);
        if d.abs() < MIN_MOVE { end } else { end + d }
    };
    let new = new.max(min_end);
    if (new - end).abs() > 1e-6 {
        reasons.push(format!(
            "end: tail placed in the {} after '{}' ({:+.2} s)",
            if in_silence { "silence trough" } else { "word gap" },
            words_text(words, m.saturating_sub(1), 2.min(m + 1)),
            new - end
        ));
    }
    new
}

fn silences_of(store: &ProjectStore, source_id: &SourceId) -> Result<Option<Vec<(f64, f64)>>> {
    let rel = Path::new("analysis").join(source_id.as_str()).join("audio_events.json");
    if !store.resolve(&rel).is_file() {
        return Ok(None);
    }
    let ev: AudioEvents = store.read_json(&rel)?;
    Ok(Some(ev.silences.iter().map(|s| (s.start, s.end)).collect()))
}

fn load_clip(ctx: &Ctx, id: &ClipId, expect: Option<u64>) -> Result<(ProjectStore, Clip)> {
    let store = ctx.store()?;
    let clip = store.load_clip(id).map_err(|e| {
        if e.code == ErrorCode::NotFound {
            RustlerError::new(ErrorCode::NotFound, "edit.refine", format!("clip {id} not found")).hint("run `rustler clip list` to see clip ids")
        } else {
            e
        }
    })?;
    if let Some(r) = expect {
        if r != clip.revision {
            return Err(RustlerError::new(
                ErrorCode::RevisionConflict,
                "edit.refine",
                format!("clip {id} is at revision {} but expect_revision was {r}", clip.revision),
            )
            .hint("reload the clip (`rustler clip show`) and re-apply the change"));
        }
    }
    Ok((store, clip))
}

pub async fn refine(ctx: Ctx, input: RefineIn) -> Result<RefineOut> {
    let max_shift = input.max_shift.unwrap_or(1.5);
    if !max_shift.is_finite() || !(0.0..=10.0).contains(&max_shift) {
        return Err(verr(format!("max_shift {max_shift} must be between 0 and 10 seconds")).hint("the default is 1.5"));
    }
    let (store, old) = load_clip(&ctx, &input.clip_id, input.expect_revision)?;
    if old.segments.is_empty() {
        return Err(verr("clip has no segments"));
    }
    // (segment index, refine start, refine end)
    let plan: Vec<(usize, bool, bool)> = match &input.segment_id {
        Some(id) => {
            let idx = old.segments.iter().position(|s| &s.id == id).ok_or_else(|| {
                let ids: Vec<&str> = old.segments.iter().map(|s| s.id.as_str()).collect();
                RustlerError::new(ErrorCode::NotFound, "edit.refine", format!("segment {id} not in clip {}", old.id)).hint(format!("segments: {}", ids.join(", ")))
            })?;
            vec![(idx, true, true)]
        }
        None if old.segments.len() == 1 => vec![(0, true, true)],
        None => vec![(0, true, false), (old.segments.len() - 1, false, true)],
    };

    let mut new = old.clone();
    let mut warnings = Vec::new();
    let mut results: Vec<SegmentRefine> = Vec::new();
    for (idx, do_start, do_end) in plan {
        let seg = &old.segments[idx];
        let rel = Path::new("analysis").join(seg.source_id.as_str()).join("transcript.json");
        if !store.resolve(&rel).is_file() {
            return Err(verr(format!("source {} has no transcript", seg.source_id))
                .hint(format!("run `rustler analyze transcribe {}` first", seg.source_id)));
        }
        let transcript: Transcript = store.read_json(&rel)?;
        let silences = silences_of(&store, &seg.source_id)?;
        if silences.is_none() {
            warnings.push(format!(
                "no audio_events.json for {}: used word gaps instead of silences; run `rustler analyze audio {}` for better cut points",
                seg.source_id, seg.source_id
            ));
        }
        let sil = silences.unwrap_or_default();
        let duration = store.load_source(&seg.source_id).map(|s| s.probe.duration).unwrap_or(0.0);
        let words = &transcript.words;
        let mut reasons = Vec::new();
        let mut start = seg.start;
        let mut end = seg.end;
        if do_start {
            start = refine_start(words, &sil, seg.start, max_shift, &mut reasons);
        }
        if do_end {
            end = refine_end(words, &sil, seg.end, max_shift, start + MIN_LEN, duration, &mut reasons);
        }
        if start < 0.0 {
            start = 0.0;
        }
        if end - start < MIN_LEN {
            warnings.push(format!("segment {}: refinement would leave {:.2} s; kept the original range", seg.id, end - start));
            reasons.push("kept original: refined range too short".into());
            start = seg.start;
            end = seg.end;
        }
        if reasons.is_empty() {
            reasons.push("already clean: starts on a spoken word and ends on a sentence end".into());
        }
        let first_before = first_playing(words, seg.start).unwrap_or(0);
        let first_after = first_playing(words, start).unwrap_or(0);
        let last_of = |t: f64| words.iter().rposition(|w| w.start < t - 0.02).unwrap_or(0);
        let (lb, la) = (last_of(seg.end), last_of(end));
        results.push(SegmentRefine {
            segment_id: seg.id.clone(),
            source_id: seg.source_id.clone(),
            scope: match (do_start, do_end) {
                (true, true) => "start+end",
                (true, false) => "start",
                _ => "end",
            }
            .into(),
            before: Range { start: seg.start, end: seg.end },
            after: Range { start: (start * 1000.0).round() / 1000.0, end: (end * 1000.0).round() / 1000.0 },
            changed: (start - seg.start).abs() > 1e-6 || (end - seg.end).abs() > 1e-6,
            opening_before: words_text(words, first_before, 6),
            opening_after: words_text(words, first_after, 6),
            closing_before: words_text(words, (lb + 1).saturating_sub(6), 6.min(lb + 1)),
            closing_after: words_text(words, (la + 1).saturating_sub(6), 6.min(la + 1)),
            reasons,
        });
        new.segments[idx].start = (start * 1000.0).round() / 1000.0;
        new.segments[idx].end = (end * 1000.0).round() / 1000.0;
    }

    let changed = new != old;
    let mut revision = old.revision;
    if changed && !ctx.dry_run {
        crate::extras::save_clip(&store, &mut new, old.revision)?;
        revision = new.revision;
        // A moved segment start shifts its face-tracked crop path (keys are segment-relative).
        warnings.extend(crate::extras::carry_crop_paths(&store, &old, &new)?);
        store.append_event(&Event {
            ts: Utc::now(),
            actor: ctx.actor,
            kind: "edit.refine".to_string(),
            project_revision: store.load_project()?.revision,
            data: json!({
                "clip_id": new.id,
                "from_revision": old.revision,
                "to_revision": new.revision,
                "detail": results.iter().filter(|r| r.changed).map(|r| json!({"segment_id": r.segment_id, "before": r.before, "after": r.after})).collect::<Vec<_>>(),
            }),
        })?;
    }
    let mut next = Vec::new();
    if ctx.dry_run && changed {
        next.push(format!("apply it: rerun without --dry-run (clip {} stays at revision {})", old.id, old.revision));
    } else if changed {
        next.push(format!("check the result: `rustler see tiles` / `rustler clip show {}`; undo with `rustler history undo {}`", old.id, old.id));
    } else {
        next.push("nothing to refine: the cut points already sit on a spoken word and a sentence end".into());
    }
    Ok(RefineOut { clip_id: old.id.clone(), dry_run: ctx.dry_run, revision, changed, max_shift, segments: results, warnings, next })
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "edit.refine",
        "Repair a clip's cut points: skip weak openers (so/and/um/toh/matlab...), end on a sentence-final word, put start and end in silence troughs (within max_shift); supports --dry-run",
        &["clip_id"],
        Annotations::default(),
        vec![
            Example { description: "preview the refinement of a clip", cli: "rustler edit refine clp_X --dry-run" },
            Example { description: "refine one segment, allow moves up to 2 s", cli: "rustler edit refine clp_X --segment-id seg_Y --max-shift 2" },
        ],
        refine,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
        Word { i, text: text.into(), start, end, conf: None, speaker: None }
    }

    #[test]
    fn opener_skip_and_trough() {
        // "So yeah the plan works." then a pause
        let words = vec![w(0, "So", 10.0, 10.2), w(1, "yeah", 10.25, 10.5), w(2, "the", 10.8, 10.9), w(3, "plan", 10.95, 11.2), w(4, "works.", 11.25, 11.7)];
        let mut r = Vec::new();
        let s = refine_start(&words, &[], 9.9, 1.5, &mut r);
        assert!(s > 10.6 && s < 10.8, "start {s} {r:?}");
        let e = refine_end(&words, &[], 11.75, 1.5, 11.0, 0.0, &mut r);
        assert!(e > 11.7 && e <= 11.7 + MAX_TAIL + 1e-9, "end {e}");
    }
}
