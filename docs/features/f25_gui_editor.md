# f25 — GUI editor: timeline, program monitor, region editor, inspector panels

**Status:** Built (browser-verified; Tauri shell not built)  |  **Depends on:** f24, f12, f15, f16, f18, f21, f22, f23  |  **Owner:** B4 agent

**Scope note:** the feature index splits the editor into f25 (timeline), f26 (preview + region editor) and f27 (caption editor, asset browser, render queue, QC view).
They were built together as one Premiere-style editor inside the existing local web GUI (`rustler gui`, f24), not as a Tauri app, so this one file covers f25, f26 and f27.

## Purpose

A human can watch and correct what the agent did, or edit by hand, in the same project at the same time: drag regions on the source, trim on a timeline, tweak
captions and overlays, run QC and fixes, queue renders. Every control calls a registry command (`--actor human`); there is no editing logic in the GUI.

## Scope

**In:**
- **Source panel / region editor (f26):** draggable and resizable region boxes on a source frame (`layout.edit_region`), `layout.detect_webcam` and a draw-two-boxes streamer split (`layout.from_frame_boxes`).
- **Program monitor:** 9:16 live preview and the exact libass frame (`render.preview_frame`), served through `/api/frame`.
- **Multi-track timeline (f25):** tracks V / CAP / TXT / A1; trim with word snap (`edit.trim --snap word`), split (`edit.split`), delete, reorder, overlay retime, zoom.
- **Inspector tabs (f27):** Clip, Layout, Captions, Overlays, Audio, QC + fixes (`qc.run`, `fix.suggest`, `fix.apply`), Render queue (background jobs with progress and cancel), Brief, Variants, History; a music asset picker (`asset.list`) in the Audio tab; generic forms for commands without a dedicated panel.
- Keyboard shortcuts; live sync with agent edits (events stream + clip revision polling).
**Out (explicitly):** Tauri desktop shell, multi-clip project timeline, waveform editing, keyframe curves, GUI redo beyond `history.restore`.

## User-facing Behavior

`rustler gui` opens `127.0.0.1:<port>/?token=...` (token required, host checked). Edits made by the agent appear without reload (`/api/events`, `/api/clip_state`);
the GUI sends `expect_revision` so a human edit never silently overwrites a newer agent edit (exit 9 equivalent is shown as a conflict). Long commands run as jobs
(`/api/jobs`, progress, cancel).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-gui/src/lib.rs` (routes: `/api/commands`, `/api/run`, `/api/jobs`, `/api/jobs/cancel`, `/api/frame`, `/api/clip_state`, `/api/font/{name}`, `/api/file`, `/api/events`)
- `crates/rustler-gui/src/ui/index.html`, `crates/rustler-gui/src/ui/style.css`
- `crates/rustler-gui/src/ui/js/`: `boot.js`, `core.js`, `commands.js`, `forms.js`, `project.js`, `sources.js`, `clips.js`, `editor.js`, `monitor.js`, `regions.js`, `timeline.js`, `inspector.js`, `renders.js`, `activity.js` (14 modules, no build step)
- Tests: `crates/rustler-gui/tests/gui.rs` (token, module list, jobs with progress and cancel, frame route, allow-listed fonts, clip state, commands/run, files and events)

**Read-only dependencies (consume, never edit):**
- `crates/rustler-registry` (commands), every command crate listed above (the GUI only calls them).

## Rules and Edge Cases

1. No GUI-only behaviour: anything the GUI does is a registry command, so the CLI and MCP can do it too.
2. Fonts are served only from the installed allow-list; files only from inside the project.
3. Binds to loopback with a per-run token; all state-changing requests need it.
4. Concurrent edits are guarded by revisions; the GUI re-reads after a conflict.

## Tasks

- [x] Region editor, monitor, timeline, inspector tabs, render queue, keyboard shortcuts, live sync
- [ ] Tauri shell and an installable desktop app (deferred)
- [ ] Dedicated panels for speaker colours (f29), custom caption styles and a full asset browser (f23); today use the CLI or the generic command forms

## Acceptance Criteria

- [x] Verified in a real browser (Playwright) at 1440 and 1280 px against real projects (progress log)
- [x] `cargo test -p rustler-gui` passes (11 tests)

## Verification

`cargo test -p rustler-gui`; manual: `rustler gui`, open the printed URL, edit a region and see the preview update; make an edit from the CLI and watch the timeline refresh.

## Status Log

- 2026-11-15 — Full editor merged (B4); music asset picker uses f23. Not in the GUI: speaker colours, custom caption style creation, a full asset browser.
