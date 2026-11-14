---
name: fix_qc_failures
title: "Fix QC failures on a clip"
description: "Read the latest QC report and contact sheet for a clip, map each failure and warning to an edit, re-render and re-check until it passes."
arguments:
  - name: clip_id
    description: "The clip id (clp_…) whose QC failed or warned."
    required: true
---

Fix clip **{{clip_id}}** until QC passes and the contact sheet looks right. Use `quality-checklist`
as your reference. Never use `--yes` to export past a real failure.

## 1. Gather facts
1. `rustler clip show {{clip_id}}` → segments (source times + `out_start/out_end`), layout, captions,
   renders.
2. `rustler qc run {{clip_id}}` (re-runs on the latest render; exit 7 = failed). Read every check:
   `name`, `status`, `value`, `expected`, `fix_hint`.
3. Open the contact sheet image. Note each visual problem with its output time label.

## 2. Map problems to edits
QC times are output times; find the segment whose `out_start ≤ t < out_end`, then the source time
`segment.start + (t − out_start)`.

| Problem | Edit |
|---|---|
| `black_frames` fail | `see frame <src> --t <source t>`; move the segment boundary off the black (later start unit / `edit trim`), or fix a region rect pointing at an empty area |
| `duration` / `av_sync` fail | re-render once; if it repeats, trim the segment end by 0.1-0.2 s (`edit trim --end-delta`) |
| `streams` / `dimensions` fail | `rustler doctor run`; re-render with `--prefer-gpu false` |
| `loudness` warn | lower/remove music bed; cut the extreme passage; accept only if it sounds fine and note it |
| `long_silence` warn | `edit remove-pauses {{clip_id}} --min-gap 0.5` or trim at the hinted time |
| `frozen_video` warn | if not a slide, cut that stretch (`edit split` + `edit delete-segment`) |
| face cut off / wrong person | `clip set-layout` with a new `focus_x`, `crop track-face`, or `blur-fit` |
| captions over face / in UI zone | `clip set-captions {{clip_id}} --position-y <0.66-0.78>` |
| captions unreadable | `--preset boxed-highlight` |
| slow first seconds / weak ending | change start/end unit (`cut-points`) |

Commands marked "(if available)" in the guides may not exist yet: check `rustler commands list`. If an
edit is impossible, recreate the clip with `clip create` using corrected units.

## 3. Apply one fix per problem, then verify cheaply
- After layout/caption edits: `rustler render preview-frame {{clip_id}} --out-t <problem time>` and
  look before re-rendering.
- If an edit made it worse: `rustler history undo {{clip_id}}`.

## 4. Re-render and re-check
`rustler render run {{clip_id}}` → `rustler qc run {{clip_id}}` → open the new contact sheet.
Repeat at most 3 rounds for the same problem. Still failing → explain why and suggest a different
moment or source.

## 5. Report
List each original problem → the edit you made → the result (pass/warn accepted with reason). End
with the current QC status and the contact-sheet observations. Export only if the user asked for it
in this task or the original job required it: `rustler export clip {{clip_id}}`.
