---
name: direct_campaign
title: "Direct a campaign: brief to strategy to moments to hooks to batch"
description: "Act as the director for a clipping job: read the campaign/brief, set the strategy (payout model, clip count, duration window), pick diverse moments, write hooks, build one look, edit for retention, score, QC and ship the batch."
arguments:
  - name: source_id
    description: "Source to clip (src_…), already imported and analysed (`rustler analyze quick`)."
    required: true
  - name: campaign_id
    description: "Campaign id or name from <project>/campaigns/ (optional; default the project's brief.json)."
    required: false
---

You are the **director** for this job. Rustler measures and advises; you decide. The goal is clips strangers
hold and watch to the end. Guides: `director-retention` (read first), `finding-hooks`, `cut-points`,
`layouts-and-framing`, `captions-and-style`, `quality-checklist`, `briefs-and-batches`.

## 1. Brief → strategy
`rustler campaign analyze --campaign-id {{campaign_id}}` (leave out `--campaign-id` when empty to read the brief).
Write down, before touching the source:
- hard rules (`hard_rules.must`, `hard_rules.must_not`), `required` mentions/hashtags/credit, `duration_window`;
- `strategy.payout_model`: **per_view** → optimise retention and volume; **per_clip** → match the creator's
  style exactly, fewer polished clips;
- `strategy.recommended_clips` for {{source_id}} — a ceiling, not a quota;
- the `checklist` every clip must pass.
No brief at all? Ask the user for the rules if money depends on them; otherwise plan per_view and set what you
know with `rustler brief set …`.

## 2. Moments
`rustler highlights candidates {{source_id}} --count 15` (use the brief's min/max length), then read each window
(`rustler transcript read-window {{source_id}} …`). Score each candidate with the `finding-hooks` rubric and keep
only strong, **diverse** moments (no two making the same point). For each kept moment note: the hook line, the
open loop, the payoff, the emotion.

## 3. Hooks
For every moment write the hook: on-screen text (≤ 7 words) + first spoken line + visual opening (normal start,
or cold open on the payoff). For the top moments run the `hook_lab` prompt to compare 5 variants. Pick the
hook-title style from `rustler overlay hook-styles` (`hook-big`, `hook-box-yellow`, `hook-outline`, `hook-pov`,
`hook-question`); a cold open is `rustler edit cold-open <clip_id> --start-unit A --end-unit B`.

## 4. One look (template)
Decide once for the whole batch: caption style, layout per shot type, hook-title style, punch-in strength, music
policy. Per-clip pay: copy the creator's own shorts (`learn-style-from-videos`, prompt `learn_style_from_folder`).
Make the look a **template** and apply the same one to every clip: start from `rustler template list` (starters
`podcast-viral`, `streamer-split`), or style one clip by hand and `rustler template create <name> --from-clip <clip_id>`,
then `rustler template apply <name> --clip-ids <ids> --dry-run` and without `--dry-run` (the `apply_template_batch`
prompt walks through it; guide `templates-and-brand`). Hook text stays per clip.

## 5. Edit each clip for retention
1. `rustler clip create …` on sentence boundaries (smart defaults pick the layout per shot and trim fillers and
   pauses; read `smart.decisions`, or pass `--raw`; add `--template <name>` to apply the look at once);
   `rustler edit refine <clip_id>`. Cold open on the payoff when the hook is not the first line:
   `rustler edit cold-open <clip_id> --start-unit A --end-unit B`. Bigger rebuilds (reorder, B-roll, speed,
   freeze frames): `rustler timeline show <clip_id>` then `rustler timeline apply` (`editing-toolkit`).
2. Layout and captions (the template, `rustler layout plan <clip_id>` to check the framing); the hook title:
   `rustler overlay add-text <clip_id> "<hook>" --style hook-big --start 0 --end 3` (a style from `overlay hook-styles`;
   `--sfx` adds a whoosh). Pace at cuts: `rustler transition set <clip_id> <seg_id> flash|whip`.
3. Credit/watermark from the brief: `rustler overlay add-text <clip_id> "<credit>" --style lower-third`.
4. `rustler clip retention <clip_id>`: fix the dead zones that matter, choosing the fix yourself after looking
   (jump cut, text pop, layout change; a held, subtle punch-in only where the moment earns it, never by default); make the ending sentence-final, loopable if you can.
5. `rustler render run <clip_id> --quality draft`, `rustler clip retention <clip_id>` again (now measured on the
   render), look at `rustler render preview-frame <clip_id> --out-t 0.3`.
6. `rustler clip score-set <clip_id> --hook H --flow F --value V --trend T --notes "<quote per part>"`.
7. Campaign jobs: attach the clip and write its posting copy (`rustler clip meta-context <clip_id>`, then
   `rustler clip meta-set <clip_id> --campaign-id {{campaign_id}} --input-json @meta.json`) and record every agent rule
   you checked (`rustler campaign mark <clip_id> <rule_id> --pass true --reason "what you saw"`).
8. `rustler brief check <clip_id>` (when a brief exists), `rustler qc run <clip_id>` and look at the contact sheet.

Drop a clip whose `retention_score` stays under 70 after fixes or whose own score is under 70 — a weak clip
costs reach for the strong ones.

## 6. Ship
`rustler render batch --all true --quality final --only-missing true`; then pick each clip's cover from its final
render (`rustler clip cover <clip_id> --auto`, look at the image); then `rustler export batch …` (or
`rustler export clip <clip_id>` one by one: it copies the cover and writes the post JSON). Report to the user: per clip the name, duration, both scores, the
hook, what you fixed, and anything from the checklist you could not verify. Never publish or upload.
