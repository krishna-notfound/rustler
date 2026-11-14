# f22 — Brief, compliance check, variants, batch render and export

**Status:** Built (tests pass)  |  **Depends on:** f09, f16, f18, f21  |  **Owner:** C1 agent

## Purpose

Client and campaign jobs have rules (length, credit, banned words, hashtags). Rustler stores the brief once, checks every clip against it, drafts the post
text, makes comparable variants of a strong clip, and renders and exports many clips with QC and brief gates.

## Scope

**In:** `brief.set` (merge; lists replace; `replace: true` overwrites), `brief.show`, `brief.clear` (`<project>/brief.json`); `brief.check <clip>` (rules: duration with
transition overlap subtracted, aspect, caption_style, layout, required_text, banned_words (whole word, case/accent/punctuation-insensitive, over transcript + raw
transcript + caption words + overlays), credit/watermark overlays, QC; saved to `clips/<id>/brief_check.json`); `brief.description <clip>` (hook, credit, mentions,
hashtags; deterministic draft); `variant.create <clip> {caption_styles?, layouts?, hooks?}` (one clone per combination, `extras.variant_of`, layouts preflighted on
the base), `variant.list`, `variant.compare --clip-ids` (draft preview frames at the same `out_t` in one labelled grid); `render.batch` (bounded concurrency: default 1,
capped at 2 on NVENC, else 4; a failing clip never stops the rest; `only_missing`); `export.batch` (runs QC and the brief check, exports passing clips, lists skipped with
reasons); the export report embeds the brief status. `project.status` shows each clip's brief check.
**Out (explicitly):** uploading or posting, LLM titling, scheduling.

## User-facing Behavior

`rustler brief set --platform shorts --min-duration 20 --max-duration 58 --credit-text "Clip from @x" ...`, `rustler brief check clp_X`, `rustler variant create clp_X
--caption-styles '["hormozi-pop","clean-white"]'`, `rustler variant compare --clip-ids '[...]'` (open the image), `rustler render batch --all true`,
`rustler export batch --all true`. Knowledge: `briefs-and-batches`, prompt `batch_from_brief`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/brief_cmds.rs`
- `crates/rustler-commands/src/variant_cmds.rs` (variants, `render.batch`, `export.batch`)
- Tests: `crates/rustler-commands/tests/brief.rs` (4 tests), `crates/rustler-commands/tests/variant.rs` (2 tests: 2x2 variants with extras copied, compare grid, batch with a failing clip, export skipping a brief-failing clip)

**Read-only dependencies (consume, never edit):**
- `qc_cmds.rs` (`qc.run`, `export.clip`), `layout_cmds.rs` (`layout.apply_preset`), `overlay_cmds.rs` (`overlay.add_text`), custom caption styles (f23).

## Rules and Edge Cases

1. Hashtags and mentions are post text (`info` items), not video; credit and watermark are on-screen overlays.
2. `export.batch` skips clips with no render, a stale render (edited after rendering; `--allow-stale`), failing QC or a failing brief. Do not disable the gates to hit a count.
3. `variant.create` with a bad layout creates nothing (preflight on the base with a dry run).
4. `render.batch --only-missing` compares the render time with the clip and extras files and the QC report time.

## Tasks

- [x] Brief storage, check rules, description draft
- [x] Variants and compare grid
- [x] render.batch and export.batch with gates
- [ ] Not verified: Hindi/Devanagari banned words beyond what the saved transcripts contain

## Acceptance Criteria

- [x] `tests/brief.rs` and `tests/variant.rs` pass against real ffmpeg + NVENC (progress log)
- [x] Docs pass (2026-11-15): `brief set`, `brief check`, `brief description`, `variant create --dry-run`, `render batch --dry-run`, `export batch --dry-run`, `project status` ran on the demo project

## Verification

`cargo test -p rustler-commands --test brief --test variant`. Manual: `rustler brief check <clip>` after a render and QC.

## Status Log

- 2026-11-15 — Built in a worktree branch and merged (C1); comparison grid checked visually.
