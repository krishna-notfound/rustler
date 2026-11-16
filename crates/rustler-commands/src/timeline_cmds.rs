//! Commands: timeline.show, timeline.schema, timeline.apply (plan Task 1.2).
//!
//! The timeline v2 document (`rustler.timeline@2`, [`rustler_core::Timeline`]) is the whole clip:
//! segments, every layer, captions, audio, transitions and meta. `timeline.show` prints it with
//! the resolved times of every item, `timeline.apply` changes anything in one revision-checked
//! step (RFC 6902 JSON-Patch ops or a whole document), validated before anything is written, with
//! history snapshots of both storage files (`history undo` covers every layer).

use crate::clip_cmds::log_event;
use crate::timeline_doc::{self, DocEnv, Resolved};
use rustler_core::{ClipId, ErrorCode, RustlerError, TIMELINE_SCHEMA, Timeline};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

type Result<T> = rustler_core::Result<T>;

fn verr(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

/// Rules every agent should know before writing a document (also in `timeline.schema`).
pub const NOTES: &[&str] = &[
    "All item times (start/end, out_start/out_end, at, out_t) are on the EDIT timeline: the sum of the segments' source lengths (tracks.video src_out - src_in). `resolved` in timeline.show maps every item to the rendered file (speeds, freezes and transitions applied).",
    "Word anchors ({source_id, word_i}) follow re-cuts: start_word/end_word on images, masks, cutaways; at_word {source_id, word_index} on sfx; word_anchor on caption word overrides. An anchor whose word was cut out is dropped with a warning at render, never placed at a wrong time.",
    "Rects are normalized 0..1 of the canvas, x+w <= 1, y+h <= 1, w and h > 0.02.",
    "timeline.apply needs base_revision = the revision timeline.show printed; a stale revision fails with revision_conflict and writes nothing (re-show, re-apply).",
    "JSON-Patch paths: /tracks/text/0/start, /tracks/images/-, /tracks/video/1/speed, /tracks/captions/preset, /tracks/audio/music, /meta/score ... ('-' appends).",
    "Ids: keep existing ids; give new items a new unique id per track (img_N, msk_N, cut_N, ovl_N, fx_N, sfx_N, frz_N). New video segments need a seg_ id (any unique string starting with seg_).",
    "tracks.extra holds extras.json keys this build does not know; they are written back verbatim.",
    "Draw order: video -> full cutaways -> masks -> effects -> PiP cutaways -> images (by z) -> captions -> text -> images with above_captions (by z) -> progress bar.",
];

// ---------------------------------------------------------------------------------------------
// timeline.show
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowIn {
    pub clip_id: ClipId,
}

#[derive(Serialize, JsonSchema)]
pub struct ShowOut {
    pub clip_id: ClipId,
    /// Pass this as base_revision to timeline.apply.
    pub revision: u64,
    pub document: Timeline,
    /// Every item's edit-timeline and rendered times (word anchors resolved).
    pub resolved: Resolved,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

async fn show(ctx: Ctx, i: ShowIn) -> Result<ShowOut> {
    let store = ctx.store()?;
    let (doc, mut warnings) = timeline_doc::assemble(&store, &i.clip_id)?;
    let env = DocEnv::new(&store, &doc)?;
    let resolved = timeline_doc::resolve(&env, &doc);
    for it in &resolved.items {
        if let Some(p) = &it.problem {
            warnings.push(format!("{} {} will not render: {p}", it.track, it.id));
        }
    }
    let id = doc.id.clone();
    Ok(ShowOut {
        clip_id: id.clone(),
        revision: doc.revision,
        document: doc,
        resolved,
        warnings,
        next: vec![
            format!("rustler timeline apply {id} --base-revision <revision> --input-json @patch.json"),
            "rustler timeline schema".to_string(),
        ],
    })
}

// ---------------------------------------------------------------------------------------------
// timeline.schema
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaIn {}

#[derive(Serialize, JsonSchema)]
pub struct SchemaOut {
    pub schema_id: String,
    /// JSON Schema of the document.
    pub json_schema: Value,
    pub notes: Vec<String>,
}

async fn schema(_ctx: Ctx, _i: SchemaIn) -> Result<SchemaOut> {
    let s = schemars::schema_for!(Timeline);
    Ok(SchemaOut {
        schema_id: TIMELINE_SCHEMA.to_string(),
        json_schema: serde_json::to_value(s).unwrap_or(Value::Null),
        notes: NOTES.iter().map(|s| s.to_string()).collect(),
    })
}

// ---------------------------------------------------------------------------------------------
// timeline.apply
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyIn {
    pub clip_id: ClipId,
    /// The revision your document / patch is based on (timeline.show `revision`).
    pub base_revision: u64,
    /// RFC 6902 JSON-Patch operations on the document, e.g.
    /// `[{"op":"replace","path":"/tracks/text/0/start","value":1.5}]`.
    #[serde(default)]
    pub patch: Option<Vec<Value>>,
    /// A whole replacement document (instead of `patch`).
    #[serde(default)]
    pub document: Option<Value>,
    /// Validate and show the result without saving (same as --dry-run).
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct ApplyOut {
    pub clip_id: ClipId,
    /// The new revision (unchanged on a dry run or when nothing changed).
    pub revision: u64,
    /// One line per change ("text ovl_1: changed start, end").
    pub changes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub document: Timeline,
    pub resolved: Resolved,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

/// Locates a deserialization error inside the tracks (serde's message has no path).
fn locate_error(doc: &Value) -> Option<String> {
    use rustler_core::timeline::*;
    fn each<T: serde::de::DeserializeOwned>(doc: &Value, ptr: &str) -> Option<String> {
        let list = doc.pointer(ptr)?.as_array()?;
        list.iter().enumerate().find_map(|(k, v)| serde_json::from_value::<T>(v.clone()).err().map(|e| format!("{ptr}/{k}: {e}")))
    }
    each::<rustler_core::VideoSeg>(doc, "/tracks/video")
        .or_else(|| each::<FreezeItem>(doc, "/tracks/freezes"))
        .or_else(|| each::<CutawayItem>(doc, "/tracks/cutaways"))
        .or_else(|| each::<ImageItem>(doc, "/tracks/images"))
        .or_else(|| each::<MaskItem>(doc, "/tracks/masks"))
        .or_else(|| each::<TextItem>(doc, "/tracks/text"))
        .or_else(|| each::<EffectItem>(doc, "/tracks/effects"))
        .or_else(|| each::<TransitionItem>(doc, "/tracks/transitions"))
        .or_else(|| each::<SfxItem>(doc, "/tracks/audio/sfx"))
        .or_else(|| each::<WordOverride>(doc, "/tracks/captions/word_overrides"))
}

async fn apply(ctx: Ctx, i: ApplyIn) -> Result<ApplyOut> {
    const STAGE: &str = "timeline.apply";
    let store = ctx.store()?;
    let dry = ctx.dry_run || i.dry_run.unwrap_or(false);
    let (current, _) = timeline_doc::assemble(&store, &i.clip_id)?;
    if current.revision != i.base_revision {
        return Err(RustlerError::new(
            ErrorCode::RevisionConflict,
            STAGE,
            format!("clip {} is at revision {} but base_revision was {}", i.clip_id, current.revision, i.base_revision),
        )
        .hint(format!("someone changed the clip meanwhile: run `rustler timeline show {}` and re-apply on its revision", i.clip_id)));
    }
    let value = match (i.patch, i.document) {
        (Some(_), Some(_)) => return Err(verr(STAGE, "give either patch or document, not both")),
        (None, None) => {
            return Err(verr(STAGE, "nothing to apply: give patch (JSON-Patch ops) or document")
                .hint("`rustler timeline apply <clip_id> --base-revision N --input-json @patch.json`"));
        }
        (None, Some(doc)) => doc,
        (Some(ops), None) => {
            let ops: json_patch::Patch = serde_json::from_value(Value::Array(ops)).map_err(|e| {
                verr(STAGE, format!("patch is not valid JSON-Patch: {e}"))
                    .hint("ops look like {\"op\":\"replace\",\"path\":\"/tracks/text/0/start\",\"value\":1.5}")
            })?;
            let mut doc = serde_json::to_value(&current).map_err(|e| verr(STAGE, e.to_string()))?;
            json_patch::patch(&mut doc, &ops).map_err(|e| {
                verr(STAGE, format!("patch operation {} failed at {}: {}", e.operation, e.path, e.kind))
                    .hint("`rustler timeline show <clip_id>` prints the document the paths point into")
            })?;
            doc
        }
    };
    let mut new: Timeline = serde_json::from_value(value.clone()).map_err(|e| {
        let at = locate_error(&value).unwrap_or_else(|| e.to_string());
        verr(STAGE, format!("document does not match rustler.timeline@2: {at}")).hint("`rustler timeline schema` documents every field")
    })?;
    if new.id != current.id {
        return Err(verr(STAGE, format!("/id: the document is for clip {}, not {}", new.id, current.id)));
    }
    new.revision = current.revision;
    let mut warnings = timeline_doc::validate(&store, &new, Some(&current)).map_err(|mut e| {
        e.stage = STAGE.into();
        e
    })?;
    let preset = &new.tracks.captions.settings.preset;
    if preset != &current.tracks.captions.settings.preset {
        crate::asset_cmds::check_caption_style(&ctx, STAGE, preset)?;
    }
    let changes = timeline_doc::diff(&current, &new);
    let id = current.id.clone();
    let next = vec![format!("rustler render preview-frame {id} --out-t 1"), format!("rustler render run {id} --quality draft")];
    if dry {
        let env = DocEnv::new(&store, &new)?;
        let resolved = timeline_doc::resolve(&env, &new);
        return Ok(ApplyOut { clip_id: id, revision: current.revision, changes, warnings, document: new, resolved, dry_run: Some(true), next });
    }
    if changes.is_empty() {
        warnings.push("no change; nothing was saved".into());
        let env = DocEnv::new(&store, &current)?;
        let resolved = timeline_doc::resolve(&env, &current);
        return Ok(ApplyOut { clip_id: id, revision: current.revision, changes, warnings, document: current, resolved, dry_run: None, next });
    }
    let revision = timeline_doc::disassemble(&store, &new, i.base_revision).map_err(|mut e| {
        e.stage = STAGE.into();
        e
    })?;
    log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": id, "from_revision": i.base_revision, "to_revision": revision, "changes": changes}))?;
    let (doc, w) = timeline_doc::assemble(&store, &id)?;
    warnings.extend(w);
    let env = DocEnv::new(&store, &doc)?;
    let resolved = timeline_doc::resolve(&env, &doc);
    Ok(ApplyOut { clip_id: id, revision, changes, warnings, document: doc, resolved, dry_run: None, next })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    let read = Annotations { read_only: true, ..write };
    reg.register(typed(
        "timeline.show",
        "The whole clip as one timeline v2 document (rustler.timeline@2): segments, cutaways, images, masks, text, captions + word overrides, audio + music + sfx, effects, transitions, freezes, progress bar, meta; plus every item's resolved edit and rendered times (word anchors resolved) and its revision",
        &["clip_id"],
        read,
        vec![Example { description: "see everything on a clip's timeline", cli: "rustler timeline show clp_x" }],
        show,
    ));
    reg.register(typed(
        "timeline.schema",
        "JSON Schema of the timeline v2 document plus the rules (edit-timeline times, word anchors, rects, ids, JSON-Patch paths); validate your document or patch against it before timeline.apply",
        &[],
        read,
        vec![Example { description: "print the schema", cli: "rustler timeline schema" }],
        schema,
    ));
    reg.register(typed(
        "timeline.apply",
        "Change anything on a clip in one step: RFC 6902 JSON-Patch ops or a whole document, checked against base_revision (stale -> revision_conflict, nothing written), fully validated (rects, times, files, sources, anchors) with every error listed by path; saves clip.json + extras.json as one new revision (history undo covers every layer) and returns the changes, warnings, new document and resolved times. --dry-run validates only",
        &["clip_id"],
        write,
        vec![
            Example { description: "move a hook title (patch in a file; PowerShell-safe)", cli: "rustler timeline apply clp_x --base-revision 7 --input-json @patch.json" },
            Example { description: "validate a whole document without saving", cli: "rustler timeline apply clp_x --base-revision 7 --input-json @doc.json --dry-run" },
        ],
        apply,
    ));
}
