---
id: streamer-clips
title: "Streamer and reaction clips: webcam split end to end"
summary: "Stream VOD to a webcam-on-top / gameplay-below clip: detect-webcam, split layout, region tweaks, face checks, reusable presets, what to cut and what to say, and the speaker commands when your build has them."
order: 14
tags: [streamer, webcam, split-vertical, layout, regions, detect-webcam, reaction, gaming]
---

# Streamer and reaction clips

A streamer clip works when the viewer sees **the reaction (face) and the cause (game, video) at the
same time**. In 9:16 that is a stacked layout: webcam on top, gameplay below. This guide is the whole
chain; the rectangle maths and the region commands are in `layouts-and-framing`.

## 1. Find the moment

Streams are long and mostly dead air. Use the cheap signals first.

```
rustler source info <vod-url>                       # heatmap peaks and chapters, no download
rustler source import <vod-url> --sections 1:12:40-1:16:00 --sections 2:05:00-2:09:30
rustler analyze quick src_X
rustler analyze deep src_X                          # audio events: laughter, cheering, applause; faces: webcam?
rustler highlights candidates src_X --count 10 --platform tiktok
```

- `analyze deep` joins the loudest **reaction moments** (cheer, laugh, applause) with the line that was
  said. A shout or laugh right after a game event is the clip; the setup is 3-8 seconds before it.
- `analyze faces` says whether a face is present most of the time (`face_time_pct`) and suggests a
  layout. A small face that stays in one corner means a webcam overlay: go to step 2.
- Chat-heavy "just chatting" streams with no gameplay and a full-screen face: skip the split and use
  `center-crop` + `crop track-face`.
- Read the transcript around each candidate (`transcript read-window src_X 4120 4160`) and LOOK at it
  (`see tiles src_X --around "no way"`): is the webcam visible, is the game event readable?


## 2. Pick the webcam and gameplay boxes yourself (recommended — you can see)

You are a vision model: you don't need a detector to find a facecam. Ask for a frame with a coordinate
grid, read the box edges off the labels, and build the layout from those numbers:

```bash
rustler see frame --source-id <src> --t 30 --grid true        # 1280-wide frame with x0.1…x0.9 / y0.1…y0.9 lines
# read it: e.g. facecam spans x 0.775–0.985, y 0.02–0.50; gameplay is the rest
rustler layout from-frame-boxes <clip> --input-json @boxes.json
#   boxes.json: {"clip_id":"<clip>","boxes":[{"role":"top","rect":{"x":0.775,"y":0.02,"w":0.21,"h":0.48}},
#                                           {"role":"bottom","rect":{"x":0.0,"y":0.0,"w":0.77,"h":1.0}}]}
rustler see frame --source-id <src> --t 30 --regions-of <clip>   # your boxes drawn on the frame with their numbers
rustler layout edit-region <clip> <region-id> --nudge '{"dy":0.02}'  # fix anything that's off, then look again
```

Check two or three different times in the video (the facecam rarely moves, but overlays can). This costs
only a few frame grabs — no models run. `layout detect-webcam` (next section) is an optional shortcut.

## 2b. Optional shortcut: let Rustler find the webcam

```
rustler clip create --name clutch --source-id src_X --start-unit 310 --end-unit 322 --expect-text "no way" --layout blur-fit
rustler layout detect-webcam src_X --clip-id clp_X
```

It samples 6 frames (`--samples N`, or `--at '[120,480,900]'`), finds a **small face (< 15 % of the frame)
at the same spot in at least half of them** and returns: the webcam rect (snapped to the overlay's borders
found by edge detection: `rect_source: "edges"`; when no clear border exists it falls back to the face
expanded 2.6x2.2, `rect_source: "expansion"`), a `gameplay` rect that avoids the webcam, a `split`, `confidence`
(`face_hits / samples`), and ready-to-run `commands`, including the exact set-layout line. Run that
line:

```
rustler clip set-layout clp_X --preset split-vertical --params '{"top":{...webcam...},"bottom":{...gameplay...},"split":0.5}'
rustler render preview-frame clp_X --out-t 1.0        # LOOK
```

With `rect_source: "expansion"` the rect can include a **sliver of gameplay or overlay border** around the
facecam; even when snapped, always check the preview and nudge (`layout edit-region clp_X top --nudge '{"dw":-0.02,"dh":-0.02}'`; on Windows PowerShell 5.1
quote JSON via `--input-json @file.json`, see `troubleshooting`).

If it fails with "no small, consistent face": the webcam may be hidden in those samples, the face is
full-screen, or there is no webcam. Sample elsewhere (`--at '[...]'` at times when the streamer is
on camera) or pick the rect by hand (`layouts-and-framing` steps 1-3).

Draw the boxes yourself when detection is close but wrong:

```
rustler see frame src_X --t 900 --width 960                    # look, read the box in pixels, divide by the image size
rustler layout from-frame-boxes clp_X --boxes '[{"role":"top","rect":{"x":0.71,"y":0,"w":0.29,"h":0.53}},{"role":"bottom","rect":{"x":0.08,"y":0,"w":0.63,"h":1}}]' --split 0.42
```

## 3. Tune the regions

`layout show clp_X` prints each region with source and canvas pixels and warnings. The two warnings
that matter: **"will crop heavily"** (source and destination shapes differ by more than 25 %: change
`--split` or the rect) and **"will look soft"** (webcam upscaled more than 2.6x: use a smaller split
such as 0.35). Nudge instead of redrawing:

```
rustler layout edit-region clp_X top --nudge '{"dx":-0.01,"dw":-0.02}'          # drop the overlay border
rustler layout edit-region clp_X bottom --src '{"x":0.1,"y":0,"w":0.6,"h":1}'   # recentre the action
rustler layout add-region clp_X --src '{...}' --dst '{...}'                      # picture-in-picture (e.g. the kill feed)
```

Check: face fills the top, not cut at chin or forehead, no overlay border, no duplicate webcam in the
gameplay pane, gameplay readable.

## 4. Segments with different framing

A clip can mix layouts per segment: `edit split clp_X <out_t>`, then `clip set-layout clp_X
--segment-id seg_B --preset center-crop` for the moment the streamer stands up or goes full-screen.
Add a transition between them only if the change would otherwise look broken
(`transition set clp_X seg_A dissolve`, see `editing-toolkit`).

## 5. Reuse the layout

The same stream has the same overlay for hours. Save it once, apply it everywhere:

```
rustler layout save-preset streamer-split clp_X --description "webcam top, game bottom"
rustler layout apply-preset clp_Y streamer-split
rustler layout presets                    # built-in + saved presets
```

Presets live in `<project>/layouts/`. Variants can use them too:
`variant create clp_X --layouts '["streamer-split","blur-fit"]'`.

## 6. Captions, hook, sound

- Captions: `caption set clp_X --style hormozi-pop --position-y 0.72`. In a split the captions sit
  near the seam or low in the gameplay pane; check on the preview that they never cover the face
  (QC's `caption_over_face` warns). Streamers speak fast and swear: `--max-words-per-line 2`.
- Hook title: the gameplay outcome in under 7 words (`overlay add-text clp_X "He clutched 1v4" --start 0 --end 2.5`).
- Sound: the streamer's voice is the dialogue (`audio denoise clp_X --strength light`); game audio is
  in the same track and `loudnorm` handles the level. Music is rarely needed; a well-placed `audio
  add-sfx` on the reaction beat (`--at-word`) works better. See `audio`, `assets`.
- Effects: `effect add clp_X punch-in --start 2.1 --end 2.6` on the reaction.

## 7. Check the finished clip

`render run` → `qc run` → open the contact sheet. Streamer-specific failures:

| You see | Fix |
|---|---|
| QC `face_visible_ratio` warn on a split | wrong webcam rect: `layout detect-webcam src_X --clip-id clp_X` again or `layout edit-region` |
| Streamer's face twice | the `bottom` rect includes the webcam: shrink it, `edit-region bottom --src ...` |
| Webcam border visible | `edit-region top --nudge '{"dx":0.01,"dw":-0.02}'` |
| Blurry face | `--split` too big for the webcam size: use 0.33-0.38 |
| Captions on the face | `fix suggest clp_X` → `fix apply clp_X --check caption_over_face` |
| Dead air between plays | `edit remove-pauses clp_X --keep 0.25`, `edit refine clp_X` |

## 8. Two people on screen: speakers

For collabs, reactions with a guest, or a stream with two cams, use the local diarization commands:

```
rustler analyze speakers src_X --num-speakers 2        # who speaks when; labels every transcript word S1, S2, ...
rustler speaker rename src_X S1 Streamer               # display names (summaries); colours are keyed by id
rustler caption speaker-colors clp_X --enabled true --colors '{"S1":"#7CFF6B","S2":"#5EE0FF"}'
rustler crop track-speaker clp_X                       # center-crop follows the ACTIVE speaker's face
```

`crop track-speaker` needs a `center-crop` segment (not the split layout) and tells you per turn which
face it chose and why; details and limits are in `layouts-and-framing`, caption colours in
`captions-and-style`. In a webcam split the streamer's face is already on top: speaker colours are
the useful part there. If the models are missing the command says so (`rustler setup run` refreshes the
engine); the heuristic fallback can split one speaker into two, so check `analyze speakers` first.
