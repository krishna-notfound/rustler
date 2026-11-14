# f15 — Layout region system and streamer webcam split

**Status:** Verified  |  **Depends on:** f08, f12, f14  |  **Owner:** A2 agent

## Purpose

Layouts beyond three fixed presets: a layout is a list of **regions** (source rect drawn into a canvas rect, with
`fit` and `z`). Agents (and the GUI) can detect a streamer webcam, build a webcam-top / gameplay-bottom split,
add picture-in-picture, nudge rectangles, and save a layout as a reusable preset.

## Scope

**In:** `layout.show`, `layout.add_region`, `layout.edit_region` (src/dst/fit/z or `--nudge` deltas), `layout.remove_region`, `layout.reorder`,
`layout.from_frame_boxes` (roles top/bottom/main/inset), `layout.save_preset` / `layout.apply_preset` / `layout.delete_preset` / `layout.presets`
(custom presets in `<project>/layouts/<name>.json`), `layout.list_presets` (built-ins), `layout.detect_webcam` (YuNet face sampling, small consistent
face = webcam, returns webcam + gameplay rects, split, confidence and ready commands). Render compiler draws regions in `z` order;
quality warnings "will crop heavily" and "will look soft".
**Out (explicitly):** crop keyframe animation inside a region (crop paths from `crop track-face/track-speaker` only), per-region effects,
layout assets in the asset library beyond the three built-ins.

## User-facing Behavior

`rustler layout detect-webcam <src> --clip-id <clip>` prints the rects and the `clip set-layout --preset split-vertical --params ...` line; `layout show <clip>`
prints regions with normalised and pixel rects and warnings; all edits are revisions (`history undo`). Rects must lie in 0..1 and be larger than 0.02;
a bad input changes nothing. Knowledge: `layouts-and-framing`, `streamer-clips`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/layout_cmds.rs`
- `crates/rustler-core/src/layout.rs` (region model, presets, pixel conversion, validation)
- Region drawing in `crates/rustler-media/src/compile.rs` (layout section)
- Tests: `crates/rustler-commands/tests/layout.rs`, `crates/rustler-core/tests/layout.rs`

**Read-only dependencies (consume, never edit):**
- `crates/rustler-engine-bridge` and `engine/rustler_engine/faces.py` (face detection), `crates/rustler-core` clip model.

## Data and Interfaces

Region: `{id, src: Rect, dst: Rect, fit: cover|contain, z}`; `Rect` normalised `{x,y,w,h}`. `layout.detect_webcam` result: `face`, `webcam`, `gameplay`,
`split`, `confidence` (`face_hits/samples`), `commands[]`, `from_frame_boxes_input`. Full schemas: `rustler commands describe layout.<action>`.

## Rules and Edge Cases

1. The suggested webcam rect can include a sliver of gameplay or overlay border: always `render preview-frame` and nudge.
2. Hand-edited layouts have `preset: null`; `crop track-face` / `crop track-speaker` need the `center-crop` preset.
3. `--segment-id` limits an edit to one segment, otherwise all segments (regions matched by id).
4. Custom preset names cannot shadow built-ins; a saved preset applies to any clip of the project.

## Tasks

- [x] Region model, commands, compiler support, warnings
- [x] `detect_webcam` with real face sampling
- [x] GUI region editor (f25) consumes `layout.edit_region`

## Acceptance Criteria

- [x] detect-webcam found a corner webcam in 4/4 samples and produced a working split (progress log)
- [x] Preview frames checked visually for the split and region edits
- [x] `layout` tests (commands 9, core 9) pass

## Verification

`cargo test -p rustler-core --test layout`, `cargo test -p rustler-commands --test layout`. Manual: `rustler layout detect-webcam <src>` on a stream capture
(the bundled JFK demo has no webcam and correctly reports "no small, consistent face").

## Status Log

- 2026-11-15 — Built (A2) and merged; guide `layouts-and-framing` updated.
- 2026-11-15 — Docs pass: `layout detect-webcam` against the demo returned the expected "no small, consistent face" validation error.
