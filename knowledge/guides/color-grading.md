---
id: color-grading
title: "Colour grading: measure, decide, grade, preview"
summary: "When to grade (only when the footage needs it, never by default), subtle beats strong, matching shots in a multi-camera clip, keeping skin natural, presets and LUTs; the grade.* commands (set, show, presets, clear, preview, measure) and how a grade renders."
order: 23
tags: [grade, color, colour, lut, presets, exposure, white-balance, look, matching]
---

# Colour grading

Rustler can colour-grade a clip or one segment of it. **It never grades on its own**: no clip, starter
template or auto command adds a grade. You decide, Rustler measures, previews and renders what you set.

## When to grade

Grade only when it helps the clip:

- the footage is flat, dark, or has a colour cast (a white shirt looks yellow or blue);
- the shots of a multi-camera clip do not match (one camera darker or warmer than the other);
- the brief or the creator's style asks for a look (warm, moody, black and white ...).

Most well-lit podcast footage needs nothing. If in doubt, measure and leave it.

## The loop

1. `rustler grade measure <clip_id>` reads a few frames of each segment: brightness (on a dark
   studio background it judges the lit area, not the average), clipped blacks / whites, saturation,
   the white-balance cast of near-grey pixels, and which segments stand out from the rest. It
   returns plain-language `read` lines and `suggestions` (commands). **It changes nothing.**
   `rustler grade measure <source_id> --t 12 --t 40` works on a source before you cut.
2. `rustler grade set <clip_id> ...` stores a grade (see below). It merges onto what is stored;
   `--reset` starts from empty.
3. `rustler grade preview <clip_id> --out-t <t>` writes one jpg: left = before, right = after, the
   whole render graph (layout, captions, overlays). **Look at it.** Check a face and the brightest
   area (a white shirt, a window).
4. Adjust or go back: `rustler grade show <clip_id>` prints the stored grade and the exact ffmpeg filter
   chain per segment; `rustler grade clear <clip_id>` removes it; every change is a clip revision, so
   `rustler history undo <clip_id>` restores the previous grade (a render also adds a revision: undo
   steps over it first).
5. `rustler render run <clip_id>`.

## Subtle beats strong

- Start with a preset at `--intensity 0.5` to `0.8` and raise it only if the preview needs it.
- Exposure moves of 0.1 to 0.2, contrast 1.05 to 1.15, vibrance 0.1 to 0.3 are the normal range.
  `grade set` warns when a value is strong.
- Use vibrance before saturation: it lifts muted colours and leaves skin (already saturated) alone.
- Skin tones: after a grade, faces should look like people, not orange or grey. If skin shifts, lower
  `--intensity`, or move temperature back toward 6500.
- Never push exposure when `measure` says highlights are clipped.

## Settings (`grade set`)

| Flag | Range | What it does |
|---|---|---|
| `--preset` | see below | starting look; explicit flags override its numbers; `none` removes it |
| `--exposure` | -1..1 | brighter / darker midtones; black and white points stay put (nothing clips) |
| `--contrast` | 0.5..1.5 | S-curve around mid grey (1 = unchanged) |
| `--saturation` | 0..2 | 0 = black and white, 1 = unchanged |
| `--vibrance` | -1..1 | skin-friendly saturation |
| `--temperature` | 2000..12000 K | the light colour: lower = warmer picture, higher = cooler (6500 = unchanged) |
| `--tint` | -1..1 | green (-) / magenta (+) |
| `--blacks` | -1..1 | crush (-) or lift / fade (+) the black point |
| `--shadows` / `--midtones` / `--highlights` | "r,g,b", each -1..1 | colour push by brightness, e.g. teal shadows `-0.05,0,0.06`, warm highlights `0.05,0.01,-0.05` |
| `--lut` | `asset:<id>` or a `.cube` path | a 3D LUT; `none` removes it |
| `--lut-strength` | 0..1 | mix of the LUT (default 1) |
| `--intensity` | 0..1 | strength of the whole grade (0 = off) |

Presets (`rustler grade presets` prints each with its numbers and filter chain): `natural-pop` (light
contrast + vibrance, the safe one), `warm`, `cool`, `cinematic` (teal shadows, warm highlights, a bit of
contrast), `moody` (darker, desaturated, deeper blacks), `bright-clean` (lifted midtones), `bw`,
`vintage` (faded blacks, softer colour, warm highlights).

## Matching shots (multi-camera clips)

`grade measure <clip_id>` lists segments that are much darker / brighter or warmer / cooler than the
rest and suggests `grade set <clip_id> --segment-id <seg> ...` for each. The suggestions go half-way on
purpose. Match only shots of the same scene and camera setup (a wide shot is often darker by design).

A segment override is merged onto the clip grade: `rustler grade set <clip_id> --segment-id seg_b --exposure 0.15`
keeps the clip look and brightens that shot. `--reset --segment-id seg_b` makes the override stand alone
(that segment gets only what you set, nothing from the clip grade; with no values it stays ungraded).
`--lut none` on a segment blocks the clip's LUT there. Overrides follow their picture through splits
and other re-cuts; `grade clear <clip_id> --segment-id seg_b` drops one.

## LUTs

Add a `.cube` (3D, `LUT_3D_SIZE`) to the library: `rustler asset add look.cube --kind lut` (record its
license; Rustler bundles no third-party LUTs). Then `rustler grade set <clip_id> --lut asset:<id> --lut-strength 0.5`.
Film-emulation LUTs are usually too strong at 1.0; 0.3 to 0.6 is typical. A LUT runs after the other
settings.

## Templates

A template saved from a graded clip (`template create --from-clip`) keeps the clip-wide grade
(`grade` field; per-segment overrides are not captured), and `template apply` sets it on the target
clips. The starters `podcast-viral` and `streamer-split` have no grade.

## How it renders

The grade is a short ffmpeg chain on each segment's source picture, before the layout crops / scales /
blur-fills it and before captions, hook titles, images and the progress bar, so every region of a split
layout and the blurred background get the same grade and text keeps its exact colours. Order:
`colorcorrect` (shadow / highlight colour by brightness, tint), `colortemperature`, `curves` (exposure,
contrast, blacks), `colorchannelmixer` (saturation), `vibrance`, `lut3d` (blended when the strength is
below 1). A neutral grade adds no filter. Transitions blend the two graded segments, so a dissolve
between differently graded shots does not flash.
