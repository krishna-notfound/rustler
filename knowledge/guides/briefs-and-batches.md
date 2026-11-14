---
id: briefs-and-batches
title: "Briefs, variants, batch render/export and project status"
summary: "Save the job brief once and check every clip against it, draft the post text, make and compare variants, render and export many clips with QC and brief gates, track project status, pack a project."
order: 16
tags: [brief, variants, batch, export, compliance, project-status, pack, campaign]
---

# Briefs, variants, batches

Use this when a job has rules (campaign, client, platform) or when you are producing more than a
couple of clips. A single casual clip does not need a brief.

## 1. The brief

The brief is **one file per project**: `<project>/brief.json`. Rustler stores and checks it; it never
interprets prose. You turn the human's brief into fields (see the `batch_from_brief` prompt).

```
rustler brief set --creator "Jane Doe" --campaign "Spring launch" --platform shorts \
    --min-duration 20 --max-duration 58 --aspect 9:16 --caption-style hormozi-pop \
    --credit-text "Clip from @janedoe" --watermark-text "@clipper" \
    --banned-words '["sponsor","crypto"]' --required-text '["link in bio"]' \
    --required-hashtags '["#ad"]' --required-mentions '["@janedoe"]' \
    --payout "per view" --notes "no politics"
rustler brief show
rustler brief clear                       # destructive: needs --yes
```

- `brief set` **merges** into what is saved; a list you pass replaces that list; `--replace true` starts
  over. `platform`: `tiktok | reels | shorts | all`. `layout` and `caption-style` pin a preset for every clip.
- `credit-text` and `watermark-text` are **on-screen**: an overlay must contain them
  (`overlay add-text clp_X "Clip from @janedoe" --style lower-third`).
- `required-hashtags` and `required-mentions` are **post text**, not video: a leading `#`/`@` is added
  when missing.
- `required-text` must appear in captions, overlays or speech; `banned-words` must appear in none of
  them (whole-word, case/accent/punctuation-insensitive, over the transcript, the romanised transcript,
  caption words and overlays).

## 2. Check every clip

```
rustler brief check clp_X
```

Returns `status` (`pass | warn | fail`) and one entry per rule: duration (the transition overlap is
already subtracted), aspect, caption style, layout, required text, banned words, credit/watermark
overlay, and QC (needs a render and its QC report; runs nothing itself). Each failing rule carries the
command that fixes it; `post_items` lists what goes into the post description (hashtags, mentions).
Saved as `clips/<id>/brief_check.json`. Fix, re-render, re-check until the status is not `fail`.

```
rustler brief description clp_X          # hook line from the transcript, credit, mentions, required hashtags
```

The draft is deterministic: **rewrite it** into a title (≤ 60 chars, about this moment), a 1-2 sentence
description and 3-5 hashtags (required ones first). See `platform-specs`.

## 3. Variants: when there is a real question

```
rustler variant create clp_X --caption-styles '["hormozi-pop","clean-white"]' --layouts '["center-crop","blur-fit"]'
rustler variant create clp_X --hooks '["He quit his job for this","Nobody tells you this"]'
rustler variant list clp_X
rustler variant compare --clip-ids '["clp_X","clp_V1","clp_V2","clp_V3"]' --out-t 1.0
```

- One clone per **combination** of caption style × layout × hook title (names `<name>-v1`, `-v2`, …);
  everything else (cuts, overlays, audio, effects) is copied. Layouts may be built-ins or saved presets.
  Layouts are checked on the base first, so a bad one creates nothing. Hooks are `hook-top` overlays.
- `variant compare` composes **draft preview frames of 2 to 12 clips at the same output time** into one
  labelled grid image. **Open the image** and choose; then render and QC only the winner (and maybe the
  runner-up). Do not render every variant.
- Make variants only for clips you believe score ≥ 85 and have a real open question (two hooks, two
  layouts). Four variants of a weak clip are still a weak clip. Report which variant you recommend and why.

## 4. Render and export in batches

```
rustler render batch --all true --quality draft                 # a first look at everything
rustler render batch --clip-ids '["clp_A","clp_B"]' --parallel 2 --only-missing true
rustler export batch --all true
```

- `render batch` runs with bounded concurrency (default 1; capped at 2 with NVENC, else 4). **A
  failing clip never stops the others**; the result lists successes and failures. `--only-missing true`
  skips clips whose latest render of that quality is newer than their last edit. Use `draft` to
  iterate, `final` (default) for delivery.
- `export batch` runs QC where it is missing and the brief check when the project has a brief, exports
  only the **passing** clips to `exports/` and lists every skipped clip with the reason and the fix:
  no render, a stale render (edited after rendering; `--allow-stale true` overrides), failing QC
  (`--require-qc false`), failing brief (`--require-brief false`). Do not turn the gates off to
  "get the count"; fix the clips. It is the batch version of `export clip`.
- Single clip: `export clip clp_X --name <brief-slug>-<nn>-<short-title>`. The report embeds the QC
  result and the brief status.
- Neither batch command replaces **looking**: open each contact sheet (`qc run` path) before you
  report a clip as done.

## 5. Project status and handing over

```
rustler project status            # per source and per clip
rustler project show              # sources, counts, suggested next command
```

`project status` is the "where am I" call, useful at the start of a session and before reporting: per
source it shows transcript words, units and whether `analyze deep` ran; per clip the revision, the last
render and whether it is **stale** (edited after rendering, with the reason), QC result, brief check
result, whether it was exported, plus suggested next commands.

```
rustler project pack                                   # ../<project>.rustler.zip: json, analysis, clips, layouts, assets, exports
rustler project pack D:/share/ep212.rustler.zip --include-media true --include-renders false
rustler project unpack D:/share/ep212.rustler.zip D:/work/ep212
rustler project list                                   # recent projects
rustler project open D:/work/ep212
```

By default media is **not** packed (small file; the receiver re-imports the sources, or pass
`--include-media true`). Unpack fixes media paths and registers the project; it refuses to overwrite
an existing project unless `--force true`.

## 6. Report

For every batch: the brief checklist with met/not-met, per clip the file, source + units, duration,
score, QC and what you saw on the contact sheet, title, description, hashtags; rejected candidates;
shortfall against the requested count and why. Template in `platform-specs`. Never upload.
