---
name: apply_template_batch
title: "Apply one template to many clips, check each, fix, and export"
description: "Pick or save a template, dry-run it, apply it to a set of clips in one undoable step, write each clip's own hook, look at every result, run QC and export the passing clips."
arguments:
  - name: template
    description: "Template name (optionally name@version), e.g. podcast-viral or streamer-split. Leave empty to save the look of the clip given in from_clip first."
    required: false
  - name: clip_ids
    description: "Comma-separated clip ids (clp_...) to restyle, or the word all for every clip in the project."
    required: true
  - name: from_clip
    description: "A hand-styled clip (clp_...) to capture as a new template when no template name is given."
    required: false
---

Give the clips **{{clip_ids}}** one consistent look from the template **{{template}}** (when empty: capture
the look of **{{from_clip}}** first). Read the guide first: `rustler guide show templates-and-brand`.
Rustler calls no AI APIs: you choose, you look at the images. Never upload or post anything.

## 1. Choose or capture the template
- `rustler template list` shows project, global and built-in templates; `rustler template show <name>` prints the
  JSON, what it needs (`missing`) and its `notes` (what the template does not do for you).
- Starters: `podcast-viral` (hormozi-pop captions, big boxed hook, progress bar, -14 LUFS; no automatic zooms, you add punch-ins yourself; leaves
  framing alone) and `streamer-split` (webcam-over-gameplay split, hook on the seam).
- No template yet: style **one** clip by hand until it looks right (LOOK at `rustler render preview-frame`),
  then `rustler template create <name> --from-clip {{from_clip}}`. Read `refs` and `notes` in the output: they
  list assets the look needs and what was not captured (word overrides, masks, cutaways, effects at content
  times, text).

## 2. Dry run, then apply
1. `rustler template apply <name> --clip-ids <ids> --dry-run` (`--all` instead of `--clip-ids` styles every clip). Every clip is validated before anything is saved;
   read the listed changes. A missing font, image, music or sfx is a `not_found` error naming it: add it
   (`rustler asset add`) or override it, do not drop it silently.
2. Differences for this run go in `--overrides` (a merge patch, for example `{"captions":{"position_y":0.65}}`
   or `{"hook":{"text":"He quit"}}`); on PowerShell 5.1 put them in a file with `--input-json @file.json`, because
   quotes inside flags are stripped.
3. `rustler template apply <name> --clip-ids <ids>`. Each clip gets ONE new revision: `rustler history undo <clip_id>`
   reverts it. Re-applying the same template is a no-op (items it added carry `by_template`).
4. The template restyles existing hook titles but writes no hook text of its own. Write each clip's hook (at most 7
   words): `rustler overlay add-text <clip_id> "<hook>" --style hook-big --start 0 --end 3`
   (`rustler overlay hook-styles` lists the looks; `--sfx` adds a whoosh).

## 3. Check every clip, not just one
- Framing is per shot and the template may not set it: `rustler layout plan <clip_id>` shows the plan; fix a
  wrong one with `clip set-layout`, `crop track-face` or `crop track-active`.
- `rustler render preview-frame <clip_id> --out-t 0.5` and a mid-clip frame: hook readable and clear of the
  captions, captions on the safe band, logo not covering a face. Look at the images.
- `rustler render run <clip_id>`, then `rustler qc run <clip_id>` and **open the contact sheet**. Use
  `rustler fix suggest <clip_id>` for what failed. A look that works for talking heads can fail for a wide shot:
  change that clip by hand (`timeline apply` or the specific command) instead of weakening the template.
- With a brief: `rustler brief check <clip_id>`.

## 4. Cover, post copy, export
- `rustler clip cover <clip_id> --auto` saves a cover frame; `rustler export clip <clip_id>` copies it.
- Posting copy per clip: `rustler clip meta-context <clip_id>`, then `rustler clip meta-set <clip_id> --input-json @meta.json`
  (guide `campaigns-and-posting`).
- `rustler export batch --all true` exports only clips that pass QC and the brief and tells you why it skipped
  the rest.

## 5. Report
Per clip: id, template used (`template_ref` in `rustler timeline show <clip_id>`), the hook, what you changed by
hand and why, QC result and what you saw on the contact sheet. List clips that did not fit the template and what
you did instead. Say what you could not verify. Never publish.
