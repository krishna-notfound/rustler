---
id: director-retention
title: "Director's playbook: make them hold and watch"
summary: "Retention is the game: hook formulas for the first second, the 3-second rule, open loops and payoff, a visual change every 2-3 s, emotional arc, loopable endings, caption pacing, music energy, Hinglish hooks, what kills retention, and how to use clip.retention, clip.score_set and campaign.analyze."
order: 19
tags: [director, retention, hooks, pacing, loop, score, campaign, editorial]
---

# Director's playbook

The user's rule: *"it's the game of views — if we can't make someone hold and watch our reel, we won't go
viral."* You are the director. Rustler measures and advises (`clip.retention`, `campaign.analyze`); you
decide what holds a stranger and you make every edit earn its place. Nothing here calls an AI service.

## 1. The loop you run on every clip

1. `rustler campaign analyze` — the job's hard rules, payout model, duration window and checklist. Per-view
   pay: retention + volume. Per-clip pay: match the creator exactly, fewer polished clips.
2. Pick moments (`finding-hooks`), cut them (`cut-points`), frame them (`layouts-and-framing`).
3. `rustler clip retention <clip_id>` — read `retention_score`, `breakdown`, `first_frame_hook`,
   `dead_zones` (each with exact fix commands), `take_fixes` (restarts and repeats, action `bad_take`, each
   with the `edit remove-words` command that keeps the cleaner take; see `rustler edit find-bad-takes`) and
   `ending`.
4. Apply the fixes that make sense for *this* moment (not all of them), re-run `clip retention`.
5. `rustler render run <clip_id> --quality draft`, then `clip retention` again: with a current render it also
   measures frame differences and the real audio (`measured_on: render`).
6. `rustler clip score-set <clip_id> --hook H --flow F --value V --trend T --notes "why"` — your judgement,
   0-25 each, a quoted line for each part. QC, look at the contact sheet, export (the report carries both scores).

## 2. The first second (hook)

A scroller decides in about one second. `first_frame_hook.present` needs at least one of: text on screen
before 0.3 s, speech starting before 0.5 s, motion in the first second. Aim for two.

Hook formulas (pick the one the moment really supports; never promise what the clip does not pay off):

| Formula | Shape | Example |
|---|---|---|
| Bold claim | a strong, specific statement | "Your accent is not the problem." |
| Result first | show/say the outcome, then how | "He went from 3/10 to fluent in 90 days." |
| Question | a question the viewer wants answered | "Why can't you speak, even after 10 years?" |
| Pattern interrupt | unexpected sound/visual/line in frame 0 | cold open on a laugh or a slammed table |
| POV | "POV: …" puts the viewer in the scene | "POV: your boss hears you speak for the first time" |
| Number | a concrete count or figure | "3 words that make you sound smarter" |
| Controversy | a take people want to argue with | "Grammar doesn't matter. Here's what does." |
| Cold open | the strongest line first, then cut back to the setup | open on the punchline, then "here's what happened" |

**Hook styles.** `rustler overlay hook-styles` lists the viral presets; each animates in and can add a whoosh at its start. All five start at the same size (0.10 of the width); long text wraps to two lines and only then shrinks to stay inside the safe area:

| Style | Look and entry | Use for |
|---|---|---|
| `hook-big` | two bold uppercase lines, white with thick black outline, ~10% of the width; pops in | bold claim, result first, number |
| `hook-box-yellow` | black text on a yellow box; slides in from above | pattern interrupt, number, controversy |
| `hook-outline` | yellow text, thick black outline; typed out letter by letter | question, cold open lines |
| `hook-pov` | left-aligned dark box, yellow "POV:" (added if you leave it out); slides in from the left | POV formula |
| `hook-question` | dark box, the closing "?" in red (added if missing); zooms in | question formula |

```
rustler overlay add-text clp_X "He quit his job for this" --style hook-big --start 0 --end 2.5 --sfx asset:<whoosh_id>
rustler overlay add-text clp_X "you finally quit your job" --style hook-pov --anim typewriter   # --anim none|pop|zoom-in|slide|typewriter
rustler render preview-frame clp_X --out-t 1.0     # look: settled state; the first frame shows the start of the animation
```

`--sfx` takes a file, `asset:<id>` (`asset list --kind sfx`) or a URL; the whoosh lives and dies with the hook
(moving or removing the hook moves or removes it). Hook text ≤ 7 words, one idea, same payoff as the first spoken line.

`clips auto` lists 2-3 **suggested** hook lines per clip in `fixes_suggested` (action `hook_title`, from the
opening question and the strongest lines, ≤ 7 words, plain text, each with its `overlay add-text ... --style
hook-big` command) and, when the clip opens on a question, a `cold_open` suggestion (`edit cold-open <clip_id>
--start S --end E` on the strongest answer line). They come from word signals only (emphasis words, numbers,
"you", questions) and can quote a misheard word or a half-sentence: treat them as a starting point, rewrite
them, and never run one you have not read.
Keep the hook out of the caption band (captions sit near 70% of the height; hooks default to 14-17%).

Rules: the hook text (`overlay add-text --style hook-top`, ≤ 7 words) and the first spoken line must point at
the same payoff. Start on a word, never on breath or "so, um". A leading silence shows up as a `silence` dead
zone at 0 with a `trim_head` fix.

## 3. The 3-second rule and open loops

By second 3 the viewer must know *why to keep watching*: a question is open, a claim needs proof, a story has a
stake. That is an **open loop**. Every open loop needs its **payoff** before the end; the last third of the clip
pays the biggest loop. Never open a loop the clip cannot close (it reads as bait and kills replays).

## 4. Pacing: change the picture when the content earns it

`clip.retention` counts visual changes: cuts, jump cuts, layout changes, transitions, scene cuts, effects
(punch-in start and end), text pops, images/cutaways and frame-difference spikes on the render. A stretch over
6 s with none is a **static dead zone**. Target `pacing.per_10s` ≥ 1.5 (about one change every 6-7 s).

**Zooms are your decision, never automatic.** Rustler does not add punch-ins by itself: not at `clip create`,
not from the starter templates, not in `clips auto`. `clip retention` only *suggests* them. Watch the clip and
zoom only where a moment earns it (the punchline, a number, a reveal, a strong reaction). Constant zooming in and
out is the fastest way to make a clip look cheap; a talking head with good captions holds attention on its own.

Ways to break a static stretch, in order of subtlety:
- **Punch-in** on the moment that matters: `rustler effect add <clip_id> punch-in --start a --end b` (or
  `--words`). Hold it for the phrase (3-8 s, like cutting to a tighter second camera), keep it subtle
  (`--amount` 1.06-1.12), and use at most one every ~6-8 s. A 0.5 s zoom in/out "pump" every couple of seconds
  reads as nervous, not energetic. The `punch_in` suggestion of `clip retention` comes from the punch-in planner
  (stressed words, ≤ 1.5 changes per 10 s, 0.5 s from cuts and transitions, never on graphic / blur-fit segments
  or cutaways, never on top of an existing zoom, `--focus-x/--focus-y` on the face when `faces.json` exists);
  run it only if you agree after looking. When nothing fits, the suggestion is `punch_in_skipped`: use a text pop
  or a cutaway there, or leave it.
- **Text pop**: 1-3 words of the line that matters, `overlay add-text … --style plain`, ~1.2 s.
- **Jump cut**: remove a pause (`edit remove-pauses`) or a weak sentence; the reframe itself is a change.
- **Layout change**: split-screen to the listener's reaction, or wide → tight.
- **Cutaway / B-roll**: show what is talked about. Only real, licensed footage from the project.

- **Flash / whip at a cut**: `rustler transition set <clip_id> <segment_id> flash` (0.1 s white flash; `flashblack` for
  black) or `whip` (0.2 s horizontal smear into the next shot). They overlap the two shots, so the clip gets
  0.1-0.2 s shorter. Use on a topic change or a punchline, at most once every ~8 s, never on every cut.

**Cover frame.** After the final render: `rustler clip cover <clip_id> --auto`, then open the returned image. It picks a
sharp frame with a face in a good place and not on a cut or transition; the picture comes from the render, so
captions and the hook are in it. Not happy: `--out-t <seconds>`. `export clip` copies it as `<name>.cover.jpg`.

Do not stack changes on top of each other in the same half-second; rhythm beats density.

## 5. Silence, captions, energy

- **Silence ≥ 0.8 s** is a dead zone (`remove_pauses` fix). Dramatic pauses are allowed when the next line
  is the payoff — keep at most one.
- **Captions** are on for every clip with speech (most viewers watch muted). 2.0-4.5 words/s reads well;
  denser needs shorter caption chunks, sparser means the pace is too slow.
- **Energy**: the loudness curve should not sag. Seconds far below the median are a trimmed-take or a
  drifted-off speaker; cut them. Music should match the energy of the line (calm bed under a story, a lift into
  the payoff) and duck under the voice.

## 6. Emotional arc

Hook (tension) → context (as short as possible) → escalation → payoff → button (a final line or beat that lands
it). One clip, one emotion at its peak: surprise, laugh, anger, awe, relief. If you cannot name the emotion, the
moment is weak.

## 7. Endings that replay

`ending.kind`: `loopable` (last line connects back to the opening, so the replay flows), `cta`, `sentence_final`,
`hard_stop` (ends mid-sentence; 0 points). Prefer loopable: end on a line that the opening answers or repeats,
or cut so the last word leads into the first ("…and that's why" → "Your accent is not the problem."). A CTA
("follow for part 2") is fine when there is a part 2. Never end on a breath or a half word.

## 8. Hinglish hooks

Write the hook the way the speaker talks: Roman script, short, the punch word in English if that is how it is
said ("Ye galti 90% log karte hain", "Salary 3x kaise hui?"). Keep captions and hook in the same script
(`hinglish` guide). Numbers and money in digits.

## 9. What kills retention

- A slow first second: logo, black, "hi guys", breath, a wide shot of nothing.
- Context before the hook; inside references ("as I said earlier").
- 3+ seconds of the same locked shot with no line landing.
- Silence, filler words, repeated sentences, a trailing "yeah… so".
- Captions that are late, too dense, or cover the face.
- A hook that is not paid off; an ending that just stops.
- Music fighting the voice.

## 10. The numbers and what they are not

`retention_score` (0-100) = hook 25 + pacing 25 + visual_dead 15 + silence 10 + captions 10 + ending 10 + energy 5;
`formula` in the output spells out every rule. It measures *craft signals*, not whether the moment is good — a
perfectly paced boring clip still fails. That judgement is yours: `clip score-set` (hook / flow / value / trend,
0-25 each, OpusClip-style factors). Ship clips where both are strong; `retention_score` ≥ 70 and your total ≥ 70
is a good bar. Known blind spots: graphics that build up inside a still shot and slow camera moves are not
counted as visual changes; Rustler cannot judge humour or meaning.
