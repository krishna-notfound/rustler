# f32 — Style references: learn a clip style from a folder of example videos

**Status:** Built  |  **Depends on:** f06 (frames/tiles), f07b (engine faces), f21 (QC audio)  |  **Owner:** style-references agent (v2 Task 1.6)

## Purpose

The user wants the agent to learn a clip style from example videos: the agent LOOKS at frames like a vision model,
and Rustler measures what eyes cannot count. Templates come later; this feature produces the **profile** the agent turns
into a style spec (and later `template.create ... based_on`).

## Scope

**In:**
- `reference.import {name, folder|files, recursive?, replace?, max_seconds?}` — stores a set under
  `<project>/references/<name>/` (`reference.json`, `proxies/vNN.mp4` <= 540 px short side at 15 fps, `audio/vNN.wav` 16 kHz mono).
  Video files only (`mp4 mov mkv webm m4v avi flv wmv mpg mpeg ts`); unreadable files go to `skipped`. Originals are never touched.
- `reference.list`, `reference.show {name, video?}` (profile + images).
- `reference.analyze {name, max_videos?, no_faces?}` → per video and aggregate profile saved as `profile.json`; images
  (`image`: contact sheets first, then first-3-second filmstrips) for the agent to look at.
- Measures (engine `reference.measure`, `engine/rustler_engine/reference.py`; method text in each block and in
  `engine/PROTOCOL.md`): cuts per minute + shot-length stats (luma-normalised frame differences, adaptive threshold, histogram cue
  for flat footage), caption band position/size + words per line + fill colours (edge-density bands, k-means on text pixels),
  hook text in the first 3 s, zoom/punch rate (ORB similarity-transform scale, left and right strips must agree), per-shot face
  count and layout class (YuNet, skipped with a note when unavailable) plus frame structure (split screen / inset bars), music
  heuristic. Rust side: integrated LUFS/LRA/true peak (ffmpeg ebur128 on the original), speech rate from a `.srt/.vtt` sidecar,
  sheets, aggregate (medians, pooled shot lengths, merged colours, layout shares, plain-language `summary`).
- Knowledge: guide `learn-style-from-videos`, prompt `learn_style_from_folder`.

**Out:** templates (`template.create`, Task 1.5), OCR text reading (the `ocr` extra is not used here), transcription of references
(speech rate needs a subtitle sidecar), GUI.

## Files

`crates/rustler-commands/src/reference_cmds.rs` (+ lib.rs registration), `crates/rustler-commands/tests/reference.rs`,
`engine/rustler_engine/reference.py` (+ `rpc.py` registration), `engine/tests/test_reference.py`, `engine/PROTOCOL.md`,
`knowledge/guides/learn-style-from-videos.md`, `knowledge/prompts/learn_style_from_folder.md`.

## Known limits (stated in the output `notes` and the guide)

Measured on a low-res proxy with heuristics. Zoom needs texture on both sides of the frame (dark podcast footage: ~6 percent
of frame pairs measurable, so zoom is under-counted); a busy animated chart can resemble caption text (the caption pick prefers the
edge-dense band that changes within shots and not just with picture motion); a rapid head turn can add a spurious cut;
music is a pause-depth/pulse heuristic; speech rate needs a subtitle sidecar.

## Status Log

- 2026-11-16: built; engine tests (10) and command tests (2 integration + 5 unit) pass; real run on the 5 trial exports recorded in `docs/progress-log.md`.
