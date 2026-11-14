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
2. `rustler caption styles` → preset and custom style names and looks (`asset show style-<name>` previews one).
3. `rustler render preview-frame {{clip_id}} --out-t 1.0` → see the current look.

## 2. Translate the request into settings

| Request words | Settings |
|---|---|
| "hormozi", "punchy", "hype", "bold pop" | `--style hormozi-pop --max-words-per-line 2` |
| "karaoke", "default", "classic" | `--style bold-karaoke --max-words-per-line 3` |
| "boxed", "readable on bright", "pill" | `--style boxed-highlight` |
| "clean", "minimal", "calm", "premium" | `--style clean-white --max-words-per-line 4` |
| "captions higher / lower" | `--position-y` −/+ 0.04-0.08, staying within 0.18-0.78 |
| "Hindi script" / "Roman Hinglish" | `--script devanagari` / `--script latin` |
| "no captions" | `--enabled false` |
| "show the full frame", "don't crop" | `clip set-layout {{clip_id}} --preset blur-fit` |
| "follow the face", "zoom on speaker" | `--preset center-crop` + `crop track-face` (two people: `analyze speakers` + `crop track-speaker`) |
| "streamer layout", "cam on top" | `layout detect-webcam <src> --clip-id {{clip_id}}` → its `set-layout` line (`streamer-clips`) |
| "add music", "calmer music", "no music" | `asset search <mood>` then `audio add-music --path asset:<id> --volume-db -18` / lower `--volume-db` / `audio remove-music` |
| "tighter", "faster" | `edit remove-pauses --min-gap 0.4 --keep 0.2` |
| "hook text", "title on top" | `overlay add-text {{clip_id}} "<text>" --style hook-top --start 0 --end 2.5` (`overlay edit` to change) |
| "punchier", "more energy" | `effect add {{clip_id}} punch-in --start <t> --end <t+0.5>`, `audio add-sfx`, `overlay progress-bar {{clip_id}} --enabled true` |
| "smooth cuts" | `transition set {{clip_id}} <seg_id> dissolve --duration 0.3` |
| "colour per speaker" | `analyze speakers <src>` then `caption speaker-colors {{clip_id}} --enabled true` |
| "a custom look" | `caption create-style <name> --base <preset> --active-color "#RRGGBB"` then `--style <name>` |

If a request still can't be met (e.g. a font that is not in the library: `asset add` it with a license
first), apply the closest style and say exactly what differs.

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
