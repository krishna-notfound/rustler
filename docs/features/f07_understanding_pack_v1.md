# f07 — Understanding Pack v1 (see, scenes, audio events)

**Status:** Verified  |  **Depends on:** f04, f05, f06  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

The agent must be able to see and hear a source before choosing a moment. `see.*` commands return frames,
labelled tile grids (contact sheets) and short previews. Scene and audio analysis add shot boundaries and
loud or silent regions, so the agent can pick cuts by evidence rather than guessing from text alone.

## Scope

**In:**
- `see.frame`, `see.tiles` (labelled grid; columns auto-fit to the count; `--around` used in the e2e run), `see.preview` (behaviour per `rustler commands describe see.preview`).
- `analyze.scenes` (PySceneDetect wrapper, engine `scenes.detect`).
- `analyze.audio` (silence and loudness peaks, audio events).

**Out (explicitly):**
- Pixel-level OCR and on-screen text reading (RapidOCR was in the spec; not built).
- Audio classifier models (ONNX audio events from the spec are not built; only the signal-based analysis is).
- Filmstrip view (named in the feature index; not built).

## User-facing Behavior

- `see.tiles --around <t>` returns a labelled grid of frames around a time; the grid was inspected visually in the e2e run (layout and labels correct).
- Scene detection returns `{start, end}` ranges.
- A video with no audio track returns an empty audio-event list (`audio_events_video_without_audio_is_empty`).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-media/src/frames.rs`, `tiles.rs`, `audio_analysis.rs`
- `crates/rustler-commands/src/see_cmds.rs`
- `crates/rustler-core/src/model/analysis.rs`
- `engine/rustler_engine/scenes.py`

**Read-only dependencies:**
- `crates/rustler-commands/src/analyze_cmds.rs` — hosts `analyze.scenes` and `analyze.audio` (shared with f06).
- `crates/rustler-engine-bridge` — calls `scenes.detect` (f04).

## Data and Interfaces

- Commands: `see.frame`, `see.tiles`, `see.preview`, `analyze.scenes`, `analyze.audio`.
- Engine: `scenes.detect {video_path, threshold: 27.0}` returns `{scenes: [{start, end}]}`.
- Files: `scenes.json`, `audio_events.json`.
- Types: `Scenes`, `AudioEvents`, `LoudnessPoint`, `Silence` (`model/analysis.rs`).

## Rules and Edge Cases

1. Tile grids auto-fit columns to the number of tiles (fix made after e2e inspection).
2. Audio analysis on a video without audio returns empty results, not an error.
3. Frame extraction works on rotated and odd-size sources (dims from f05 probe).

## Tasks

- [x] Frame, tile and preview extraction (`frames.rs`, `tiles.rs`)
- [x] Scene detection through the engine (`scenes.py`)
- [x] Audio analysis (`audio_analysis.rs`)

## Acceptance Criteria

- [x] See tests (`crates/rustler-media/tests/see.rs`, 4 tests; `crates/rustler-commands/tests/see.rs`, 3 tests)
- [x] Audio tests (`crates/rustler-media/tests/audio.rs`: `audio_events_find_silences`, `audio_events_peaks_on_loud_burst`, `audio_events_video_without_audio_is_empty`)
- [x] Scene tests (`engine/tests/test_scenes.py`, 6 tests)
- [x] Live: `see tiles --around` and `see frame` inspected visually in the e2e run

## Verification

- `cargo test -p rustler-media --test see --test audio` and `cargo test -p rustler-commands --test see` (PowerShell).
- `uv run --project engine pytest -q engine/tests/test_scenes.py`.
- Pass = tests green and a tile grid of a real clip shows the expected frames.

## Status Log

- 2026-11-15 — `see.*` verified in the end-to-end run (tile grid and contact sheet inspected visually). `analyze.scenes` and `analyze.audio` covered by merged tests; no separate live run is recorded in the progress log.
