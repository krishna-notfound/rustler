//! Timeline edit commands: `edit.trim`, `edit.split`, `edit.delete_segment`, `edit.move_segment`,
//! `edit.set_range`, `edit.remove_pauses`, `edit.remove_words`, plus snapshot history
//! (`history.list`, `history.undo`, `history.restore`).
//!
//! Every mutating command: loads the clip, optionally checks `expect_revision`, edits a copy,
//! saves with `base_revision` = the loaded revision (so concurrent writers get `RevisionConflict`),
//! appends an [`Event`] and returns the updated clip summary. With `ctx.dry_run` nothing is written
//! and `{"dry_run":true,"would":<new clip>}` is returned instead.
//!
//! Word-aware commands read `analysis/<source>/transcript.json` (and `units.json` when present;
//! units are rebuilt from the words otherwise).

use chrono::{DateTime, Utc};
use rustler_core::units::{build_units, find_quote};
use rustler_core::{
    Clip, ClipId, ErrorCode, Event, ProjectStore, Result, RustlerError, Segment, SegmentId, SourceId, TimeMap,
    Transcript, Unit, Word,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Shortest segment piece an edit may produce (about one frame at 30 fps).
const MIN_SEG: f64 = 0.04;
/// Padding rules shared with `clip.create` in unit mode.
const PAD_BEFORE: f64 = 0.08;
const PAD_AFTER: f64 = 0.15;
/// Words removed by `edit.remove_words` when no list is given.
const DEFAULT_FILLERS: &[&str] = &["um", "uh", "umm", "uhh", "hmm", "erm"];

pub fn register(reg: &mut Registry) {
    let edit = Annotations::default();
    reg.register(typed(
        "edit.trim",
        "Trim a segment's start/end by deltas (seconds) or to absolute source times; optional word snap.",
        &["clip_id"],
        edit,
        vec![
            Example {
                description: "cut 0.5 s off the head and 1 s off the tail of the first segment",
                cli: "rustler edit trim clp_X --input-json '{\"start_delta\":0.5,\"end_delta\":-1.0}'",
            },
            Example {
                description: "set absolute source times and snap them to the nearest word boundary",
                cli: "rustler edit trim clp_X --start 12.62 --end 14.03 --snap word",
            },
        ],
        trim,
    ));
    reg.register(typed(
        "edit.split",
        "Split the segment containing output time out_t into two segments (same layout).",
        &["clip_id", "out_t"],
        edit,
        vec![Example { description: "split at 3.0 s of the clip output", cli: "rustler edit split clp_X 3.0" }],
        split,
    ));
    reg.register(typed(
        "edit.delete_segment",
        "Delete a segment (the last remaining segment cannot be deleted).",
        &["clip_id", "segment_id"],
        edit,
        vec![Example { description: "delete one segment", cli: "rustler edit delete-segment clp_X seg_Y" }],
        delete_segment,
    ));
    reg.register(typed(
        "edit.move_segment",
        "Move a segment to a new 0-based index in the timeline.",
        &["clip_id", "segment_id", "to_index"],
        edit,
        vec![Example { description: "make a segment the first one", cli: "rustler edit move-segment clp_X seg_Y 0" }],
        move_segment,
    ));
    reg.register(typed(
        "edit.set_range",
        "Re-cut a segment by transcript unit ids (same padding as clip.create); expect_text guards the opening words.",
        &["clip_id"],
        edit,
        vec![
            Example {
                description: "re-cut the first segment to units 12..14",
                cli: "rustler edit set-range clp_X --start-unit 12 --end-unit 14",
            },
            Example {
                description: "move only the start, verifying the opening words",
                cli: "rustler edit set-range clp_X --start-unit 11 --expect-text \"so my fellow\"",
            },
        ],
        set_range,
    ));
    reg.register(typed(
        "edit.remove_pauses",
        "Cut long word gaps out of every segment, leaving `keep` seconds of each pause.",
        &["clip_id"],
        edit,
        vec![
            Example { description: "tighten pauses with the defaults (0.5 s gaps -> 0.12 s)", cli: "rustler edit remove-pauses clp_X" },
            Example {
                description: "only gaps of 0.8 s or more, keep 0.2 s",
                cli: "rustler edit remove-pauses clp_X --min-gap 0.8 --keep 0.2",
            },
        ],
        remove_pauses,
    ));
    reg.register(typed(
        "edit.remove_words",
        "Cut exact word spans out: filler words (um, uh, umm, uhh, hmm, erm) by default, or listed words / word indices.",
        &["clip_id"],
        edit,
        vec![
            Example { description: "remove default fillers", cli: "rustler edit remove-words clp_X" },
            Example {
                description: "remove specific words and one word index",
                cli: "rustler edit remove-words clp_X --words '[\"basically\"]' --word-indices '[42]'",
            },
        ],
        remove_words,
    ));
    reg.register(typed(
        "history.list",
        "List the clip's available snapshot revisions with timestamps and segment counts.",
        &["clip_id"],
        Annotations { read_only: true, ..Default::default() },
        vec![Example { description: "show undo history", cli: "rustler history list clp_X" }],
        history_list,
    ));
    reg.register(typed(
        "history.undo",
        "Undo the last edit: restore the previous state as a NEW revision (repeat to step further back).",
        &["clip_id"],
        edit,
        vec![Example { description: "undo the last edit", cli: "rustler history undo clp_X" }],
        history_undo,
    ));
    reg.register(typed(
        "history.restore",
        "Restore a specific snapshot revision as a NEW revision (use it to redo).",
        &["clip_id", "revision"],
        edit,
        vec![Example { description: "go back to revision 2", cli: "rustler history restore clp_X 2" }],
        history_restore,
    ));
}

// ---------------------------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SegmentSummary {
    pub id: SegmentId,
    pub source_id: SourceId,
    pub start: f64,
    pub end: f64,
    pub out_start: f64,
    pub out_end: f64,
    pub layout_preset: Option<String>,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct ClipSummary {
    pub clip_id: ClipId,
    pub revision: u64,
    pub segments: Vec<SegmentSummary>,
    pub duration: f64,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct RemovedWord {
    pub segment_id: SegmentId,
    pub i: u32,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Cut {
    pub segment_id: SegmentId,
    pub start: f64,
    pub end: f64,
}

/// Command-specific extras, flattened into the output next to the clip summary.
#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct Extras {
    /// Seconds of output time removed by the edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_words: Option<Vec<RemovedWord>>,
    /// Source-time ranges that were cut out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cuts: Option<Vec<Cut>>,
    /// Ids of segments created by the edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_segment_ids: Option<Vec<SegmentId>>,
    /// Snapshot revision whose content was restored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Either the applied result (clip summary + extras) or the dry-run preview.
#[derive(Serialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum EditOut {
    Applied {
        #[serde(flatten)]
        summary: ClipSummary,
        #[serde(flatten)]
        extras: Extras,
    },
    DryRun {
        dry_run: bool,
        would: Box<Clip>,
        #[serde(flatten)]
        extras: Extras,
    },
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

fn verr(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn summarize(clip: &Clip) -> ClipSummary {
    let tm = TimeMap::from_segments(&clip.segments);
    let segments = tm
        .segments()
        .iter()
        .zip(&clip.segments)
        .map(|(m, s)| SegmentSummary {
            id: s.id.clone(),
            source_id: s.source_id.clone(),
            start: s.start,
            end: s.end,
            out_start: m.out_start,
            out_end: m.out_end,
            layout_preset: s.layout.preset.clone(),
        })
        .collect();
    ClipSummary { clip_id: clip.id.clone(), revision: clip.revision, segments, duration: tm.duration() }
}

fn clip_duration(clip: &Clip) -> f64 {
    TimeMap::from_segments(&clip.segments).duration()
}

/// Loads the clip and enforces `expect_revision`.
fn load(ctx: &Ctx, stage: &str, clip_id: &ClipId, expect: Option<u64>) -> Result<(ProjectStore, Clip)> {
    let store = ctx.store()?;
    let clip = store.load_clip(clip_id).map_err(|e| {
        if e.code == ErrorCode::NotFound {
            RustlerError::new(ErrorCode::NotFound, stage, format!("clip {clip_id} not found"))
                .hint("run `rustler clip list` to see clip ids")
        } else {
            e
        }
    })?;
    if let Some(r) = expect {
        if r != clip.revision {
            return Err(RustlerError::new(
                ErrorCode::RevisionConflict,
                stage,
                format!("clip {clip_id} is at revision {} but expect_revision was {r}", clip.revision),
            )
            .hint("reload the clip (`rustler clip show`) and re-apply the change"));
        }
    }
    Ok((store, clip))
}

/// Dry-run preview, no-op passthrough, or revision-checked save + event.
fn finish(
    ctx: &Ctx,
    store: &ProjectStore,
    kind: &str,
    old: &Clip,
    mut new: Clip,
    mut extras: Extras,
    detail: serde_json::Value,
) -> Result<EditOut> {
    if ctx.dry_run {
        return Ok(EditOut::DryRun { dry_run: true, would: Box::new(new), extras });
    }
    if new == *old {
        extras.warnings.push("edit made no change; nothing was saved".to_string());
        return Ok(EditOut::Applied { summary: summarize(old), extras });
    }
    let base = old.revision;
    // Layers (extras.json) of the superseded revision, so history undo/restore covers them too.
    crate::timeline_doc::snapshot_extras(store, &old.id, base)?;
    // Time-based layers follow their content through the re-cut; items whose content was cut
    // out are dropped, items past the new end clamped (history undo/restore puts the snapshot
    // back instead).
    let mut relayered: Option<serde_json::Map<String, serde_json::Value>> = None;
    if !kind.starts_with("history.") {
        let map = crate::extras::load(store, &old.id)?;
        match crate::timeline_doc::follow_recut(old, &new, &map) {
            Ok((m, w)) => {
                relayered = m;
                extras.warnings.extend(w);
            }
            Err(e) => extras.warnings.push(format!("layers were not re-timed to the edit: {}", e.message)),
        }
    }
    store.save_clip(&mut new, base)?;
    if let Some(m) = &relayered {
        crate::extras::save(store, &old.id, m)?;
    }
    // Face-tracked crop paths are keyed by segment id; move them onto the re-cut segments.
    extras.warnings.extend(crate::extras::carry_crop_paths(store, old, &new)?);
    store.append_event(&Event {
        ts: Utc::now(),
        actor: ctx.actor,
        kind: kind.to_string(),
        project_revision: store.load_project()?.revision,
        data: json!({
            "clip_id": new.id,
            "from_revision": base,
            "to_revision": new.revision,
            "detail": detail,
        }),
    })?;
    Ok(EditOut::Applied { summary: summarize(&new), extras })
}

fn seg_index(clip: &Clip, id: Option<&SegmentId>, stage: &str) -> Result<usize> {
    match id {
        None if clip.segments.is_empty() => Err(verr(stage, "clip has no segments")),
        None => Ok(0),
        Some(id) => clip.segments.iter().position(|s| &s.id == id).ok_or_else(|| {
            let ids: Vec<&str> = clip.segments.iter().map(|s| s.id.as_str()).collect();
            RustlerError::new(ErrorCode::NotFound, stage, format!("segment {id} not in clip {}", clip.id))
                .hint(format!("segments: {}", ids.join(", ")))
        }),
    }
}

fn finite(stage: &str, name: &str, v: Option<f64>) -> Result<()> {
    match v {
        Some(x) if !x.is_finite() => Err(verr(stage, format!("{name} must be a finite number"))),
        _ => Ok(()),
    }
}

/// `start < end`, inside `0..source duration`, at least `MIN_SEG` long.
fn validate_range(store: &ProjectStore, stage: &str, source_id: &SourceId, start: f64, end: f64) -> Result<()> {
    let source = store.load_source(source_id)?;
    let dur = source.probe.duration;
    if start < -1e-9 {
        return Err(verr(stage, format!("start {start:.3} is before the beginning of the source")));
    }
    if dur > 0.0 && end > dur + 1e-6 {
        return Err(verr(stage, format!("end {end:.3} is past the source duration {dur:.3}"))
            .hint("source duration comes from the source probe"));
    }
    if end <= start {
        return Err(verr(stage, format!("start {start:.3} must be less than end {end:.3}")));
    }
    if end - start < MIN_SEG {
        return Err(verr(stage, format!("segment would be {:.3} s long; minimum is {MIN_SEG} s", end - start))
            .hint("use edit.delete_segment to remove a segment"));
    }
    Ok(())
}

fn transcript_rel(source_id: &SourceId) -> PathBuf {
    Path::new("analysis").join(source_id.as_str()).join("transcript.json")
}

/// Lazily loads and caches each source's transcript (`None` when not transcribed).
struct Transcripts<'a> {
    store: &'a ProjectStore,
    map: HashMap<SourceId, Option<Transcript>>,
}

impl<'a> Transcripts<'a> {
    fn new(store: &'a ProjectStore) -> Self {
        Self { store, map: HashMap::new() }
    }

    fn get(&mut self, source_id: &SourceId) -> Result<Option<&Transcript>> {
        if !self.map.contains_key(source_id) {
            let rel = transcript_rel(source_id);
            let t = if self.store.resolve(&rel).is_file() { Some(self.store.read_json::<Transcript>(&rel)?) } else { None };
            self.map.insert(source_id.clone(), t);
        }
        Ok(self.map.get(source_id).and_then(|t| t.as_ref()))
    }

    fn require(&mut self, source_id: &SourceId, stage: &str) -> Result<&Transcript> {
        self.get(source_id)?.ok_or_else(|| {
            verr(stage, format!("source {source_id} has no transcript"))
                .hint(format!("run `rustler analyze transcribe {source_id}` first"))
        })
    }
}

fn load_units(store: &ProjectStore, source_id: &SourceId, t: &Transcript) -> Result<Vec<Unit>> {
    let rel = Path::new("analysis").join(source_id.as_str()).join("units.json");
    if store.resolve(&rel).is_file() {
        let v: serde_json::Value = store.read_json(&rel)?;
        let list = if v.is_array() { v } else { v.get("units").cloned().unwrap_or_default() };
        if let Ok(units) = serde_json::from_value::<Vec<Unit>>(list) {
            return Ok(units);
        }
    }
    Ok(build_units(&t.words))
}

fn norm_token(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Merges overlapping/adjacent `(start, end)` ranges, sorted.
fn merge_ranges(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1e-9 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Removes `cuts` (source time, merged) from `seg`. Returns the surviving pieces: the original
/// segment when untouched, the same id when exactly one piece survives, new ids when it splits.
fn apply_cuts(seg: &Segment, cuts: &[(f64, f64)]) -> Vec<Segment> {
    if cuts.is_empty() {
        return vec![seg.clone()];
    }
    let mut ranges: Vec<(f64, f64)> = Vec::new();
    let mut cursor = seg.start;
    for &(a, b) in cuts {
        if a - cursor >= MIN_SEG {
            ranges.push((cursor, a));
        }
        cursor = cursor.max(b);
    }
    if seg.end - cursor >= MIN_SEG {
        ranges.push((cursor, seg.end));
    }
    let single = ranges.len() == 1;
    ranges
        .into_iter()
        .map(|(s, e)| Segment {
            id: if single { seg.id.clone() } else { SegmentId::new() },
            source_id: seg.source_id.clone(),
            start: s,
            end: e,
            layout: seg.layout.clone(),
        })
        .collect()
}

/// Replaces segments by the per-segment results of `cuts_for`; returns the new ids created.
fn rebuild_with_cuts(
    clip: &mut Clip,
    stage: &str,
    cuts_by_seg: &HashMap<SegmentId, Vec<(f64, f64)>>,
) -> Result<Vec<SegmentId>> {
    let mut out: Vec<Segment> = Vec::new();
    let mut created: Vec<SegmentId> = Vec::new();
    for seg in &clip.segments {
        let cuts = cuts_by_seg.get(&seg.id).map(|c| merge_ranges(c.clone())).unwrap_or_default();
        let pieces = apply_cuts(seg, &cuts);
        for p in &pieces {
            if p.id != seg.id {
                created.push(p.id.clone());
            }
        }
        out.extend(pieces);
    }
    if out.is_empty() {
        return Err(verr(stage, "the edit would remove all of the clip's content")
            .hint("narrow the edit or use history.undo"));
    }
    clip.segments = out;
    Ok(created)
}

// ---------------------------------------------------------------------------------------------
// edit.trim
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapMode {
    /// Snap changed boundaries to the nearest word start/end from the source transcript.
    Word,
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct TrimInput {
    pub clip_id: ClipId,
    /// Segment to trim; default = the first segment.
    #[serde(default)]
    pub segment_id: Option<SegmentId>,
    /// Seconds added to the segment start (positive = trim the head).
    #[serde(default)]
    pub start_delta: Option<f64>,
    /// Seconds added to the segment end (negative = trim the tail).
    #[serde(default)]
    pub end_delta: Option<f64>,
    /// Absolute new start in source seconds (instead of start_delta).
    #[serde(default)]
    pub start: Option<f64>,
    /// Absolute new end in source seconds (instead of end_delta).
    #[serde(default)]
    pub end: Option<f64>,
    /// `"word"`: snap the boundaries this call changes to the nearest word boundary.
    #[serde(default)]
    pub snap: Option<SnapMode>,
    /// Fail with RevisionConflict unless the clip is at this revision.
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

fn nearest(candidates: &[f64], t: f64) -> Option<f64> {
    candidates.iter().copied().min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
}

async fn trim(ctx: Ctx, input: TrimInput) -> Result<EditOut> {
    const STAGE: &str = "edit.trim";
    for (n, v) in [("start_delta", input.start_delta), ("end_delta", input.end_delta), ("start", input.start), ("end", input.end)] {
        finite(STAGE, n, v)?;
    }
    if input.start.is_some() && input.start_delta.is_some() {
        return Err(verr(STAGE, "give either start or start_delta, not both"));
    }
    if input.end.is_some() && input.end_delta.is_some() {
        return Err(verr(STAGE, "give either end or end_delta, not both"));
    }
    let start_changes = input.start.is_some() || input.start_delta.is_some();
    let end_changes = input.end.is_some() || input.end_delta.is_some();
    if !start_changes && !end_changes {
        return Err(verr(STAGE, "nothing to trim").hint("pass start_delta/end_delta or start/end"));
    }
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let idx = seg_index(&old, input.segment_id.as_ref(), STAGE)?;
    let seg = &old.segments[idx];
    let mut new_start = input.start.unwrap_or(seg.start + input.start_delta.unwrap_or(0.0));
    let mut new_end = input.end.unwrap_or(seg.end + input.end_delta.unwrap_or(0.0));
    let mut extras = Extras::default();

    if input.snap == Some(SnapMode::Word) {
        let mut tr = Transcripts::new(&store);
        match tr.get(&seg.source_id)? {
            Some(t) if !t.words.is_empty() => {
                let mut cands: Vec<f64> = t.words.iter().flat_map(|w| [w.start, w.end]).collect();
                cands.sort_by(f64::total_cmp);
                if start_changes {
                    new_start = nearest(&cands, new_start).unwrap_or(new_start);
                }
                if end_changes {
                    new_end = nearest(&cands, new_end).unwrap_or(new_end);
                }
            }
            _ => extras.warnings.push(format!(
                "snap=word ignored: source {} has no transcript (run `rustler analyze transcribe`)",
                seg.source_id
            )),
        }
    }
    validate_range(&store, STAGE, &seg.source_id, new_start, new_end)?;

    let mut new = old.clone();
    new.segments[idx].start = new_start;
    new.segments[idx].end = new_end;
    let detail = json!({
        "segment_id": seg.id,
        "before": {"start": seg.start, "end": seg.end},
        "after": {"start": new_start, "end": new_end},
    });
    extras.removed_seconds = Some(clip_duration(&old) - clip_duration(&new));
    finish(&ctx, &store, STAGE, &old, new, extras, detail)
}

// ---------------------------------------------------------------------------------------------
// edit.split
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct SplitInput {
    pub clip_id: ClipId,
    /// Output (clip) time in seconds at which to split.
    pub out_t: f64,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn split(ctx: Ctx, input: SplitInput) -> Result<EditOut> {
    const STAGE: &str = "edit.split";
    finite(STAGE, "out_t", Some(input.out_t))?;
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let tm = TimeMap::from_segments(&old.segments);
    let idx = tm
        .segments()
        .iter()
        .position(|m| input.out_t >= m.out_start && input.out_t < m.out_end)
        .ok_or_else(|| {
            verr(STAGE, format!("out_t {} is outside the clip (duration {:.3} s)", input.out_t, tm.duration()))
                .hint("out_t is output time: 0 .. clip duration")
        })?;
    let m = &tm.segments()[idx];
    let cut = m.src_start + (input.out_t - m.out_start);
    if cut - m.src_start < MIN_SEG || m.src_end - cut < MIN_SEG {
        return Err(verr(STAGE, format!("out_t {} is too close to a segment boundary", input.out_t))
            .hint("use edit.trim to adjust boundaries"));
    }
    let seg = &old.segments[idx];
    let a = Segment { id: SegmentId::new(), source_id: seg.source_id.clone(), start: seg.start, end: cut, layout: seg.layout.clone() };
    let b = Segment { id: SegmentId::new(), source_id: seg.source_id.clone(), start: cut, end: seg.end, layout: seg.layout.clone() };
    let extras = Extras { new_segment_ids: Some(vec![a.id.clone(), b.id.clone()]), ..Default::default() };
    let detail = json!({"split_segment": seg.id, "out_t": input.out_t, "source_t": cut, "new": [a.id, b.id]});
    let mut new = old.clone();
    new.segments.splice(idx..=idx, [a, b]);
    finish(&ctx, &store, STAGE, &old, new, extras, detail)
}

// ---------------------------------------------------------------------------------------------
// edit.delete_segment / edit.move_segment
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct DeleteSegmentInput {
    pub clip_id: ClipId,
    pub segment_id: SegmentId,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn delete_segment(ctx: Ctx, input: DeleteSegmentInput) -> Result<EditOut> {
    const STAGE: &str = "edit.delete_segment";
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let idx = seg_index(&old, Some(&input.segment_id), STAGE)?;
    if old.segments.len() == 1 {
        return Err(verr(STAGE, "cannot delete the last segment of a clip")
            .hint("a clip needs at least one segment; trim it instead"));
    }
    let mut new = old.clone();
    let removed = new.segments.remove(idx);
    let extras = Extras { removed_seconds: Some(removed.end - removed.start), ..Default::default() };
    finish(&ctx, &store, STAGE, &old, new, extras, json!({"segment_id": removed.id, "start": removed.start, "end": removed.end}))
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct MoveSegmentInput {
    pub clip_id: ClipId,
    pub segment_id: SegmentId,
    /// Final 0-based position of the segment.
    pub to_index: usize,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn move_segment(ctx: Ctx, input: MoveSegmentInput) -> Result<EditOut> {
    const STAGE: &str = "edit.move_segment";
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let idx = seg_index(&old, Some(&input.segment_id), STAGE)?;
    if input.to_index >= old.segments.len() {
        return Err(verr(
            STAGE,
            format!("to_index {} is out of range (clip has {} segments)", input.to_index, old.segments.len()),
        )
        .hint("to_index is 0-based"));
    }
    let mut new = old.clone();
    let seg = new.segments.remove(idx);
    new.segments.insert(input.to_index, seg);
    finish(
        &ctx,
        &store,
        STAGE,
        &old,
        new,
        Extras::default(),
        json!({"segment_id": input.segment_id, "from_index": idx, "to_index": input.to_index}),
    )
}

// ---------------------------------------------------------------------------------------------
// edit.set_range
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct SetRangeInput {
    pub clip_id: ClipId,
    /// Segment to re-cut; default = the first segment.
    #[serde(default)]
    pub segment_id: Option<SegmentId>,
    /// Unit id whose start becomes the segment start (padded 0.08 s, clamped to the previous word end).
    #[serde(default)]
    pub start_unit: Option<u32>,
    /// Unit id whose end becomes the segment end (padded 0.15 s, clamped to the next word start).
    #[serde(default)]
    pub end_unit: Option<u32>,
    /// Opening words expected at start_unit; mismatch -> Validation.
    #[serde(default)]
    pub expect_text: Option<String>,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

fn word_pos(words: &[Word], i: u32) -> Option<usize> {
    words.iter().position(|w| w.i == i)
}

fn padded_start(words: &[Word], unit: &Unit) -> f64 {
    let base = unit.start - PAD_BEFORE;
    let v = match word_pos(words, unit.word_start).and_then(|p| p.checked_sub(1)) {
        Some(p) => base.max(words[p].end),
        None => base.max(0.0),
    };
    v.min(unit.start)
}

fn padded_end(words: &[Word], unit: &Unit, source_duration: f64) -> f64 {
    let base = unit.end + PAD_AFTER;
    let v = match word_pos(words, unit.word_end).and_then(|p| words.get(p + 1)) {
        Some(next) => base.min(next.start),
        None if source_duration > 0.0 => base.min(source_duration),
        None => base,
    };
    v.max(unit.end)
}

async fn set_range(ctx: Ctx, input: SetRangeInput) -> Result<EditOut> {
    const STAGE: &str = "edit.set_range";
    if input.start_unit.is_none() && input.end_unit.is_none() {
        return Err(verr(STAGE, "pass start_unit and/or end_unit").hint("find unit ids with `rustler transcript get <source>`"));
    }
    if input.expect_text.is_some() && input.start_unit.is_none() {
        return Err(verr(STAGE, "expect_text needs start_unit"));
    }
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let idx = seg_index(&old, input.segment_id.as_ref(), STAGE)?;
    let seg = &old.segments[idx];
    let mut tr = Transcripts::new(&store);
    let transcript = tr.require(&seg.source_id, STAGE)?.clone();
    let units = load_units(&store, &seg.source_id, &transcript)?;
    let source_duration = store.load_source(&seg.source_id)?.probe.duration;
    let find = |id: u32| -> Result<&Unit> {
        units.iter().find(|u| u.id == id).ok_or_else(|| {
            verr(STAGE, format!("unit {id} not found ({} units in source {})", units.len(), seg.source_id))
                .hint("list units with `rustler transcript get <source>`")
        })
    };
    let su = input.start_unit.map(&find).transpose()?;
    let eu = input.end_unit.map(&find).transpose()?;
    if let (Some(s), Some(e)) = (su, eu) {
        if s.id > e.id {
            return Err(verr(STAGE, format!("start_unit {} is after end_unit {}", s.id, e.id)));
        }
    }
    if let (Some(text), Some(u)) = (&input.expect_text, su) {
        if !find_quote(&transcript.words, u.word_start, text) {
            let n = text.split_whitespace().count().max(1);
            let pos = word_pos(&transcript.words, u.word_start).unwrap_or(0);
            let said: Vec<&str> = transcript.words[pos..].iter().take(n).map(|w| w.text.as_str()).collect();
            return Err(verr(STAGE, format!("opening words do not match: transcript says '{}'", said.join(" ")))
                .hint("check the unit id with `rustler transcript get`"));
        }
    }
    let new_start = su.map_or(seg.start, |u| padded_start(&transcript.words, u));
    let new_end = eu.map_or(seg.end, |u| padded_end(&transcript.words, u, source_duration));
    validate_range(&store, STAGE, &seg.source_id, new_start, new_end)?;

    let mut new = old.clone();
    new.segments[idx].start = new_start;
    new.segments[idx].end = new_end;
    let detail = json!({
        "segment_id": seg.id,
        "start_unit": input.start_unit, "end_unit": input.end_unit,
        "before": {"start": seg.start, "end": seg.end},
        "after": {"start": new_start, "end": new_end},
    });
    let extras = Extras { removed_seconds: Some(clip_duration(&old) - clip_duration(&new)), ..Default::default() };
    finish(&ctx, &store, STAGE, &old, new, extras, detail)
}

// ---------------------------------------------------------------------------------------------
// edit.remove_pauses
// ---------------------------------------------------------------------------------------------

fn default_min_gap() -> f64 {
    0.5
}
fn default_keep() -> f64 {
    0.12
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct RemovePausesInput {
    pub clip_id: ClipId,
    /// Minimum word gap (seconds) that counts as a pause. Default 0.5.
    #[serde(default = "default_min_gap")]
    pub min_gap: f64,
    /// Seconds of each pause to keep (split evenly around the cut). Default 0.12.
    #[serde(default = "default_keep")]
    pub keep: f64,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn remove_pauses(ctx: Ctx, input: RemovePausesInput) -> Result<EditOut> {
    const STAGE: &str = "edit.remove_pauses";
    if !input.min_gap.is_finite() || input.min_gap <= 0.0 {
        return Err(verr(STAGE, "min_gap must be a positive number of seconds"));
    }
    if !input.keep.is_finite() || input.keep < 0.0 {
        return Err(verr(STAGE, "keep must be >= 0 seconds"));
    }
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let mut tr = Transcripts::new(&store);
    let mut cuts_by_seg: HashMap<SegmentId, Vec<(f64, f64)>> = HashMap::new();
    let mut cut_list: Vec<Cut> = Vec::new();
    for seg in &old.segments {
        let t = tr.require(&seg.source_id, STAGE)?;
        for pair in t.words.windows(2) {
            let g0 = pair[0].end.max(seg.start);
            let g1 = pair[1].start.min(seg.end);
            let gap = g1 - g0;
            if gap + 1e-9 < input.min_gap || gap <= input.keep + 1e-9 {
                continue;
            }
            let (a, b) = (g0 + input.keep / 2.0, g1 - input.keep / 2.0);
            cuts_by_seg.entry(seg.id.clone()).or_default().push((a, b));
            cut_list.push(Cut { segment_id: seg.id.clone(), start: a, end: b });
        }
    }
    let mut new = old.clone();
    let created = rebuild_with_cuts(&mut new, STAGE, &cuts_by_seg)?;
    let removed = clip_duration(&old) - clip_duration(&new);
    let mut extras = Extras {
        removed_seconds: Some(removed),
        cuts: Some(cut_list),
        new_segment_ids: Some(created),
        ..Default::default()
    };
    if cuts_by_seg.is_empty() {
        extras.warnings.push(format!("no pauses of {} s or more found", input.min_gap));
    }
    let detail = json!({"min_gap": input.min_gap, "keep": input.keep, "removed_seconds": removed});
    finish(&ctx, &store, STAGE, &old, new, extras, detail)
}

// ---------------------------------------------------------------------------------------------
// edit.remove_words
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct RemoveWordsInput {
    pub clip_id: ClipId,
    /// Words to remove (case/punctuation-insensitive exact match). Default when neither this nor
    /// word_indices is given: um, uh, umm, uhh, hmm, erm.
    #[serde(default)]
    pub words: Option<Vec<String>>,
    /// Transcript word indices (`Word.i`) to remove. When given without `words`, only these are removed.
    #[serde(default)]
    pub word_indices: Option<Vec<u32>>,
    /// Source the indices refer to; required only when the clip uses several sources.
    #[serde(default)]
    pub source_id: Option<SourceId>,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn remove_words(ctx: Ctx, input: RemoveWordsInput) -> Result<EditOut> {
    const STAGE: &str = "edit.remove_words";
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    let patterns: HashSet<String> = match (&input.words, &input.word_indices) {
        (Some(w), _) => w.iter().map(|s| norm_token(s)).filter(|s| !s.is_empty()).collect(),
        (None, None) => DEFAULT_FILLERS.iter().map(|s| s.to_string()).collect(),
        (None, Some(_)) => HashSet::new(),
    };
    let indices: HashSet<u32> = input.word_indices.iter().flatten().copied().collect();
    let index_source: Option<SourceId> = if indices.is_empty() {
        None
    } else if let Some(s) = &input.source_id {
        Some(s.clone())
    } else {
        let mut srcs: Vec<&SourceId> = old.segments.iter().map(|s| &s.source_id).collect();
        srcs.dedup();
        let uniq: HashSet<&SourceId> = srcs.iter().copied().collect();
        if uniq.len() != 1 {
            return Err(verr(STAGE, "word_indices need source_id because the clip uses several sources"));
        }
        Some(srcs[0].clone())
    };

    let mut tr = Transcripts::new(&store);
    if let Some(src) = &index_source {
        let t = tr.require(src, STAGE)?;
        let known: HashSet<u32> = t.words.iter().map(|w| w.i).collect();
        let mut missing: Vec<u32> = indices.iter().copied().filter(|i| !known.contains(i)).collect();
        missing.sort_unstable();
        if !missing.is_empty() {
            return Err(verr(STAGE, format!("word indices not in the transcript of {src}: {missing:?}")));
        }
    }
    let mut cuts_by_seg: HashMap<SegmentId, Vec<(f64, f64)>> = HashMap::new();
    let mut removed_words: Vec<RemovedWord> = Vec::new();
    for seg in &old.segments {
        let t = tr.require(&seg.source_id, STAGE)?;
        for w in &t.words {
            if w.end <= seg.start || w.start >= seg.end {
                continue;
            }
            let by_index = index_source.as_ref() == Some(&seg.source_id) && indices.contains(&w.i);
            if !(by_index || patterns.contains(&norm_token(&w.text))) {
                continue;
            }
            cuts_by_seg.entry(seg.id.clone()).or_default().push((w.start.max(seg.start), w.end.min(seg.end)));
            removed_words.push(RemovedWord { segment_id: seg.id.clone(), i: w.i, text: w.text.clone(), start: w.start, end: w.end });
        }
    }
    let mut new = old.clone();
    let created = rebuild_with_cuts(&mut new, STAGE, &cuts_by_seg)?;
    let removed = clip_duration(&old) - clip_duration(&new);
    let mut extras = Extras {
        removed_seconds: Some(removed),
        new_segment_ids: Some(created),
        removed_words: Some(removed_words.clone()),
        ..Default::default()
    };
    if removed_words.is_empty() {
        extras.warnings.push("no matching words found inside the clip's segments".to_string());
    }
    let detail = json!({
        "removed": removed_words.iter().map(|w| json!({"i": w.i, "text": w.text})).collect::<Vec<_>>(),
        "removed_seconds": removed,
    });
    finish(&ctx, &store, STAGE, &old, new, extras, detail)
}

// ---------------------------------------------------------------------------------------------
// history.*
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct HistoryListInput {
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct HistoryEntry {
    pub revision: u64,
    /// File modification time (RFC 3339): for a snapshot, when it was superseded.
    pub timestamp: String,
    pub segments: usize,
    pub duration: f64,
    /// True for the live revision (clip.json).
    pub current: bool,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct HistoryList {
    pub clip_id: ClipId,
    pub current_revision: u64,
    /// Ascending by revision; every non-current entry can be passed to history.restore.
    pub revisions: Vec<HistoryEntry>,
}

fn file_timestamp(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339())
        .unwrap_or_default()
}

async fn history_list(ctx: Ctx, input: HistoryListInput) -> Result<HistoryList> {
    const STAGE: &str = "history.list";
    let (store, clip) = load(&ctx, STAGE, &input.clip_id, None)?;
    let dir = store.clip_dir(&clip.id).join("history");
    let mut revs: Vec<HistoryEntry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(rev) = name.strip_suffix(".clip.json").and_then(|s| s.parse::<u64>().ok()) else { continue };
            let snap = store.load_clip_snapshot(&clip.id, rev)?;
            revs.push(HistoryEntry {
                revision: rev,
                timestamp: file_timestamp(&entry.path()),
                segments: snap.segments.len(),
                duration: clip_duration(&snap),
                current: false,
            });
        }
    }
    revs.push(HistoryEntry {
        revision: clip.revision,
        timestamp: file_timestamp(&store.clip_dir(&clip.id).join("clip.json")),
        segments: clip.segments.len(),
        duration: clip_duration(&clip),
        current: true,
    });
    revs.sort_by_key(|e| e.revision);
    Ok(HistoryList { clip_id: clip.id.clone(), current_revision: clip.revision, revisions: revs })
}

/// new_revision -> revision whose content it restored (from undo/restore events of this clip).
fn restore_map(store: &ProjectStore, clip_id: &ClipId) -> HashMap<u64, u64> {
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string(store.root.join("events.ndjson")) else { return map };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if !matches!(v["kind"].as_str(), Some("history.undo" | "history.restore")) {
            continue;
        }
        let d = &v["data"];
        if d["clip_id"].as_str() != Some(clip_id.as_str()) {
            continue;
        }
        if let (Some(to), Some(from)) = (d["to_revision"].as_u64(), d["detail"]["restored_from"].as_u64()) {
            map.insert(to, from);
        }
    }
    map
}

/// Restores snapshot `target` as a new revision (keeps the clip's current render records).
fn restore_revision(ctx: &Ctx, stage: &str, store: &ProjectStore, old: &Clip, target: u64, kind: &str) -> Result<EditOut> {
    if target == old.revision {
        return Err(verr(stage, format!("revision {target} is already the current revision")));
    }
    let mut new = store.load_clip_snapshot(&old.id, target)?;
    new.renders = old.renders.clone();
    let extras = Extras { restored_from: Some(target), ..Default::default() };
    let mut out = finish(ctx, store, kind, old, new, extras, json!({"restored_from": target}))?;
    // The layers of that revision too (overlays, sfx, effects, images, masks, ...).
    if let EditOut::Applied { extras, .. } = &mut out
        && !crate::timeline_doc::restore_extras(store, &old.id, target)?
    {
        extras.warnings.push(format!(
            "revision {target} has no layer snapshot (saved by an older build or a command that does not snapshot layers); extras.json was left as it is"
        ));
    }
    Ok(out)
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct HistoryUndoInput {
    pub clip_id: ClipId,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn history_undo(ctx: Ctx, input: HistoryUndoInput) -> Result<EditOut> {
    const STAGE: &str = "history.undo";
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    // Walk back to the edit that produced the current content, then to the state before it, so
    // repeated undo steps further back instead of toggling.
    let map = restore_map(&store, &old.id);
    let mut origin = old.revision;
    while let Some(&from) = map.get(&origin) {
        origin = from;
    }
    // Revision 1 is the first saved state of a clip, so it has no predecessor snapshot.
    let nothing = || verr(STAGE, "nothing to undo").hint("the clip is at its original revision");
    if origin <= 1 {
        return Err(nothing());
    }
    restore_revision(&ctx, STAGE, &store, &old, origin - 1, STAGE).map_err(|e| if e.code == ErrorCode::NotFound { nothing() } else { e })
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct HistoryRestoreInput {
    pub clip_id: ClipId,
    /// A snapshot revision from `history.list`.
    pub revision: u64,
    #[serde(default)]
    pub expect_revision: Option<u64>,
}

async fn history_restore(ctx: Ctx, input: HistoryRestoreInput) -> Result<EditOut> {
    const STAGE: &str = "history.restore";
    let (store, old) = load(&ctx, STAGE, &input.clip_id, input.expect_revision)?;
    restore_revision(&ctx, STAGE, &store, &old, input.revision, STAGE)
}
