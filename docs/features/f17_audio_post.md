# f17 — Audio Post (music bed and ducking only)

**Status:** Verified  |  **Depends on:** f08, f12  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

**Scope note:** only the music bed with speech ducking is built. The feature index entry also names 2-pass
loudnorm, denoise, SFX and voice-over; none of those is built.

## Purpose

A clip can carry a background music track that is quieter under speech and returns between phrases. An agent
adds the bed with one command and removes it with another. The mix is done by FFmpeg inside the render graph,
so the render stays a single pass.

## Scope

**In:**
- `audio.add_music {clip_id, path, volume_db, duck}`; `audio.remove_music {clip_id}`.
- Music settings stored in the clip's `extras.json`.
- Ducking through `sidechaincompress`, mixed with `amix`, with fades (plan parameters: volume -18 dB default, threshold 0.08, ratio 5, attack 100 ms, release 1000 ms, `normalize=0`).
- Music-bed graph built by the render compiler (`compile.rs`).

**Out (explicitly):**
- Two-pass loudnorm. Single-pass loudnorm lands near -15.9 LUFS on half-silent clips (progress log, open).
- Denoise, sound effects and voice-over (not built).
- A licensed music library or asset manifest (f23, planned).

## User-facing Behavior

- `audio.add_music` with `duck: true` lowers the bed under speech; with `duck: false` it mixes directly.
- `audio.remove_music` removes the bed from the clip and leaves the speech track unchanged.
- The music file is an input path; no music is bundled with Rustler.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- None exclusively. The feature is spread across shared files; see below.

**Shared / read-only:**
- `crates/rustler-commands/src/clip_cmds.rs` — `audio.add_music` and `audio.remove_music` (shared with f03, f14, f16).
- `crates/rustler-media/src/compile.rs` — music-bed graph (owned by f08).
- `crates/rustler-media/src/render.rs` — render path that runs the graph (f08).
- Project file `extras.json` in the clip folder — written by the commands above (f03 store).

## Data and Interfaces

- Command input: `clip_id`, `path`, `volume_db` (default -18), `duck` (default true).
- Stored in `extras.json` on the clip.
- Compiler graph (`crates/rustler-media/src/compile.rs`, around lines 305–309): `amix=inputs=2:...:normalize=0` and `sidechaincompress=threshold=0.08:ratio=5:attack=100:release=1000`.

## Rules and Edge Cases

1. Ducking must be measurable: the progress log records -3.9 dB of ducking under speech.
2. A bed without `duck` mixes directly, with no sidechain.
3. Removing the bed must not touch the speech track.
4. Music paths are passed like source paths; spaces and Devanagari in music file names are not separately tested.

## Tasks

- [x] `audio.add_music` and `audio.remove_music` commands
- [x] Music-bed graph with ducking in `compile.rs`
- [ ] 2-pass loudnorm (open)
- [ ] Denoise, SFX, voice-over (not started)

## Acceptance Criteria

- [x] Compiler graph tests: `music_bed_ducks_under_speech`, `music_bed_without_duck_mixes_directly`, `split_vertical_captions_music_graph` (`crates/rustler-media/tests/compile.rs`)
- [x] Render test: `render_music_bed_ducks` (`crates/rustler-media/tests/render.rs`)
- [x] Command test: `music_bed_is_stored_in_extras_and_crop_commands_validate` (`crates/rustler-commands/tests/clip.rs`)
- [x] Render with captions and music: `preview_frame_and_render_center_crop_with_captions_and_music` (`crates/rustler-commands/tests/render.rs`)

## Verification

- `cargo test -p rustler-media --test compile --test render` and `cargo test -p rustler-commands --test clip --test render` (PowerShell).
- `rustler audio add-music` on a clip, then render and measure; pass = speech louder than the bed by the ducking margin.

## Status Log

- 2026-11-15 — Music bed and ducking merged and verified in the render: ducking measured -3.9 dB. Single-pass loudnorm gave -15.9 LUFS on half-silent clips; two-pass loudnorm is the open follow-up.
