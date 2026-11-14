# f19 — Cut refinement (weak openers, sentence ends, silence troughs)

**Status:** Verified  |  **Depends on:** f06, f12  |  **Owner:** A3 agent

## Purpose

A clip cut on unit ids still often starts on "so, um, toh" or ends mid-breath. `edit.refine` repairs the cut points deterministically so the first word is
the hook and the last word closes a sentence, with boundaries in silence rather than mid-word.

## Scope

**In:** `edit.refine <clip> {max_shift?, segment_id?}` (supports `--dry-run`): skips weak openers (English and Hinglish list: so, and, um, toh, matlab, haan, ...),
ends on a sentence-final word, moves both ends into silence troughs within `max_shift` (default 1.5 s). Without `--segment-id` only the clip's outer ends move
(inner cut points are deliberate). Related tools in the same area, already built: `edit.trim --snap word`, `edit.remove_pauses`, `edit.remove_words`
(`edit_cmds.rs`).
**Out (explicitly):** choosing which moment to clip (agent), fillers inside sentences beyond the listed words, audio-energy VAD beyond word gaps.

## User-facing Behavior

`rustler edit refine clp_X [--dry-run]` returns what moved (before/after source times, reason per end). Each real run is a new revision (`history undo`).
Knowledge: `cut-points`, `editing-toolkit`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/cut_cmds.rs`
- Tests: `crates/rustler-commands/tests/cut.rs` (7 tests), `crates/rustler-commands/tests/edit.rs`

**Read-only dependencies (consume, never edit):**
- `crates/rustler-core` units and timemap, transcript words from `analysis/<src>/transcript.json`, `edit_cmds.rs` helpers.

## Rules and Edge Cases

1. Word boundaries and silences come from the saved word-level transcript and audio analysis.
2. `max_shift` (default 1.5 s) bounds how far a boundary may move.
3. `--expect-revision N` guards concurrent edits (exit 9).
4. Overlays/effects are time-anchored and are not moved: re-check them after refining.

## Tasks

- [x] Weak-opener rewind, sentence end, silence trough
- [x] Dry run and revision handling

## Acceptance Criteria

- [x] Test clips: opener skipped, end on sentence-final word, boundary lands in silence (`tests/cut.rs`)
- [x] `edit refine --dry-run` writes nothing

## Verification

`cargo test -p rustler-commands --test cut --test edit`. Manual: `rustler edit refine <clip> --dry-run` (verified in the docs pass on the demo clip).

## Status Log

- 2026-11-15 — Built and merged (A3) with highlights; real run on the demo ranked and refined cleanly.
