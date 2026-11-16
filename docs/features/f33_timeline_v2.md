# f33 — Timeline v2 document + timeline edit commands (hooks, merges, speed, freezes)

**Status:** Built  |  **Depends on:** f03, f18, f19, layers (1.3), f29, f31  |  **Owner:** timeline agent (wave C, Opus)

## Purpose

User goal: the agent has FULL control of the whole timeline: it sees every item with exact times and ids, edits
anything in one step, can write a whole timeline, and builds hooks by merging/reordering pieces (plan Tasks 1.1,
1.2 and "Added to wave C").

## Scope

**In:**
- `rustler.timeline@2` types in `rustler-core` (`Timeline`, `Tracks`, `VideoSeg`, item types under
  `rustler_core::timeline`), covering every extras key 1:1; unknown keys kept in `tracks.extra`, unknown item
  fields kept in each item's `extra`.
- `timeline_doc::assemble` / `disassemble` (clip.json + extras.json, revision-checked, extras snapshotted to
  `history/<rev>.extras.json`), `validate` (errors listed by JSON path), `resolve` (edit + rendered times),
  `diff` (human-readable changes), re-cut helpers (content-following remap).
- `timeline.show`, `timeline.schema`, `timeline.apply {clip_id, base_revision, patch | document, dry_run}`.
- `edit.insert_segment`, `edit.cold_open`, `clip.merge`, `clip.duplicate`, `edit.speed` (extras `segment_speed`),
  `edit.freeze` (extras `freezes`).
- Render: segment speed (`setpts` + `atempo`) and freeze frames (`loop` + silence) in `compile.rs`;
  `TransitionMap` retimes edit time through speeds and freezes, so captions, overlays, effects, sfx, layers,
  preview frames, QC duration, retention and `render.run` all use the effective duration.
- `history undo/restore` restore extras.json too (edit_cmds `finish` snapshots, `restore_revision` restores).

**Out:** a `flash` transition kind (cold open uses a `flash` effect), GUI for the new tracks (dropped by the user),
migrating v1 layer commands onto `timeline.apply` (they still write their own key without a revision bump).

## Storage mapping

| extras key | document |
|---|---|
| (clip.json segments) + `crop_paths`, `segment_speed` | `tracks.video[]` (`crop_path`, `speed`) |
| `freezes` | `tracks.freezes` |
| `cutaways` / `images` / `masks` | same names |
| `overlays` | `tracks.text` |
| (clip.json captions) + `caption_word_overrides`, `speaker_colors` | `tracks.captions` (`word_overrides`, `speaker_colors`) |
| (clip.json audio) + `audio_fx`, `music`, `sfx` | `tracks.audio` |
| `effects` / `transitions` / `progress_bar` | same names |
| `meta` (`score`, `retention`, others), `post`, `variant_of`, `variant_info` | `meta` |
| `template_ref` | `template_ref` |
| anything else | `tracks.extra` |

## Files

- `crates/rustler-core/src/model/timeline.rs` (+ `model/mod.rs` export), `crates/rustler-core/tests/timeline.rs`
- `crates/rustler-commands/src/{timeline_doc.rs, timeline_cmds.rs, timeline_edit_cmds.rs}` (+ `lib.rs`)
- `crates/rustler-commands/tests/{timeline.rs, timeline_edit.rs}`
- small edits: `crates/rustler-media/src/compile.rs` (speed, freezes, `TransitionMap::with_retime`),
  `edit_cmds.rs` (extras snapshot/restore), `fx_cmds.rs` (`transition_map` + `apply_to_compile` read speeds and
  freezes), `audio_cmds.rs` (two `TransitionMap::new` -> `fx_cmds::transition_map`)
- knowledge: `editing-toolkit.md` (section 0 + section 1 rows), `cli-reference-conventions.md` (command map)

## Decisions

- Item times stay on the **edit timeline** (sum of source lengths) like every v1 command; the rendered time comes
  from `TransitionMap` (retime, then transitions). A time exactly on a freeze maps to the start of the hold.
- Field names inside a track keep the stored names (`start`/`end` and `box` for text, `word_index` for sfx anchors)
  so v1 data and v1 commands agree with the document; the plan's `When` enum was not added (stored items mix
  out times and word anchors).
- An unresolvable anchor that already failed on disk is a warning on apply; a new one is an error.
- Cold open keeps a hook-top title that opened the clip at the start; other layers follow their content.
- `clip.merge`: fresh segment ids, colliding item ids renumbered, first clip's captions/music/progress bar win,
  `meta.merged_from` records the inputs.

## Status Log

- 2026-11-17: built; tests `rustler-core --test timeline` (4), `rustler-commands --test timeline` (9),
  `--test timeline_edit` (8, incl. a real render through QC); media/core/commands suites green. Real check on a
  temp copy of rustler-real1 (cold open + merge + speed + freeze, rendered and looked at).
