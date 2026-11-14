---
id: editing-toolkit
title: "Editing toolkit: cuts, hook titles, transitions, effects, progress bar"
summary: "Every edit command with when to use it and example sequences: refine/trim/split/remove-pauses/remove-words, history, overlay hook titles and lower thirds, transitions, effects, the progress bar, and the polish order."
order: 15
tags: [edit, overlay, hook, transition, effect, progress-bar, history, polish]
---

# Editing toolkit

All of these edit the clip (`clips/<id>/clip.json` and `extras.json`), each saves a **new revision**
and nothing is rendered until `render run`. `history undo clp_X` reverts the last change;
`history list` shows revisions; `history restore clp_X <revision>` brings one back (as a new revision).
Pass `--dry-run` to see what would change, `--expect-revision N` to guard against concurrent edits.

Two kinds of time: cut commands (`edit trim --start/--end`) use **source** seconds; everything on the
output timeline (`edit split <out_t>`, `overlay --start/--end`, `effect --start/--end`, `audio add-sfx
--at`) uses **output (clip) time**, the numbers `clip show` prints in `timeline`.

## 1. Cut commands

| Command | Use it to | Notes |
|---|---|---|
| `edit set-range clp_X --start-unit 41 --end-unit 47 --expect-text "…"` | re-cut a segment by unit ids | same checks and padding as `clip create`; `--segment-id` picks the segment |
| `edit refine clp_X` | repair the cut edges automatically | skips weak openers (so, and, um, toh, matlab…), ends on a sentence-final word, moves both ends into silence troughs within `--max-shift` (default 1.5 s). Refines the clip's outer ends unless `--segment-id` is given. Try `--dry-run` first |
| `edit trim clp_X --start-delta 0.3` / `--end-delta -0.2` | nudge an edge | `--snap word` snaps to the nearest word boundary; absolute `--start/--end` in source seconds |
| `edit split clp_X 8.0` | split at an output time | creates two segments (same layout); needed before per-segment layouts or transitions |
| `edit delete-segment clp_X seg_B` | drop a segment | the last one cannot be deleted |
| `edit move-segment clp_X seg_B 0` | reorder (0-based index) | |
| `edit remove-pauses clp_X --min-gap 0.5 --keep 0.25` | cut dead air | pauses ≥ `min-gap` seconds are cut down to `keep` seconds (defaults 0.5 / 0.12) |
| `edit remove-words clp_X` | cut fillers | um, uh, umm, uhh, hmm, erm by default; `--words '["basically"]'` or `--word-indices '[...]'` for others |
| `clip add-segment clp_X --source-id src_X --start-unit … --end-unit …` | build a multi-part clip | hook line first, payoff after (see `cut-points`) |

`remove-pauses` and `remove-words` shift everything after the cut earlier: captions follow
automatically; look at `overlay list`, `effect list` and `audio list` afterwards (word-anchored sound
effects follow their word; time-anchored items do not).

## 2. Hook title and text overlays

```
rustler overlay add-text clp_X "He quit his job for this" --style hook-top --start 0 --end 3
rustler overlay add-text clp_X "Dr. Jane Doe, MIT" --style lower-third --start 2 --end 6
rustler overlay add-text clp_X "@creator" --style plain --position-y 0.9 --box false
rustler overlay edit clp_X ovl_1 --text "Nobody saw this coming" --end 2.5
rustler overlay list clp_X          # text, wrapped lines, pixel position
rustler overlay remove clp_X ovl_1
```

- Styles: `hook-top` (default; large, top, y 0.12), `lower-third` (y 0.80), `plain` (centre, y 0.50). Defaults for
  size, position and the dark box come from the style; override with `--position-y`, `--font-size-factor`, `--box`.
- Text ≤ 120 characters, wrapped to at most 2 lines. **Hooks read best at ≤ 7 words** and should state
  the payoff or tension, not describe the video ("He quit his job for this", not "Video about quitting").
- Overlays are burned in **above the captions** in the same render. Keep the hook out of the platform UI
  zones (top 8 %): the `hook-top` default (centre at y 0.12) sits just below them. Hook on screen for 2-3 s starting at 0; QC's
  `hook_text_present` expects text or captions in the first 3 s.
- Required credit or watermark from a brief goes here too (`lower-third` or `plain`), see `briefs-and-batches`.
- Hinglish / Devanagari text: see `hinglish` (the font must have the glyphs; QC's contact sheet shows □ when it does not).

## 3. Transitions between segments

A transition belongs to the **cut between two segments**: `transition set clp_X <after_segment_id> <kind>`
joins that segment into the next one.

```
rustler clip show clp_X                                  # get the seg_ ids from `timeline`
rustler transition set clp_X seg_A dissolve              # 0.3 s default
rustler transition set clp_X seg_A fade --duration 0.5   # dip to black
rustler transition set clp_X seg_B cut                   # explicit hard cut again
rustler transition list clp_X                            # windows + edit vs rendered duration
rustler transition clear clp_X                           # all hard cuts (or --after-segment-id seg_A)
```

Kinds: `cut`, `fade` (dip to black), `dissolve` (cross-blend), `wipeleft`, `wiperight`, `slideup`,
`zoomin`. The duration is the overlap in seconds (default 0.3, rounded to whole frames, max 2). The
overlap is **frame-exact and shortens the clip** by that amount (audio cross-fades too); captions and
overlay times are remapped for you, and QC and `brief check` use the shortened duration.

Taste: talking-head jump cuts need **no** transition (a hard cut is the genre); use `dissolve` 0.2-0.3 s
only when the topic or place changes, `slideup`/`zoomin` for a deliberate "next" beat in listicles, and
never more than one kind per clip.

## 4. Camera effects

```
rustler effect add clp_X punch-in --start 4.2 --end 6.0          # instant zoom (default 1.15x)
rustler effect add clp_X zoom-slow --start 0 --end 5 --amount 1.2   # slow push 1.0 -> 1.2
rustler effect add clp_X shake --start 7.0 --end 7.4 --amount 0.02  # impact
rustler effect add clp_X flash --start 3.0 --end 3.25               # white flash on a beat
rustler effect list clp_X          # effects with rendered windows, and the progress bar
rustler effect remove clp_X fx_1
```

Effects are applied to the composed frame **under the captions and overlays**. `amount`: zoom factor
1.01-2 for punch-in / zoom-slow; shake amplitude 0.002-0.08 of the frame; none for flash. Place a
`punch-in` on the emphasis word (times from `transcript get`, converted to output time with `clip show`),
a `flash` or `shake` on a joke/impact, a `zoom-slow` under a long calm statement. Two to three effects
in 30 seconds is plenty; effects on every sentence read as noise. On a split layout the zoom covers
both panes: prefer an `overlay` or sound effect there.

## 5. Progress bar

```
rustler overlay progress-bar clp_X --enabled true                       # top, 8 px, #FFE000
rustler overlay progress-bar clp_X --enabled true --color "#FFFFFF" --position bottom --height 6
rustler overlay progress-bar clp_X --enabled false
```

A thin bar that fills the width over the clip, on top of everything. It helps retention on 30-60 s
explainers; skip it on very short clips. At `bottom` it sits in the platform's lower UI zone, so
`top` is the default.

## 6. Example sequences

**Talking-head polish (≈ 40 s):**
```
rustler edit refine clp_X
rustler edit remove-pauses clp_X --keep 0.25
rustler edit remove-words clp_X
rustler overlay add-text clp_X "Nobody tells you this" --start 0 --end 2.5
rustler effect add clp_X punch-in --start 11.4 --end 12.0
rustler overlay progress-bar clp_X --enabled true
rustler render preview-frame clp_X --out-t 1.0
```

**Two-part clip (setup, then payoff):**
```
rustler clip create --name two-part --source-id src_X --start-unit 12 --end-unit 14 --expect-text "…"
rustler clip add-segment clp_X --source-id src_X --start-unit 88 --end-unit 93 --expect-text "…"
rustler clip show clp_X                              # seg ids
rustler transition set clp_X seg_A dissolve --duration 0.25
rustler overlay add-text clp_X "Later that night…" --style plain --start 6.5 --end 8.5     # times AFTER the transition shrinks the clip
```

**Reaction beat (sound + zoom + flash):**
```
rustler audio add-sfx clp_X asset:sfx-... --at-word '{"source_id":"src_X","word_index":412}' --volume-db -8
rustler effect add clp_X punch-in --start 9.1 --end 9.6
rustler effect add clp_X flash --start 9.1 --end 9.25
```

After any sequence: `render preview-frame` at the hook (1.0 s), at an effect time and at the last
second; then `render run`, `qc run`, contact sheet. If you do not like a step, `history undo clp_X`
(repeat to go further back) or remove just that item (`overlay remove`, `effect remove`,
`transition clear`).

## 7. What not to do

- No effect or transition in the first 0.5 s: the hook frame must be clean.
- Do not hide a weak cut with a transition; fix the cut (`edit refine`, `cut-points`).
- Do not stack hook title + big caption + lower third + progress bar at once: pick what carries the
  clip. If `caption_over_face` or `caption_safe_zone` warns, move the caption (`caption set --position-y`).
- Time-anchored items (overlays, effects, `--at` sound effects) do not follow later cuts: add them last.
