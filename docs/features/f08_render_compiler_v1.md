# f08 — Render Compiler v1

**Status:** Verified  |  **Depends on:** f03, f05  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

An agent never writes FFmpeg by hand. It describes a clip (segments, layout, captions, music) and calls
`render.run`; the render compiler turns that clip into one FFmpeg filtergraph and runs it. Output is a
9:16 short-form clip with loudness normalised, using NVENC when present.

## Scope

**In:**
- Compile `clip.json` into one FFmpeg filtergraph (`compile.rs`), run it (`render.rs`, `proc_render.rs`), pick an encoder (`encoders.rs`).
- Layouts: `crop` (center crop), `blur-fit`, `split-vertical`. Crop pan driven by `sendcmd` keyframes (f14).
- Caption burn-in through libass (f16) and music bed mixing (f17).
- Single-pass `loudnorm`.
- `render.run` and `render.preview_frame`.

**Out (explicitly):**
- Custom multi-region layouts and keyframed region editing (f15, planned).
- Transitions, hook titles and effects (f18, planned).
- Two-pass loudnorm (open issue; see below).
- Rendering multiple clips in parallel and GPU throughput tuning (f28, later).

## User-facing Behavior

- The verified render of a Devanagari-named source with captions and music produced 1080x1920 output (progress log).
- `render.preview_frame` renders one frame for a quick visual check before a full render.
- A video without audio gets a silent AAC track so downstream tools see a stream.
- Media failures use exit code 6 (`media`); see f02 for the full exit-code list.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-media/src/compile.rs`, `render.rs`, `proc_render.rs`, `encoders.rs`
- `crates/rustler-commands/src/render_cmds.rs`

**Read-only dependencies:**
- `crates/rustler-media/src/facepath.rs` — crop keys used by the compiler (f14).
- `crates/rustler-captions` — ASS output for burn-in (f16).
- `crates/rustler-core/src/model/clip.rs` and `timemap.rs` — clip model and time mapping (f03).
- `~/.rustler/bin/ffmpeg` installed by f01.

## Data and Interfaces

- Commands: `render.run`, `render.preview_frame`. Progress stage `render.prepare` is emitted during setup of a render.
- Encoders probed by `doctor.run`: `h264_nvenc`, `hevc_nvenc`, `h264_qsv`, `libx264`.
- `CropKey` is unified with `facepath` (facepath re-exports `compile::CropKey`).

## Rules and Edge Cases

1. Paths with spaces, brackets and Devanagari render correctly (`render_handles_awkward_filename`).
2. A video with no audio gets a silent AAC track (`render_video_without_audio_gets_silent_track`).
3. Plan requirement (Review Focus 3): output dimensions are even. Not separately confirmed in the progress log.
4. Single-pass loudnorm lands near -15.9 LUFS on half-silent clips; two-pass is the planned fix (open).

## Tasks

- [x] Filtergraph compiler, encoder selection, render and preview (`crates/rustler-media`)
- [x] Render commands (`render_cmds.rs`)
- [x] Real NVENC render verified

## Acceptance Criteria

- [x] Compiler tests (`crates/rustler-media/tests/compile.rs`, 16 tests)
- [x] Render tests (`crates/rustler-media/tests/render.rs`, 10 tests, including NVENC and awkward file names)
- [x] Render command tests (`crates/rustler-commands/tests/render.rs`, 3 tests)
- [x] Live: Devanagari-named source with captions and music rendered 1080x1920; e2e `render run` used h264_nvenc

## Verification

- `cargo test -p rustler-media --test compile --test render` and `cargo test -p rustler-commands --test render` (PowerShell).
- Pass = tests green; `render.run` output plays and matches the requested layout.

## Status Log

- 2026-11-15 — Render compiler merged (26 tests incl. NVENC). Music ducking measured -3.9 dB and crop pan checked by frame colour. Real Devanagari-named render verified 1080x1920. Open: single-pass loudnorm to two-pass.
