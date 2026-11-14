---
name: polish_clip
title: "Polish a finished cut: hook, sound, transitions, effects, fix loop"
description: "Take a clip that is already cut and laid out and add the finishing layer: hook title, sound effects on emphasis, transitions, progress bar, then render, QC and the fix loop."
arguments:
  - name: clip_id
    description: "The clip id (clp_…) to polish. It should already have the right cut and layout."
    required: true
---

Polish clip **{{clip_id}}**. The cut and the layout are assumed right; if they are not, fix those first
(`cut-points`, `layouts-and-framing`). Guides: `editing-toolkit`, `audio`, `assets`, `quality-checklist`.
Less is more: every addition must earn its place. `rustler history undo {{clip_id}}` reverts a step.

## 1. Read the state
`rustler clip show {{clip_id}}` (segment ids, output times, existing extras), `rustler audio list {{clip_id}}`,
`rustler overlay list {{clip_id}}`, `rustler effect list {{clip_id}}`, `rustler transition list {{clip_id}}`,
`rustler render preview-frame {{clip_id}} --out-t 1.0` (look). Read the words with their times:
`rustler transcript get <source_id> --start <seg start> --end <seg end>`.

## 2. Tighten (before adding anything timed)
`rustler edit refine {{clip_id}}`, `rustler edit remove-pauses {{clip_id}} --keep 0.25`,
`rustler edit remove-words {{clip_id}}`. Time-anchored items (overlays, effects, `--at` effects) do not
follow later cuts, so finish cutting first.

## 3. Hook title
`rustler overlay add-text {{clip_id}} "<hook, <= 7 words>" --style hook-top --start 0 --end 2.5`. State the
payoff or the tension ("He quit his job for this"), never describe the video. A brief's credit or watermark:
`overlay add-text … --style lower-third`. Preview at 0.5 s: readable, not over the face, not in the top 8 %.

## 4. Sound
- `rustler audio denoise {{clip_id}} --strength light` for room noise or hiss (`medium`/`strong` only if needed).
- Sound effects on emphasis: pick the emphasised words from the transcript, then `rustler asset search
  <whoosh|impact|pop|swipe> --kind sfx`, `rustler audio add-sfx {{clip_id}} asset:<id> --at-word
  '{"source_id":"<src>","word_index":<i>}' --volume-db -9`. One or two per clip; follow the word, stay under the voice.
- Music only if it helps and its license is known (`asset list --kind music`; add yours with `asset add --license`):
  `audio add-music {{clip_id}} --path asset:<id> --volume-db -18`.
- A quiet take next to a loud one: `audio volume {{clip_id}} --db 3 --segment-id <seg>`.

## 5. Motion
- Transitions only between segments where the place or topic changes: `rustler transition set {{clip_id}}
  <seg_id> dissolve --duration 0.25` (this shortens the clip by the overlap; re-check the brief duration).
- Effects on beats: `effect add {{clip_id}} punch-in --start <t> --end <t+0.6>` on the emphasis word,
  `flash` or `shake` on an impact. Two or three in 30 s. Nothing in the first 0.5 s.
- `overlay progress-bar {{clip_id}} --enabled true` on 30-60 s explainers.
Output times come from `clip show`; `render preview-frame {{clip_id}} --out-t <t>` at each effect and at the
last second. Look before moving on.

## 6. Render, watch, fix loop
1. `rustler render run {{clip_id}}` → `rustler qc run {{clip_id}}` → **open the contact sheet**.
2. `rustler fix suggest {{clip_id}}` → `rustler fix apply {{clip_id}}` runs the auto-safe fixes (captions off the
   face, pauses, re-render, re-QC); decide the creative ones yourself with the commands listed.
3. `rustler qc compare {{clip_id}} <old render> <new render>` and open the image to confirm it improved.
4. Brief present? `rustler brief check {{clip_id}}` and fix what it lists.
5. Stop after 3 rounds per problem; remove the addition that causes it (`overlay remove`, `effect remove`,
   `transition clear`, `audio remove-sfx`).

## 7. Report
What you added (hook text, sound effects with ids and levels, transitions, effects, progress bar), what you
left out and why, QC result, contact-sheet observations, final duration. Export only if asked:
`rustler export clip {{clip_id}}`.
