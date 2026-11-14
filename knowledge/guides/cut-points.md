---
id: cut-points
title: "Cut points: choosing start and end units"
summary: "How to pick start/end transcript units, padding, sentence boundaries, weak openers to skip (English + Hinglish), ending on the payoff, pause/filler removal and multi-segment montages."
order: 5
tags: [editing, cuts, units, pacing, montage]
---

# Cut points

The moment is chosen (`finding-hooks`). Now cut it so the first word hits and the last word lands.

## 1. Units, not seconds

`transcript get` prints numbered **units** (sentences, or clauses for long sentences):

```
#211 [00:30:31.9–00:30:34.0] And honestly that was the year.
#212 [00:30:34.4–00:30:37.1] Nobody tells you this about your first business.
#213 [00:30:37.3–00:30:41.8] You will lose money on the thing you're best at.
...
#219 [00:31:02.5–00:31:05.0] That mistake cost me forty thousand dollars.
```

Create clips with `--start-unit 212 --end-unit 219 --expect-text "nobody tells you"`. Rustler:

- starts 0.08 s before the first word of #212 (but never before the previous word ends),
- ends 0.15 s after the last word of #219 (but never after the next word starts),
- checks your `--expect-text` against the real opening words (fuzzy match) and refuses on mismatch.

Units have a `boundary_strength`: 1.0 = punctuation sentence end, 0.8 = pause-based break, 0.5 =
comma clause split. Prefer starting and ending at units with strength ≥ 0.8.

## 2. Choosing the start

1. Find the **hook line** — the first sentence that passes the 2-second test.
2. Check whether the hook needs one sentence of setup to make sense. If yes, start one unit earlier
   **only if** that unit is itself short (< 4 s) and not filler.
3. If the hook unit begins with a weak opener, still start on that unit (units can't split words),
   then trim the opener: `edit trim clp_X --segment-id seg_Y --start-delta 0.4` (if available —
   check `commands describe edit.trim` for the sign convention), using word times from `transcript get`.
4. Never start on an answer whose question is off-clip. Either include the question (if short and
   punchy) or pick another start.
5. A pronoun start ("He said…", "That's when…") needs the referent inside the first 3 seconds or it
   fails the stand-alone test.

### Weak openers to skip

| English | Hinglish |
|---|---|
| so, and, but, um, uh, like, yeah, okay, right, well, anyway, basically, actually, I mean, you know, so yeah, and then, the thing is | toh, matlab, haan, na, accha, dekho, yaar, bas, aur, phir, woh, toh basically, matlab ki, ki, arre, hmm |

Rule: if removing the first 1-3 words leaves a sentence that still makes sense, they are an opener
and should go.

## 3. Choosing the end

- **End on the payoff**, ideally the strongest line, not the qualification after it. "That mistake
  cost me forty thousand dollars." is the end; "…but you know, everyone's different" is not.
- **Let the reaction breathe.** For jokes, include the laugh: keep 0.5-1.5 s after the punchline (the
  laughter usually shows as a loudness peak right after the last word). Extend with
  `edit trim --end-delta` if the unit end clips the laugh.
- For stories, end on the resolution line or the reaction to it, not on the next topic.
- Avoid ending on a question you don't answer, unless the clip is deliberately a cliffhanger part 1.
- If the clip is too long, cut from the middle (remove a tangent; see §5), never by chopping off the
  payoff. If it's still too long, move the start later, not the end earlier.

## 4. Padding and silence

- Lead-in: 0.08-0.15 s before the first word. More looks like dead air.
- Tail: 0.15-0.40 s after the last word; up to 1.5 s if a reaction/laugh follows.
- If the source has a breath or room noise at a cut, a slightly later start (+0.05 s) is better than
  a mouth click.

## 5. Removing pauses and fillers inside a clip

Dead air kills retention. Tighten after creating the clip:

```
rustler edit remove-pauses clp_X --min-gap 0.5        # (if available) splits segments at word gaps ≥ 0.5 s
rustler edit remove-words clp_X                       # default list: um uh umm uhh hmm erm
rustler clip show clp_X                               # check the new segments and total duration
```

Guidance (from production pacing rules):

| Content | Max natural pause to keep |
|---|---|
| Talking head / podcast | 0.5-0.7 s |
| Screen share / tutorial | up to 2 s (viewer is reading) |
| Slides, code, text on screen | do not cut pauses |
| After a punchline / before a reveal | keep 0.3-0.8 s (comedic timing) |

- Keep ~0.25 s of each removed pause so speech doesn't sound glued.
- Don't create pieces shorter than 0.4 s; don't make cuts shorter than 0.2 s.
- Every cut is a visible jump on a static camera. 2-5 jump cuts per 30 s are fine on a talking head;
  more feels choppy unless the speaker moves a lot.
- `history undo clp_X` reverts the last edit if it sounds wrong.

## 6. Multi-segment clips and montages

Use when the best version of a moment is spread out: a question at 12:10 and its answer at 14:40, or
"three times he said X".

```
rustler clip create --name "three-rules" --source-id src_A --start-unit 140 --end-unit 141 --expect-text "there are three rules"
rustler clip add-segment clp_X --source-id src_A --start-unit 152 --end-unit 155 --expect-text "rule one is"
rustler clip add-segment clp_X --source-id src_A --start-unit 171 --end-unit 173 --expect-text "rule two"
```

Rules:

- Each segment must stand on a sentence boundary at both ends.
- Never join segments in a way that changes what the speaker meant (no splicing "I would never" +
  "do drugs"). Faithfulness beats punchiness.
- Segments can come from different sources (e.g., two section sources of one stream).
- Each segment has its own layout; set it with `clip set-layout --segment-id`.
- Keep segment order chronological unless a reordering is clearly honest (e.g. putting the
  punchline-teaser first, "cold open", then the full story).

## 7. Quick checks before rendering

- `clip show clp_X` → total duration within the brief range?
- Read the clip's text start to finish: does the first sentence hook, does the last one land?
- `see tiles <source> --start <seg start> --end <seg end>`: speaker visible at the start?
- `render preview-frame clp_X --out-t 0.5` and `--out-t <last second>`: face and captions fine?

## 8. Worked example (illustrative)

Transcript:

```
#88 [12:01.0–12:03.2] So yeah, um, basically what happened was,
#89 [12:03.4–12:07.9] my co-founder emptied the company account the night before launch.
#90 [12:08.1–12:11.0] Every rupee. Gone.
#91 [12:11.6–12:16.4] And the crazy part is, he texted me "best of luck" the next morning.
#92 [12:17.0–12:19.0] [laughter] I still have that text.
#93 [12:19.5–12:24.0] Anyway, so the second thing about launching is pricing...
```

Bad cut: #88-#93 (weak opener, ends on a new topic). Good cut: start #89
(`--expect-text "my co-founder emptied"`), end #92 to keep the laugh, ~16 s. Hook text: "My
co-founder emptied our account before launch".
