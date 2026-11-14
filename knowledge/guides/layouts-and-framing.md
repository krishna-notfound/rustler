---
id: layouts-and-framing
title: "Layouts and framing for 9:16"
summary: "When to use center-crop, blur-fit or split-vertical, how to measure a streamer's webcam rectangle from a frame, face tracking, when not to crop, and platform UI safe zones."
order: 6
tags: [layout, framing, crop, split, streamer, face-tracking, safe-zone]
---

# Layouts and framing

Output canvas: **1080×1920, 9:16, 30 fps**. All rectangles in Rustler are **normalised 0-1**
`{x, y, w, h}` (x,y = top-left corner) relative to the source's display size (after rotation).

## 1. The presets

| Preset | What it does | Use when | Avoid when |
|---|---|---|---|
| `center-crop` | Cuts a full-height 9:16 window out of the source. From 16:9 the window is 31.6 % of the width. `focus_x` (0-1) sets its centre. | One person talking; the subject stays in a narrow column; phone footage | Two people far apart, slides, text, wide shots, gameplay with HUD at the edges |
| `blur-fit` | Whole source frame fitted to the width, centred, on a blurred darkened copy of itself. From 16:9 the video band covers y 0.342-0.658 (608 px tall). | Slides, screen shares, wide shots, two-shots, memes with text, anything where the full frame matters | Talking heads (face becomes tiny) |
| `split-vertical` | Two regions stacked: `top` source rect → upper part, `bottom` source rect → lower part. `split` = top height fraction (default 0.5). Both use cover fit. | Streamers (webcam top, gameplay bottom), reaction videos, podcast with a screen share | Sources with no separate second area of interest |

```
rustler clip create ... --layout center-crop --layout-params '{"focus_x":0.42}'
rustler clip set-layout clp_X --preset blur-fit
rustler clip set-layout clp_X --segment-id seg_Y --preset split-vertical \
    --params '{"top":{"x":0.753,"y":0.656,"w":0.241,"h":0.333},"bottom":{"x":0.22,"y":0,"w":0.56,"h":1},"split":0.44}'
```

Without `--segment-id`, `clip set-layout` applies to every segment. Anything beyond the three presets (picture-in-picture, two panes of any shape) is built from regions: see
"Adjusting regions" below. `rustler layout presets` lists built-in and saved presets.

## 2. Choosing a layout (decision rules)

1. Look first: `rustler see tiles <src> --start <a> --end <b> --count 12`.
2. One face visible in ≥ 80 % of tiles, inside a ~30 %-wide column → `center-crop` (+ `crop
   track-face` if it moves).
3. Webcam overlay + gameplay/screen → `split-vertical`.
4. Two people both needed and > 35 % of the width apart → `blur-fit` (or a 2-speaker split if
   available). Do not let a crop swing between them.
5. Readable text, slides, code, charts, a whiteboard, a wide establishing shot → `blur-fit`.
6. **Unsure → `blur-fit`.** A small but complete picture beats a crop that cuts off the important
   thing.
7. Layouts can change per segment: a podcast clip can be `center-crop` on the speaker and `blur-fit`
   on the 4 seconds where a chart is on screen.

## 3. center-crop: setting focus_x

- `focus_x` = the horizontal centre of the subject's face in the source, normalised.
- Measure it on a `see frame`: face centre pixel x ÷ image width. Example: face centred at x=410 in a
  640-px-wide frame → `focus_x = 0.64`.
- The 9:16 window from 16:9 is 0.316 wide, so effective `focus_x` is clamped to 0.158-0.842.
- Leave **headroom**: eyes should land at roughly 30-40 % of canvas height. center-crop keeps full
  height, so this is automatic for normal framing; for very low-framed sources prefer `blur-fit`.

### Face tracking (if available — `crop track-face`)

```
rustler crop track-face clp_X                     # all segments
rustler crop track-face clp_X --segment-id seg_Y  # one segment
```

Detects faces at ~4 fps on the proxy, smooths the path (median + One-Euro filter + dead zone) and
drives the crop's x position. Use it when the speaker moves or sits off-centre. Check the contact
sheet afterwards: face inside the frame in every tile, no jumps. With two faces it follows the
dominant one; if it swings between people, go back to a fixed `focus_x` or `blur-fit`.

## 4. split-vertical for streamers (webcam top / gameplay bottom)

### Step 0 (fastest): let Rustler find the webcam

```
rustler layout detect-webcam src_A --clip-id clp_X
```

Samples 6 frames (`--samples N`, or exact times `--at '[120,480,900]'`), runs face detection and looks for
a **small face (< 15 % of the frame) at the same spot in at least half the samples**. It returns the webcam
rect (face box expanded 2.6x wide / 2.2x tall, clamped to the frame, snapped flush to a nearby edge), a
gameplay rect that avoids the webcam, a `split`, and a ready-to-run line:

```
rustler clip set-layout clp_X --preset split-vertical --params '{"top":{...webcam...},"bottom":{...gameplay...},"split":0.5}'
```

`set_layout_input` / `from_frame_boxes_input` in the result are the same thing as JSON. If it fails with
"no small, consistent face" (full-frame face cam, no webcam, face not visible) pick the rect by hand with
Steps 1-3 below. **Always check the result** with `rustler render preview-frame clp_X --out-t 1.0`: the
suggestion is a good starting point, not a measurement (it can include a few percent of overlay border;
nudge it, see "Adjusting regions").

### Step 1: find the webcam rectangle (by hand)

```
rustler see frame src_A --t 120 --width 640
rustler see frame src_A --t 900 --width 640     # second check: webcam overlays usually don't move
```

On the image, read the webcam box in pixels: left, top, right, bottom. Convert:
`x = left/img_w`, `y = top/img_h`, `w = (right−left)/img_w`, `h = (bottom−top)/img_h`.
Then **inset 2-3 %** on each side to drop the overlay's border/frame.

Example (illustrative): 640×360 frame, webcam bottom-right from (482,236) to (636,356) →
`{"x":0.753,"y":0.656,"w":0.241,"h":0.333}`.

### Step 2: pick `split` to match the webcam's shape

Each region is cover-fitted: if the shapes differ, the edges get cropped. Choose the split so the top
area has the same aspect as the webcam rect:

- webcam aspect in pixels: `A = (w × src_width) / (h × src_height)` (16:9 source: `A = 1.778 × w/h`)
- `split ≈ 0.5625 / A`, then clamp to 0.30-0.50.

Example: `A = 1.778 × 0.241/0.333 = 1.29` → `split ≈ 0.44`.

Webcam upscale = 1080 ÷ (w × src_width). Above ~2.6× it gets soft: if the webcam rect is under
~400 px wide, use a smaller split (0.33-0.38) so the face isn't blown up.

### Step 3: pick the gameplay rectangle

Bottom area aspect = `1080 / (1920 × (1 − split))`. For split 0.44 that is ≈ 1.0 (square).
From a 16:9 source the rect needs `w ≈ aspect / 1.778` at full height (≈ 0.56 here).

- Centre it on where the action is (crosshair, character, ball, chat moment), usually x ≈ 0.2-0.25.
- **Exclude the webcam overlay** from the bottom rect, or the streamer's face appears twice.
- Keep critical HUD (score, timer, kill feed) only if it matters to the moment.

### Step 4: verify

`rustler render preview-frame clp_X --out-t 1.0` → look: face filling the top, not cut at the
chin or forehead, no overlay border, gameplay readable, no duplicate webcam.

Variations: webcam on the left → same steps; full-screen face-cam moments (streamer stands up) →
change that segment to `center-crop`; "just chatting" with no gameplay → `center-crop`.

### Adjusting regions (webcam top / gameplay bottom, picture-in-picture, custom layouts)

A layout is a list of **regions**: a source rect (`src`, on the source frame) drawn into a destination rect
(`dst`, on the 1080x1920 canvas) with a `fit` (`cover` fills and crops, `contain` fits whole) and a `z`
(higher = in front). All rects are normalized 0-1; `layout show` also prints pixel rects.

```
rustler layout show clp_X                                   # regions with src_px (source pixels) and dst_px (canvas pixels), warnings
rustler layout from-frame-boxes clp_X --boxes '[{"role":"top","rect":{"x":0.71,"y":0,"w":0.29,"h":0.53}},{"role":"bottom","rect":{"x":0.08,"y":0,"w":0.63,"h":1}}]'
rustler layout from-frame-boxes clp_X --split 0.4 --boxes '[...]'        # force the top pane height
rustler layout edit-region clp_X top --nudge '{"dx":-0.02,"dy":0.01}'    # move the selection inside the source frame
rustler layout edit-region clp_X top --nudge '{"dw":-0.02,"dh":-0.02}'   # shrink it (drop the overlay border)
rustler layout edit-region clp_X bottom --src '{"x":0.1,"y":0,"w":0.6,"h":1}'   # replace a rect
rustler layout edit-region clp_X bottom --dst '{"x":0,"y":0.44,"w":1,"h":0.56}' --fit contain --z 0
rustler layout add-region clp_X --src '{"x":0.75,"y":0.02,"w":0.23,"h":0.3}' --dst '{"x":0.6,"y":0.04,"w":0.36,"h":0.14}'   # picture-in-picture, in front by default
rustler layout reorder clp_X --region-ids '["bottom","top","r1"]'        # back to front
rustler layout remove-region clp_X r1
```

- `--segment-id seg_Y` limits any of these to one segment; omitted = every segment (regions are matched by id).
- Roles for `from-frame-boxes`: `top` + `bottom` (vertical split, optional `inset`), or `main` (full canvas,
  `--main-fit contain` gives a blur-fit look) plus an optional `inset`; `{"role":"inset","rect":...,"dst":...}`.
- Rects must lie inside 0-1 and be larger than 0.02 in w and h; otherwise the command fails and nothing is saved.
- Heed the warnings: **"will crop heavily"** (source and destination aspect differ by more than 25 % with
  `cover`: pick a rect with the destination's aspect or change the split) and **"will look soft"** (a small
  source rect upscaled more than 2.6x).
- Hand-edited layouts show `preset: null`; `crop track-face` needs the `center-crop` preset.
- Save and reuse a layout (per project, in `layouts/<name>.json`):
  `rustler layout save-preset streamer-split clp_X --description "webcam top, game bottom"`,
  `rustler layout apply-preset clp_Y streamer-split`, `rustler layout presets`, `rustler layout delete-preset streamer-split`.
  `apply-preset` also takes the built-ins (`rustler layout apply-preset clp_X split-vertical --params '{...}'`).

## 5. When NOT to crop

Force `blur-fit` (or a fit region) when any is true:

- Readable on-screen text: slides, code, tweets, chat logs, captions burned into the source.
- Screen recording or tutorial where the cursor travels the whole width.
- Two or more people who all matter and sit far apart.
- Wide sports/gameplay shots where the action moves across the frame quickly.
- The source is already vertical or square (don't crop 9:16 out of 9:16).

## 6. Platform UI safe zones (1080×1920)

Platform buttons and text cover parts of the video. Keep faces, captions and key action out of them.

| Zone | Covered by | Keep clear |
|---|---|---|
| Top 0-8 % (0-150 px) | status bar, "Following / For You", Reels header | no captions, no eyes |
| Bottom 78-100 % (≈ 420 px) | username, description, music ticker, Shorts subscribe | no captions, no key action |
| Right 87-100 % (≈ 140 px) from y 45-90 % | like/comment/share/remix buttons | no text, no faces |
| Left 0-5 % | edge cropping on some devices | margin |

**Safe content box: x 0.05-0.87, y 0.08-0.78.** Captions' bottom edge (`position_y`) ≤ 0.78.

## 7. Checklist for framing (look at the contact sheet)

- Face fully inside the frame in every tile (no half faces, no cut-off chins).
- No black bars (unless intended `blur-fit` band — blur, not black).
- Nothing important under the right-side buttons or bottom text area.
- Webcam overlay border not visible; no duplicate face in a split.
- Picture not visibly blurry from upscaling.
