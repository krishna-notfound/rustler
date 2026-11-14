---
name: review_clip
title: "Review a clip like a senior editor"
description: "Watch a clip (QC report, contact sheet, transcript of the cut), critique it against the quality checklist and hook rubric, then fix what is fixable."
arguments:
  - name: clip_id
    description: "The clip id (clp_…) to review."
    required: true
---

Review clip **{{clip_id}}** as a demanding senior short-form editor, then fix it. Be honest: the
goal is a clip that earns, not a clip that passes. Guides: `quality-checklist`, `finding-hooks`,
`cut-points`, `captions-and-style`, `layouts-and-framing`.

## 1. Watch
1. `rustler clip show {{clip_id}}` → segments, layout, captions, renders. If there is no render yet,
   `rustler render run {{clip_id}}`.
2. `rustler qc run {{clip_id}}` → read every check (face checks: `caption_over_face`, `face_visible_ratio`,
   `face_cut_off`; and `hook_text_present`, `duration_for_platform`). `rustler fix suggest {{clip_id}}` lists the fixes.
3. Open the QC contact sheet image. Also check `render preview-frame` at 0.5 s and at the last second.
4. Read exactly what is said: `rustler transcript get <source_id> --start <seg start> --end <seg end>`
   for each segment.
5. If you can watch video, `see preview <source_id> --start … --end …` for the source range.

If you cannot see images, say so and limit the review to QC data and the transcript.

## 2. Critique (write it down, be specific)

**Editorial (score each 0-20, total /100):**
- Hook: do the first 2 seconds force a stranger to stay? Quote the opening words.
- Stands alone: any missing context, unexplained pronouns, references to earlier parts?
- Payoff: what does the viewer get? Does the clip end on it, or trail off / cut the laugh?
- Emotion / energy: is the delivery charged? Is the reaction visible?
- Shareability: would someone send this?

**Craft (pass / problem with output time):**
- Framing: face fully visible while speaking, right person, no black bars, no duplicate webcam.
- Captions: position (not over face, not in UI zones), readability, spelling of names/numbers, sync.
- Pacing: dead air, too many jump cuts, slow middle.
- Audio: loudness check, music level vs speech, copyrighted music risk.
- Platform/brief: duration range, required tags/credits, forbidden content.

Verdict: **ship** (≥ 80 and no craft problems), **fix** (fixable problems), or **drop** (< 60 or the
moment itself doesn't work — no amount of styling saves a weak moment).

## 3. Fix
For each fixable problem, make one targeted edit (see the fix table in `quality-checklist`):
start/end units, `edit trim`, `edit remove-pauses`, `clip set-layout`, `crop track-face`,
`clip set-captions`. Preview with `render preview-frame`, then `render run` → `qc run` → contact
sheet again. Max 3 rounds per problem; `history undo {{clip_id}}` if a change makes it worse.

## 4. Report
- Before/after scores (part scores), verdict.
- Problems found → edits made → result.
- Remaining issues you could not fix and why.
- Suggested title (≤ 60 chars, about this moment), description, 3-5 hashtags.
Do not export or upload unless the user asked for export in this task.
