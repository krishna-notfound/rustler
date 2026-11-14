# f18 — Overlays, hook titles, transitions, camera effects, progress bar

**Status:** Built (tests pass, measured renders)  |  **Depends on:** f08, f12  |  **Owner:** A1 + B1 agents

## Purpose

The finishing layer of a short: a hook title or lower third burned into the render, frame-exact transitions
between segments, short camera effects on beats, and a progress bar. All happen inside the single ffmpeg encode.

## Scope

**In:** `overlay.add_text` (styles `hook-top`, `lower-third`, `plain`; <= 120 chars, 2 lines), `overlay.edit`, `overlay.list`, `overlay.remove`,
`overlay.progress_bar`; `transition.set` (`cut|fade|dissolve|wipeleft|wiperight|slideup|zoomin`, default 0.3 s, whole frames, max 2 s), `transition.list`,
`transition.clear`; `effect.add` (`punch-in|zoom-slow|shake|flash`), `effect.list`, `effect.remove`. Stored in `clips/<id>/extras.json`. Captions and overlay
times are remapped around transitions; QC and `brief.check` use the shortened duration.
**Out (explicitly):** stickers/images as layers, per-word animated text, keyframed overlays, custom xfade expressions, transitions inside a single segment.

## User-facing Behavior

Overlays render above captions; effects apply to the composed frame under captions and overlays; a transition shortens the clip by its overlap
(`transition list` shows edit vs rendered duration). Time arguments are output (edit-timeline) seconds. Knowledge: `editing-toolkit`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/overlay_cmds.rs`, `crates/rustler-commands/src/fx_cmds.rs`, `crates/rustler-commands/src/extras.rs`
- Post-processing graph in `crates/rustler-media/src/compile.rs` (overlays, effects, xfade/acrossfade, `effective_duration`)
- Tests: `crates/rustler-commands/tests/overlay.rs`, `crates/rustler-commands/tests/fx.rs`, `crates/rustler-media/tests/post_compile.rs`, `crates/rustler-media/tests/post_render.rs`

**Read-only dependencies (consume, never edit):**
- `crates/rustler-captions` (ASS), `crates/rustler-core` timemap/clip model.

## Rules and Edge Cases

1. Overlays, effects and `--at` sound effects are time-anchored: they do not follow later cuts (`edit trim` etc.). Re-check `overlay list` / `effect list`.
2. `transition.set` needs the `seg_` id of the segment before the cut; `cut` means a hard cut.
3. Effect amounts: zoom 1.01-2; shake 0.002-0.08; flash has none.
4. Extras writers must not clobber each other's keys (shared helper in `extras.rs`; verified: overlay survives `audio add-music`).

## Tasks

- [x] Overlays and hook titles in the same encode
- [x] Transitions (xfade + acrossfade, frame-exact) and effects, progress bar
- [x] QC / brief duration uses effective duration

## Acceptance Criteria

- [x] Overlay text visible in the first frame; transition shortens the clip by exactly the overlap
- [x] `overlay`, `fx` and `post_compile` tests pass

## Verification

`cargo test -p rustler-commands --test overlay --test fx`, `cargo test -p rustler-media --test post_compile`. Manual (docs pass, 2026-11-15, demo clip):
`overlay add-text`, `overlay progress-bar`, `effect add punch-in`, `transition set dissolve --duration 0.4` all ok; `audio list` showed
`rendered_duration` 0.4 s shorter than the edit duration.

## Status Log

- 2026-11-15 — Overlays merged (A1); transitions, effects, progress bar merged (B1); duration check fixed for transitions (`f276000`).
