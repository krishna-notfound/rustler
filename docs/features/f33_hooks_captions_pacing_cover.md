# f33 — Hook presets, keyword/emoji captions, flash/whip transitions, cover frame

**Status:** Built  |  **Depends on:** f09 (overlays), f10/f17 (captions), f18 (transitions), f23 (asset library, sfx assets)  |  **Owner:** hooks agent (v2 wave D, design §5.5 items 3-6)

## Purpose

OpusClip-style polish the agent can apply from the CLI/MCP: viral hook titles that animate in (with a whoosh), captions
that highlight only the keywords, a quick flash or whip at a cut, and a good cover frame saved with the export.

## Scope

**In**
- **Hook styles** (`overlay.add_text --style ...`, `overlay.hook_styles {}` lists them): `hook-big` (two uppercase bold lines,
  0.10 of the width, thick outline, pop), `hook-box-yellow` (black on yellow box, slide from above), `hook-outline` (yellow,
  thick outline, typewriter), `hook-pov` (left-aligned box, yellow "POV:" added when missing, slide from the left),
  `hook-question` (red closing "?" added when missing, zoom-in). `--anim none|pop|zoom-in|slide|typewriter` overrides the entry
  animation of any style; `--sfx <file|asset:id|url> [--sfx-volume-db]` adds a whoosh (a normal `audio.add_sfx` entry at the hook's
  start, stored as `sfx_id` on the overlay; moved by `overlay.edit --start`, removed by `overlay.remove`). Old styles
  (`hook-top`, `lower-third`, `plain`) render exactly as before.
- **Captions:** presets `keyword-highlight` (static lines; words with their own colour get that colour and 1.15x size) and
  `emoji-karaoke` (per-word emoji from `caption.word --emoji`); `caption.keywords {clip_id, words[] | auto, max?, color?}` marks
  keywords as caption word colour overrides (auto = numbers, EN + Hinglish + a few Devanagari emphasis words, Title Case names;
  one per ~6 words; never adjacent; existing colours kept; `--dry-run` previews). Emoji are drawn when an emoji font exists
  (system or `~/.rustler/fonts`), else dropped from the captions with a warning.
- **Pacing:** `transition.set` kinds `flash` (xfade fadewhite, default 0.1 s), `flashblack` (fadeblack, 0.1 s), `whip` (xfade hblur, 0.2 s).
- **Cover:** `clip.cover {clip_id, auto?, out_t?}` on the latest render; saves `clips/<id>/cover.jpg`, records extras key `cover`
  (`file`, `out_t`, `file_t`, `method`, `score`, `faces_measured`); `export.clip` copies `<name>.cover.jpg` and sets `post.json`
  `cover` to the record with the copied file name.

**Out:** colour emoji (libass draws the system emoji font as a one-colour outline), real blink/mouth-open detection (a local
contrast proxy at the eye landmarks is used), clean (caption-free) cover frames, GUI.

## Files

`crates/rustler-captions/src/{presets,custom,ass,emoji}.rs`, `crates/rustler-media/src/compile.rs` (overlay styles/animations,
transition kinds), `crates/rustler-commands/src/{overlay_cmds,fx_cmds,keyword_cmds,cover_cmds}.rs` (+ `lib.rs`, one-line hooks in
`render_cmds.rs` and `qc_cmds.rs`), tests `crates/rustler-captions/tests/keyword_presets.rs`,
`crates/rustler-media/tests/hooks.rs`, `crates/rustler-commands/tests/hooks_captions.rs`, knowledge guides
`captions-and-style.md`, `director-retention.md`.

## Known limits

Cover scoring: sharpness = Laplacian variance of the face box (or the middle of the frame without a face), face size/position,
eye-patch contrast and nose-between-eyes as "eyes open / facing camera" proxies; without the engine, sharpness only. Hook
typewriter starts empty on frame 0 (the first frame is the start of the animation). A bigger "?" is only applied to unboxed
hook-question text (it would stretch the box). With `keyword-highlight`, speaker colours also count as "own colour" (enlarged).

## Status Log

- 2026-11-16: built and tested (see the progress log entry of the same date).
