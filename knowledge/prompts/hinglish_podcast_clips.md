---
name: hinglish_podcast_clips
title: "Clip a Hinglish podcast"
description: "Make clips from a Hindi-English podcast: correct transcription settings, Hinglish hook picking, romanised captions and Hinglish titles."
arguments:
  - name: source
    description: "URL or local file path of the Hinglish podcast/interview."
    required: true
  - name: count
    description: "Maximum number of clips (default 4)."
    required: false
---

Make up to **{{count}}** (empty = 4) clips from the Hinglish podcast **{{source}}**. Read `hinglish` first, then
use `finding-hooks`, `cut-points`, `captions-and-style` and `quality-checklist`. Never upload.

## 1. Get the source
- URL → `rustler source info` + `rustler source fetch-transcript --lang auto` to scan (Hindi auto
  captions may come back in Devanagari; they are romanised for reading when the project script is
  latin). Pick windows (heatmap spikes, chapters), then `source import --sections …`.
- File → `rustler source import "<path>"`.

## 2. Transcribe with the right language
1. Decide the dominant language before transcribing: from the scan transcript (URL), the title and
   description, or the brief. Mostly Hindi → `--language hi`; mostly English → `--language en`.
   Most Hinglish podcasts are Hindi-dominant. Avoid `auto` here: it guesses from the first ~30 s,
   which are often an English intro.
2. Prefer `--model medium` for the final transcript if time allows (better code-switching).
3. `rustler analyze transcribe <src> --language <hi|en> --model <small|medium>` (rerun with
   `--max-seconds` until `complete: true`), then `analyze scenes` and `analyze audio`.
4. Sanity check with `transcript get`: real Hinglish words ("kya", "matlab", "yaar") in Latin; no
   English *translation* of Hindi speech. Wrong → rerun with the other language setting (see
   `hinglish` §1 about saved parts being reused).

## 3. Pick moments
Use the `finding-hooks` rubric. Strong Hinglish podcast moments: money in lakh/crore, salary/EMI/
startup failures, family and marriage pressure, career confessions, roasts and banter with real
laughter, contrarian takes on Indian systems (education, jobs). Cross-check with `see tiles --around`
(search in Latin spelling; search is romanisation-insensitive).

Weak openers to skip: toh, matlab, haan, na, accha, dekho, yaar, bas, aur, phir, woh, arre.

## 4. Cut and style
- `clip create … --start-unit <a> --end-unit <b> --expect-text "<first words as printed>"`.
- Two-person podcasts: if both faces matter and sit far apart → `blur-fit`; else `center-crop` on the
  speaker with `crop track-face` (if available).
- Captions: `--script latin` (default), `--max-words-per-line 2` or `3` (romanised Hindi words are
  long); `bold-karaoke` for energetic talk, `clean-white` for emotional/slow parts. Use
  `--script devanagari` only if the brief asks; then check for □ boxes in the preview.
- Check name/brand/slang spellings in the captions on the preview and contact sheet.

## 5. Render, watch, fix, export
`render preview-frame` → `render run` → `qc run` → open the contact sheet → fix → `export clip`.

## 6. Report
Per clip: file, units, duration, score with part scores, QC + contact-sheet notes, a Hinglish title
in Latin script (≤ 60 chars, about this moment, e.g. "Isne 20 lakh gawa diye ek galti se"), a
1-2 sentence description, 3-5 hashtags mixing niche and Indian tags, and any caption spelling issues
you could not fix.
