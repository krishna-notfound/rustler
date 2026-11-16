---
id: editing-toolkit
title: "Editing toolkit: timeline document, cuts, cold opens, merges, speed, freezes, hook titles, transitions, effects, layers"
summary: "Every edit command with when to use it and example sequences: the whole-clip timeline document (timeline show/schema/apply), refine/trim/split/remove-pauses/remove-words, insert-segment, cold-open, clip merge/duplicate, speed, freeze frames, history, overlay hook titles and lower thirds, transitions, effects, the progress bar, image layers, masks, cutaways (B-roll), keyword captions, and the polish order."
order: 15
tags: [edit, timeline, cold-open, merge, speed, freeze, overlay, hook, transition, effect, progress-bar, history, polish, image, mask, cutaway, b-roll, keyword]
---

# Editing toolkit

All of these edit the clip (`clips/<id>/clip.json` and `extras.json`), each saves a **new revision**
and nothing is rendered until `render run`. `history undo clp_X` reverts the last change;
`history list` shows revisions; `history restore clp_X <revision>` brings one back (as a new revision).
Pass `--dry-run` to see what would change, `--expect-revision N` (v1 cut commands) or `--base-revision N`
(timeline, layer and timeline-edit commands) to guard against concurrent edits.

Two kinds of time: cut commands (`edit trim --start/--end`) use **source** seconds; everything on the
output timeline (`edit split <out_t>`, `overlay --start/--end`, `effect --start/--end`, `audio add-sfx
--at`) uses **output (clip) time**, the numbers `clip show` prints in `timeline`.

## 0. The whole clip as one document (timeline v2)

`rustler timeline show clp_X` prints the clip as one `rustler.timeline@2` document: `tracks.video`
(segments with `src_in`/`src_out`, layout, `crop_path`, `speed`), `freezes`, `cutaways`, `images`,
`masks`, `text` (hook titles), `captions` (settings + `word_overrides` + `speaker_colors`), `audio`
(loudness, denoise, volumes, `music`, `sfx`), `effects`, `transitions`, `progress_bar`, `meta`
(score, retention, post copy) and `tracks.extra` (keys this build does not know, kept verbatim).
`resolved` lists every item with its edit time and its time in the rendered file (word anchors
resolved; `problem` says why an item will not render). `rustler timeline schema` prints the JSON
Schema and the rules.

Change anything in one step with `timeline apply` and the `revision` that `show` printed:

```
rustler timeline show clp_X                                   # note "revision": 7
rustler timeline apply clp_X --base-revision 7 --input-json @patch.json --dry-run
rustler timeline apply clp_X --base-revision 7 --input-json @patch.json
```
`patch.json` (RFC 6902 JSON-Patch; `-` appends; or `"document": {...}` for a whole document):
```json
{"patch": [
  {"op": "replace", "path": "/tracks/text/0/start", "value": 0.4},
  {"op": "replace", "path": "/tracks/video/2/speed", "value": 1.2},
  {"op": "add", "path": "/tracks/effects/-", "value": {"id": "fx_7", "kind": "punch-in", "start": 9.1, "end": 9.6}}
]}
```
Everything is validated before anything is written (rects 0..1, times inside the clip, files and
sources exist, new word anchors resolve, transition overlaps fit) and every error names its path
(`/tracks/images/0/rect: ...`). A stale `base_revision` (someone edited meanwhile) fails with
`revision_conflict` and writes nothing: show again, re-apply. The result lists the `changes`, the new
document and its resolved times. `history undo` reverts a whole apply, every layer included.

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

| `edit insert-segment clp_X --source-id src_X --start-unit 40 --end-unit 42 --at-index 1` | put a range anywhere | or `--at-out-t 12.5` (splits the segment there); layers after it move with their content |
| `edit cold-open clp_X --start-unit 57 --end-unit 57` | open on the hook line | the range goes first (cut out of where it was if the clip has it) + a white flash at the cut out of the hook (`--flash false` to skip); every layer stays on its words, except a hook-top title that opened the clip, which keeps opening it |
| `clip merge clp_A clp_B --name a-b --transition dissolve --transition-duration 0.2` | join clips (hook clip + main clip, part 1 + part 2) | new clip with ALL layers: times shift by each clip's offset, word anchors keep their words; the first clip's captions, music and progress bar win |
| `clip duplicate clp_X --name clp-x-v2` | try another hook or cut safely | copies clip + every layer |
| `edit speed clp_X --segment-id seg_B --factor 1.2` | tighten slow talk (0.5-2.0; 1 resets) | pitch kept; captions and layers follow; the rendered clip gets shorter |
| `edit freeze clp_X --out-t 6.2 --duration 0.8` | hold a reaction frame (silent) | the rendered clip gets longer by the hold; an item starting at `out_t` shows during the hold |

Times stay on the **edit timeline** (sum of the segments' source lengths) even with speeds and
freezes: `timeline show` -> `resolved` and `audio list` print where each item lands in the rendered
file; `render run`, QC `duration` and `clip retention` use the rendered length.

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
rustler effect add clp_X punch-in --words amazing,look          # on every spoken 'amazing' / 'look'
rustler effect list clp_X          # effects with rendered windows, and the progress bar
rustler effect remove clp_X fx_1
```

Effects are applied to the composed frame **under the captions and overlays**. `amount`: zoom factor
1.01-2 for punch-in / zoom-slow; shake amplitude 0.002-0.08 of the frame; none for flash. With `--words` (comma list; case and punctuation ignored) no time conversion is needed: every occurrence of each word inside the clip gets an effect from 0.05 s before the word to 0.25 s after it, a word that is not in the clip comes back in `warnings`, and `--source-id` limits the search to one source. Otherwise place a
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

## 6. Layers: images, masks, cutaways, keyword captions

Each layer kind has its own commands and its own `extras.json` key (`images`, `masks`, `cutaways`,
`caption_word_overrides`). Rects are normalized to the 1080x1920 canvas (`{"x","y","w","h"}`, all
inside 0..1, w and h > 0.02). Times are edit-timeline seconds like overlays; instead you can hang an
item on transcript words with `--start-word` / `--end-word` `{"source_id":"src_X","word_i":42}`
(indices from `transcript get`). Word-anchored items follow re-cuts; if a re-cut removes the word the
item is skipped with a warning (`list` shows a `problem`), never shown at a wrong time. Every write
takes `--base-revision` (stale -> `revision_conflict`) and `--dry-run`. On PowerShell 5.1 pass JSON
values with `--input-json @file.json`.

```
rustler image add clp_X --file logo.png --rect '{"x":0.74,"y":0.03,"w":0.22,"h":0.08}' --anim-in fade
rustler image add clp_X --asset ast_fire --rect '{"x":0.4,"y":0.55,"w":0.2,"h":0.12}' --start-word '{"source_id":"src_X","word_i":42}' --anim-in pop
rustler image edit clp_X img_1 --opacity 0.6 --z 2 --above-captions true
rustler image list clp_X          # resolved + rendered times, problems
rustler image remove clp_X img_1
```
`image add` takes `--asset`, `--file` or `--url` (png, jpg, webp, bmp, or a GIF that loops), fits the
image inside the rect (`--fit contain`, or `cover` to fill it), with `--scale`, `--opacity` 0..1,
`--rotation` degrees, `--anim-in` / `--anim-out` (`none | fade | pop | slide_up`, `--anim-duration`,
default 0.3 s) and `--z` (higher on top). Images sit **below the captions** unless
`--above-captions true`. Default time: the whole clip.

```
rustler mask add clp_X --kind blur --rect '{"x":0.75,"y":0.02,"w":0.23,"h":0.07}'                # channel logo
rustler mask add clp_X --kind box --rect '{"x":0.05,"y":0.78,"w":0.9,"h":0.08}' --start 2 --end 9   # burned-in subtitles
rustler mask edit clp_X msk_1 --kind pixelate --strength 40
rustler mask list clp_X
rustler mask remove clp_X msk_1
```
Masks hide something in the **source picture** (pop-ups, watermarks, someone else's subtitles): `blur`
(`--strength` = radius px, default 20), `box` (solid `--color`, default black), `pixelate` (`--strength`
= block px, default 24). Look at a `see frame` first to place the rect. A camera effect zooms the mask
together with the picture.

```
rustler cutaway add clp_X --source-id src_B --src-start 12 --start 3 --end 5.5 --mode full
rustler cutaway add clp_X --file chart.png --mode pip --rect '{"x":0.52,"y":0.58,"w":0.44,"h":0.25}' --start 6 --end 9
rustler cutaway edit clp_X cut_1 --mode pip --rect '{"x":0.5,"y":0.6,"w":0.45,"h":0.25}'
rustler cutaway list clp_X
rustler cutaway remove clp_X cut_1
```
A cutaway is B-roll: another imported source, an asset, a file or a URL (video or image). `full`
replaces the picture for the range while the speech keeps playing; `pip` shows it inside the rect.
`--fit cover` (default) fills, `contain` shows all of it. `--mute false` mixes the cutaway's own audio
in (images and silent files stay muted). Keep full cutaways short (1-3 s) and on a sentence that
describes what is shown.

```
rustler caption word clp_X --match million --color "#FFE000" --scale 1.3      # every "million" in the clip
rustler caption word clp_X --source-id src_X --word-i 42 --emoji "🔥"
rustler caption word clp_X --source-id src_X --word-i 17 --hide true          # leave a word out of the captions
rustler caption words-list clp_X
rustler caption words-clear clp_X                                             # or --source-id/--word-i for one
```
Keyword captions: a colour, a size factor (0.5-2.5), an emoji after the word, or hide it. Calling it
again on the same word merges the changes. While the word is spoken the style's active-word highlight
colour wins; the size stays. One or two keywords per sentence at most.

**Stacking order** (bottom to top): video -> full cutaways -> masks -> camera effects -> PiP
cutaways -> images (by `z`) -> captions -> text overlays -> images with `above_captions` -> progress bar.

## 7. Example sequences

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

**Cold open + hook clip merge:**
```
rustler clip duplicate clp_X --name clp-x-cold                       # keep the original
rustler edit cold-open clp_Y --start-unit 57 --end-unit 57 --expect-text "That is when"
rustler render preview-frame clp_Y --out-t 0.3                      # hook frame
rustler clip merge clp_teaser clp_Y --name teaser-plus-story --transition fade --transition-duration 0.2
rustler timeline show clp_Z                                          # every layer with its new time
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

## 8. What not to do

- No effect or transition in the first 0.5 s: the hook frame must be clean.
- Do not hide a weak cut with a transition; fix the cut (`edit refine`, `cut-points`).
- Do not stack hook title + big caption + lower third + progress bar at once: pick what carries the
  clip. If `caption_over_face` or `caption_safe_zone` warns, move the caption (`caption set --position-y`).
- Time-anchored items (overlays, effects, `--at` sound effects, layers given in seconds) do not follow v1 re-cuts (`remove-pauses`, `remove-words`, `trim`): add them last, or anchor layers to words (`--start-word`). `insert-segment`, `cold-open` and `clip merge` do move them with their content.
- `history undo` restores segments **and** layers. Every layer command (`overlay add-text/edit/remove`, `image/mask/cutaway add/edit/remove`, `effect add/remove`, `transition set/clear`, `audio add-sfx/remove-sfx/add-music/remove-music/denoise/volume`, `overlay progress-bar`, `caption word/keywords`, `crop track-face/clear`, `grade set/clear`) saves its own revision, so one undo removes exactly that change (a hook and its whoosh are one change). Revisions that only recorded a render or a QC report are skipped, so undo after `render run` still removes the last real edit. Layer commands that take `--base-revision` need the clip's new revision after each change (`clip show`).
- No speed outside about 0.85-1.3 on speech unless it is a deliberate effect; a freeze longer than ~1.5 s feels like a stall.
