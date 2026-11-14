---
id: finding-hooks
title: "Finding hooks: picking moments that hold strangers"
summary: "What makes a short-form moment perform, a harsh 0-100 scoring rubric, signals to cross-check, clip length and count rules, diversity and red flags."
order: 4
tags: [selection, hooks, scoring, virality, editorial]
---

# Finding hooks

Your viewer is a stranger scrolling at speed, sound on or off, who has never seen the source video.
They give a clip about **two seconds**. Everything below serves that fact.

**Most of any long video is not clippable.** A 60-minute podcast usually holds 3-8 moments worth
posting; a 3-hour stream maybe 5-15. Saying "this section has nothing" is a correct answer.

## 1. The five tests (in priority order)

1. **2-second test.** Would the first 2 seconds make a cold viewer stay? The opening line must
   create a question, make a bold or contrarian claim, state a specific number, or drop the viewer
   into tension. It must not open on "so", "yeah", "um", "and then", a topic change, or the end of an
   answer to a question nobody heard.
2. **Stands alone.** No missing context: no unexplained "he", "that thing", "like I said", "going
   back to". A viewer must understand it with zero knowledge of the show, the guest or the stream.
3. **Payoff.** The clip delivers something: a punchline, a reveal, a lesson, a number, a reaction, a
   resolved story. If you cannot say in one sentence what the viewer walks away with, it scores under
   50.
4. **Emotion / energy.** Laughter, anger, shock, awkwardness, vulnerability, hype. Flat delivery of
   good content loses to charged delivery of ordinary content.
5. **Shareability.** Would someone send it to a friend, comment, argue, or save it? Quotable lines,
   relatable pain ("this is literally me"), hot takes and useful "value bombs" travel.

## 2. Moment types that work

| Type | Shape | Illustrative opening line |
|---|---|---|
| Bold / contrarian claim | claim → reason → proof | "College is the worst investment you can make right now." |
| Number shock | number → context → consequence | "I lost 4 lakh rupees in eleven minutes." |
| Story with a turn | setup → tension → twist | "My landlord called me at 3 a.m. and said, don't come home." |
| Open loop | promise → delay → delivery | "There's one question that ends every bad interview." |
| Conflict / disagreement | position A → pushback → clash | "No, that's completely wrong, and here's why." |
| Humour | setup → punchline → laughter | (a joke that lands with real laughter in the audio) |
| Value bomb | problem → exact steps → result | "Here's the exact message I send to get a reply." |
| Reaction / fail / clutch (streams) | build-up → event → reaction | gameplay moment + streamer's scream on the webcam |
| Vulnerable confession | admission → why → what changed | "I didn't talk to my dad for six years." |

## 3. Good vs bad openings (illustrative)

| Bad (score low) | Why | Better start for the same moment |
|---|---|---|
| "So yeah, anyway, going back to what he said…" | filler + needs context | start at "He told me to quit my job." |
| "And that's why it matters." | the end of an answer | rewind to the question or the claim |
| "Um, I think, like, there's this thing where…" | slow, vague | start at the concrete claim 4 s later |
| "Welcome back to the show, today we…" | housekeeping | never clip intros unless the intro is the hook |
| "That was crazy." | pronoun, no setup | start where the crazy thing begins |

"About this moment, not the video": the hook and title name the concrete thing inside the clip
("He paid ₹2 lakh for a fake visa"), not the episode ("Best of episode 12").

## 4. Scoring rubric (0-100, be harsh)

Score each candidate on five parts, 0-20 each, and add them. Write the part scores down; they make
reranking honest.

| Part | 0-5 | 6-13 | 14-20 |
|---|---|---|---|
| Hook (first 2-3 s) | filler, mid-sentence, dead air | decent line, slow build | claim/number/question/tension in the first sentence |
| Stands alone | needs earlier context | small gaps | fully self-contained |
| Payoff | none / trails off | soft lesson | clear punchline, reveal, number, resolution |
| Emotion / energy | flat | some energy | laughter, anger, shock, real stakes |
| Shareability | nobody would send it | niche interest | quotable, relatable, debate-starting |

Bands:

- **85-100**: genuinely strong. Make it first, try variants.
- **70-84**: solid. Make it if the count allows.
- **50-69**: watchable but forgettable. Only if the brief demands volume, and say so in the report.
- **< 50**: do not make it.

Calibration: if more than a third of your candidates score ≥ 85, you are too generous. Re-score
against your single best moment.

## 5. Cross-check with signals (don't trust words alone)

Rustler gives you cheap, deterministic signals. A great transcript line with no matching signal is
fine; a signal with no good line is a lead to read, not a clip.

| Signal | Where | What it suggests |
|---|---|---|
| Loudness peaks | `analyze audio` → `peaks` (≥ median + 6 LU) | shouting, laughing, hype, crowd |
| Long silence before a line | `silences` | dramatic pause, or dead air to cut |
| Scene cuts dense in a range | `analyze scenes` | edited source: action or montage |
| Heatmap spike | `source info` | viewers rewatched; payoff is near the spike |
| Questions / exclamations | transcript text `?` `!` | Q&A hooks, reactions |
| Faces in tiles | `see tiles --around "<line>"` | is the reaction visible? is the speaker on screen? |
| Laughter / `[Laughter]` tags | auto captions or audio peak after a line | joke landed |

Always look at tiles for your top candidates. Check that the speaker is on screen when the hook line
plays, that the reaction is visible, and that nothing visually confusing (slides, B-roll, black) sits
in the first 2 seconds.

## 6. Length guidance

Default when the brief is silent:

| Platform | Sweet spot | Hard limits to respect |
|---|---|---|
| YouTube Shorts | 20-45 s | ≤ 60 s is safest (longer allowed, see `platform-specs`) |
| Instagram Reels | 15-45 s | ≤ 90 s for best reach |
| TikTok | 25-60 s | > 60 s only if the brief or a monetisation rule needs it |

- **A tight 25-second clip beats a padded 60-second one.** Never stretch to reach a length.
- If the brief sets a minimum (common in view campaigns, e.g. "≥ 30 s"), choose moments that are
  naturally that long; don't pad with filler.
- Stories may need 45-75 s. Pure reactions can be 8-15 s.

## 7. How many clips per source

- Brief count is a **maximum** unless it says "exactly".
- Rough expectation for dense talk: 1 posting-quality clip per 8-12 min of content; streams vary
  wildly (long dull stretches, then bursts).
- Without a brief: aim for `min(8, max(2, minutes / 10))` candidates ≥ 70, make the best ones first.

## 8. Diversity

- No two clips may make the same point, tell the same story or land the same joke.
- Overlap between two clips ≤ 25 % of the shorter one; different angles on one topic are separate
  clips only if each stands alone.
- Spread across the timeline and across tones (funny, serious, useful) when quality allows.

## 9. Red flags (fix the cut or drop the moment)

- Needs prior context ("as I mentioned", "that guy", inside jokes from earlier).
- Slow start: the real hook is 5+ seconds in → move the start to the hook (see `cut-points`).
- Ends mid-thought, on a trailing qualification ("…but yeah, it depends"), or before the laugh.
- Speaker off screen during the hook (slide, B-roll, someone else's face) on a talking clip.
- Sponsor reads, ads, self-promotion, "like and subscribe", housekeeping.
- Content the brief forbids, or that is hateful, sexual, dangerous, or defames a real person by
  stripping context. A spicy line that changes meaning out of context is not a hook; it is a risk.
- Copyrighted music playing loudly in the source (see `audio`).

## 10. Write it down

For each chosen clip keep: units, verbatim opening words (your `--expect-text`), one-sentence payoff,
part scores and total, layout plan, a ≤ 10-word hook text, a title idea. This becomes the export
report and lets another agent continue your work.
