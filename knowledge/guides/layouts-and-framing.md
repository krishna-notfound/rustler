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

Without `--segment-id`, `clip set-layout` applies to every segment. Later presets such as
`split-2speaker`, `screencast+inset` and `pip` may exist: check `rustler commands describe clip.set_layout`.

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

### Step 1: find the webcam rectangle

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
