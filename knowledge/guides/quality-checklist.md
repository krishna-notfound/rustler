---
id: quality-checklist
title: "Quality checklist: QC, the contact sheet and the fix loop"
summary: "How to read qc run output, each check and its fix, what to look for on the contact sheet, the re-render loop and when to make variants."
order: 9
tags: [qc, quality, review, contact-sheet, fix-loop, variants]
---

# Quality checklist

**Exit code 0 is not a good video.** A clip is done only when (1) `qc run` passed, (2) you opened the
contact sheet and it passes the visual checklist below, and (3) it matches the brief.

## 1. Run QC

```
rustler render run clp_X                 # → render_id, path, encoder, duration
rustler qc run clp_X                     # latest render by default; --render-id rnd_… for another
```

Exit 0 → `passed: true`. **Exit 7 → `passed: false`** (at least one Fail); the error hint points to
the report file. Either way, read every check:

```json
{"name": "black_frames", "status": "fail", "value": [{"start": 12.4, "end": 13.1}],
 "expected": "no black interval ≥ 0.5 s", "fix_hint": "check segment/layout covering black area"}
```

`status` is `pass`, `warn` or `fail`. Fails block export (`export clip` refuses without `--yes`; do
not use `--yes` to skip a real failure). Warns need a decision and a note in the report.

## 2. Each check and its fix

| Check | Rule | Typical cause | Fix |
|---|---|---|---|
| `streams` | h264 video + aac audio present | encoder/render failure | re-run `render run`; if repeated, `doctor run`, try `--prefer-gpu false` |
| `dimensions` | equals canvas (1080×1920) | wrong canvas setting | check `clip show` canvas; re-render |
| `duration` | file within 0.10 s of planned timeline | broken segment, source shorter than expected | `clip show`; fix the segment that runs past source end |
| `av_sync` | video and audio stream lengths within 0.10 s | odd source timestamps (VFR streams) | re-render; if persistent, trim the segment ends by 0.1 s |
| `loudness` | −14 ± 1.5 LUFS, true peak ≤ −1.0 dBTP (warn) | very dynamic audio, loud music bed | see `audio`; lower music, cut extreme part, or accept with a note |
| `black_frames` | no black ≥ 0.5 s (fail) | segment over a fade/black part of the source; split rect pointing at an empty area; blur-fit on a letterboxed source | move segment boundaries off the black; fix the region rect; check `see frame` at that time |
| `frozen_video` | no frozen picture ≥ 2 s (warn) | paused stream, static slide, "be right back" screen | fine for slides; otherwise cut it out |
| `long_silence` | no silence ≥ 1.5 s (warn) | dead air | `edit remove-pauses` or trim at the hinted time |
| `contact_sheet` | always pass | — | **open the image** (next section) |

Later QC versions may add `caption_safe_zone`, `caption_over_face`, `face_visible`,
`hook_in_first_3s` and `brief`. Treat their `fix_hint` the same way: it names the edit to make.

Output time vs source time: QC reports times in **output (clip) time**. `clip show clp_X` lists each
segment's `out_start/out_end` so you can find the segment and the source time behind any QC time.

## 3. Read the contact sheet (mandatory)

`qc/<render-id>.jpg`: 12 frames from the finished render, labelled with output time. Over MCP it
comes back as an image; on the CLI open the file path with your image viewer/vision. If you cannot
view images, say so explicitly in your report; do not claim visual checks you did not do.

Check every tile:

1. **Hook frame (tiles 1-2):** the speaker/subject is visible and the hook caption is readable
   immediately. No black, no logo card, no blurred transition at 0 s.
2. **Face visible** whenever someone talks: whole face inside the frame, not cut at forehead/chin,
   not hidden behind captions.
3. **Captions** not over the face, not in the bottom 22 % / top 8 % / right 13 % UI zones, readable
   contrast, spelled right, no □ boxes (missing glyphs).
4. **No black bars** or unintended letterboxing; blur-fit background is blurred, not black.
5. **Layout right:** split has the face on top and the action below, no duplicate webcam, no overlay
   border; center-crop isn't sitting on an empty wall or between two people.
6. **Sharpness:** no extremely upscaled, mushy region.
7. **Continuity:** cuts don't land mid-gesture in an ugly way; no flash frames.
8. **Last tile:** ends on the payoff/reaction, not on someone starting a new sentence.

## 4. The fix loop

```
qc run → read checks + contact sheet → list problems → one edit per problem →
render preview-frame at the problem time → render run → qc run → contact sheet again
```

| What you see | Edit |
|---|---|
| Face cut off / crop on the wrong person | `clip set-layout --params '{"focus_x":…}'` or `crop track-face`, or switch to `blur-fit` |
| Captions over the face | `clip set-captions --position-y` (lower to 0.74-0.78, or up to 0.22) |
| Captions hard to read | `--preset boxed-highlight` |
| Slow first 2 s | move start: recreate with a later `--start-unit` or `edit trim --start-delta` |
| Ends weak / cuts the laugh | `edit trim --end-delta` or a later `--end-unit` |
| Dead air in the middle | `edit remove-pauses` |
| Webcam border visible | inset the `top` rect 2-3 % more |
| Wrong edit | `history undo clp_X` |

Use `render run --quality draft` while iterating if renders are slow; render `final` once for the
export. Stop after 3 failed fix rounds on the same problem: pick another moment or report the issue.

## 5. Brief compliance (before export)

Check against the brief: platform, duration range, caption style, required text/tags/credit,
forbidden topics/words/brands, number of clips, naming. Note each item in the report as met / not
met.

## 6. Variants

Make variants when a strong clip (≥ 85) has a real open question, not by default:

- Two hooks: start at the claim vs start at the number.
- Two layouts: `center-crop` vs `blur-fit` when the shot is borderline.
- Two caption styles for a niche you don't know.

Variants are separate clips (`clip create` again with the same units and a `-v2` name, or a duplicate
command if available). QC and watch each. Report which you recommend and why.

## 7. Export

```
rustler export clip clp_X --name "cofounder-emptied-account"
```

Writes `exports/<name>.mp4` + `exports/<name>.report.json` (clip, render, QC, source refs). Add your
own notes to your final message: score, what you checked, warns accepted, title/description/hashtags
(see `platform-specs`). Never upload.
