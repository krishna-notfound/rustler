---
id: learn-style-from-videos
title: "Learn a clip style from a folder of example videos"
summary: "Import example shorts, let Rustler measure pacing, captions, hook text, zoom, loudness and music, LOOK at the contact sheets and first-3-second filmstrips, then write the style down as a template spec."
order: 20
tags: [reference, style, template, contact-sheet, captions, pacing]
---

# Learn a clip style from example videos

Use this when the human says "make my clips look like these" and gives a folder of finished shorts
(their own, or ones they like). You do the seeing and the judging; Rustler measures what eyes cannot
count and shows you the pictures. Nothing here calls an AI service or downloads a heavy model.

```
rustler reference import my-style --folder C:\refs\shorts        # proxies + audio; originals untouched
rustler reference analyze my-style                               # measures + sheets (about 10 s per 40 s video)
rustler reference list
rustler reference show my-style --video v02                      # look at one video again
```

`import` takes a `--folder` (add `--recursive` for sub-folders) and/or `--files`. Only videos are kept
(`mp4 mov mkv webm m4v avi flv wmv mpg mpeg ts`); anything unreadable is listed under `skipped`. A set
holds a handful of examples (max 60); five to ten of one creator is ideal. Only the first 180 s of each
video is analysed (`--max-seconds`). The set lives in `<project>/references/<name>/`; `analyze` writes
`profile.json` there.

## 1. LOOK first

`analyze` returns an `image` list: one **contact sheet** per video (12-15 evenly spaced frames, cue words
under each when a `.srt`/`.vtt` sits next to the video), then one **first-3-seconds filmstrip** per video
(8 frames). Open them before reading any number. Ask yourself:

- Where is the speaker/subject in frame, how big is the face, is there a second layer (split screen, bars,
  chart, B-roll)?
- What does the caption look like: font weight, case, colour, outline or box, highlighted word colour, how
  many words at once?
- What is on screen in the first second: a text box, a question, a face already talking, motion?
- Is there a logo, watermark, progress bar, handle or lower-third that is part of the style?

The numbers below can be wrong on unusual footage; the pictures decide.

## 2. Read the profile

`aggregate` is the cross-video summary (`aggregate.summary` is plain sentences); `videos[]` has the
same blocks per video. Each block names its method in the field `method`.

| Block | What it tells you | Use it for |
|---|---|---|
| `cuts.per_minute`, `cuts.shot_length` | hard visual cuts per minute, shot length median/p25/p75 | cut pacing target; how long a shot may stay |
| `zoom.per_minute`, `zoom.events`, `zoom.coverage` | smooth zooms and punch-ins (scale jumps at a cut) per minute | `effect.add` punch-ins; coverage low (dark/flat footage) = under-counted, trust your eyes |
| `text.caption` | `y_center`/`height` of the caption band (fractions of frame height), `words_per_line`, `fill_colors` (text colours by share), `outline_or_box_color` | caption preset, `position_y` (about the band's bottom edge `y1`), max words per line, colours |
| `text.hook` | text up in the first 3 s and gone afterwards: box position, lines, `visible_to` | hook title position and duration |
| `text.static_overlays` | recurring bands that never change (logo, watermark, handle) | probably part of the style; check the sheet |
| `layout_distribution`, `shots[].layout` | per shot: `face_closeup`, `face_medium`, `face_small`, `multi_face`, `no_face`, plus `+split_screen` / `+inset_bars` | layout mix (crop+track vs blur-fit vs split) |
| `loudness.integrated_lufs` | integrated loudness of the original audio | the render `target_lufs` (see `audio`) |
| `music` | heuristic: `music_present`, `likelihood`, pause depth, pulse | whether to add a music bed; weak signal, listen/check the sheet |
| `speech` | words per minute from a `.srt/.vtt` sidecar (skipped without one) | pace of the speech the style suits |

Honest limits (also in `notes`): measured on a 540 px proxy at 15 fps; text bands are edge-density
heuristics (a busy chart can look like a caption; the profile picks the band that changes within shots and
is edge-dense, and lists the rest as `static_overlays`); zoom needs texture on both sides of the frame;
`faces` is `available: false` with a note when the YuNet model is missing; music is a heuristic.
When a number and a sheet disagree, say so in your write-up and trust the sheet.

## 3. Turn it into a spec

Write a plain spec the human can read and later turn into a template (the `template.*` commands arrive
with the templates feature; until they exist, keep the spec as a note and apply it with today's commands):

```
style: my-style           based_on: reference set my-style (5 videos)
pacing:   ~8 cuts/min, shots 3-8 s; punch-in on emphasis words (~1/min)
layout:   face_closeup 90 percent -> crop + track; chart inserts full-frame
captions: bold uppercase, white fill, green highlight word (#4df873), black outline,
          2 words per line, bottom edge at position_y 0.73
hook:     white text on a dark box, top third (y 0.08-0.15), 2 lines, visible 0-3.2 s
audio:    -14 LUFS, no music
```

Map each line to a command: captions to `clip set-captions` / `caption create-style` (see
`captions-and-style`; the profile's caption `y1` is the bottom edge, which is what `position_y` means),
hook to `overlay add-text` (see `finding-hooks`), layout to the `layout` commands, punch-ins to `effect add`,
pacing to your cut choices (see `cut-points`), loudness to the render `target_lufs` (see `audio`). Then make one clip, `see frame` it next to the reference sheet, and adjust.

## 4. Check your work

Compare like with like: render a clip, run `rustler reference import mine --files <render>` and
`reference analyze mine`, and compare the new profile with the reference aggregate (cuts per minute, caption
`y_center`, colours, hook `visible_to`). Differences of 20 percent are normal; a caption band at a different
height or a missing hook is not.
