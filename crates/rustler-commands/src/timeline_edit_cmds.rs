//! Commands: edit.insert_segment, edit.cold_open, clip.merge, clip.duplicate, edit.speed,
//! edit.freeze (plan "Added to wave C": custom clip merge for hooks, more timeline control).
//!
//! Every command is a typed helper over the timeline v2 document: it assembles the clip,
//! changes the document, validates it like `timeline.apply` and writes it back as one new
//! revision (history snapshots of clip.json and extras.json, so `history undo` covers every
//! layer). Re-cuts (insert, cold open) keep every layer on its content: time-based items move
//! with the words they were on, word-anchored items keep their anchors, transitions and segment
//! volumes follow their segment.

use crate::clip_cmds::log_event;
use crate::timeline_doc::{self, DocEnv, Piece, Remap, SegTimes, identity_pieces, next_item_id, rebuild, sub_segment};
use rustler_core::layout::LayoutParams;
use rustler_core::timeline::{EffectItem, EffectKind, FreezeItem, TransitionItem, TransitionKind};
use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError, Segment, SegmentId, SourceId, Timeline, VideoSeg};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

type Result<T> = rustler_core::Result<T>;

/// Leftover pieces shorter than this (seconds) are dropped when a range is cut out.
const MIN_PIECE: f64 = 0.1;
/// Boundary snap for `at_out_t` (seconds).
const SNAP: f64 = 0.05;
/// Length of the cold-open flash (seconds).
const FLASH_LEN: f64 = 0.25;

fn verr(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn restage(stage: &str) -> impl Fn(RustlerError) -> RustlerError + '_ {
    move |mut e| {
        e.stage = stage.to_string();
        e
    }
}

/// Result of every command here.
#[derive(Serialize, JsonSchema, Debug)]
pub struct TimelineEditOut {
    pub clip_id: ClipId,
    /// New revision (pass it as base_revision next time).
    pub revision: u64,
    /// One line per change.
    pub changes: Vec<String>,
    /// Edit-timeline duration (sum of segment lengths; what item times are on).
    pub edit_duration: f64,
    /// Length of the rendered file (speeds, freezes, transitions applied).
    pub rendered_duration: f64,
    pub video: Vec<SegTimes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_segment_id: Option<SegmentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn load_current(ctx: &Ctx, stage: &str, clip_id: &ClipId, base: Option<u64>) -> Result<(ProjectStore, Timeline)> {
    let store = ctx.store()?;
    let (cur, _) = timeline_doc::assemble(&store, clip_id).map_err(restage(stage))?;
    if let Some(b) = base
        && b != cur.revision
    {
        return Err(RustlerError::new(
            ErrorCode::RevisionConflict,
            stage,
            format!("clip {clip_id} is at revision {} but base_revision was {b}", cur.revision),
        )
        .hint(format!("run `rustler timeline show {clip_id}` and retry with its revision")));
    }
    Ok((store, cur))
}

/// Validates `new`, writes it (unless dry run) and builds the output. `current` = None creates a
/// new clip (added to the project).
#[allow(clippy::too_many_arguments)]
fn commit(
    ctx: &Ctx,
    store: &ProjectStore,
    stage: &str,
    current: Option<&Timeline>,
    new: Timeline,
    mut warnings: Vec<String>,
    new_segment_id: Option<SegmentId>,
    detail: Value,
) -> Result<TimelineEditOut> {
    warnings.extend(timeline_doc::validate(store, &new, current).map_err(restage(stage))?);
    let changes = match current {
        Some(c) => timeline_doc::diff(c, &new),
        None => vec![format!("clip {} created ({} segments)", new.id, new.tracks.video.len())],
    };
    let id = new.id.clone();
    let next = vec![
        format!("rustler timeline show {id}"),
        format!("rustler render preview-frame {id} --out-t 0.5"),
        format!("rustler render run {id} --quality draft"),
    ];
    let out = |doc: &Timeline, revision: u64, changes: Vec<String>, warnings: Vec<String>, dry: bool| -> Result<TimelineEditOut> {
        let env = DocEnv::new(store, doc)?;
        let r = timeline_doc::resolve(&env, doc);
        Ok(TimelineEditOut {
            clip_id: doc.id.clone(),
            revision,
            changes,
            edit_duration: r.edit_duration,
            rendered_duration: r.rendered_duration,
            video: r.video,
            new_segment_id: new_segment_id.clone(),
            dry_run: dry.then_some(true),
            warnings,
            next: next.clone(),
        })
    };
    let base = current.map_or(0, |c| c.revision);
    if ctx.dry_run {
        return out(&new, base, changes, warnings, true);
    }
    if current.is_some() && changes.is_empty() {
        warnings.push("edit made no change; nothing was saved".into());
        return out(&new, base, changes, warnings, false);
    }
    let revision = timeline_doc::disassemble(store, &new, base).map_err(restage(stage))?;
    if current.is_none() {
        let mut project = store.load_project()?;
        let pbase = project.revision;
        project.clips.push(id.clone());
        store.save_project(&mut project, pbase)?;
    }
    let mut detail = detail;
    if let Value::Object(m) = &mut detail {
        m.insert("clip_id".into(), json!(id));
        m.insert("to_revision".into(), json!(revision));
    }
    log_event(ctx, store, stage, detail)?;
    let (doc, w) = timeline_doc::assemble(store, &id)?;
    warnings.extend(w);
    out(&doc, revision, changes, warnings, false)
}

/// A new segment resolved exactly like `clip.add_segment` (units, expect_text, source checks,
/// layout presets), without saving anything.
async fn resolve_segment(ctx: &Ctx, stage: &str, body: Value) -> Result<Segment> {
    let mut reg = Registry::new();
    crate::clip_cmds::register(&mut reg);
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let v = reg.dispatch("clip.add_segment", body, dry).await.map_err(restage(stage))?;
    let last = v["segments"].as_array().and_then(|s| s.last()).cloned().unwrap_or(Value::Null);
    serde_json::from_value(last).map_err(|e| RustlerError::new(ErrorCode::Internal, stage, format!("resolved segment: {e}")))
}

fn to_video(s: Segment) -> VideoSeg {
    VideoSeg { id: s.id, source_id: s.source_id, src_in: s.start, src_out: s.end, layout: s.layout, crop_path: None, speed: None }
}

/// Range of a new segment: transcript units (preferred) or source seconds.
#[derive(Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct RangeIn {
    /// First transcript unit (preferred over raw times).
    pub start_unit: Option<u32>,
    /// Last transcript unit (default: start_unit).
    pub end_unit: Option<u32>,
    /// Source seconds (when no units are given).
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// The first words you expect; a mismatch is a validation error.
    pub expect_text: Option<String>,
}

fn range_body(clip_id: &ClipId, source_id: &SourceId, r: &RangeIn, layout: Option<&str>, params: Option<&LayoutParams>) -> Value {
    let mut b = json!({"clip_id": clip_id, "source_id": source_id});
    let m = b.as_object_mut().expect("object");
    for (k, v) in [
        ("start_unit", r.start_unit.map(|x| json!(x))),
        ("end_unit", r.end_unit.map(|x| json!(x))),
        ("start", r.start.map(|x| json!(x))),
        ("end", r.end.map(|x| json!(x))),
        ("expect_text", r.expect_text.as_ref().map(|x| json!(x))),
        ("layout", layout.map(|x| json!(x))),
        ("layout_params", params.map(|x| json!(x))),
    ] {
        if let Some(v) = v {
            m.insert(k.into(), v);
        }
    }
    b
}

// ---------------------------------------------------------------------------------------------
// edit.insert_segment
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct InsertSegmentIn {
    pub clip_id: ClipId,
    /// Insert before the segment at this index (0 = first; = segment count appends).
    pub at_index: Option<usize>,
    /// Or: insert at this edit-timeline time; inside a segment the segment is split there.
    pub at_out_t: Option<f64>,
    pub source_id: SourceId,
    #[serde(flatten)]
    pub range: RangeIn,
    /// Layout preset (default: the neighbouring segment's layout when it shows the same source,
    /// else its preset).
    pub layout: Option<String>,
    pub layout_params: Option<LayoutParams>,
    /// Fail with revision_conflict unless the clip is at this revision.
    pub base_revision: Option<u64>,
}

async fn insert_segment(ctx: Ctx, i: InsertSegmentIn) -> Result<TimelineEditOut> {
    const STAGE: &str = "edit.insert_segment";
    let (store, cur) = load_current(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let video = &cur.tracks.video;
    let n = video.len();
    // Where: (index, split point in source seconds of the segment before it).
    let (index, split): (usize, Option<(usize, f64)>) = match (i.at_index, i.at_out_t) {
        (Some(_), Some(_)) => return Err(verr(STAGE, "give at_index or at_out_t, not both")),
        (None, None) => return Err(verr(STAGE, "give at_index (0 = first) or at_out_t (edit-timeline seconds)")),
        (Some(k), None) if k > n => return Err(verr(STAGE, format!("at_index {k} is past the end (the clip has {n} segments)"))),
        (Some(k), None) => (k, None),
        (None, Some(t)) => {
            let dur = cur.edit_duration();
            if !(t.is_finite() && t >= 0.0 && t <= dur + SNAP) {
                return Err(verr(STAGE, format!("at_out_t {t} is outside the clip (0 .. {dur:.2}s edit timeline)")));
            }
            let mut o = 0.0;
            let mut found = (n, None);
            for (k, v) in video.iter().enumerate() {
                let len = v.src_out - v.src_in;
                if (t - o).abs() <= SNAP {
                    found = (k, None);
                    break;
                }
                if t < o + len - SNAP {
                    found = (k + 1, Some((k, v.src_in + (t - o))));
                    break;
                }
                o += len;
            }
            found
        }
    };
    let neighbour = video.get(index.saturating_sub(1).min(n.saturating_sub(1))).or(video.first());
    let same_source = neighbour.filter(|v| v.source_id == i.source_id && i.layout.is_none() && i.layout_params.is_none());
    let layout_name = i.layout.clone().or_else(|| neighbour.and_then(|v| v.layout.preset.clone()));
    let body = range_body(&cur.id, &i.source_id, &i.range, layout_name.as_deref(), i.layout_params.as_ref());
    let mut seg = to_video(resolve_segment(&ctx, STAGE, body).await?);
    if let Some(v) = same_source {
        seg.layout = v.layout.clone();
    }
    let new_id = seg.id.clone();

    let mut pieces = identity_pieces(video);
    if let Some((k, s)) = split {
        let v = &video[k];
        pieces.splice(
            k..=k,
            [
                Piece { seg: sub_segment(v, v.src_in, s, true), origins: vec![(v.id.clone(), v.src_in, s)] },
                Piece { seg: sub_segment(v, s, v.src_out, false), origins: vec![(v.id.clone(), s, v.src_out)] },
            ],
        );
    }
    pieces.insert(index, Piece { seg, origins: Vec::new() });
    let (new, warnings) = rebuild(&cur, pieces);
    let detail = json!({"segment_id": new_id, "index": index, "split": split.map(|s| s.1)});
    commit(&ctx, &store, STAGE, Some(&cur), new, warnings, Some(new_id), detail)
}

// ---------------------------------------------------------------------------------------------
// edit.cold_open
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct ColdOpenIn {
    pub clip_id: ClipId,
    /// Source of the hook (default: the first segment's source).
    pub source_id: Option<SourceId>,
    #[serde(flatten)]
    pub range: RangeIn,
    /// White flash at the cut out of the hook (default true; an `effects` item you can edit).
    pub flash: Option<bool>,
    pub base_revision: Option<u64>,
}

async fn cold_open(ctx: Ctx, i: ColdOpenIn) -> Result<TimelineEditOut> {
    const STAGE: &str = "edit.cold_open";
    let (store, cur) = load_current(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let video = &cur.tracks.video;
    let source_id = match &i.source_id {
        Some(s) => s.clone(),
        None => video.first().map(|v| v.source_id.clone()).ok_or_else(|| verr(STAGE, "clip has no segments"))?,
    };
    let first_layout = video.iter().find(|v| v.source_id == source_id).and_then(|v| v.layout.preset.clone());
    let body = range_body(&cur.id, &source_id, &i.range, first_layout.as_deref(), None);
    let resolved = to_video(resolve_segment(&ctx, STAGE, body).await?);
    let (hs, he) = (resolved.src_in, resolved.src_out);

    let mut hook_origins = Vec::new();
    let mut overlapped: Option<&VideoSeg> = None;
    let mut whole: Option<VideoSeg> = None;
    let mut rest: Vec<Piece> = Vec::new();
    let mut warnings = Vec::new();
    for v in video {
        let (a, b) = (hs.max(v.src_in), he.min(v.src_out));
        if v.source_id != source_id || b - a <= 0.02 {
            rest.extend(identity_pieces(std::slice::from_ref(v)));
            continue;
        }
        hook_origins.push((v.id.clone(), a, b));
        overlapped.get_or_insert(v);
        if (hs - v.src_in).abs() < 0.02 && (he - v.src_out).abs() < 0.02 {
            whole = Some(v.clone());
            continue;
        }
        let mut keep_id = true;
        if a - v.src_in >= MIN_PIECE {
            rest.push(Piece { seg: sub_segment(v, v.src_in, a, true), origins: vec![(v.id.clone(), v.src_in, a)] });
            keep_id = false;
        } else if a > v.src_in + 1e-6 {
            warnings.push(format!("{:.2}s left of segment {} before the hook were dropped", a - v.src_in, v.id));
        }
        if v.src_out - b >= MIN_PIECE {
            rest.push(Piece { seg: sub_segment(v, b, v.src_out, keep_id), origins: vec![(v.id.clone(), b, v.src_out)] });
        } else if v.src_out > b + 1e-6 {
            warnings.push(format!("{:.2}s left of segment {} after the hook were dropped", v.src_out - b, v.id));
        }
    }
    if rest.is_empty() {
        return Err(verr(STAGE, "the hook range covers the whole clip; nothing would follow it")
            .hint("pick a shorter range (one or two sentences) inside or outside the clip"));
    }
    let hook = match (whole, overlapped) {
        (Some(v), _) => v,
        (None, Some(v)) => {
            let mut s = sub_segment(v, hs, he, false);
            s.speed = None;
            s
        }
        (None, None) => resolved,
    };
    let hook_id = hook.id.clone();
    let moved = !hook_origins.is_empty();
    let mut pieces = vec![Piece { seg: hook, origins: hook_origins }];
    pieces.extend(rest);
    let (mut new, w) = rebuild(&cur, pieces);
    warnings.extend(w);
    // A hook title that opened the clip keeps opening it (it belongs to the start, not to the
    // words it happened to sit on).
    for (old, t) in cur.tracks.text.iter().zip(new.tracks.text.iter_mut()) {
        if old.id == t.id && old.style.is_hook() && old.start < 0.05 {
            t.start = old.start;
            t.end = old.end;
        }
    }
    let hook_len = he - hs;
    if i.flash.unwrap_or(true) {
        let dur = new.edit_duration();
        let id = next_item_id("fx", new.tracks.effects.iter().map(|e| e.id.as_str()));
        new.tracks.effects.push(EffectItem {
            id,
            kind: EffectKind::Flash,
            start: hook_len,
            end: (hook_len + FLASH_LEN).min(dur),
            amount: None,
            extra: Default::default(),
        });
    }
    let detail = json!({"hook_segment_id": hook_id, "start": hs, "end": he, "moved_from_clip": moved, "flash": i.flash.unwrap_or(true)});
    let mut out = commit(&ctx, &store, STAGE, Some(&cur), new, warnings, Some(hook_id), detail)?;
    out.next.insert(0, format!("rustler render preview-frame {} --out-t {:.2}", out.clip_id, hook_len + 0.1));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// clip.merge / clip.duplicate
// ---------------------------------------------------------------------------------------------

fn default_transition_duration() -> f64 {
    0.3
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct MergeIn {
    /// Clips to concatenate, in order (at least 2; the same clip may repeat).
    pub clip_ids: Vec<ClipId>,
    pub name: String,
    /// Transition between the clips (cut | fade | dissolve | wipeleft | wiperight | slideup | zoomin).
    pub transition: Option<TransitionKind>,
    /// Transition overlap in seconds (default 0.3).
    #[serde(default = "default_transition_duration")]
    pub transition_duration: f64,
}

/// Shifts every time-based item of `t` by `off` seconds and clamps it to `[off, off + len]`.
fn shift_items(t: &mut Timeline, off: f64, len: f64) {
    let end = off + len;
    let tr = &mut t.tracks;
    let sh = |v: &mut f64| *v = (*v + off).min(end);
    for x in &mut tr.text {
        sh(&mut x.start);
        sh(&mut x.end);
    }
    for x in &mut tr.effects {
        sh(&mut x.start);
        sh(&mut x.end);
    }
    let timings = tr
        .images
        .iter_mut()
        .map(|x| &mut x.timing)
        .chain(tr.masks.iter_mut().map(|x| &mut x.timing))
        .chain(tr.cutaways.iter_mut().map(|x| &mut x.timing));
    for tm in timings {
        // A missing start means "from the start of that clip", a missing end "to its end".
        tm.out_start = Some(tm.out_start.unwrap_or(0.0) + off).map(|v| v.min(end));
        if tm.out_end.is_some() || tm.end_word.is_none() {
            tm.out_end = Some((tm.out_end.unwrap_or(len) + off).min(end));
        }
    }
    for x in &mut tr.audio.sfx {
        if let Some(a) = &mut x.at {
            sh(a);
        }
    }
    for x in &mut tr.freezes {
        sh(&mut x.out_t);
    }
}

fn prefix_of(id: &str) -> &str {
    id.rsplit_once('_').map_or("item", |(p, _)| p)
}

async fn merge(ctx: Ctx, i: MergeIn) -> Result<TimelineEditOut> {
    const STAGE: &str = "clip.merge";
    if i.clip_ids.len() < 2 {
        return Err(verr(STAGE, "give at least 2 clip_ids to merge").hint("use clip.duplicate to copy one clip"));
    }
    if i.name.trim().is_empty() {
        return Err(verr(STAGE, "name must not be empty"));
    }
    if !(i.transition_duration.is_finite() && i.transition_duration > 0.0 && i.transition_duration <= 2.0) {
        return Err(verr(STAGE, "transition_duration must be > 0 and <= 2 s"));
    }
    let store = ctx.store()?;
    let mut docs = Vec::new();
    for id in &i.clip_ids {
        docs.push(timeline_doc::assemble(&store, id).map_err(restage(STAGE))?.0);
    }
    let first = docs[0].clone();
    let mut warnings = Vec::new();
    let mut out = first.clone();
    out.id = ClipId::new();
    out.name = i.name.clone();
    out.revision = 0;
    out.meta = Default::default();
    out.meta.other.insert("merged_from".into(), json!(i.clip_ids));
    let tr = &mut out.tracks;
    tr.video.clear();
    tr.freezes.clear();
    tr.cutaways.clear();
    tr.images.clear();
    tr.masks.clear();
    tr.text.clear();
    tr.effects.clear();
    tr.transitions.clear();
    tr.audio.sfx.clear();
    tr.audio.segment_volumes.clear();
    tr.captions.word_overrides.clear();

    let mut off = 0.0;
    let mut last_seg: Option<SegmentId> = None;
    for (n, d) in docs.iter().enumerate() {
        if d.canvas != first.canvas {
            return Err(verr(STAGE, format!("clip {} has a different canvas than {}", d.id, first.id)));
        }
        if n > 0 {
            if d.tracks.audio.music != first.tracks.audio.music {
                warnings.push(format!("music of {} is not used: the merged clip keeps the first clip's music bed", d.id));
            }
            if d.tracks.captions.settings != first.tracks.captions.settings {
                warnings.push(format!("caption settings of {} differ: the merged clip uses the first clip's", d.id));
            }
            for k in d.tracks.extra.keys().filter(|k| !first.tracks.extra.contains_key(*k)) {
                warnings.push(format!("extras key '{k}' of {} is not carried over (unknown to this build)", d.id));
            }
        }
        let len = d.edit_duration();
        let mut part = d.clone();
        // Fresh segment ids (the same clip may be merged twice).
        let mut ids = std::collections::HashMap::new();
        for v in &mut part.tracks.video {
            let new_id = SegmentId::new();
            ids.insert(v.id.clone(), new_id.clone());
            v.id = new_id;
        }
        shift_items(&mut part, off, len);
        // Unique item ids per track.
        let taken: Vec<(&'static str, String)> = timeline_doc::item_ids(&out);
        let mut used: std::collections::HashMap<&'static str, Vec<String>> = std::collections::HashMap::new();
        for (track, id) in taken {
            used.entry(track).or_default().push(id);
        }
        macro_rules! renumber {
            ($list:expr, $track:expr) => {
                for x in $list.iter_mut() {
                    let u = used.entry($track).or_default();
                    if u.contains(&x.id) {
                        x.id = next_item_id(prefix_of(&x.id), u.iter().map(|s| s.as_str()));
                    }
                    u.push(x.id.clone());
                }
            };
        }
        let p = &mut part.tracks;
        renumber!(p.freezes, "freezes");
        renumber!(p.cutaways, "cutaways");
        renumber!(p.images, "images");
        renumber!(p.masks, "masks");
        renumber!(p.text, "text");
        renumber!(p.effects, "effects");
        renumber!(p.audio.sfx, "sfx");
        let tr = &mut out.tracks;
        if let (Some(prev), Some(kind)) = (&last_seg, i.transition)
            && !p.video.is_empty()
        {
            tr.transitions.push(TransitionItem {
                after_segment_id: prev.clone(),
                kind,
                duration: i.transition_duration,
                extra: Default::default(),
            });
        }
        for t in &p.transitions {
            if let Some(a) = ids.get(&t.after_segment_id) {
                tr.transitions.push(TransitionItem { after_segment_id: a.clone(), ..t.clone() });
            }
        }
        for (sid, db) in &p.audio.segment_volumes {
            if let Some(a) = ids.iter().find(|(o, _)| o.as_str() == sid).map(|(_, a)| a) {
                tr.audio.segment_volumes.insert(a.as_str().to_string(), *db);
            }
        }
        for w in &p.captions.word_overrides {
            if !tr.captions.word_overrides.iter().any(|x| x.word_anchor == w.word_anchor) {
                tr.captions.word_overrides.push(w.clone());
            }
        }
        last_seg = p.video.last().map(|v| v.id.clone()).or(last_seg);
        tr.video.append(&mut p.video);
        tr.freezes.append(&mut p.freezes);
        tr.cutaways.append(&mut p.cutaways);
        tr.images.append(&mut p.images);
        tr.masks.append(&mut p.masks);
        tr.text.append(&mut p.text);
        tr.effects.append(&mut p.effects);
        tr.audio.sfx.append(&mut p.audio.sfx);
        off += len;
    }
    let detail = json!({"clip_ids": i.clip_ids, "transition": i.transition});
    let mut res = commit(&ctx, &store, STAGE, None, out, warnings, None, detail)?;
    res.next.insert(0, format!("rustler clip retention {}", res.clip_id));
    Ok(res)
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct DuplicateIn {
    pub clip_id: ClipId,
    pub name: String,
}

async fn duplicate(ctx: Ctx, i: DuplicateIn) -> Result<TimelineEditOut> {
    const STAGE: &str = "clip.duplicate";
    if i.name.trim().is_empty() {
        return Err(verr(STAGE, "name must not be empty"));
    }
    let store = ctx.store()?;
    let (mut doc, warnings) = timeline_doc::assemble(&store, &i.clip_id).map_err(restage(STAGE))?;
    doc.id = ClipId::new();
    doc.name = i.name.clone();
    doc.revision = 0;
    doc.meta.other.insert("duplicated_from".into(), json!(i.clip_id));
    let detail = json!({"from": i.clip_id});
    commit(&ctx, &store, STAGE, None, doc, warnings, None, detail)
}

// ---------------------------------------------------------------------------------------------
// edit.speed / edit.freeze
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct SpeedIn {
    pub clip_id: ClipId,
    pub segment_id: SegmentId,
    /// 0.5 (half speed) .. 2.0 (double); 1 resets.
    pub factor: f64,
    pub base_revision: Option<u64>,
}

async fn speed(ctx: Ctx, i: SpeedIn) -> Result<TimelineEditOut> {
    const STAGE: &str = "edit.speed";
    use rustler_media::compile::{MAX_SPEED, MIN_SPEED};
    if !(i.factor.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&i.factor)) {
        return Err(verr(STAGE, format!("factor {} must be between {MIN_SPEED} and {MAX_SPEED}", i.factor))
            .hint("1.1-1.25 tightens slow talk without sounding fast; 1 resets"));
    }
    let (store, cur) = load_current(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let mut new = cur.clone();
    let seg = new.tracks.video.iter_mut().find(|v| v.id == i.segment_id).ok_or_else(|| {
        let ids: Vec<&str> = cur.tracks.video.iter().map(|v| v.id.as_str()).collect();
        RustlerError::new(ErrorCode::NotFound, STAGE, format!("segment {} not in clip {}", i.segment_id, i.clip_id))
            .hint(format!("segments: {}", ids.join(", ")))
    })?;
    seg.speed = ((i.factor - 1.0).abs() > 1e-9).then_some(i.factor);
    let detail = json!({"segment_id": i.segment_id, "factor": i.factor});
    commit(&ctx, &store, STAGE, Some(&cur), new, Vec::new(), None, detail)
}

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct FreezeIn {
    pub clip_id: ClipId,
    /// Edit-timeline seconds where the picture holds.
    pub out_t: f64,
    /// Seconds to hold (0 < duration <= 10).
    pub duration: f64,
    pub base_revision: Option<u64>,
}

async fn freeze(ctx: Ctx, i: FreezeIn) -> Result<TimelineEditOut> {
    const STAGE: &str = "edit.freeze";
    let (store, cur) = load_current(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let mut new = cur.clone();
    let id = next_item_id("frz", new.tracks.freezes.iter().map(|f| f.id.as_str()));
    new.tracks.freezes.push(FreezeItem { id: id.clone(), out_t: i.out_t, duration: i.duration, extra: Default::default() });
    let detail = json!({"freeze_id": id, "out_t": i.out_t, "duration": i.duration});
    let mut out = commit(&ctx, &store, STAGE, Some(&cur), new, Vec::new(), None, detail)?;
    out.next.insert(0, format!("rustler render preview-frame {} --out-t {:.2}", out.clip_id, i.out_t));
    Ok(out)
}

// Remap is part of this module's public toolkit through timeline_doc; keep the import used.
#[allow(dead_code)]
fn _uses(_: &Remap) {}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    reg.register(typed(
        "edit.insert_segment",
        "Insert a source range (transcript units or seconds) as a new segment at an index or at an edit-timeline time (splitting the segment there); every layer after it moves with its content",
        &["clip_id"],
        write,
        vec![
            Example { description: "put units 40-42 of a source first", cli: "rustler edit insert-segment clp_x --source-id src_y --start-unit 40 --end-unit 42 --at-index 0" },
            Example { description: "insert a reaction at 12.5 s", cli: "rustler edit insert-segment clp_x --source-id src_y --start 300 --end 303.5 --at-out-t 12.5" },
        ],
        insert_segment,
    ));
    reg.register(typed(
        "edit.cold_open",
        "Hook first: put a range (units or seconds) at the very start of the clip, cutting it out of where it was if the clip already contains it, plus a white flash at the cut out of the hook (flash: false to skip); layers follow their content, word anchors keep working",
        &["clip_id"],
        write,
        vec![
            Example { description: "open on the payoff line", cli: "rustler edit cold-open clp_x --start-unit 57 --end-unit 57" },
            Example { description: "seconds, no flash", cli: "rustler edit cold-open clp_x --start 812.4 --end 815.9 --flash false" },
        ],
        cold_open,
    ));
    reg.register(typed(
        "clip.merge",
        "New clip that plays clips back to back with ALL their layers (captions, text, images, masks, cutaways, sfx, effects, transitions, freezes, speeds, crop paths): time-based items shift by each clip's offset, word-anchored items keep their anchors; optional transition between the clips",
        &["clip_ids"],
        write,
        vec![Example { description: "hook clip + main clip with a quick dissolve", cli: "rustler clip merge clp_hook clp_main --name hook-plus-main --transition dissolve --transition-duration 0.2" }],
        merge,
    ));
    reg.register(typed(
        "clip.duplicate",
        "Copy a clip with every layer (clip.json + extras) as a new clip, e.g. to try another hook or cut",
        &["clip_id"],
        write,
        vec![Example { description: "copy before trying a cold open", cli: "rustler clip duplicate clp_x --name clp-x-cold-open" }],
        duplicate,
    ));
    reg.register(typed(
        "edit.speed",
        "Play one segment faster or slower (0.5-2.0; pitch kept with atempo); captions, layers and word anchors follow, the rendered duration changes (QC and retention use it)",
        &["clip_id"],
        write,
        vec![Example { description: "tighten a slow answer", cli: "rustler edit speed clp_x --segment-id seg_y --factor 1.2" }],
        speed,
    ));
    reg.register(typed(
        "edit.freeze",
        "Freeze frame: hold the picture at an edit-timeline time for some seconds (silent), then continue; the rendered clip gets longer by the hold",
        &["clip_id"],
        write,
        vec![Example { description: "hold the reaction for 0.8 s", cli: "rustler edit freeze clp_x --out-t 6.2 --duration 0.8" }],
        freeze,
    ));
}
