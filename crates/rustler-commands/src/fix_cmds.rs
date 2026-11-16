//! Commands: fix.suggest, fix.apply (Task B3, quality loop v2).
//!
//! `fix.suggest` turns the latest QC report of a clip into an ordered list of concrete
//! `rustler ...` commands. `fix.apply` executes only the suggestions that are deterministic
//! and non-creative (`auto_safe`): moving captions off a face or out of the platform UI zone,
//! removing pauses, and re-rendering. It never touches creative choices (hooks, layouts,
//! cuts, music). Steps run through the same registry commands an agent would call.

use crate::qc_cmds::{execute_qc_with, pick_render};
use rustler_core::{ClipId, ErrorCode, Event, ProjectStore, QcStatus, RenderId, RenderRecord, RustlerError};
use rustler_media::qc::QcView;
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "fix.suggest",
        "Turn the latest QC report of a clip into an ordered list of fixes: each has the check, a severity, why, the exact `rustler ...` commands and whether it is auto_safe (deterministic, non-creative). Runs QC first when the render has no report.",
        &["clip_id"],
        Annotations { long_running: true, ..Annotations::default() },
        vec![Example { description: "what should I fix on this clip?", cli: "rustler fix suggest clp_X" }],
        fix_suggest,
    ));
    reg.register(typed(
        "fix.apply",
        "Apply the auto_safe fixes from `fix.suggest` (caption position, remove pauses, re-render) and re-run QC on the new render. Never changes creative choices. --check applies one suggestion only.",
        &["clip_id"],
        Annotations { long_running: true, ..Annotations::default() },
        vec![
            Example { description: "apply every safe fix, re-render and re-check", cli: "rustler fix apply clp_X" },
            Example {
                description: "apply one fix without re-rendering yet",
                cli: "rustler fix apply clp_X --check caption_over_face --rerender false",
            },
        ],
        fix_apply,
    ));
}

const STAGE: &str = "fix";

// ---------------------------------------------------------------------------------------
// suggestions
// ---------------------------------------------------------------------------------------

/// One recommended fix.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Suggestion {
    /// The QC check this fixes.
    pub check: String,
    /// high (QC failed) | medium (warning that hurts the clip) | low (polish / informational)
    pub severity: String,
    pub why: String,
    /// Exact commands to run, in order, with the real clip id.
    pub commands: Vec<String>,
    /// True only for deterministic, non-creative fixes that `fix.apply` may run for you.
    pub auto_safe: bool,
}

/// What `fix.apply` does for an auto-safe suggestion.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    RemovePauses,
    SetCaptionY(f64),
    Rerender,
}

/// Order in which warnings are listed (lower first); failures always come before warnings.
const PRIORITY: &[&str] = &[
    "streams",
    "dimensions",
    "duration",
    "av_sync",
    "black_frames",
    "hook_text_present",
    "face_visible_ratio",
    "face_cut_off",
    "caption_over_face",
    "duration_for_platform",
    "frozen_video",
    "first_frame_not_black",
    "hook_window",
    "long_silence",
    "caption_safe_zone",
    "loudness",
];

fn priority(name: &str) -> usize {
    PRIORITY.iter().position(|p| *p == name).unwrap_or(PRIORITY.len())
}

/// Warnings that are polish rather than something that hurts the clip.
fn is_low(name: &str) -> bool {
    matches!(name, "first_frame_not_black" | "hook_window" | "long_silence" | "caption_safe_zone" | "loudness")
}

/// `rustler ...` commands inside a fix hint, in order.
pub fn commands_from_hint(hint: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = hint;
    while let Some(i) = rest.find("rustler ") {
        let tail = &rest[i..];
        let mut end = tail.len();
        for stop in [";", ")", "|", " (", ", rustler "] {
            if let Some(j) = tail.find(stop) {
                end = end.min(j);
            }
        }
        let cmd = tail[..end].trim().trim_end_matches(['.', ':', ',']).trim().to_string();
        if !cmd.is_empty() && !out.contains(&cmd) {
            out.push(cmd);
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// The explanatory half of a hint (before the first `; ` or `: rustler`).
fn why_from_hint(hint: &str) -> String {
    let cut = ["; ", ": rustler "].iter().filter_map(|s| hint.find(s)).min().unwrap_or(hint.len());
    hint[..cut].trim().to_string()
}

fn suggest_from_report(clip_id: &ClipId, v: &QcView) -> Vec<(Suggestion, Vec<Op>)> {
    let cid = clip_id.to_string();
    let render = format!("rustler render run {cid}");
    let mut out: Vec<(Suggestion, Vec<Op>)> = Vec::new();
    for c in v.report.checks.iter().filter(|c| c.status != QcStatus::Pass) {
        let skipped = c.value.get("skipped").is_some();
        let hint = c.fix_hint.clone().unwrap_or_default();
        let mut why = if hint.is_empty() { c.name.clone() } else { why_from_hint(&hint) };
        why = format!("{why} (expected: {})", c.expected);
        let (mut commands, mut ops) = (commands_from_hint(&hint), Vec::new());
        let mut auto_safe = false;
        match c.name.as_str() {
            _ if skipped => {
                commands = vec!["rustler doctor".into(), format!("rustler qc run {cid} --allow-fail")];
                why = format!("{}: the Python engine was unavailable, so this check did not run", c.name);
            }
            "loudness" => {
                commands = vec![render.clone()];
                ops = vec![Op::Rerender];
                auto_safe = true;
            }
            "long_silence" => {
                commands = vec![format!("rustler edit remove-pauses {cid}"), render.clone()];
                ops = vec![Op::RemovePauses, Op::Rerender];
                auto_safe = true;
            }
            "caption_safe_zone" => {
                commands = vec![format!("rustler clip set-captions {cid} --position-y 0.70"), render.clone()];
                ops = vec![Op::SetCaptionY(0.70), Op::Rerender];
                auto_safe = true;
            }
            "caption_over_face" => {
                if let Some(y) = c.value.get("suggested_position_y").and_then(Value::as_f64) {
                    commands = vec![format!("rustler clip set-captions {cid} --position-y {y:.2}"), render.clone()];
                    ops = vec![Op::SetCaptionY(y), Op::Rerender];
                    auto_safe = true;
                }
            }
            _ => {}
        }
        let severity = if skipped || (c.status == QcStatus::Warn && is_low(&c.name)) {
            "low"
        } else if c.status == QcStatus::Fail {
            "high"
        } else {
            "medium"
        };
        out.push((
            Suggestion { check: c.name.clone(), severity: severity.into(), why, commands, auto_safe },
            ops,
        ));
    }
    // failures first, then by impact; checks skipped for lack of the engine last
    out.sort_by_key(|(s, _)| {
        let sev = match s.severity.as_str() {
            "high" => 0,
            "medium" => 1,
            _ => 2,
        };
        let skipped = s.commands.first().is_some_and(|c| c == "rustler doctor");
        (skipped, sev, priority(&s.check))
    });
    out
}

/// The render to look at, its QC report (running QC when missing) and the ordered suggestions.
async fn suggest_inner(
    ctx: &Ctx,
    store: &ProjectStore,
    clip_id: &ClipId,
    render_id: Option<&RenderId>,
    faces: bool,
) -> rustler_core::Result<(RenderRecord, QcView, bool, Vec<(Suggestion, Vec<Op>)>)> {
    let clip = store.load_clip(clip_id)?;
    let render = pick_render(&clip, render_id)?;
    let saved = render.qc.as_ref().and_then(|p| crate::qc_cmds::read_qc(store, p));
    let (v, ran) = match saved {
        Some(v) => (v, false),
        None => (execute_qc_with(ctx, store, clip, &render, !ctx.dry_run, faces).await?.0, true),
    };
    let s = suggest_from_report(clip_id, &v);
    Ok((render, v, ran, s))
}

#[derive(Deserialize, JsonSchema)]
pub struct FixSuggestInput {
    pub clip_id: ClipId,
    /// Render to look at; default the latest.
    pub render_id: Option<RenderId>,
    /// When the render has no QC report yet, run QC with the engine face checks too (they decide
    /// `caption_over_face` suggestions). A saved report is used as it is: run `qc run --faces` first.
    #[serde(default)]
    pub faces: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct FixSuggestOutput {
    pub clip_id: ClipId,
    pub render_id: RenderId,
    pub passed: bool,
    pub verdict: String,
    /// True when the render had no QC report and QC ran first.
    pub ran_qc: bool,
    pub suggestions: Vec<Suggestion>,
    pub look_at: Vec<PathBuf>,
    pub next: Vec<String>,
}

async fn fix_suggest(ctx: Ctx, input: FixSuggestInput) -> rustler_core::Result<FixSuggestOutput> {
    let store = ctx.store()?;
    let (render, v, ran_qc, s) = suggest_inner(&ctx, &store, &input.clip_id, input.render_id.as_ref(), input.faces).await?;
    let suggestions: Vec<Suggestion> = s.into_iter().map(|(s, _)| s).collect();
    let mut next = Vec::new();
    if suggestions.iter().any(|s| s.auto_safe) {
        next.push(format!("rustler fix apply {}", input.clip_id));
    }
    if suggestions.is_empty() {
        next.push(format!("rustler export clip {}", input.clip_id));
    } else {
        next.push(format!("rustler qc run {} --allow-fail", input.clip_id));
    }
    let look_at = vec![store.resolve(&v.report.contact_sheet)];
    Ok(FixSuggestOutput {
        clip_id: input.clip_id,
        render_id: render.id,
        passed: v.report.passed,
        verdict: v.verdict,
        ran_qc,
        suggestions,
        look_at,
        next,
    })
}

// ---------------------------------------------------------------------------------------
// fix.apply
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct FixApplyInput {
    pub clip_id: ClipId,
    /// Render whose QC report drives the fixes; default the latest.
    pub render_id: Option<RenderId>,
    /// Apply only the suggestion for this check (it must be auto_safe).
    pub check: Option<String>,
    /// Apply every auto_safe suggestion (default true; ignored when `check` is given).
    #[serde(default = "yes")]
    pub all_safe: bool,
    /// Re-render and re-run QC after the changes (default true).
    #[serde(default = "yes")]
    pub rerender: bool,
    /// Run the engine face checks when QC runs (needed to find/verify `caption_over_face`; starts the engine).
    #[serde(default)]
    pub faces: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, JsonSchema)]
pub struct AppliedStep {
    /// The registry command that ran.
    pub command: String,
    /// The equivalent CLI line.
    pub cli: String,
    /// What changed, in one line.
    pub change: String,
}

#[derive(Serialize, JsonSchema)]
pub struct FixAfter {
    pub render_id: RenderId,
    pub passed: bool,
    pub verdict: String,
    /// Suggestions still open on the new render.
    pub remaining: Vec<Suggestion>,
    pub look_at: Vec<PathBuf>,
}

#[derive(Serialize, JsonSchema)]
pub struct FixApplyOutput {
    pub clip_id: ClipId,
    /// Checks whose fixes were applied.
    pub fixed_checks: Vec<String>,
    pub applied: Vec<AppliedStep>,
    /// Suggestions left alone because they need a creative decision (commands to run yourself).
    pub not_applied: Vec<Suggestion>,
    pub revision_before: u64,
    pub revision_after: u64,
    /// Present when a new render was made and re-checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<FixAfter>,
    pub dry_run: bool,
    pub next: Vec<String>,
}

fn local_registry() -> Registry {
    let mut reg = Registry::new();
    crate::register_all(&mut reg);
    reg
}

fn fail_with(applied: &[AppliedStep], step: &str, e: RustlerError) -> RustlerError {
    let done = if applied.is_empty() {
        "nothing was changed".to_string()
    } else {
        format!("already applied: {}", applied.iter().map(|a| a.cli.clone()).collect::<Vec<_>>().join(" | "))
    };
    let hint = e.hint.clone().unwrap_or_default();
    RustlerError::new(e.code, STAGE, format!("fix step `{step}` failed: {} ({done})", e.message))
        .hint(format!("{hint} Re-run `rustler fix suggest <clip_id>` to see what is left"))
}

async fn fix_apply(ctx: Ctx, input: FixApplyInput) -> rustler_core::Result<FixApplyOutput> {
    let store = ctx.store()?;
    let (render, _view, _ran, all) = suggest_inner(&ctx, &store, &input.clip_id, input.render_id.as_ref(), input.faces).await?;
    let before = store.load_clip(&input.clip_id)?;

    // choose suggestions
    let chosen: Vec<&(Suggestion, Vec<Op>)> = match &input.check {
        Some(name) => {
            let hit = all.iter().find(|(s, _)| &s.check == name).ok_or_else(|| {
                RustlerError::new(
                    ErrorCode::NotFound,
                    STAGE,
                    format!("no open suggestion for check `{name}` on render {}", render.id),
                )
                .hint(format!(
                    "open suggestions: {}; list them with `rustler fix suggest {}`",
                    all.iter().map(|(s, _)| s.check.as_str()).collect::<Vec<_>>().join(", "),
                    input.clip_id
                ))
            })?;
            if !hit.0.auto_safe {
                return Err(RustlerError::new(
                    ErrorCode::Validation,
                    STAGE,
                    format!("`{name}` needs a creative choice, so fix.apply will not do it for you"),
                )
                .hint(format!("run yourself: {}", hit.0.commands.join(" ; "))));
            }
            vec![hit]
        }
        None if input.all_safe => all.iter().filter(|(s, _)| s.auto_safe).collect(),
        None => {
            return Err(RustlerError::new(ErrorCode::Validation, STAGE, "nothing selected: pass --check <name> or leave all_safe on")
                .hint(format!("see `rustler fix suggest {}`", input.clip_id)));
        }
    };
    let not_applied: Vec<Suggestion> = all
        .iter()
        .filter(|(s, _)| !chosen.iter().any(|(c, _)| c.check == s.check))
        .map(|(s, _)| s.clone())
        .collect();

    // plan: remove-pauses, then caption position (first wins; caption_over_face sorts before the safe-zone one), then one re-render
    let ops: Vec<Op> = chosen.iter().flat_map(|(_, o)| o.iter().copied()).collect();
    let mut plan: Vec<Op> = Vec::new();
    if ops.contains(&Op::RemovePauses) {
        plan.push(Op::RemovePauses);
    }
    if let Some(y) = ops.iter().find_map(|o| if let Op::SetCaptionY(y) = o { Some(*y) } else { None }) {
        plan.push(Op::SetCaptionY(y));
    }
    let wants_render = input.rerender && ops.contains(&Op::Rerender);
    let fixed_checks: Vec<String> = chosen.iter().map(|(s, _)| s.check.clone()).collect();
    let cid = input.clip_id.to_string();

    let mut applied: Vec<AppliedStep> = Vec::new();
    let describe = |op: &Op| match op {
        Op::RemovePauses => ("edit.remove_pauses".to_string(), format!("rustler edit remove-pauses {cid}")),
        Op::SetCaptionY(y) => ("clip.set_captions".to_string(), format!("rustler clip set-captions {cid} --position-y {y:.2}")),
        Op::Rerender => ("render.run".to_string(), format!("rustler render run {cid}")),
    };
    if ctx.dry_run {
        for op in plan.iter().chain(wants_render.then_some(&Op::Rerender)) {
            let (command, cli) = describe(op);
            applied.push(AppliedStep { command, cli, change: "dry run: not executed".into() });
        }
        return Ok(FixApplyOutput {
            clip_id: input.clip_id,
            fixed_checks,
            applied,
            not_applied,
            revision_before: before.revision,
            revision_after: before.revision,
            after: None,
            dry_run: true,
            next: vec![format!("rustler fix apply {cid}")],
        });
    }

    let reg = local_registry();
    for op in &plan {
        let (command, cli) = describe(op);
        let (inp, change) = match op {
            Op::RemovePauses => (json!({ "clip_id": input.clip_id }), "pauses shortened".to_string()),
            Op::SetCaptionY(y) => (
                json!({ "clip_id": input.clip_id, "position_y": y }),
                format!("caption position_y {:.2} -> {y:.2}", before.captions.position_y),
            ),
            Op::Rerender => unreachable!("render is run after the clip edits"),
        };
        let res = reg.dispatch(&command, inp, ctx.clone()).await;
        if let Err(e) = res {
            return Err(fail_with(&applied, &cli, e));
        }
        applied.push(AppliedStep { command, cli, change });
    }

    let mut after = None;
    if wants_render {
        let (command, cli) = describe(&Op::Rerender);
        let quality = serde_json::to_value(render.quality).unwrap_or(json!("final"));
        let out = reg
            .dispatch(&command, json!({ "clip_id": input.clip_id, "quality": quality }), ctx.clone())
            .await
            .map_err(|e| fail_with(&applied, &cli, e))?;
        let new_id: RenderId = out
            .get("render_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| RustlerError::new(ErrorCode::Internal, STAGE, "render.run returned no render_id"))?;
        applied.push(AppliedStep { command, cli, change: format!("new render {new_id}") });
        reg
            .dispatch("qc.run", json!({ "clip_id": input.clip_id, "render_id": new_id, "allow_fail": true, "faces": input.faces }), ctx.clone())
            .await
            .map_err(|e| fail_with(&applied, "rustler qc run --allow-fail", e))?;
        applied.push(AppliedStep {
            command: "qc.run".into(),
            cli: format!("rustler qc run {cid} --allow-fail"),
            change: "re-checked the new render".into(),
        });
        let (_, v, _, rest) = suggest_inner(&ctx, &store, &input.clip_id, Some(&new_id), input.faces).await?;
        let look_at = vec![store.resolve(&v.report.contact_sheet)];
        after = Some(FixAfter {
            render_id: new_id,
            passed: v.report.passed,
            verdict: v.verdict,
            remaining: rest.into_iter().map(|(s, _)| s).collect(),
            look_at,
        });
    }

    let now = store.load_clip(&input.clip_id)?;
    store.append_event(&Event {
        ts: chrono::Utc::now(),
        actor: ctx.actor,
        kind: "fix.apply".into(),
        project_revision: store.load_project()?.revision,
        data: json!({
            "clip_id": input.clip_id,
            "fixed": fixed_checks,
            "steps": applied.iter().map(|a| a.cli.clone()).collect::<Vec<_>>(),
            "revision": [before.revision, now.revision],
        }),
    })?;
    let mut next = Vec::new();
    match &after {
        Some(a) if a.remaining.is_empty() => next.push(format!("rustler export clip {cid}")),
        Some(_) => next.push(format!("rustler fix suggest {cid}")),
        None if !plan.is_empty() => next.push(format!("rustler render run {cid}")),
        None => {}
    }
    Ok(FixApplyOutput {
        clip_id: input.clip_id,
        fixed_checks,
        applied,
        not_applied,
        revision_before: before.revision,
        revision_after: now.revision,
        after,
        dry_run: false,
        next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_extracted_from_hints() {
        let h = "video is frozen from 3.0s (output time); look at the contact sheet, then trim the segment: rustler edit trim clp_1 --end-delta -2 (or split and drop it: rustler edit split clp_1 --out-t 3.0)";
        assert_eq!(
            commands_from_hint(h),
            vec!["rustler edit trim clp_1 --end-delta -2", "rustler edit split clp_1 --out-t 3.0"]
        );
        let h2 = "too long; shorten it: rustler edit trim clp_1 --end-delta -15 (or tighten the cut: rustler edit refine clp_1, rustler edit remove-pauses clp_1)";
        assert_eq!(
            commands_from_hint(h2),
            vec![
                "rustler edit trim clp_1 --end-delta -15",
                "rustler edit refine clp_1",
                "rustler edit remove-pauses clp_1"
            ]
        );
        assert_eq!(why_from_hint(h2), "too long");
    }
}
