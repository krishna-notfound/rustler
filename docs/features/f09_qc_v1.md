# f09 — Quality Control v1 (QC and export)

**Status:** Verified  |  **Depends on:** f08  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

Exit code 0 does not mean a good clip. After every render an agent runs `qc.run`, which checks the output
against objective measures and returns a report with a fix hint for each failed check. `export.clip` then
copies the finished clip, its report and its captions into a delivery folder. Nothing is published.

## Scope

**In:**
- `qc.run`: 12 checks, each with a `fix_hint` naming the command to run next.
- Check families named in the feature index (`docs/features/README.md`, f09 row): integrity, loudness, black/freeze/silence, duration.
- Exit code 7 when a check fails (unless `allow_fail` is passed).
- `export.clip`: copies the MP4, `report.json` and SRT (when captions are on) to the export folder.

**Out (explicitly):**
- Caption readability, face visibility and hook checks (f21, planned).
- Vision-based review of frames by a model (no AI API integrations).
- Publishing or uploading exports (never auto-publish).

## User-facing Behavior

- `qc.run` returns the checks with pass/fail and a fix hint per check.
- `export.clip` copies the rendered MP4 and report; a custom name is supported; with captions off no SRT is written.
- A failed check returns exit 7 (`allow_fail` changes this; see `rustler commands describe qc.run`).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-media/src/qc.rs`
- `crates/rustler-commands/src/qc_cmds.rs`
- `crates/rustler-core/src/model/qc.rs` (`QcReport`, `QcCheck`, `QcStatus`)

**Read-only dependencies:**
- `crates/rustler-media/src/render.rs` — the rendered file that QC inspects (f08).
- `crates/rustler-commands/src/render_cmds.rs` — the render record that export copies (f08).

## Data and Interfaces

- Commands: `qc.run`, `export.clip`.
- Files: `report.json` (QC report), the rendered MP4, and `captions.srt` when captions are on.
- Types: `QcReport`, `QcCheck`, `QcStatus` (`model/qc.rs`).

## Rules and Edge Cases

1. Each failed check must include a `fix_hint` naming a real command.
2. A failed check exits 7; a passing run exits 0.
3. Custom export names are supported (`export_with_custom_name_and_captions_off_has_no_srt`).
4. Captions off means no SRT in the export.

## Tasks

- [x] Twelve QC checks with fix hints (`qc.rs`)
- [x] `qc.run` and `export.clip` commands (`qc_cmds.rs`)

## Acceptance Criteria

- [x] QC checks (`crates/rustler-media/tests/qc.rs`, 7 tests)
- [x] QC and export commands (`crates/rustler-commands/tests/qc.rs`, 9 tests, including `export_copies_render_report_and_srt`)
- [x] Live: `qc run` on the e2e clip gave 12/12 pass with -14.3 LUFS in the browser QC view (progress log)

## Verification

- `cargo test -p rustler-media --test qc` and `cargo test -p rustler-commands --test qc` (PowerShell).
- `rustler qc run` on a rendered clip; pass = 12/12 checks pass and exit 0.

## Status Log

- 2026-11-15 — QC v1 and export merged (19 tests). End-to-end `qc run` passed 12/12; export clip produced MP4, report and SRT. QC table verified in the real browser GUI.
