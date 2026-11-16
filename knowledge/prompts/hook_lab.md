---
name: hook_lab
title: "Hook lab: write 5 hook variants, pick the strongest, optionally A/B them"
description: "For one clip, write five hooks (on-screen text + first spoken line + cold-open choice), judge them with a rubric, apply the winner and optionally render variants to compare."
arguments:
  - name: clip_id
    description: "The clip (clp_…) whose opening you are working on."
    required: true
---

Work on the first second of **{{clip_id}}**. Guide: `director-retention` (hook formulas, the 3-second rule).

## 1. Read the clip
`rustler clip show {{clip_id}}` (segments, existing overlays), `rustler clip retention {{clip_id}} --per-second false`
(`first_frame_hook`, `speech_start_delay`, `ending`), and the transcript of the whole clip:
`rustler transcript get <source_id> --start <segment start> --end <segment end>`. Find the payoff line and the
strongest line (they may be the same).

## 2. Write 5 variants
Use five different formulas from the guide (bold claim, result first, question, pattern interrupt, POV, number,
controversy, cold open). For each variant write:
- **text**: on-screen hook, ≤ 7 words, in the speaker's language/script (Hinglish in Roman script);
- **first line**: the first spoken words the clip starts on (a real line from the transcript — you cannot add speech);
- **opening**: `normal` (start where the clip starts) or `cold open` (start on the payoff line, then cut back);
- **promise**: what the viewer will get, and the second where the clip pays it.

## 3. Judge
Score each 0-5 on: **stops the scroll** (would a stranger stop?), **clarity** (understood in one second, muted),
**truth** (the clip pays it off; no bait), **specificity** (numbers, names, stakes), **fit** (text and first line
point at the same payoff), **voice** (sounds like the speaker / the creator's style). Show the table; pick the
highest total; break ties by truth, then clarity.

## 4. Apply the winner
- Text: `rustler overlay add-text {{clip_id}} "<text>" --style hook-top --start 0 --end 2.5` (edit an existing
  hook with `rustler overlay edit {{clip_id}} <overlay_id> --text "<text>"`).
- First line: move the start onto it with `rustler edit trim {{clip_id}} --start <source time> --snap word`.
- Cold open: add the payoff range as its own segment (`rustler clip add-segment {{clip_id}} …`) and move it first
  (`rustler edit move-segment {{clip_id}} <segment_id> 0`); add a short flash or a hard cut back to the setup.
- Re-check: `rustler clip retention {{clip_id}}` → `first_frame_hook.present` true with two or more reasons,
  `speech_start_delay` < 0.5 s; preview `rustler render preview-frame {{clip_id}} --out-t 0.3`.

## 5. Optional A/B
To compare the top 2-3 texts: `rustler variant create {{clip_id}} --hooks '["<text 1>","<text 2>","<text 3>"]'`,
render them (`rustler render batch --clip-ids '["<variant ids>"]' --quality draft`), and
`rustler variant compare --clip-ids '["<ids>"]' --out-t 0.5` to look side by side. Record your choice and why in
`rustler clip score-set {{clip_id}} … --notes "<hook chosen and why>"`.
