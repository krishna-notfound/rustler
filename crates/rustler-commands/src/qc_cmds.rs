//! Commands: qc.run, export.clip (Task 11).
//!
//! `qc.run` runs the QC checks on a render, saves the report to `clips/<id>/qc/<rid>.json`,
//! points `RenderRecord.qc` at it (revision-checked `save_clip`) and returns the report with a
//! `verdict` and `look_at` (images to open). A failed QC is `Err(QcFailed)` (exit 7) unless
//! `allow_fail`/`--yes` is given. `export.clip` copies a QC-passed render into `exports/`
//! with a `.report.json` (and a `.srt` sidecar when captions are on and a transcript exists).

use chrono::Utc;
use rustler_captions::{CaptionWord, write_srt};
use rustler_core::model::CaptionScript;
use rustler_core::{
    Clip, ClipId, ErrorCode, Event, ProjectStore, QcStatus, RenderId, RenderRecord, RustlerError, Transcript,
};
use rustler_media::qc::{QcView, default_font, names_with, view};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::{Tool, ToolPaths};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const STAGE: &str = "qc";

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "qc.run",
        "Run QC on a render (latest by default): streams, dimensions, duration, A/V sync, loudness, black/frozen video, silence, hook, caption safe zone, plus a contact sheet you must look at. Exit 7 when it fails.",
        &["clip_id"],
        Annotations { long_running: true, ..Annotations::default() },
        vec![
            Example { description: "QC the latest render of a clip", cli: "rustler qc run clp_X" },
            Example {
                description: "get the failing report as data instead of exit 7",
                cli: "rustler qc run clp_X --allow-fail",
            },
        ],
        qc_run,
    ));
    reg.register(typed(
        "export.clip",
        "Copy a QC-passed render to exports/<name>.mp4 with <name>.report.json (and <name>.srt when captions are on). Refuses a failed QC unless --yes.",
        &["clip_id"],
        Annotations::default(),
        vec![
            Example { description: "export the latest render", cli: "rustler export clip clp_X" },
            Example { description: "export under a chosen file name", cli: "rustler export clip clp_X --name my-hook-clip" },
        ],
        export_clip,
    ));
}

// ---------------------------------------------------------------------------------------
// qc.run
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct QcRunInput {
    pub clip_id: ClipId,
    /// Render to check; default the latest render of the clip.
    pub render_id: Option<RenderId>,
    /// Return a failed report as data (`passed:false`) instead of the `qc_failed` error. `--yes` does the same.
    #[serde(default)]
    pub allow_fail: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct QcRunOutput {
    #[serde(flatten)]
    pub view: QcView,
    /// Where the report was saved (`clips/<id>/qc/<render id>.json`).
    pub report_path: PathBuf,
    /// True when `--dry-run`: QC ran into a scratch dir and nothing was saved.
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn pick_render(clip: &Clip, render_id: Option<&RenderId>) -> rustler_core::Result<RenderRecord> {
    match render_id {
        Some(id) => clip.renders.iter().find(|r| &r.id == id).cloned().ok_or_else(|| {
            RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no render {id}", clip.id))
                .hint(format!("see renders with `rustler clip show {}`", clip.id))
        }),
        None => clip.renders.last().cloned().ok_or_else(|| {
            RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no renders yet", clip.id))
                .hint(format!("render it first: rustler render run {}", clip.id))
        }),
    }
}

fn tools(ctx: &Ctx) -> rustler_core::Result<ToolPaths> {
    let paths = ToolPaths::from_home(&ctx.home);
    paths.require(Tool::Ffmpeg)?;
    paths.require(Tool::Ffprobe)?;
    Ok(paths)
}

/// Runs QC. With `persist`: saves the report JSON, sets `RenderRecord.qc` via a revision-checked
/// `save_clip` and appends an event. Without: everything goes to a scratch dir.
async fn execute_qc(
    ctx: &Ctx,
    store: &ProjectStore,
    mut clip: Clip,
    render: &RenderRecord,
    persist: bool,
) -> rustler_core::Result<(QcView, PathBuf)> {
    let paths = tools(ctx)?;
    let font = default_font(&paths).ok_or_else(|| {
        RustlerError::new(ErrorCode::DependencyMissing, STAGE, "no font available for the contact sheet")
            .hint("run `rustler setup` (installs fonts)")
    })?;
    let out_dir = if persist {
        store.clip_dir(&clip.id).join("qc")
    } else {
        std::env::temp_dir().join(format!("rustler-qc-dry-{}-{}", std::process::id(), render.id))
    };
    let render_path = store.resolve(&render.path);
    let report = rustler_media::run_qc(&paths, &clip, render, &render_path, &font, &out_dir).await?;
    let v = view(report);
    let report_path = out_dir.join(format!("{}.json", render.id));
    if !persist {
        return Ok((v, report_path));
    }
    store.write_json(&report_path, &v)?;

    let base = clip.revision;
    let rec = clip.renders.iter_mut().find(|r| r.id == render.id);
    if let Some(rec) = rec {
        rec.qc = Some(store.relativize(&report_path));
    }
    store.save_clip(&mut clip, base)?;
    store.append_event(&Event {
        ts: Utc::now(),
        actor: ctx.actor,
        kind: "qc.run".into(),
        project_revision: store.load_project()?.revision,
        data: json!({
            "clip_id": clip.id,
            "render_id": render.id,
            "passed": v.report.passed,
            "failed": names_with(&v.report, QcStatus::Fail),
            "warned": names_with(&v.report, QcStatus::Warn),
            "report": store.relativize(&report_path),
        }),
    })?;
    Ok((v, report_path))
}

/// Fix hints of every failing check, joined for an error hint.
fn failing_hints(v: &QcView) -> String {
    v.report
        .checks
        .iter()
        .filter(|c| c.status == QcStatus::Fail)
        .filter_map(|c| c.fix_hint.as_deref().map(|h| format!("{}: {h}", c.name)))
        .collect::<Vec<_>>()
        .join(" | ")
}

async fn qc_run(ctx: Ctx, input: QcRunInput) -> rustler_core::Result<QcRunOutput> {
    let store = ctx.store()?;
    let clip = store.load_clip(&input.clip_id)?;
    let render = pick_render(&clip, input.render_id.as_ref())?;
    let (v, report_path) = execute_qc(&ctx, &store, clip, &render, !ctx.dry_run).await?;
    let mut warnings = Vec::new();
    if ctx.dry_run {
        warnings.push("dry run: QC ran into a scratch directory; no report saved and the clip is unchanged".to_string());
    }
    if !v.report.passed {
        if input.allow_fail || ctx.yes {
            warnings.push(format!("QC failed ({}); returned as data because allow_fail/--yes is set", v.verdict));
        } else {
            return Err(RustlerError::new(
                ErrorCode::QcFailed,
                STAGE,
                format!("QC failed: {}", names_with(&v.report, QcStatus::Fail).join(", ")),
            )
            .hint(format!(
                "report: {}; contact sheet (open it): {}; fixes: {}; pass --allow-fail to get the report as data",
                report_path.display(),
                v.report.contact_sheet.display(),
                failing_hints(&v)
            )));
        }
    }
    Ok(QcRunOutput { view: v, report_path, dry_run: ctx.dry_run, warnings })
}

// ---------------------------------------------------------------------------------------
// export.clip
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ExportInput {
    pub clip_id: ClipId,
    /// Render to export; default the latest.
    pub render_id: Option<RenderId>,
    /// File name stem in `exports/`; default the clip name.
    pub name: Option<String>,
    /// Export even though QC failed (same as the global `--yes`).
    #[serde(default)]
    pub allow_fail: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct ExportOutput {
    pub video: PathBuf,
    pub report: PathBuf,
    /// Caption sidecar, when captions are enabled and the transcript covers the clip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub srt: Option<PathBuf>,
    pub qc_passed: bool,
    pub verdict: String,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A safe file-name stem: no path separators or characters Windows rejects.
fn sanitize_name(raw: &str, fallback: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_matches('.').trim();
    let s: String = s.chars().take(80).collect();
    let s = s.trim_end().to_string();
    if s.is_empty() { fallback.to_string() } else { s }
}

/// Caption words in output time from the clip's segments (own mapping, no romanization:
/// the saved transcript is already in the project's caption script).
fn srt_words(store: &ProjectStore, clip: &Clip, warnings: &mut Vec<String>) -> Vec<CaptionWord> {
    let mut transcripts: HashMap<String, Option<Transcript>> = HashMap::new();
    let mut words = Vec::new();
    let mut out0 = 0.0;
    for seg in &clip.segments {
        let key = seg.source_id.to_string();
        let tr = transcripts.entry(key.clone()).or_insert_with(|| {
            store
                .read_json::<Transcript>(&Path::new("analysis").join(&key).join("transcript.json"))
                .ok()
        });
        if let Some(tr) = tr {
            let compatible = clip.captions.script == CaptionScript::AsTranscribed || tr.script == clip.captions.script;
            if compatible {
                for w in &tr.words {
                    if w.end <= seg.start || w.start >= seg.end {
                        continue;
                    }
                    words.push(CaptionWord {
                        text: w.text.clone(),
                        out_start: out0 + (w.start.max(seg.start) - seg.start),
                        out_end: out0 + (w.end.min(seg.end) - seg.start),
                    });
                }
            } else {
                warnings.push(format!(
                    "srt skipped for source {key}: transcript script {:?} differs from caption script {:?}",
                    tr.script, clip.captions.script
                ));
            }
        }
        out0 += (seg.end - seg.start).max(0.0);
    }
    words
}

async fn export_clip(ctx: Ctx, input: ExportInput) -> rustler_core::Result<ExportOutput> {
    let store = ctx.store()?;
    let clip = store.load_clip(&input.clip_id)?;
    let render = pick_render(&clip, input.render_id.as_ref())?;
    let src = store.resolve(&render.path);
    if !src.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, "export", format!("render file missing: {}", src.display()))
            .hint(format!("re-render: rustler render run {}", clip.id)));
    }
    let mut warnings = Vec::new();

    // QC gate: use the saved report, or run QC now when there is none.
    let saved = render
        .qc
        .as_ref()
        .and_then(|p| store.read_json::<QcView>(p).ok());
    let qc: QcView = match saved {
        Some(v) => v,
        None if ctx.dry_run => {
            warnings.push("no QC report yet; a real export would run QC first".to_string());
            return dry_export(&store, &clip, &input, warnings);
        }
        None => execute_qc(&ctx, &store, clip.clone(), &render, true).await?.0,
    };
    if !qc.report.passed && !(ctx.yes || input.allow_fail) {
        return Err(RustlerError::new(
            ErrorCode::Validation,
            "export",
            format!("QC failed for render {}: {}", render.id, names_with(&qc.report, QcStatus::Fail).join(", ")),
        )
        .hint(format!(
            "fix and re-render ({}), or export anyway with --yes",
            failing_hints(&qc)
        )));
    }
    if !qc.report.passed {
        warnings.push(format!("exported despite failed QC: {}", qc.verdict));
    }

    let name = sanitize_name(input.name.as_deref().unwrap_or(&clip.name), clip.id.as_str());
    let dir = store.exports_dir();
    let video = dir.join(format!("{name}.mp4"));
    let report = dir.join(format!("{name}.report.json"));
    let srt_path = dir.join(format!("{name}.srt"));

    let srt_text = if clip.captions.enabled {
        let words = srt_words(&store, &clip, &mut warnings);
        if words.is_empty() {
            warnings.push("no srt written: no transcript words inside the clip's segments".to_string());
            None
        } else {
            Some(write_srt(&words, clip.captions.max_words_per_line))
        }
    } else {
        None
    };

    if ctx.dry_run {
        warnings.push("dry run: nothing copied or written".to_string());
        return Ok(ExportOutput {
            video,
            report,
            srt: srt_text.map(|_| srt_path),
            qc_passed: qc.report.passed,
            verdict: qc.verdict,
            dry_run: true,
            warnings,
        });
    }

    std::fs::create_dir_all(&dir).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, "export", format!("cannot create {}: {e}", dir.display()))
    })?;
    tokio::fs::copy(&src, &video).await.map_err(|e| {
        RustlerError::new(ErrorCode::Internal, "export", format!("cannot copy render to {}: {e}", video.display()))
    })?;
    let mut srt_out = None;
    if let Some(text) = srt_text {
        std::fs::write(&srt_path, text).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "export", format!("cannot write {}: {e}", srt_path.display()))
        })?;
        srt_out = Some(srt_path);
    }

    let source_refs: Vec<_> = clip
        .segments
        .iter()
        .map(|s| {
            let src = store.load_source(&s.source_id).ok();
            json!({
                "segment_id": s.id,
                "source_id": s.source_id,
                "source_name": src.as_ref().map(|x| x.name.clone()),
                "origin": src.as_ref().map(|x| x.origin.clone()),
                "start": s.start,
                "end": s.end,
            })
        })
        .collect();
    store.write_json(
        &report,
        &json!({
            "clip": clip,
            "render": render,
            "qc": qc,
            "brief": crate::brief_cmds::brief_status(&store, &clip.id),
            "source_refs": source_refs,
            "created_at": Utc::now(),
        }),
    )?;
    store.append_event(&Event {
        ts: Utc::now(),
        actor: ctx.actor,
        kind: "export.clip".into(),
        project_revision: store.load_project()?.revision,
        data: json!({
            "clip_id": clip.id,
            "render_id": render.id,
            "video": store.relativize(&video),
            "qc_passed": qc.report.passed,
            "srt": srt_out.as_ref().map(|p| store.relativize(p)),
        }),
    })?;
    Ok(ExportOutput {
        video,
        report,
        srt: srt_out,
        qc_passed: qc.report.passed,
        verdict: qc.verdict,
        dry_run: false,
        warnings,
    })
}

/// `--dry-run` with no QC report yet: paths only.
fn dry_export(
    store: &ProjectStore,
    clip: &Clip,
    input: &ExportInput,
    mut warnings: Vec<String>,
) -> rustler_core::Result<ExportOutput> {
    let name = sanitize_name(input.name.as_deref().unwrap_or(&clip.name), clip.id.as_str());
    let dir = store.exports_dir();
    warnings.push("dry run: nothing copied or written".to_string());
    Ok(ExportOutput {
        video: dir.join(format!("{name}.mp4")),
        report: dir.join(format!("{name}.report.json")),
        srt: None,
        qc_passed: false,
        verdict: "QC not run yet (dry run)".to_string(),
        dry_run: true,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::sanitize_name;

    #[test]
    fn names_are_filesystem_safe() {
        assert_eq!(sanitize_name("a/b\\c:d?.", "fb"), "a_b_c_d_");
        assert_eq!(sanitize_name("  ..  ", "fb"), "fb");
        assert_eq!(sanitize_name("मेरा क्लिप (1)", "fb"), "मेरा क्लिप (1)");
        assert_eq!(sanitize_name(&"x".repeat(200), "fb").len(), 80);
    }
}
