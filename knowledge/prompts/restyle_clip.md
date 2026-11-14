---
name: restyle_clip
title: "Restyle a clip"
description: "Change a clip's caption style, caption position, layout or music to a requested look, then re-render and check it."
arguments:
  - name: clip_id
    description: "The clip id (clp_…) to restyle."
    required: true
  - name: style
    description: "The requested look in plain words, e.g. 'hormozi style, captions higher', 'calmer, no music', 'boxed captions for bright gameplay', 'show the full frame'."
    required: true
---

Restyle clip **{{clip_id}}** to: **{{style}}**. Keep the cut (start/end) unchanged unless the request
is about pacing. Reference guides: `captions-and-style`, `layouts-and-framing`, `audio`.

## 1. Read the current state
1. `rustler clip show {{clip_id}}` → layout per segment, caption settings, audio settings.
2. `rustler caption list-styles` (if available) → exact preset names and looks.
3. `rustler render preview-frame {{clip_id}} --out-t 1.0` → see the current look.

## 2. Translate the request into settings

| Request words | Settings |
|---|---|
| "hormozi", "punchy", "hype", "bold pop" | `--preset hormozi-pop --max-words-per-line 2` |
| "karaoke", "default", "classic" | `--preset bold-karaoke --max-words-per-line 3` |
| "boxed", "readable on bright", "pill" | `--preset boxed-highlight` |
| "clean", "minimal", "calm", "premium" | `--preset clean-white --max-words-per-line 4` |
| "captions higher / lower" | `--position-y` −/+ 0.04-0.08, staying within 0.18-0.78 |
| "Hindi script" / "Roman Hinglish" | `--script devanagari` / `--script latin` |
| "no captions" | `--enabled false` |
| "show the full frame", "don't crop" | `clip set-layout {{clip_id}} --preset blur-fit` |
| "follow the face", "zoom on speaker" | `--preset center-crop` + `crop track-face` (if available) |
| "streamer layout", "cam on top" | `split-vertical` with measured rects (`layouts-and-framing` §4) |
| "add music", "calmer music", "no music" | `audio add-music` (−18 dB, duck) / lower `volume_db` / remove if a command exists |
| "tighter", "faster" | `edit remove-pauses --min-gap 0.4` |

If a request can't be done with the available commands (e.g. a custom font or colour not in any
preset), apply the closest preset and say exactly what differs.

## 3. Apply and preview
```
rustler clip set-captions {{clip_id}} --style <preset> --position-y <y> --max-words-per-line <n>
rustler clip set-layout {{clip_id}} --preset <preset> [--params '{…}']
rustler render preview-frame {{clip_id}} --out-t 1.0
rustler render preview-frame {{clip_id}} --out-t <a moment where the face is lowest/largest>
```
Look: captions clear of the face and of platform UI (bottom 22 %, top 8 %, right 13 %), readable,
layout framing correct. Adjust before rendering.

## 4. Render and watch
`rustler render run {{clip_id}}` → `rustler qc run {{clip_id}}` → open the contact sheet; check
against `quality-checklist`. If the user wanted to compare looks, keep the old render id and report
both render paths.

## 5. Report
What changed (old → new settings), QC result, contact-sheet observations, and any part of the
request you could not match exactly.
