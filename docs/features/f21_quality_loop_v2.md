# f21 — Quality loop v2 (face/caption/hook checks, fix.suggest, fix.apply, qc.compare)

**Status:** Built (tests pass)  |  **Depends on:** f09, f14, f16  |  **Owner:** B3 agent

## Purpose

Turn "QC failed" into "here are the commands that fix it". New checks catch what the first 12 missed (captions over a face, face cut off, no hook text), and
the fix loop lets the agent apply safe repairs automatically and compare renders, while creative choices stay with the agent.

## Scope

**In:** QC checks `caption_over_face`, `face_visible_ratio`, `face_cut_off`, `hook_text_present`, `duration_for_platform` (engine-backed checks degrade to `warn`
with `skipped: engine unavailable`); `fix.suggest <clip> {render_id?}` (ordered: failures first, then warnings by impact, each with `check`, `severity`, `why`,
`commands[]`, `auto_safe`); `fix.apply <clip> {check?, all_safe?, rerender?}` (executes only `auto_safe` fixes: caption position, `edit remove-pauses`, plain
re-render; re-renders and re-runs QC; lists `not_applied`); `qc.compare <clip> <render_a> <render_b>` (side-by-side contact sheet + improved/regressed/same).
**Out (explicitly):** vision-model review (dropped with f13: the agent looks at the sheet), automatic creative fixes.

## User-facing Behavior

`rustler fix suggest clp_X` → list; `rustler fix apply clp_X` → changes + new render + QC + `after.look_at` (open that contact sheet);
`qc compare` returns an image path. `fix.suggest` runs QC first when the render has no report. Knowledge: `quality-checklist`, prompt `fix_qc_failures`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/fix_cmds.rs`, `crates/rustler-commands/src/qc_cmds.rs`
- `crates/rustler-media/src/qc.rs`
- Tests: `crates/rustler-commands/tests/fix.rs` (9 tests), `crates/rustler-commands/tests/qc.rs`, `crates/rustler-media/tests/qc.rs`

**Read-only dependencies (consume, never edit):**
- `engine/rustler_engine/faces.py` (`faces.track` on the render), `crates/rustler-captions`, clip/extras model.

## Rules and Edge Cases

1. `fix.apply` never touches hook text, layout, cuts, music or trims.
2. Face checks only exist where the face matters (center-crop, split top pane); `caption_over_face` only when captions are on.
3. `qc.run` exits 7 on a fail; never use `--yes`/`--allow-fail` to hide it.
4. `export.clip` refuses a failed QC; `export.batch` skips and reports.

## Tasks

- [x] New checks with fix hints
- [x] fix.suggest, fix.apply, qc.compare

## Acceptance Criteria

- [x] `tests/fix.rs` covers suggest ordering, `auto_safe` apply, re-render and re-QC (passing per the progress log)
- [x] Docs pass: `fix suggest`, `fix apply --dry-run`, `qc run` ran on the demo clip (QC passed with one `face_visible_ratio` warning)

## Verification

`cargo test -p rustler-commands --test fix --test qc`, `cargo test -p rustler-media --test qc`. Manual: `render run` then `qc run`, `fix suggest`.

## Status Log

- 2026-11-15 — Built and merged (B3). Workspace tests green on the branch.
