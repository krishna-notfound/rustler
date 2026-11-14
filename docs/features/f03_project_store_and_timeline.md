# f03 — Project Store and Timeline

**Status:** Verified  |  **Depends on:** f01  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

A Rustler project is a portable folder that an agent can hand to another agent and resume. The store
keeps `project.json`, source records, clips (timeline of segments, layout, captions, audio) and an
append-only event log. Writes are atomic so an interrupted command never leaves a half-written file.

## Scope

**In:**
- Project directory format: `project.json`, `source.json`, `clip.json`, `events.ndjson`, history revisions `{revision}.clip.json`.
- Atomic JSON writes (temp file + rename) and revision tracking.
- Timeline model: `Clip`, `Segment`, `Layout`, `Region`, `Rect`, `Fit`, `Background`, `CaptionTrack`, `AudioSettings`, `Canvas`.
- `TimeMap` (source time to output time) and layout presets (`layout.rs`).
- Project and clip commands: `project.create`, `project.show`, `project.set_settings`, `project.set_permissions`, `clip.create`, `clip.list`, `clip.show`, `clip.add_segment`, `clip.set_layout`, `layout.list_presets`.

**Out (explicitly):**
- Multi-region custom layouts with keyframes (f15, planned).
- Edit operations and undo/redo (f12).
- Caption and audio/crop commands that live in `clip_cmds.rs` (owned by f14, f16, f17).
- Portable project migration between Rustler versions (not designed).

## User-facing Behavior

- `project.create` makes the folder; `project.show` prints it; `project.set_permissions` sets filesystem roots, network and downloads flags.
- `clip.create` takes `--start-unit/--end-unit` (or `--start/--end` seconds), `--expect-text`, layout and caption preset. Without units it errors `validation` with a hint to transcribe first.
- `clip.list` / `clip.show` return the clip with segments and layout.
- Every mutation appends a line to `events.ndjson`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-core/src/lib.rs`, `ids.rs`, `events.rs`, `store.rs`, `timemap.rs`, `layout.rs`
- `crates/rustler-core/src/model/mod.rs`, `project.rs`, `clip.rs`, `source.rs`
- `crates/rustler-commands/src/project_cmds.rs`
- `crates/rustler-commands/src/clip_cmds.rs` (shared: clip and layout commands here; caption, audio and crop commands are owned by f14, f16, f17)

**Read-only dependencies:**
- `crates/rustler-core/src/error.rs` — error codes used by the store.
- `crates/rustler-registry` — `Ctx` and `typed()` used by the commands.

## Data and Interfaces

- IDs: prefixed ULIDs (`SourceId`, `ClipId`, `RenderId`, `ProjectId`) in `ids.rs`.
- `ProjectStore` API in `store.rs` (paths, atomic JSON I/O, revisions, events).
- `TimeMap` in `timemap.rs` maps source time to output time; the render compiler and captions consume it.
- Layout presets in `layout.rs` (unknown preset lists the valid names).

## Rules and Edge Cases

1. `project.json` and `clip.json` are written atomically; a partial write must not replace the last good file (`atomic_write_survives_partial`).
2. A stale revision maps to exit 9 (`RevisionConflict`); undo tests check that revisions bump (`crates/rustler-commands/tests/edit.rs`).
3. Paths with spaces, brackets and Devanagari are supported (verified in the render path).
4. An unknown layout preset returns the list of valid names (`unknown_preset_lists_presets`).

## Tasks

- [x] Store, IDs, events, TimeMap, layout presets (`crates/rustler-core`)
- [x] Project and clip commands (`project_cmds.rs`, `clip_cmds.rs`)

## Acceptance Criteria

- [x] Store, atomic write and revisions (`crates/rustler-core/tests/store.rs`, 10 tests)
- [x] TimeMap mapping (`crates/rustler-core/tests/timemap.rs`)
- [x] Layout presets (`crates/rustler-core/tests/layout.rs`)
- [x] Frozen contract shapes (`crates/rustler-core/tests/contracts.rs`)
- [x] Clip and layout commands (`crates/rustler-commands/tests/clip.rs`: `layout_presets_set_layout_and_add_segment`, `create_without_units_hints_transcribe`)

## Verification

- `cargo test -p rustler-core` and `cargo test -p rustler-commands --test clip` (PowerShell).
- Pass = all tests green; the e2e run created, edited and rendered a clip with the real CLI.

## Status Log

- 2026-11-15 — Core store, timemap, units and layout merged (40 tests across `crates/rustler-core/tests`); clip and layout commands used in the end-to-end clip run.
