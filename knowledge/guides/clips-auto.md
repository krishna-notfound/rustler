---
id: clips-auto
title: "clips auto: the one-call pipeline, and when to do the steps yourself"
summary: "What clips auto runs (resolve with captions or a whisper fallback, rank with a first-2-seconds hook, create with smart defaults and a template, silence trims only (never zooms or grades), render batch with ETA, QC, cover, export with an empty post JSON), how to read its report, how to change each decision, and when the manual steps are the better choice."
order: 22
tags: [auto, pipeline, batch, quick, clips, rank, hook, retention, export, dry-run]
---

# clips auto

`rustler clips auto` chains commands that already exist. It does not replace your judgement: it makes a
first pass you then **read, look at and correct**. Every decision in its report comes with the command that
changes it, and you can do any step yourself at any time.

## What it runs

| Stage | Commands it calls |
|---|---|
| resolve (URL) | `source info` (one cached lookup), `source fetch-transcript` (captions, no download). **No captions:** it downloads the audio only (`source import <url> --audio-only true`) and transcribes it with whisper base (`analyze quick <src> --transcribe always --model base`) to rank; `source.transcript_via` says `captions` or `whisper`, `source.transcript_s` how long it took |
| resolve (source) | `analyze quick` when the source has no transcript |
| rank | `highlights candidates`, then a first-2-seconds hook score (question, emphasis word, number, "you", loud or fast opening; weak openers lose points). Picks never overlap each other or a clip you already made on that moment. **Intro trailer:** when the source opens with a montage of lines that are said again later (podcasts often do), moments inside it are not ranked and the originals are (a window holding the original of a trailer line gets +5 points); `ranking.intro_trailer` shows the range, each repeated line with where its original plays, and how many windows were skipped. `--include-intro true` ranks the trailer too; your own `ranges` are made as given, with a warning when one sits in the trailer |
| create | URL: `source import --sections` per pick (padded 3 s, 720p unless `--download-quality`, 2 sections at a time, progress `clips.auto.import` with `eta_s`) + `analyze quick --transcribe always` (whisper on the section: exact word timings and no caption markup; the fetched captions only when whisper is missing); then `clip create` (smart defaults, `--template`), `clip meta-set --campaign-id` when `--campaign` is given |
| fix | `clip retention --timeline-only`; applies only `trim_head`, `trim_tail`, `remove_pauses` (re-measured after each). Trims never cut into the first or last word (they stop 0.2 s from it); trims and pause cuts are **skipped** when the word timings are approximate (a captions-based transcript); a silence fix after which the hook or ending scores worse is restored (`rolled_back`). After the cuts, the punch-ins **your template** placed (an explicit punch-in pattern; the starters have none) are re-checked: one closer than 0.5 s to a cut, across one or on a graphic makes it re-plan the template's punch-ins with the template's own pattern (`fixes_applied` action `template_punch_ins`). Punch-ins, text pops and cutaways are listed in `fixes_suggested` with their commands, never applied |
| render | `render batch` (progress `clips.auto.render` with `eta_s`; the GPU encoder when `rustler doctor` lists a working one, else libx264) |
| check | `clip retention` on the render, `qc run`, `clip cover --auto` (best effort, about 3 s: it scores the QC contact sheet's 12 frames, leaves out frames where a mask is shown or the face is under a mask, card or text; `--cover false` skips it), `export clip` |

**clips auto never adds a zoom, a punch-in, a shake or a colour grade by itself.** The only zooms on its clips are
the ones in the template you passed; a grade only comes from your template's `grade`. Retention punch-ins are in
`fixes_suggested` with the exact `rustler effect add ...` command: run the ones you want.

The export writes `<name>.post.json` with **empty** title, description and hashtags: Rustler never writes copy
and never publishes. Exports whose only QC failure is `meta_present` (the copy) still go out, marked
`needs_copy: true`.

```
rustler clips auto --source-id src_X --dry-run                 # the plan: picks, why, commands; nothing written
rustler clips auto --source-id src_X --count 3 --template podcast-viral
rustler clips auto --url https://youtu.be/ID --count 5 --quality draft
rustler clips auto --source-id src_X --campaign spring          # campaign duration + platform fill the defaults
rustler clips auto --source-id src_X --input-json @ranges.json  # {"ranges":[{"start":61.2,"end":95.0,"note":"why"}]}
```

Defaults: `--count 5`, `--min-len 25`, `--max-len 60`, `--quality final`, fixes on (`--fixes false` turns them
off), smart defaults on (`--raw true` turns them off).

## Read the report

Per clip: `why` (candidate score, `hook_2s` with notes and the words said in the first 2 s, reasons),
`retention` (`before_fixes` and the latest `score`, `measured_on: render`), `fixes_applied` (each with a `status`:
`applied`, `no_change`, `skipped`, `rolled_back` or `failed`, `changed`, a `note` saying why, and an `undo` command),
`fixes_suggested` (always present: what the render measurement still suggests; a skipped or rolled-back fix says so in its `why`; plus, never applied: `bad_take` restarts and repeats with their `edit remove-words` command, 2-3 `hook_title` lines (≤ 7 words, with the `overlay add-text ... --style hook-big` command) and a `cold_open` (`edit cold-open` on the strongest answer line) when the clip opens on a question; a dry run lists the hook and cold-open suggestions with `<clip_id>`), `qc` (failed / warned checks), `export` (video, post, report) and `change[]`.
Names are `auto-<rank>-<time in the original video>-<source title slug>` with `-2`, `-3` ... when the name is
taken, so a run never replaces another clip's export; `export clip` itself refuses a name another clip exported
unless `--overwrite true`. A clip that failed has `failed_stage` (import, analyze, create, render, qc, export) and `error`; the other clips
are not affected.

Then, for every exported clip:

1. Open the video or `rustler see preview <clip_id>`; read the window with
   `rustler transcript read-window <source> --start <s> --end <e>`.
2. Write the copy: `rustler clip meta-context <clip_id>`, then `rustler clip meta-set <clip_id> --input-json @meta.json`,
   then `rustler export clip <clip_id>` to refresh the post JSON.
3. Record your own score: `rustler clip score-set <clip_id> --hook H --flow F --value V --trend T --notes "why"`.
4. Add what only you can judge: a hook title (`rustler overlay add-text`; the `hook_title` suggestions are a
   starting point, rewrite them), a cold open, the bad-take cuts you agree with, text pops and cutaways from
   `fixes_suggested`.

## Change a decision

| Decision | Command |
|---|---|
| the range | `rustler edit set-range <clip_id> --start-unit A --end-unit B` |
| a different moment | `rustler highlights candidates <source> --count 20`, then its `create_cmd` |
| smart defaults | the `undo_all` command in `change[]` (`rustler history restore <clip_id> --revision 1`) |
| the look | `rustler template apply <name> --clip-ids <clip_id>` |
| a suggested punch-in | run its command from `fixes_suggested` yourself (`rustler effect add <clip_id> punch-in ...`) |
| a restart / repeat | read `rustler edit find-bad-takes <clip_id>`, run the `edit remove-words` command you agree with; `rustler history undo <clip_id>` takes it back |
| hook title, cold open | rewrite a `hook_title` line and run `rustler overlay add-text <clip_id> "..." --style hook-big`; `rustler edit cold-open <clip_id> --start S --end E` |
| the intro-trailer skip | `rustler clips auto ... --include-intro true`, or `rustler highlights candidates <source> --include-intro true` |
| the template punch-in re-plan | the `undo` in `fixes_applied` (`rustler history restore <clip_id> <revision before it>`) |
| a trim / pause cut it made | the `undo` in `fixes_applied` (`rustler history restore <clip_id> <revision before it>`) |
| the cover frame | `rustler clip cover <clip_id> --out-t <seconds>`, then `rustler export clip <clip_id>` |
| re-render and re-check | `rustler render run <clip_id>`, `rustler qc run <clip_id>`, `rustler export clip <clip_id>` |

## When to use it, when to do it yourself

Use `clips auto` for a fast first pass: many clips from one long video, a campaign that pays per view (volume
matters), or to see quickly what the signals pick before you read the transcript.

Do the steps yourself (see `workflow-youtube-longform` and `finding-hooks`) when:

- the moment depends on meaning, humour or context the signals cannot see (the ranking is deterministic and
  reads no meaning);
- the clip needs a cold open, a merge, cutaways or a custom hook (`edit cold-open`, `clip merge`, `cutaway add`);
- the campaign pays per clip and must match a creator's style exactly (`reference analyze`, then a template);
- the video has no captions and is long: the whisper fallback downloads and transcribes the whole audio first
  (about 1/6 of the video's length on a GPU); for a 2-hour video import the sections you want yourself.

A middle way: pick the moments yourself and let it do the rest with `ranges`.
