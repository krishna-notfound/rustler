---
id: layouts-and-framing
title: "Layouts and framing for 9:16"
summary: "Smart defaults of clip create (per-shot layout, light active speaker, filler and pause trim) and how to override them; when to use center-crop, blur-fit, zoom-fit or split-vertical, face tracking, streamer webcams, when not to crop, and platform UI safe zones."
order: 6
tags: [layout, framing, crop, split, streamer, face-tracking, active-speaker, smart-defaults, safe-zone]
---

# Layouts and framing

Output canvas: **1080×1920, 9:16, 30 fps**. All rectangles in Rustler are **normalised 0-1**
`{x, y, w, h}` (x,y = top-left corner) relative to the source's display size (after rotation).


> **Reading coordinates:** `rustler see frame --source-id <src> --t <s> --grid true` overlays a labelled
> 0–1 grid so you can read region edges exactly; `--regions-of <clip>` draws the clip's current regions
> with their numbers. Use these instead of guessing coordinates.

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

0. `clip create` already applied a per-shot plan (see "Smart defaults" below): check `smart.decisions`
   and the contact sheet; the rules below are for overriding it or for `--raw` clips.
1. Look first: `rustler see tiles <src> --start <a> --end <b> --count 12`.
2. One face visible in ≥ 80 % of tiles, inside a ~30 %-wide column → `center-crop` (+ `crop
   track-face` if it moves).
3. Webcam overlay + gameplay/screen → `split-vertical`.
4. Two people in one shot: when only the person talking matters → `center-crop` + `crop track-active`
   (the smart default); when both must be seen at once and are > 35 % of the width apart → `blur-fit`.
5. Readable text, slides, code, charts, a whiteboard, a wide establishing shot → `blur-fit`.
6. **Unsure → `blur-fit`.** A small but complete picture beats a crop that cuts off the important
   thing.
7. Layouts can change per segment: a podcast clip can be `center-crop` on the speaker and `blur-fit`
   on the 4 seconds where a chart is on screen.

### Smart defaults: what `clip create` already decided

Unless you pass `--raw` (or the setting `clip.smart_defaults` is `false`), `clip create` does the layout and
framing for you and returns a `smart` report. **Read `smart.decisions` before changing anything**: each entry
says what was done, why, and `override_with` lists the exact commands that undo or replace it.

- **Per shot** (shots from `scenes.json` plus a frame-difference scan that also finds cuts in dark footage;
  faces at 5 fps with the YuNet landmarks; lite install, no extra models):
  - one face → `center-crop` + smooth track with look-room (like `crop track-face`);
  - two or more faces → `center-crop` + the light active speaker (like `crop track-active`, below);
  - no face, or graphics (flat areas + sharp edges: title cards, slides) → `blur-fit`, or **`zoom-fit`** when
    the content sits in the middle (the centred part of the frame scaled up on a blurred fill).
  - Segments are split only where the layout changes; a change shorter than 1.2 s is absorbed (no flicker).
  - Skipped (decision `skipped`) when you passed `--layout`/`--layout-params`, the source is already vertical,
    the media file is missing or the engine is not installed.
- **Cleanup** (needs exact word timings; captions-based transcripts are left alone): filler words
  (`um, uh, umm, uhh, uhm, hmm, hm, erm, ah` anywhere; `you know`, `like`, and in Hinglish `matlab, haan, na`
  only when they stand alone between pauses), then pauses longer than 1.0 s cut down to 0.15 s.
- Every step is its own clip revision (`smart.steps`): `smart.undo_all` (`rustler history restore <clip>
  --revision 1`) gives you the raw clip back; restoring a step's revision keeps the earlier steps.

Typical overrides:

```
rustler clip set-layout clp_X --segment-id seg_Y --preset blur-fit     # crop was wrong for this shot
rustler crop track-active clp_X --segment-id seg_Y --switch cut        # hard cuts between speakers
rustler crop track-face clp_X --segment-id seg_Y --style locked        # one framing, follow the largest face
rustler history restore clp_X --revision 1                             # undo all smart defaults
rustler clip create --name n --source-id src_X --start 10 --end 40 --raw   # never apply them
rustler config set clip.smart_defaults false                           # off for this project
```

### Per-shot plan without applying (`layout plan`)

```
rustler layout plan clp_X            # proposes: pieces[] with class, faces, layout, tracking, reason, out times
rustler layout plan clp_X --apply    # applies it (new revision; `undo` in the result restores the old one)
```

Use it on a clip made with `--raw`, after `clip add-segment`, or to see why a shot got its layout.

## 3. center-crop: setting focus_x

- `focus_x` = the horizontal centre of the subject's face in the source, normalised.
- Measure it on a `see frame`: face centre pixel x ÷ image width. Example: face centred at x=410 in a
  640-px-wide frame → `focus_x = 0.64`.
- The 9:16 window from 16:9 is 0.316 wide, so effective `focus_x` is clamped to 0.158-0.842.
- Leave **headroom**: eyes should land at roughly 30-40 % of canvas height. center-crop keeps full
  height, so this is automatic for normal framing; for very low-framed sources prefer `blur-fit`.

### Face tracking (`crop track-face`)

```
rustler crop track-face clp_X                     # all segments (style smooth, look-room on)
rustler crop track-face clp_X --segment-id seg_Y  # one segment
rustler crop track-face clp_X --style locked      # one framing per shot
rustler crop track-face clp_X --style follow --no-look-room
```

Detects faces (with landmarks) at ~4 fps on the proxy and **plans** the crop path over the whole
segment like a camera operator would. Use it when the speaker moves or sits off-centre. With two faces
it follows the dominant one; if it swings between people, use `crop track-speaker` (below) or a fixed
`focus_x` / `blur-fit`.

#### Camera styles (`--style`)

| Style | What the camera does | Use for |
|---|---|---|
| `smooth` (default) | Holds still while the face stays inside a safe box (±18 % of the crop width around the framing point). Moves only when the face leaves it for 0.5 s or would leave the frame, with an eased move (slow start, slow stop; ≤ 0.35 frame widths/s) to where the person settles. Starts 0.15 s before the face reaches the box edge. | Almost everything: talking heads, podcasts, people who lean. |
| `locked` | One fixed framing per shot (the median position); moves only at cuts. | A still talking head; a tripod look. Check the tiles: a big lean can leave the frame. |
| `follow` | Tighter safe box (±7 %), quicker eased moves (≤ 0.6 frame widths/s). | Someone who walks or moves a lot. |

What it always does: a single-frame false detection (a poster face, a blip) never moves the camera; when
the source cuts to another shot the crop **cuts** with it (never pans across a cut); the renderer gets a
key per frame during a move and nothing while holding, so moves are smooth and holds are perfectly still.

**Look-room** (`--look-room`, on by default; `--no-look-room` to centre the face): when the person turns
their head, the framing shifts toward the side they face by up to 10 % of the crop width (measured from the
nose against the eyes, smoothed over ~2 s; a frontal face stays centred). A profile speaker then has space
in front of the face instead of the nose touching the crop edge. The result's `look_room_shift` says how much
was used (> 0 = room on the right).

The result reports `moves` and `cuts` per segment. QC checks the motion (`camera_motion`: moves per minute,
direction changes, jitter, moves that start at full speed, top speed); a warning names the fix.
Check the contact sheet afterwards: face inside the frame in every tile, room in front of a turned face.

### Light active speaker (default for two or more faces: `crop track-active`)

Works in the lite install, needs no speaker analysis:

```
rustler crop track-active clp_X                         # all center-crop segments
rustler crop track-active clp_X --segment-id seg_Y --switch cut --style locked
rustler crop track-active clp_X --look-room false
```

For each visible face it measures mouth movement (a mouth box placed from the two mouth-corner landmarks,
head movement subtracted, normalised for dark footage) and compares the faces **while somebody is speaking**
(transcript words; else the `analyze audio` silences; else motion alone, with a warning). Hysteresis: a new
face must lead clearly for at least 1 s of its own mouth movement before the camera switches; when the lead is
ambiguous (both mouths move alike) the current face is kept; a face that moves its mouth only in silence
(chewing, laughing) counts less. Switches land on a word start after a pause and use the same smooth camera
as `crop track-speaker` (`--switch auto|cut|pan`, `--style`, look-room). When the framed face leaves the
picture (a cut) the new shot's best face is taken at once.

Read `segments[].turns[]` (`face_x`, `reason`: `only-face`, `mouth-motion`, `largest-face` = no clear talker,
`shot-change`) and `switches`. Limits: faces must be visible and roughly frontal (profiles and faces under
~2.5 % of the frame width give little mouth signal; those turns fall back to the largest face), and people
who mouth along or nod a lot can steal short turns. When `analyze speakers` has been run, `crop
track-speaker` uses the voices too and links people across cuts; prefer it for long interviews with many
close-up/wide-shot cuts.

### Following the active speaker with diarization (`crop track-speaker`)

Two people in one wide shot, both needed, but the 9:16 crop only fits one? Let the crop follow
whoever is talking. It needs the speaker analysis first, and a `center-crop` segment:

```
rustler analyze speakers src_X --num-speakers 2   # who speaks when (local models, a few seconds per minute of audio on CPU)
rustler crop track-speaker clp_X                  # all center-crop segments
rustler crop track-speaker clp_X --segment-id seg_Y
```

Pass `--num-speakers 2` for an interview: the automatic count is a guess (it counts a voice only when
it is clearly different from the others and has several seconds of speech). Check the timeline in the
result against the transcript before going on; a host who only asks short questions is easy to lose.

The crop tracks *people*, not boxes: faces are recognised across cuts (wide shot, close-up, wide shot
again keep one `face_id`; `face_id` counts people left to right). For each speaker turn it picks the
person like this, strongest evidence first (read `segments[].turns[]`, `reason` says which):

- `only-face`: one person is on screen during the turn (a close-up); the crop follows them.
- `face-id`: several people on screen, but the speaker is linked to one person because that person is
  alone on screen (close-up) during that speaker's turns. This is what makes wide shots with tiny faces work.
- `elimination`: the only unclaimed person left for the only unclaimed speaker.
- `mouth-motion`: clear winner by lip movement (frame difference in the lower face). Only used on faces at
  least 7 % of the frame wide; on the 4-6 % faces of a wide two-shot lip movement is noise and is ignored.
- `speaker-map`: motion was unclear in this turn, but this speaker's mouth moved in other turns on that face.
- `dominant`: ambiguous, the largest face was used. Look at the tiles for this turn. If it is wrong
  (profile views, tiny faces, sunglasses), use `blur-fit`, or split the clip at the turn and set
  `focus_x` per segment.

Speaker switches (`--switch`): `auto` (default) **cuts** on the turn start (a sentence boundary) when
the reframe is wider than 40 % of the frame and otherwise **pans** with an eased move of at most 0.6 s
centred on the turn start; `--switch cut` always cuts, `--switch pan` always pans. Within a turn the
camera behaves like `crop track-face` (`--style smooth|locked|follow`, `--look-room`; look-room costs one
extra face pass). Turns shorter than 1 s ("mm-hm", "yeah") never move it.
When the chosen person is not on screen after a cut (a reaction shot, the other camera), the crop
follows the largest face that is on screen instead of holding a position from the old shot; with no face
at all (a graphic) it holds the last position.

Limits: it follows whoever the diarization says is talking, so check `analyze speakers` first (two
voices that sound alike in a reverberant hall can be merged or the first words of a turn can land on the
wrong speaker; crosstalk is attributed to one speaker). Re-run it after trimming a clip: the crop path
belongs to the segment's time range. The first run downloads a 38 MB face-recognition model; offline it
still works but faces are not linked across cuts (a warning says so).

## 4. split-vertical for streamers (webcam top / gameplay bottom)

### Step 0 (fastest): let Rustler find the webcam

```
rustler layout detect-webcam src_A --clip-id clp_X
```

Samples 6 frames (`--samples N`, or exact times `--at '[120,480,900]'`), runs face detection and looks for
a **small face (< 15 % of the frame) at the same spot in at least half the samples**. It returns the webcam
rect, a
gameplay rect that avoids the webcam, a `split`, and a ready-to-run line:

```
rustler clip set-layout clp_X --preset split-vertical --params '{"top":{...webcam...},"bottom":{...gameplay...},"split":0.5}'
```

`set_layout_input` / `from_frame_boxes_input` in the result are the same thing as JSON. If it fails with
"no small, consistent face" (full-frame face cam, no webcam, face not visible) pick the rect by hand with
Steps 1-3 below. **Always check the result** with `rustler render preview-frame clp_X --out-t 1.0`: the
suggestion is a good starting point, not a measurement (nudge it, see "Adjusting regions").

**How the webcam rect is found:** the engine's `overlay.find_box` looks for the facecam overlay's borders
around the face (Canny edges averaged over the sampled frames, long straight lines on all four sides) and
snaps the rect to them, so it no longer includes a sliver of gameplay beside the camera. The result says
`rect_source: "edges"` (snapped; `rect_confidence` 0..1) or `rect_source: "expansion"` (no clear border, e.g.
a camera with a soft or borderless edge: the rect is the face box expanded 2.6x wide / 2.2x tall and a
warning tells you to check it); `expansion_webcam` is always the expansion for comparison.

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
