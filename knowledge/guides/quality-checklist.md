---
id: quality-checklist
title: "Quality checklist: QC, the contact sheet and the fix loop"
summary: "How to read qc run output, each check and its fix, fix suggest / fix apply / qc compare, what to look for on the contact sheet, the re-render loop and when to make variants."
order: 9
tags: [qc, quality, review, contact-sheet, fix-loop, fix-suggest, qc-compare, variants]
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
| `caption_safe_zone` | `position_y` between 0.10 and 0.85 (warn) | captions under the platform UI | `clip set-captions --position-y 0.70` |
| `caption_over_face` | captions on: a face box overlaps the caption band (`position_y` -0.08 .. +0.04 of the height) in at most 20 % of samples taken at 2 fps from the render (warn) | speaker's face sits where the captions are | `clip set-captions <clip> --position-y <y>`: the hint computes a `y` below the median face box, else above it |
| `face_visible_ratio` | center-crop (or the top pane of a split): a face at least 60 % inside the frame in at least 70 % of samples (warn) | crop not following the face; wrong `focus_x`; wrong webcam rect | `crop track-face <clip>`, or `layout detect-webcam <source> --clip-id <clip>` for splits |
| `face_cut_off` | a face touching the left/right frame edge in at most 25 % of samples (warn) | crop too tight or off-centre | `crop track-face <clip>` or switch to `blur-fit` |
| `hook_text_present` | an overlay starting before 3 s (`extras.json` `overlays`) or caption words in the first 3 s (warn) | late first word, captions off | `overlay add-text <clip> --text "<hook>" --start 0 --end 3` when that command exists, else trim to speech / turn captions on |
| `duration_for_platform` | within the brief's min/max (`brief.json`), else the platform default: YouTube Shorts 60 s, TikTok/Reels 90 s (warn) | over-long cut; no brief says which platform | `edit trim` / `edit refine`; or set the platform in the brief |
| `contact_sheet` | always pass | — | **open the image** (next section) |

The face checks run the engine's `faces.track` on the **render** (2 fps). If the engine is not
available they report `status: warn` with `value: {"skipped": "engine unavailable"}`; they never fail
QC. Run `rustler doctor` and re-run `qc run` to get the real answer. Face checks only exist for layouts
where the face matters (center-crop, split-vertical top pane); `caption_over_face` only when captions
are on.

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

Fast path: let Rustler turn the QC report into commands.

```
rustler fix suggest clp_X        # ordered list: {check, severity, why, commands[], auto_safe}
rustler fix apply clp_X          # runs the auto_safe ones, re-renders, re-runs QC, lists what is left
rustler fix apply clp_X --check caption_over_face --rerender false   # one fix, no re-render yet
rustler qc compare clp_X rnd_OLD rnd_NEW   # side-by-side contact sheet + per-check diff; OPEN the image
```

`fix suggest` runs QC first when the render has no report. Failures come first (`severity: high`), then
warnings by impact; checks skipped for lack of the engine come last. `auto_safe: true` means the fix is
deterministic and non-creative: moving captions off a face or out of the UI zone, `edit remove-pauses`,
and a plain re-render (loudness). `fix apply` never touches creative choices (hook text, layout, cuts,
music, trims): those suggestions stay in `not_applied` with the exact commands for you to judge. After
`fix apply`, look at the new contact sheet (`after.look_at`) and compare with `qc compare`.

Manual loop:

```
qc run → read checks + contact sheet → list problems → one edit per problem →
render preview-frame at the problem time → render run → qc run → contact sheet again
```

| What you see | Edit |
|---|---|
| Face cut off / crop on the wrong person | `clip set-layout --params '{"focus_x":…}'` or `crop track-face`, or switch to `blur-fit` |
| Captions over the face | `caption_over_face` hint gives the exact `--position-y`; or `fix apply --check caption_over_face` |
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
