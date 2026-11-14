# f20 — Highlight helpers (signals and candidates, no LLM)

**Status:** Verified  |  **Depends on:** f07, f19  |  **Owner:** A3 agent

## Purpose

Give the agent deterministic **evidence** about where a long source gets interesting, plus ranked, sentence-aligned candidate windows with a ready
`clip create` line. The agent still decides: Rustler never calls an LLM (f13 dropped), so this is signals and arithmetic only.

## Scope

**In:** `highlights.signals <src> {window?, limit?, offset?}` (per window: loudness peak above median, speech rate vs median, questions/exclamations,
emphasis words, pause before, scene cuts, YouTube heatmap when `source_info.json` exists); `highlights.candidates <src> {count?, min_len?, max_len?, platform?}`
(sentence-aligned, scored with a breakdown: hook, energy, language, completeness, heatmap, pace, length_fit, scenes; non-overlapping with a diversity penalty;
weak openers skipped; `create_cmd` and `read_cmd` included); `transcript.read_window <src> <start> <end>`. URL imports save `source_info.json`
so the heatmap feeds the signals (also `source info --source-id` reads it offline).
**Out (explicitly):** LLM scoring or rerank (dropped), virality prediction, audience data.

## User-facing Behavior

`rustler highlights candidates <src> --count 10 --platform tiktok` returns `rank`, `score`, `reasons`, `breakdown`, unit ids, `opening_text`, `closing_text`,
`create_cmd`. Platform only shifts the `length_fit` score (`shorts` 20-45 s, `reels` 15-45 s, `tiktok` 25-60 s). Missing analyses give nulls plus a warning.
Knowledge: `finding-hooks` ("evidence, not the decision"), `start-here`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/highlights_cmds.rs`
- Tests: `crates/rustler-commands/tests/highlights.rs` (7 tests), `crates/rustler-commands/tests/transcript.rs`

**Read-only dependencies (consume, never edit):**
- `analysis/<src>/` audio events, scenes, transcript/units; `crates/rustler-core` units; `cut_cmds.rs` weak-opener list.

## Rules and Edge Cases

1. Scores are relative to the source's own medians; a short or flat source yields modest scores. Say so instead of padding.
2. Candidates are non-overlapping (diversity penalty applied).
3. Always read a candidate (`transcript read-window`) and look (`see tiles`) before cutting.

## Tasks

- [x] signals, candidates, read-window
- [x] URL import saves `source_info.json` (heatmap feeds signals, live-verified)

## Acceptance Criteria

- [x] Demo run: 'ask not what your country can do for you' ranked #1 with reasons and a working `create_cmd`
- [x] Docs pass (2026-11-15): `highlights signals`, `highlights candidates --platform tiktok` ran against the demo; the `create_cmd` created a valid clip

## Verification

`cargo test -p rustler-commands --test highlights --test transcript`. Manual: `rustler highlights candidates <src>`.

## Status Log

- 2026-11-15 — Built and merged (A3); heatmap wiring added with C3.
