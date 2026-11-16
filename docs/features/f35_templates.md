# f35 — Templates (brand kit + recipe)

**Status:** Built  |  **Depends on:** f33 (timeline v2 / `timeline.apply`), f23 (asset library), f18, f15, config (Task 0.6)  |  **Owner:** templates agent (wave D, Opus)

## Purpose

User goal: the agent saves a clip's whole look once and reuses / improves it on every later job, applies it to
many clips in one step (undoable), and carries it to another machine with its fonts and assets (plan Task 1.5,
design §5.3).

## Scope

**In:**
- `rustler.template@1` document (`crates/rustler-commands/src/template_cmds.rs`, `Template`): `canvas`, `layout`
  (preset / params / regions / background / force), `captions` (enabled, preset, position_y, max_words_per_line,
  script, speaker_colors), `hook` (look, at_seam, duration, max_words, optional text), `text_styles`
  (lower-third / plain looks), `texts` (only with include_text), `images` (brand images: asset or file),
  `progress_bar`, `audio` (loudness, denoise, volume, music or `none`, sfx patterns), `effects` patterns,
  `transitions`, `fonts` (font asset ids), `notes`, `based_on`. Absent section = leave that part of the clip alone.
  Patterns place items `at: start | cuts | every (every_s) | time (t)`.
- Commands: `template.create {name, from_clip | template, include_text?, description?, scope?}`,
  `template.update {name, from_clip | template | patch (merge patch) | description}` (new version),
  `template.show`, `template.list`, `template.delete`, `template.apply {name[@ver], clip_ids | all, overrides?, dry_run?}`,
  `template.diff {a, b}`, `template.export {name, out}`, `template.import {file, scope?}`.
- Storage `templates/<name>@<ver>.json` in the project or `<home>/templates/`; resolution project → global →
  built-in (bare name = highest version in the first scope that has it; `name@N` searches every scope).
- Built-in starters `podcast-viral`, `streamer-split` (`crates/rustler-commands/starters/*.json`, embedded).
- `clip.create --template <name[@ver]>` and config `template.default` (explicit: resolved and checked before the
  clip is written, error → no clip; configured default that cannot apply → warning). The template is applied after the smart defaults (f34), as one more revision.

**Out:** spec §5.3 fields `layout_rules` (faces=1 / faces>=2 / screen), `cutaway_rules`, `cleanup`,
`keyword_highlight` / `emoji`, `export` (quality / lengths), `post_defaults`, `brand.cta_card/outro_card`, music by
tag/mood. Unknown fields are rejected, so a spec-shaped template with these fails validation (listed paths).

## How apply works

1. `prepare`: resolve `name[@ver]`, merge `overrides` (RFC 7386), validate fields, `check_refs` — every
   referenced caption style / font / image / music / sfx asset or file must exist, else `not_found` naming it.
2. Per clip: `timeline_doc::assemble` → `build` (new document) → `json_patch::diff` → `timeline_cmds::apply`
   dry run (every clip is validated before any is saved) → `timeline_cmds::apply` for real: one revision,
   history snapshot, `history undo` reverts it.
3. Items a template adds carry `by_template: "<name>@<ver>"` (+ `template_role`) in their extra fields; a re-apply
   removes those first, so nothing stacks and re-applying the same template is a no-op (0 ops). Untagged items
   are never removed. `template_ref {id, version}` is set on the document.
4. Layout: segments already on the template's preset keep their layout (crop path, detected webcam panes) unless
   `layout.force`; built-in presets are recomputed for each source's size; changing a layout away from
   center-crop drops that segment's crop path.

## Capture (`--from-clip`)

First segment's layout (center-crop / blur-fit: preset only, framing is content; split / custom: normalized
regions); caption settings; first hook-top text's look + duration; first lower-third / plain look; time-anchored
images (asset id when the file is a library asset); progress bar (or `enabled: false`); loudness, denoise,
volume, music (asset id when in the library); sfx and effects that start at 0 or on a cut become `start` /
`cuts` patterns; the first transition kind; canvas; font assets matching a custom caption style's family.
`notes` lists what was not captured (word overrides, masks, cutaways, freezes, effects/sfx at content times,
text unless `include_text`).

## Export / import bundle

Zip with `template.json`, `bundle.json` (`rustler.template-bundle@1`: asset entries + plain files),
`assets/<kind dir>/<file>`, `files/<n>-<file>`. Import adds missing assets to the project (or global) library
(existing ids are kept, warning when the sha differs), unpacks files to `templates/files/<name>@<ver>/` and
rewrites the template's file paths; same name@ver with the same content → "already installed", other content →
next free version.

## Files

- `crates/rustler-commands/src/template_cmds.rs` (new), `crates/rustler-commands/starters/{podcast-viral,streamer-split}.json` (new)
- `crates/rustler-commands/src/lib.rs` (module + register), `timeline_cmds.rs` (`apply` made `pub`),
  `clip_cmds.rs` (`ClipCreateIn.template` + apply after create)
- `crates/rustler-commands/tests/templates.rs` (new)
- knowledge: `knowledge/guides/templates-and-brand.md` (new), `learn-style-from-videos.md` (spec section points at templates)

## Decisions

- Templates live in `templates/` (not the asset library's `assets/templates/` kind) as the plan says.
- Hook text is per clip: the template restyles existing hook-top titles and only adds one when `hook.text` is
  set (template or overrides); a missing hook is a warning naming `overlay add-text`.
- `podcast-viral` does not set a layout (framing is per shot); `streamer-split` uses the documented default panes
  (facecam top-right) and keeps segments that are already split (e.g. from `layout detect-webcam`).
- Starters reference no assets, so they apply on any install.
- `template.update` on a built-in writes the agent's own version (project, else global) that shadows it.

## Status Log

- 2026-11-17: built; `cargo test -p rustler-commands --test templates` 5 passed (create_from_clip_captures_style_layout_hook_audio,
  apply_writes_timeline_items, apply_missing_asset_errors_with_name, export_import_roundtrip_with_assets,
  starter_templates_validate) + 3 unit tests; clip/config/timeline/timeline_edit/variant suites green;
  `cargo check --workspace --tests` clean. No real render with a template was run in this task.
