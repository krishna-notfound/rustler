# f14 — Face Tracking and Reframe

**Status:** Built  |  **Depends on:** f04, f08  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

For 9:16 clips cut from 16:9 sources, the speaker should stay centred. The engine finds faces with YuNet at
a low sample rate; Rust smooths the raw box track into a stable crop path; the render compiler pans the crop
with keyframes. An agent can run `crop.track_face` on a clip and clear the crop with `crop.clear`.

## Scope

**In:**
- Engine `faces.track {video_path, start, end, fps: 4.0, model_path}` (OpenCV YuNet on a proxy; model auto-downloads).
- Rust smoothing in `facepath.rs`: median filter, One-Euro filter, dead zone, speed cap. Ported from autoclip (MIT).
- Crop keys driving `sendcmd` x-keyframes in the render compiler (f08).
- Commands: `crop.track_face` (with optional `segment_id`), `crop.clear`.

**Out (explicitly):**
- Active-speaker detection and speaker-aware captions (f29, later).
- Custom multi-region layouts and keyframed region editing (f15, planned).
- Face recognition or identity. Detection only.

## User-facing Behavior

- `crop.track_face` writes crop keys for the clip's segments and reports how many segments were tracked.
- `crop.clear` removes the crop keys; the layout falls back to the clip's layout.
- Progress is reported as `faces.track` stage with percent (samples done over total).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-media/src/facepath.rs`
- `engine/rustler_engine/faces.py`
- `crates/rustler-media/tests/facepath.rs`, `engine/tests/test_faces.py`

**Shared / read-only:**
- `crates/rustler-commands/src/clip_cmds.rs` — hosts `crop.track_face` and `crop.clear` (shared with f03, f16, f17).
- `crates/rustler-media/src/compile.rs` — owns `CropKey` (f08); `facepath` re-exports it.
- `crates/rustler-engine-bridge` — calls `faces.track` (f04).

## Data and Interfaces

- Engine: `faces.track {video_path, start, end, fps, model_path}` returns per-sample boxes.
- Rust: `facepath::crop_keys(...)` returns smoothed `CropKey` values.
- `CropKey` is defined once in `compile.rs` and re-exported by `facepath.rs`.

## Rules and Edge Cases

1. Detection runs on the proxy, not the source, to keep it fast.
2. Dead zone prevents jitter when the face moves a little; speed cap limits pan velocity.
3. The model file is downloaded on first use; the progress stage `models.fetch` is emitted.

## Tasks

- [x] Engine face tracking (`faces.py`)
- [x] Rust smoothing and crop keys (`facepath.rs`)
- [x] `crop.track_face` and `crop.clear` wired (progress log: "crop.track_face/clear" merged)
- [x] Unify `CropKey` with the render compiler type (`facepath.rs` re-exports `compile::CropKey`)
- [ ] Live run of `crop.track_face` on a talking-head clip with a recorded result (not logged)

## Acceptance Criteria

- [x] Smoothing tests (`crates/rustler-media/tests/facepath.rs`, 13 tests)
- [x] Engine tests (`engine/tests/test_faces.py`, 5 tests)
- [x] Clip-level crop validation (`crates/rustler-commands/tests/clip.rs`, `music_bed_is_stored_in_extras_and_crop_commands_validate`)
- [ ] A recorded live tracking result on a real face video: not in the progress log

## Verification

- `cargo test -p rustler-media --test facepath` (PowerShell) and `uv run --project engine pytest -q engine/tests/test_faces.py`.
- Live (manual): `crop.track_face` on a clip, render, then inspect frames; pass = the face stays in frame across the clip.

## Status Log

- 2026-11-15 — Face tracking merged: engine `faces.track` plus Rust crop keys (85 Python and 13 Rust tests at the merge). `crop.track_face` and `crop.clear` are wired. Crop pan verified by frame colour in the render tests. No live face-tracking run is recorded in the progress log.
