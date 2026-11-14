---
id: workflow-local-file
title: "Workflow: clipping a local video file"
summary: "Import a file from disk, build the understanding pack (audio, scenes, transcript, frames), pick moments and deliver checked clips."
order: 3
tags: [workflow, local, import, analyze]
---

# Workflow: local video file

Use this for files the user gives you (recordings, downloads they already have, phone footage) and as
the fallback when URL import is blocked.

## Step 1: import

```
rustler project create --name "client-call-clips" --dir ./client-call-clips
rustler source import "D:/footage/My Podcast (ep 12) – final.mp4" --name ep12
```

- The media stays where it is; Rustler records the absolute path and makes a 540p proxy and a 16 kHz
  wav inside `sources/<id>/`. Do not move or rename the original afterwards.
- If `permissions.fs_roots` is set and the file is outside it, import fails with `validation`. Ask
  the user to allow the folder; do not copy files around to dodge it.
- Read the returned `probe`: `duration`, `width`×`height` (display size after rotation), `fps`,
  `has_video`, `has_audio`.

| Probe says | Meaning for you |
|---|---|
| `has_video: false` | Audio-only (podcast mp3). `clip create` refuses it; ask for the video or skip. |
| `has_audio: false` | Silent footage: no transcript, pick moments visually (`see tiles`, scenes). Render adds a silent track. |
| height > width | Already vertical (phone). Use `center-crop` with focus 0.5 or `blur-fit` only if it is not 9:16. |
| 4K / 60 fps | Fine. Output is always 1080×1920 @ 30 fps. Analysis runs on the proxy. |

## Step 2: understanding pack

```
rustler analyze quick src_ep12
```

This runs `analyze audio` (1 s loudness curve, top peaks, silences), `analyze scenes` (shot cuts) and
`analyze transcribe` (word timings + sentence units). The summary returns `duration`, `words`,
`units`, `scenes`, the top 10 loudness `peaks` and total silence.

Run the parts separately when you need control:

```
rustler analyze transcribe src_ep12 --model small --language auto --max-seconds 300   # rerun until complete
rustler analyze scenes src_ep12
rustler analyze audio src_ep12
```

Sanity-check the transcript before trusting it:

- `words` near 0 on a talking video → wrong language or music-heavy audio. Retry with `--language en`
  or `--language hi`, or a bigger `--model medium`.
- Words per second below ~1.0 on dense talk → transcription is missing speech; try a bigger model.
- Hindi/English mix → read `hinglish` first.

### Beyond speech: `analyze deep`

```
rustler analyze deep src_ep12            # on-screen text + audio events + face timeline, one summary
rustler analyze ocr src_ep12 --every 2   # parts on their own: analyze ocr | events | faces
rustler analyze events src_ep12
rustler analyze faces src_ep12 --fps 1
```

Read `understanding` in the result first (plain sentences), then the parts:

- `ocr.burned_in_captions: true` (a *burned-in captions band at y≈0.8*): the source already has subtitles.
  Do not add a second caption on that band; move Rustler captions up (`clip set-captions <clip> --position-y 0.2`)
  or disable them. A *timecode-counter* region is just a ticking number, not captions.
- `events.top_moments`: strongest laughter, applause and cheering, each joined with the transcript unit
  said just before. Look at them with `see tiles` and consider them for hooks and endings. If
  `events.model` is `heuristic`, laughter and cheering cannot be detected.
- `faces.suggestion`: `center-crop`, `blur-fit` or `split-vertical` with the reason, and `track_face` when the
  speaker moves (then run `crop track-face`). `face_time_pct` under about 20 means no face tracking.

Results are saved as `analysis/<source>/ocr.json`, `events.json`, `faces.json`, `understanding.json`. Each part that
cannot run (silent video, audio-only file, missing model) becomes a warning; the others still run.
The OCR model reads Latin and Chinese text, not Devanagari.

## Step 3: read and look

```
rustler transcript get src_ep12 --start 0 --end 600          # numbered units for the first 10 min
rustler transcript search src_ep12 --query "biggest mistake"
rustler see tiles src_ep12 --start 1830 --end 1870 --count 12
rustler see tiles src_ep12 --around "biggest mistake" --window 20
rustler see frame src_ep12 --t 1844.2 --width 960
rustler see preview src_ep12 --start 1838 --end 1862          # ≤ 60 s low-res clip, if you can watch video
```

Read the whole transcript in chunks of 10-15 min for sources under ~90 min. For longer files, start
from the loudness `peaks` and scene-dense regions, then read ±3 min around each.

Tiles are your eyes: each tile is labelled with its timestamp and the words spoken there. From one
grid you learn the shot type (talking head, two-shot, screen share, gameplay + webcam), where faces
are, whether there is burned-in text, and whether the reaction matches the words.

## Step 4: plan clips

Shortcut for long files: `rustler highlights candidates src_ep12 --count 10` returns ranked,
sentence-aligned windows with unit ids, reasons and a ready `create_cmd`; read each with
`transcript read-window` and judge it yourself (the score is evidence, not a decision; see `finding-hooks`).

For each candidate write one line: `units #a-#b · opening words · payoff · score · layout`. Use the
rubric in `finding-hooks` and the boundary rules in `cut-points`. Keep candidates ≥ 70, drop the
rest. Remove overlaps and repeats.

Example plan (illustrative):

```
1. #212-#219 "nobody tells you this about..." → payoff: the $40k lesson · 86 · center-crop + track-face
2. #455-#470 "I got fired on my birthday"     → payoff: boss's text message · 81 · center-crop
3. #702-#709 "here's the exact script"        → payoff: the 3 lines   · 72 · blur-fit (slide on screen)
```

## Step 5: build, render, watch

```
rustler clip create --name "nobody-tells-you" --source-id src_ep12 \
    --start-unit 212 --end-unit 219 --expect-text "nobody tells you" --layout center-crop
rustler crop track-face clp_X                        # faces move → follow them (`analyze deep` says when it helps)
rustler clip set-captions clp_X --style bold-karaoke --position-y 0.70
rustler edit remove-pauses clp_X --min-gap 0.5       # tighten dead air
rustler render preview-frame clp_X --out-t 1.0       # look: face framed? captions clear?
rustler render run clp_X
rustler qc run clp_X                                 # then open qc contact sheet
rustler export clip clp_X --name "nobody-tells-you"
```

Draft first when you are unsure: `render run clp_X --quality draft` is much faster; render `final`
only for the version you will export.

## Step 6: report

For every exported clip report: clip name, source + unit range, duration, layout, caption style, QC
result, what you checked on the contact sheet, a suggested title/description/hashtags (see
`platform-specs`), and any doubt (e.g. "speaker's face is half cut at 0:12").

## Silent or music-only footage

No speech means no units. Pick by eye: `analyze scenes`, then `see tiles` over scene ranges; choose
action, reveals, transformations, fails. Cut on scene changes, never mid-motion. Create with
`--start/--end` using times from scene boundaries or tile labels only. Captions are disabled
automatically; add a hook text overlay (`overlay add-text clp_X "Wait for it" --start 0 --end 2.5`).
