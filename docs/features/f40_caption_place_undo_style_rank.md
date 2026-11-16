# f40 — Caption placement, per-layer undo, template from a learned style, export by score

**Status:** Built  |  **Depends on:** f16 (captions), f18 (overlays/effects), f21 (QC v2), f22 (export batch), f31 (scores), f32 (style references), f33 (timeline + history), f35 (templates)  |  **Owner:** Opus agent (branch `worktree-agent-ae8cc8c52c794827c`)

## Purpose

Trial 3 (`docs/research/trial-3-2026-11-17.md`) found double captions: the stage shot of clip 1 showed the
source's own burned-in subtitles under ours. The user also asked for undo that removes exactly one layer
change, a template made from a learned style, and exports ranked best first. User rule: **Rustler measures and
suggests; the agent decides** — nothing here changes a clip unless the agent passes an explicit flag.

## Scope

**In:**
1. `caption.place <clip> [--apply true] [--every S] [--faces false] [--base-revision N]`: per segment, burned-in
   source text (engine-free edge-density scan of the source range, cached in `analysis/<source>/text_bands.json`,
   plus the `analyze ocr` caption band when `ocr.json` exists) and faces (`analysis/<source>/faces.json`, else
   engine `faces.track` on the segment range) mapped onto the canvas through layout regions, fit and crop path;
   a clip-wide suggested `position_y` (0.10-0.85, weighted by segment length, closest to the current spot) plus
   per-segment best position, safe ranges and what the current / suggested positions cover. `--apply true` sets
   it as one new revision. QC check `caption_over_source_text` (warn when our caption band overlaps the source's
   burned-in subtitles, or sits within 0.03 of them, for >= 0.5 s).
2. Every single-layer write is its own clip revision with an extras snapshot (`history/<rev>.extras.json`):
   overlays (a hook and its whoosh together), images, masks, cutaways, caption word overrides, keywords, effects,
   transitions, progress bar, sfx, denoise/volume, music, crop paths (grade already did). Every other clip save
   (caption/layout settings, `edit refine`, render and QC records, smart-default steps) snapshots the extras of the
   revision it supersedes. `history undo` skips revisions that only recorded a render/QC report.
3. `template.create --from-reference <set>`: maps `references/<set>/profile.json` onto a template (caption
   position / words per line / closest built-in style by accent colour, hook position + duration, loudness target)
   with `notes` for pacing, zoom (recorded only, never automatic zooms), music yes/no, shot types and what is not
   derived.
4. `export.batch` orders clips best first by the agent score (`clip.score_set`), then the retention score
   (`clip.retention`), unscored last (`--order given` keeps the given order); rank in the output, in each
   `<name>.report.json` (`rank`), optionally in the file name (`--rank-in-name true` -> `01-<name>`).
   `clip.list --sort score` lists clips the same way with `rank`, `agent_score`, `retention_score`.

**Out (explicitly):** per-segment caption positions (the frozen caption model has one `position_y`; conflicts
are reported per segment), OCR in the lite install, automatic masking of source subtitles (suggested as a
command only), music track choice in learned templates.

## User-facing Behavior

```
rustler caption place clp_X                   # measure + suggest, no change
rustler caption place clp_X --apply true      # one undoable revision
rustler template create my-look --from-reference my-style
rustler clip list --sort score
rustler export batch --all true [--order given] [--rank-in-name true]
rustler history undo clp_X                    # removes exactly the last layer change
```

## Routes / Files

**Owned files:**
- `crates/rustler-media/src/textband.rs` (band finder, clustering, frame reader)
- `crates/rustler-commands/src/caption_place_cmds.rs`
- `crates/rustler-commands/src/style_template.rs`
- `crates/rustler-commands/tests/f40.rs`
- edits: `extras.rs` (`save_clip`, `commit`, `update_revision`, `edit_key`, `history_rel`), `audio_cmds.rs`
  (`modify_key`), `overlay_cmds.rs`, `fx_cmds.rs`, `clip_cmds.rs` (`save_extras`, `mutate_clip`, `clip.list`),
  `cut_cmds.rs`, `layout_cmds.rs`, `qc_cmds.rs`, `render_cmds.rs`, `smart_cmds.rs` (`save_step`), `edit_cmds.rs`
  (`history.undo` skip), `timeline_doc.rs` (snapshot path), `template_cmds.rs`, `reference_cmds.rs`
  (`profile_rel`), `variant_cmds.rs` (`export.batch`, `scores_of`, `rank_by_score`), `rustler-media/src/qc.rs`
  (`SourceTextSpan`, `source_text_check`, public safe-zone constants)
- guides `captions-and-style`, `templates-and-brand`, `learn-style-from-videos`, `editing-toolkit`

## Rules and Edge Cases

1. Measuring never writes the clip; `--dry-run` with `--apply` writes nothing.
2. Subtitle bands are hard blockers, faces cost by share of samples covered (> 20 % is reported), static text
   is a soft blocker; text and faces outside a crop are ignored.
3. A layer command that changes nothing (e.g. progress bar off when it is already off) adds no revision.
4. `--base-revision` on layer commands now needs the clip's new revision after each change.
5. `template create` takes one of `from_clip`, `from_reference`, `template`.

## Verification

`cargo test -p rustler-commands --test f40`, lib tests `caption_place_cmds`, `style_template`, media lib
`textband` + `qc::`. Real check on a copy of the trial-3 project (see Status Log).

## Status Log

- 2026-11-17 — Built in worktree `agent-ae8cc8c52c794827c`. Tests: `tests/f40.rs` (5), lib `caption_place_cmds` (4),
  `style_template` (2), media `textband` (2); full `rustler-commands` suite green after updating `clip.rs` (music is now a
  revision, no empty `crop_paths`) and `layers.rs` (`base_revision` after a layer add).
  Real check on a copy of the trial-3 project (debug build): `caption place` on the 3 clips took ~15 s for clip 1
  (scan + engine faces). Clip 1 stage shot (blur-fit, 4.24-8.36 s): source subtitles found at canvas y 0.59-0.62
  (frame: "kind of an example of that" at y ~0.61) right above our 0.70 captions; close-ups cover the face in
  46 % / 83 % of samples at 0.70 (frame at 26 s: caption across the mouth). Suggestion 0.19 clears all; applied, preview
  frames at 6 / 15 / 26 s show captions above the heads and away from the source subtitle; draft render + QC pass
  (`caption_over_source_text` pass at 0.19, warn 4.1 s at 0.70). Clips 2 and 3: current 0.70 ok (only static text
  nearby). Undo: overlay add, effect add, render, undo -> effect gone (render revision skipped), undo -> overlay gone.
  `clip list --sort score` / `export batch --all true --rank-in-name true`: 01 clip 3 (agent 78), 02 clip 2 (61),
  03 clip 1 (retention 86 only); `rank` in each report.json.
  Seen, not fixed: `render_is_fresh` allows 2 s slack, so an undo within 2 s of a render still counts as fresh.
