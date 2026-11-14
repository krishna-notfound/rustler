# f16 — Caption Engine (4 presets + SRT)

**Status:** Verified  |  **Depends on:** f08, f14  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

**Scope note:** this file covers only what is built: four data-driven presets rendered to ASS and burned in
by libass, plus SRT export. The rest of the feature index entry (template schema, layered ASS, metrics,
face-aware placement, VTT) is not built. Custom caption styles (asset-based) and per-speaker colours were added later: see f23 and f29.

## Purpose

Short-form clips need readable captions. The agent picks a caption style by name, the engine converts the
clip's transcript units into timed ASS events, and libass burns them into the render. A matching SRT file is
exported for platforms that take sidecar captions.

## Scope

**In:**
- Four presets, data-driven in `presets.rs`: `bold-karaoke`, `clean-white`, `boxed-highlight`, `hormozi-pop`.
- Word-timed (karaoke) highlight in the active caption, chunking of units into caption lines (`chunk.rs`).
- ASS writer (`ass.rs`) and SRT export.
- Commands: `caption.set`, `caption.list_styles`, `caption.export_srt`; `clip.set_captions` is an alias of `caption.set` (`clip_cmds.rs`).
- Font: static Poppins ExtraBold (libass cannot pick weights from a variable font; progress log).

**Out (explicitly):**
- Caption template schema beyond the four presets, user-defined templates.
- Layered ASS beyond the preset output; per-word animation beyond karaoke highlight.
- Font-metric-based placement and face-aware placement (planned in f16's index entry).
- WebVTT export.
- Caption editing in the GUI (f27, planned).

## User-facing Behavior

- `caption.list_styles` lists the four preset names.
- `caption.set` (or `clip.set_captions`) sets the preset on a clip; unknown preset returns a validation error listing valid names (`unknown_preset_validation`).
- Captions are auto-disabled with a warning when the transcript has no words (plan Review Focus 5).
- `hormozi-pop` active-word scale was tuned from 115% to 108/112% after the e2e inspection, because the active word touched its neighbour.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-captions/src/lib.rs`, `ass.rs`, `presets.rs`, `chunk.rs`
- `crates/rustler-captions/tests/ass.rs`

**Shared / read-only:**
- `crates/rustler-commands/src/clip_cmds.rs` — hosts `caption.set`, `clip.set_captions`, `caption.list_styles`, `caption.export_srt` (shared with f03, f14, f17).
- `crates/rustler-media/src/compile.rs` — burns ASS into the render filtergraph (f08).
- `crates/rustler-core/src/model/clip.rs` — `CaptionTrack` (f03).

## Data and Interfaces

- `CaptionTrack` on the clip holds the preset name and enable flag.
- ASS writer input: clip timing and transcript units; output: ASS text consumed by libass.
- SRT output file: `captions.srt` in the export folder (f09).

## Rules and Edge Cases

1. Preset names are data in `presets.rs`; a `PRESETS` const and `list` must agree (`presets_table_matches_const_and_list`).
2. An empty transcript disables captions with a warning; it does not fail the render.
3. Captions render through libass, so the burn-in is verified by frames, not by ASS text alone (`render_preset_frames_with_real_ffmpeg`).

## Tasks

- [x] Four presets and list (`presets.rs`)
- [x] ASS writer, karaoke highlight, chunking (`ass.rs`, `chunk.rs`)
- [x] SRT export and caption commands
- [ ] Template schema, layered ASS, metrics, face-aware placement, VTT (not built)

## Acceptance Criteria

- [x] ASS and preset tests (`crates/rustler-captions/tests/ass.rs`, 15 tests): `presets_table_matches_const_and_list`, `unknown_preset_validation`, `srt_export`, `render_preset_frames_with_real_ffmpeg`
- [x] Caption command tests (`crates/rustler-commands/tests/clip.rs`: `caption_set_alias_styles_and_srt`)
- [x] Live: `hormozi-pop` and karaoke captions inspected in frames of the e2e clip (progress log)

## Verification

- `cargo test -p rustler-captions` (PowerShell).
- Render a clip with `caption.set` and inspect a preview frame with `render.preview_frame`; pass = active word highlighted and neighbours not touching.

## Status Log

- 2026-11-15 — Captions merged: four presets and SRT. Switched to static Poppins ExtraBold for libass. End-to-end captions inspected visually; `hormozi-pop` scale fixed. Template schema, layered ASS, metrics and VTT remain unbuilt.
