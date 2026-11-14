# f12 — Edit Operations and History

**Status:** Built  |  **Depends on:** f03  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

An agent revises a clip with small, typed operations: trim a segment, split at a time, delete or move a
segment, remove pauses or words, set a range. Each operation is recorded in history, so an agent can list
revisions, undo the last change or restore an earlier one. Operations work on transcript-aware boundaries
where a transcript exists.

## Scope

**In:**
- Commands: `edit.trim`, `edit.split`, `edit.delete_segment`, `edit.move_segment`, `edit.set_range`, `edit.remove_pauses`, `edit.remove_words`.
- History: `history.list`, `history.undo`, `history.restore`; snapshots as `history/<rev>.clip.json`.
- Word-boundary snapping for trim (only the changed boundary moves).
- `edit.remove_pauses` splits segments at word gaps and reports the removed seconds.

**Out (explicitly):**
- The "full typed op set" in the feature index (for example `layout.set`, `cut.*` beyond the list above).
- Word-anchored projection after edits (spec §5.4) beyond the trim and pause snapping above; not separately built or tested.
- Timeline editing in the GUI (f25, planned).
- Redo (only undo and restore are listed in the commands).

## User-facing Behavior

- `edit.trim` takes `clip_id`, `segment_id`, and `start_delta` / `end_delta`; the boundary snaps to words.
- `edit.split` takes `clip_id` and `out_t` and cuts the segment at that output time.
- `edit.remove_pauses` without a transcript returns a validation error.
- `history.undo` restores the previous segments and bumps the revision.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/edit_cmds.rs`

**Read-only dependencies:**
- `crates/rustler-core/src/store.rs` — revisions and snapshot files (f03).
- `crates/rustler-core/src/timemap.rs`, `units.rs` — boundary and word snapping (f03, f06).
- `crates/rustler-core/src/model/clip.rs` — `Segment`, `Clip` (f03).

## Data and Interfaces

- Commands as listed above; each takes `clip_id` and returns the updated clip and revision.
- Snapshot files: `history/<revision>.clip.json`.
- Errors: `validation` for bad ranges (`trim_validates_range`).

## Rules and Edge Cases

1. Invalid trim ranges are rejected (`trim_validates_range`).
2. Only the changed boundary snaps to a word (`trim_snap_only_moves_changed_boundary`).
3. Pause removal without a transcript is a validation error (`remove_pauses_without_transcript_is_validation`).
4. Undo bumps the revision (`undo_restores_previous_segments_and_bumps_revision`).

## Tasks

- [x] Trim, split, delete, move, set-range, remove-pauses, remove-words (`edit_cmds.rs`)
- [x] History list, undo and restore
- [ ] Redo and the full typed op set from the feature index (not built)

## Acceptance Criteria

- [x] Edit and history tests (`crates/rustler-commands/tests/edit.rs`, 20 tests), including `trim_deltas`, `split_at_out_t`, `move_segment_reorders`, `remove_pauses_splits_and_reports_removed_seconds`, `undo_restores_previous_segments_and_bumps_revision`
- [ ] Test results for this feature are not recorded in the progress log; treat as unverified until a run is logged

## Verification

- `cargo test -p rustler-commands --test edit` (PowerShell).
- Pass = all 20 tests green; `history.undo` after `edit.split` restores the previous segment list.

## Status Log

- 2026-11-15 — Edit operations and history commands are in the repo with a 20-test suite. The progress log does not record a merge entry or a test run for this feature; status stays Built, not Verified, until one is logged.
