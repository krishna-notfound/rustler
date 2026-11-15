# f29 — Speakers: diarization, speaker caption colours, active-speaker crop

**Status:** Built (tests pass; not run live in the docs pass)  |  **Depends on:** f06, f14, f16  |  **Owner:** C4 agent

## Purpose

For interviews, podcasts and collabs: know who speaks when, colour captions per speaker, and make the 9:16 crop follow the person who is talking.

## Scope

**In:** `analyze.speakers <src> {num_speakers?, min_speakers?, max_speakers?}` (local diarization: sherpa-onnx 2.0.1 + pyannote segmentation-3.0 ONNX + WeSpeaker ResNet34;
a numpy MFCC clustering fallback is labelled `model: "heuristic"`); writes `analysis/<src>/speakers.json` and sets `speaker` on `transcript.json` words (`transcript.raw.json` untouched);
`speaker.rename <src> <speaker> <name>`; `caption.speaker_colors <clip> --enabled {colors?}` (stored in `extras.speaker_colors`, honoured by `render.run` through
`rustler_captions::write_ass_colored`); `crop.track_speaker <clip> {segment_id?}` (diarized turns + mouth movement per face via engine `speakers.active`, then the same
smoothing as `crop.track_face`; per-turn `reason`: `mouth-motion`, `speaker-map`, `dominant`, `only-face`).
**Out (explicitly):** identifying speakers by name or voice print, separating overlapping speech, per-speaker audio processing.

## User-facing Behavior

`rustler analyze speakers src_X --num-speakers 2`, `rustler caption speaker-colors clp_X --enabled true --colors '{"S1":"#7CFF6B","S2":"#5EE0FF"}'`,
`rustler crop track-speaker clp_X`. A clip from an undiarized source renders with the plain colour and a render warning. Knowledge: `captions-and-style`
(speaker colours), `layouts-and-framing` (active speaker), `streamer-clips` section 8.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/speaker_cmds.rs`
- `engine/rustler_engine/speakers.py`, `engine/MODELS.md` (diarization models, URLs, sha256, licences)
- `crates/rustler-captions/src/ass.rs` (`write_ass_colored`, additive; `CaptionWord` unchanged)
- Tests: `crates/rustler-commands/tests/speakers.rs` (10 tests), `engine/tests/test_speakers.py`, `crates/rustler-captions/tests/ass_colored.rs` (6 tests)

**Read-only dependencies (consume, never edit):**
- `crates/rustler-media/src/facepath.rs` (`crop_keys`), the `crop.track_face` helpers in `clip_cmds.rs`, `render_cmds.rs` (one-line wiring for coloured ASS).

## Rules and Edge Cases

1. Speaker ids `S1`, `S2`... follow first appearance; colours are keyed by id, names are display-only.
2. The preset's active-word highlight is unchanged: pick an S1 colour other than yellow with `bold-karaoke` or `clean-white`.
3. `crop.track_speaker` needs a `center-crop` segment; turns shorter than 1.0 s never move the crop; re-run it after trimming a clip.
4. Models download once to `~/.rustler/models/diarization/` with sha256 checks; the heuristic fallback can split one speaker in two (it did on the JFK sample).

## Tasks

- [x] analyze.speakers, speaker.rename, caption.speaker_colors, crop.track_speaker
- [x] Test on real two-person footage (2 public-domain White House fireside chats; `docs/test-media.md`); fixes in the status log

## Acceptance Criteria

- [x] Real run on the 55 s JFK demo: 1 speaker, 110/110 words labelled, 7.3 s (progress log); engine test JFK + FDR + JFK = 2 speakers
- [x] Speaker-coloured preview frames checked (yellow S1 / cyan S2)
- [ ] Docs pass limitation: on the docs machine `analyze speakers` returned "engine: method not found: speakers.diarize" because the engine installed under `~/.rustler` predates it; `rustler setup run` refreshes the engine (not re-run in the docs pass)

## Verification

`cargo test -p rustler-commands --test speakers`, `cargo test -p rustler-captions --test ass_colored`, `~/.rustler/bin/uv run --project engine pytest -q engine/tests/test_speakers.py`.

## Status Log

- 2026-11-15 (real footage) - Tested on real two-person footage (wide two-shot, close-ups, zooming very wide shots; `docs/test-media.md`).
  Found and fixed: (1) diarization merged two similar voices into one speaker (guest 96 % of the time): now windowed-embedding
  re-segmentation (`speakers.py`), 91.5 % of speech time attributed correctly on the hand-labelled excerpt, auto speaker count 2 (was 7);
  (2) faces were tracked by position only, so a cut to a close-up made a new "face": SFace appearance embeddings now keep one `face_id` per
  person across cuts; (3) mouth movement is noise on 4-6 % wide faces (wide two-shot): ignored below 7 % width, speaker -> person is learned
  from close-ups (`face-id`) and elimination instead; (4) after a cut the crop followed a position from the old shot: now follows the largest
  visible face; (5) crop keys drifted for seconds between a still stretch and the next move because the renderer interpolates linearly between
  keys and the path only emitted keys when it moved (`SmoothParams::hold_keys`, on for the speaker crop; `crop track-face` still has it off);
  (6) turns shorter than 1.0 s (was 0.6) never move the crop. Crop-contains-speaker rate over 1 s samples: Vance excerpt 33 % -> 97 %,
  Boyle excerpt 95 % -> 100 % (`crop track-face`: 70 % / 63 %). Regression tests use recorded fixtures from the real runs.

- 2026-11-15 — Built (C4) and merged; registry at 118 commands.
