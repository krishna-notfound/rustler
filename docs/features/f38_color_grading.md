# f38 — Colour grading (agent-controlled)

**Status:** Built  |  **Depends on:** f10 (render compiler), f18 (effects/transitions), f23 (asset library), f33 (timeline + history), f35 (templates)  |  **Owner:** Opus agent (branch `worktree-agent-a6f98bca27f6592f8`)

## Purpose

User request: "agent have color grading feature which allow it to do anything ... smooth and good, lightweight,
easy, we don't need a huge codebase". User rule: **the system never applies a look on its own**. Rustler
measures and suggests, and the agent runs the commands. A small, FFmpeg-filters-only grade the agent sets per clip or per
segment, previews before/after, and renders in the same single encode.

## Scope

**In:** extras key `grade` (clip grade + per-segment overrides); 8 built-in presets; exposure, contrast,
saturation, vibrance, temperature, tint, blacks, shadows/midtones/highlights colour, `.cube` LUT (+ strength),
intensity; compile per segment on the source picture before layout and overlays; commands `grade.set`,
`grade.show`, `grade.presets`, `grade.clear`, `grade.preview` (side-by-side before | after jpg), `grade.measure`
(frame stats + hints + shot matching, never applies); history undo; asset kind `lut`; optional template field
`grade`; guide `color-grading`.
**Out:** automatic grading anywhere (no starter, auto command or smart default sets a grade); third-party LUTs
(none bundled); grading cutaway media and image layers (they keep their own colours); keyframed / animated
grades; HDR / 10-bit output.

## User-facing Behavior

```
rustler grade set <clip> [--segment-id S] [--reset] [--preset P|none] [--exposure E] [--contrast C]
                  [--saturation S] [--vibrance V] [--temperature K] [--tint T] [--blacks B]
                  [--shadows r,g,b] [--midtones r,g,b] [--highlights r,g,b]
                  [--lut asset:<id>|path|none] [--lut-strength X] [--intensity I]
rustler grade show <clip>          # stored grade + exact ffmpeg chain per segment
rustler grade presets
rustler grade clear <clip> [--segment-id S]
rustler grade preview <clip> --out-t T      # left = before, right = after, 960 px high
rustler grade measure <clip|source> [--t T ...]
rustler asset add look.cube --kind lut
```

`set` merges onto the stored grade (`--reset` starts empty). A segment override merges onto the clip grade
field by field; `--reset --segment-id` makes it standalone (only its own values); a segment `--lut none`
blocks the clip LUT there. Ranges: exposure/vibrance/tint/blacks -1..1, contrast 0.5..1.5, saturation 0..2,
temperature 2000..12000 K (lower = warmer picture), shadows/midtones/highlights channels -1..1,
lut_strength/intensity 0..1. Every change is a new clip revision with an extras snapshot (`history undo`).

## Routes / Files

**Owned files:**
- `crates/rustler-media/src/grade.rs` (model, presets, chain builder, `.cube` check/generator, frame stats, hints, side-by-side)
- `crates/rustler-media/tests/grade.rs`
- `crates/rustler-commands/src/grade_cmds.rs`
- `crates/rustler-commands/tests/grade.rs`
- `knowledge/guides/color-grading.md`
- `docs/features/f38_color_grading.md`

**Shared edits (minimal):** `crates/rustler-media/src/lib.rs` (module), `crates/rustler-media/src/compile.rs`
(`CompileExtras.grades`, `GraphPlan.lut_files`, grade chain after the segment trim, LUT staging),
`crates/rustler-commands/src/lib.rs` (module + register), `render_cmds.rs` (`prepare(.., graded)`,
`preview_frame_image_with`), `edit_cmds.rs` (one call: carry segment overrides through re-cuts),
`timeline_doc.rs` (shape check of `grade` in `tracks.extra`), `asset_cmds.rs` + `rustler-tools/src/assets.rs`
(asset kind `lut`, `.cube` check), `template_cmds.rs` (optional `grade` field: validate, capture, apply,
LUT ref, import remap), `knowledge/guides/start-here.md` (one line).

## Data and Interfaces

- `rustler_media::grade::{Grade, GradeDoc, Rgb, Look, presets(), chain_for(), check_cube(), cube_from_fn(),
  stats_rgb24(), hints(), measure_frame(), side_by_side()}`.
- extras `grade`: `{"clip": Grade?, "segments": {"<seg_id>": Grade}}`; Grade fields all optional
  (`preset, exposure, contrast, saturation, vibrance, temperature, tint, blacks, shadows, midtones, highlights,
  lut, lut_strength, intensity, standalone`). It passes through the timeline document as `tracks.extra.grade`.
- Filter chain (only the filters a grade needs, in order): `colorcorrect` (chroma offsets ramped by luma:
  shadows/highlights colour, midtones and tint shift both ends) → `colortemperature=temperature=K:pl=1` →
  `curves=master='17 points':interp=pchip` (blacks, exposure as a power curve, contrast S-curve; 0 and 1
  fixed, monotone) → `colorchannelmixer` (saturation around Rec.709 luma) → `vibrance` → `lut3d=file='grade_lutN.cube'`
  (with `split`/`blend=all_mode=normal:all_opacity=S` when strength < 1). Intensity scales the numbers.
- Render: `[i:v:0]...trim,setpts=PTS-STARTPTS,<grade>` then the layout `split` / crops / scale / blur / overlay,
  then join (xfade), effects, captions, text, images, progress bar. LUT files are copied into the render workdir
  (`GraphPlan.lut_files`; ffmpeg runs with cwd = workdir).

## Rules and Edge Cases

1. Nothing grades by default; neutral or `intensity 0` adds no filter.
2. `colorbalance` was tried first and rejected: its per-channel weights turn saturated yellows olive when blue is
   pushed into the shadows (seen on the trial graphics); `colorcorrect` works on chroma by luma and keeps hues.
3. Shadow colour pushes tint pure black (`colorcorrect` ramps from black), so the presets keep them small
   (cinematic blue +0.07); larger values visibly tint a black studio background.
4. Overrides on segment ids that no longer exist are ignored with a warning; `edit.*` re-cuts copy an override to
   the new pieces (largest source overlap) and drop dead ids (the history snapshot keeps them for undo).
   `timeline_edit` commands (insert, cold open) do not carry overrides yet.
5. A missing or invalid LUT fails the render with a hint (`--lut none`), never silently.
6. `grade.measure` judges brightness on the lit pixels when > 35 % of the frame is black; white balance from
   near-neutral midtone pixels; shot-matching suggestions go half-way and are capped at ±0.2 exposure.
7. `history undo` right after a render first steps over the render's revision (render.run saves the clip).

## Tasks

- [x] `grade.rs` model + presets + chain builder + LUT check + stats/hints (unit tests)
- [x] compile integration (all layouts, LUT staging, transitions) + compiler tests
- [x] `grade_cmds.rs` six commands, history path, carry through re-cuts
- [x] asset kind `lut`, template `grade` field
- [x] Integration + real render tests; real check on trial footage
- [x] Guide `color-grading`, start-here line, feature index row

## Acceptance Criteria

- [x] Neutral grade compiles to no filter; ranges validated; every preset valid and distinct.
- [x] set/show/clear/undo, per-segment override, LUT asset, template capture/apply covered by tests.
- [x] Real render: warm grade moves R-B up, bw makes R=G=B, the progress bar keeps #FFE000 and captions keep their colour.
- [x] Real trial footage previewed with natural-pop, cinematic, moody, warm, bright-clean: skin natural, nothing clips.

## Verification

`$env:CARGO_TARGET_DIR="D:\rustler-build\grade"; cargo test -p rustler-media --lib grade` (unit),
`cargo test -p rustler-media --test grade` (3), `cargo test -p rustler-commands --test grade` (3; the render test
needs ffmpeg and skips otherwise), `cargo test -p rustler-knowledge`.

## Status Log

- 2026-11-16 — Built. Presets tuned on trial3 podcast frames (close-up with white shirt, wide shot with yellow
  graphics); switched toning from `colorbalance` to `colorcorrect` after it greyed the yellow boxes. Real check on a
  copy of trial3 (`play-the-instrument`): previews for natural-pop / cinematic / moody / warm / bright-clean kept
  skin natural and the white shirt unclipped; captions unchanged; cinematic 0.8 + a segment override rendered
  (44.7 s draft in about 15 s); a second `history undo` restored the pre-override grade. Previews kept in
  `D:\rustler-build\grade-previews`.
