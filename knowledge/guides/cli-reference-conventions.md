---
id: cli-reference-conventions
title: "CLI and MCP conventions"
summary: "Command grammar, flags, --input-json, global flags, the JSON envelope, exit codes, progress on stderr, MCP tool naming, images for vision agents, and the complete command map by area."
order: 13
tags: [cli, mcp, reference, json, conventions, commands]
---

# CLI and MCP conventions

One command registry drives everything: the CLI, the MCP server and the GUI call the same commands
with the same JSON inputs. If you can do it in one, you can do it in the others.

## 1. Grammar

```
rustler <group> <action> [positional…] [--key value]… [--input-json <json | @file>] [global flags]
```

- Command `group.action` ↔ CLI `rustler group action`. Multi-word actions use `-` on the CLI and `_`
  in names: `rustler source fetch-transcript` = `source.fetch_transcript`.
- Flags are kebab-case and map to snake_case fields: `--start-unit 12` → `"start_unit": 12`.
- Values are parsed as JSON when they parse (numbers, `true/false`, arrays, objects), else strings:
  `--captions false`, `--layout-params '{"focus_x":0.4}'`.
- A flag with no value is `true`: `--yes`.
- Repeating a flag makes an array: `--sections 1:00-2:00 --sections 5:00-6:00`.
- Positionals fill the command's positional fields in order (`rustler render run clp_X` sets
  `clip_id`; `rustler transition set clp_X seg_A dissolve` sets `clip_id`, `after_segment_id`, `kind`).
  Every positional can also be given as a flag (`--clip-id clp_X`). `commands describe` prints the
  exact `usage` line, positional order included.
- `--input-json '{"clip_id":"clp_X","preset":"blur-fit"}'` or `--input-json @input.json` supplies the
  whole input; explicit flags override its fields. Use it for complex inputs or awkward quoting
  (see the Windows note below).

**Windows PowerShell 5.1:** it strips the double quotes inside arguments passed to native programs, so
`--layout-params '{"top":...}'` arrives as `{top:...}` and fails validation. Write the JSON to a file and
use `--input-json @file.json`, or escape the quotes as `\"` (`'{\"focus_x\":0.4}'`). bash, zsh and cmd
are unaffected.

## 2. Global flags (anywhere on the line)

| Flag | Effect |
|---|---|
| `--project <dir>` | project folder (else `RUSTLER_PROJECT`, else current dir if it has `project.json`) |
| `--dry-run` | validate and show what would change (`{"dry_run":true,"would":…}`); writes nothing |
| `--yes` | confirm destructive actions; don't use it to skip a failed QC |
| `--max-seconds N` | stop long work after N s, save progress; rerun to continue |
| `--actor agent\|human` | who is acting (default agent); recorded in the activity log |
| `--pretty` | indented JSON |
| `-q` | quieter stderr |

## 3. Output

stdout: **exactly one JSON envelope** per call.

```json
{"ok": true, "schema": "rustler.source.import@1", "data": {"sources": [ ... ]}, "warnings": []}
{"ok": false, "error": {"code": "not_found", "stage": "clip.show", "message": "clip clp_x not found",
 "hint": "run `rustler clip list`", "retryable": false}}
```

stderr: logs and NDJSON progress, e.g. `{"event":"progress","stage":"transcribe","pct":42.0}`.
Parse stdout only; never parse stderr for results. Many results carry `data.next`: suggested next
commands.

Exit codes: 0 ok · 2 usage/validation · 3 not found · 4 dependency missing · 5 provider/network ·
6 media/ffmpeg · 7 QC failed · 8 cancelled · 9 revision conflict · 10 internal
(details in `troubleshooting`).

## 4. IDs, times and paths

- IDs: `prj_…`, `src_…`, `clp_…`, `seg_…`, `rnd_…`, `ovl_N`, `fx_N`, `sfx_N` (prefix + ULID or counter).
  Copy them from outputs.
- Times are seconds (float) in **source time**, unless the field starts with `out_` or is documented as
  an edit-timeline time (`edit split <clip> <out_t>`, `overlay --start/--end`, `effect --start/--end`,
  `audio add-sfx --at`): those are **output (clip) time** as printed by `clip show`. Transcript lines print
  `HH:MM:SS.s` for reading; commands take seconds.
- Section sources start near 0; `original_offset` maps back to the original URL's timeline.
- Paths inside the project are stored relative to it, so a project folder can be copied (or
  `project pack`ed) to another machine and continued.
- Audio arguments take any **media reference**: a path, `asset:<id>`, an `http(s)` link (downloaded into
  `<project>/assets/_cache/`, needs the project's network+downloads permission) or
  `source:<source_id>[@start-end]` (the audio of an imported video; seconds or MM:SS) in `audio add-music --path`,
  `audio add-sfx <file>` and `asset add`. See the `assets` and `audio` guides.

## 5. Discovery

```
rustler commands list                    # {"commands":[{name, summary, annotations, cli}]}
rustler commands describe clip.create    # usage, flags (positional, required), input/output schema, examples
rustler help                             # list + the start-here chain
rustler guide list / guide show <id>     # knowledge guides; guide search "<words>"
rustler prompt list / prompt show <name> --args '{"clip_id":"clp_X"}'   # ready workflows
```

Annotations: `read_only`, `destructive` (needs `--yes`), `network`, `costs_money` (always false today:
Rustler calls no paid APIs), `long_running` (use `--max-seconds`).

## 6. MCP

Start: `rustler mcp serve` (stdio). Then:

- **Tools** = registry commands named `group_action`: `source_import`, `analyze_quick`,
  `transcript_get`, `see_tiles`, `clip_create`, `render_run`, `qc_run`, `export_clip`, … Input is the
  same JSON as `--input-json`. Global options like `project`, `dry_run`, `max_seconds` are passed as
  described in each tool's schema.
- **Images**: `see_frame`, `see_tiles`, `render_preview_frame` and the QC contact sheet come back as
  MCP image content — look at them with your vision. On the CLI the same commands return a file path
  (`data.image`); open that file. Other commands that produce a picture (`variant compare`,
  `qc compare`, `asset show` for fonts and caption styles) return a file path too.
- **Prompts** = ready workflows: `make_viral_clips`, `clip_from_stream`, `streamer_split_clip`,
  `quick_scan_source`, `polish_clip`, `review_clip`, `fix_qc_failures`, `restyle_clip`,
  `batch_from_brief`, `hinglish_podcast_clips`, `direct_campaign`, `hook_lab`, `learn_style_from_folder`,
  `write_clip_meta`, `apply_template_batch`.
- **Guides** (this knowledge book) are exposed by the server too (`rustler://guides/<id>`); read
  `start-here` first.

## 7. Command map (all 179 registered commands)

Every registered command, by area. `<x>` = positional, `--x` = flag, `*` = required, `?` = optional.
`<clip>` = clip id, `<src>` = source id. Only the useful arguments are listed:
`commands describe <name>` prints all of them with examples. `rustler commands list` is the
authority; if it shows a command that is missing here, trust it.

Commands that change a clip save a new revision (`history undo` reverts); pass `--expect-revision N`
(or `--base-revision N` on layout/caption/clip commands) to guard against concurrent edits.

### Setup, project, discovery

| Command | What it does |
|---|---|
| `setup.run {profile?, with?, gpu?, whisper_models?, models?, assets?, force?}` | install ffmpeg, yt-dlp, uv, the **lite** engine (default: whisper `base`, no heavy analyzers), fonts and built-in styles into `~/.rustler`. `--with ocr,audio-events,speakers,cuda,sfx-pack` adds optional pieces; `--profile full` adds all of them. Never removes anything already installed |
| `doctor.run` | tools, encoders, GPU, fonts, engine, install `profile` (lite/full) and `optional_features[]` (each with `available` and the exact `enable` command); `ready=true` means you can render |
| `project.create <name> --dir` | new project folder |
| `project.show` | project, sources, counts and the suggested next command |
| `project.status` | one glance: per source (words, units, deep analysis) and per clip (revision, render, QC, brief) |
| `project.list {prune_missing?}` / `project.open <dir>` | recent projects / make one the most recent |
| `project.pack <out>? {include_media?, include_renders?, include_analysis?}` | zip the project (no media by default) |
| `project.unpack <archive> <dir>` | extract a pack, fix media paths, register it |
| `project.set_permissions {network?, downloads?, fs_roots?, cookies_file?}` | what the project may download/read |
| `project.set_settings {caption_script?, default_whisper_model?, qc_faces?}` | `latin` / `devanagari` / `as_transcribed` captions; default whisper model (`base`); run the engine face checks in every `qc run` (off by default) |
| `config.list`, `config.get {key?, scope?}`, `config.set <key> <value> {scope?}`, `config.unset <key> {scope?}` | settings with their source (project, global, default), allowed values and meaning: smart defaults on/off, the default template, QC faces, render quality and so on. Project scope (`<project>/config.json`) wins over global (`<home>/config.json`); an explicit command flag always wins over a setting |
| `guide.list`, `guide.show <id>`, `guide.search <query>` | this knowledge book |
| `prompt.list`, `prompt.show <name> --args` | ready-made workflows |

CLI-only helpers (not registry commands): `rustler help`, `rustler commands list|describe`,
`rustler mcp serve`, `rustler gui`.

### Sources and understanding

| Command | What it does |
|---|---|
| `source.info <url>` (or `--source-id`) | title, chapters, most-replayed peaks, caption languages; no download |
| `source.fetch_transcript <url> {lang?}` | existing captions as a pending transcript, no download |
| `source.search <query> {limit?}` | public YouTube search |
| `source.import <input> {sections?, name?}` | local file or URL (`--sections` downloads only those windows); proxy + wav |
| `source.list`, `source.show <src>` | sources; probe and which analyses exist |
| `transcript.import <target: src or url> {file?, text?, format?, language?, times?, replace?}` | import a transcript you already have (VTT/SRT, YouTube json3, words JSON, or `[mm:ss] text` lines) and skip yt-dlp and whisper |
| `reference.import <name> {folder? \| files?, recursive?}` | make a style-reference set from example shorts (proxies and audio; originals untouched) |
| `reference.analyze <name>`, `reference.list`, `reference.show <name> {video?}` | measure pacing, captions, hook text, zoom, layout, loudness; contact sheets and first-3-second filmstrips to LOOK at (guide `learn-style-from-videos`) |
| `analyze.quick <src> {transcribe?: auto\|always\|never}` | light first look: loudness + scenes (on the proxy) + transcript + highlight signals. `auto` reuses captions fetched for a URL source instead of running whisper; `never` skips the transcript; no OCR/events/faces/speakers |
| `analyze.transcribe <src> {model?, language?, device?, force?}` | transcription alone (resumable) |
| `analyze.scenes <src>`, `analyze.audio <src>` | scene cuts; loudness curve, peaks, silences |
| `analyze.deep <src> {every?, fps?}` | on-screen text + audio events + faces + plain-language summary |
| `analyze.ocr <src>`, `analyze.events <src>`, `analyze.faces <src>` | the three parts of `analyze deep`, one at a time |
| `transcript.get <src> {start?, end?, offset?, limit?, format?}` | numbered units; pick unit ids |
| `transcript.search <src> <query> {limit?}` | phrase search with neighbouring units |
| `transcript.read_window <src> <start> <end>` | readable text of a time window with unit ids |
| `highlights.signals <src> {window?, limit?, offset?}` | per-window loudness peak, speech rate, questions, emphasis words, pause before, scene cuts, heatmap |
| `highlights.candidates <src> {count?, min_len?, max_len?, platform?, include_intro?}` | ranked sentence-aligned windows with unit ids, reasons, breakdown and a ready `create_cmd`; skips the source's intro trailer (reported in `intro_trailer`) unless `include_intro` |
| `see.frame <src> --t {width?}` (or `--clip-id --out-t`) | one jpg: **look at it** |
| `see.tiles <src> {around? \| unit? \| start?/end?, window?, count?, cols?}` | labelled contact sheet of frames |
| `see.preview <src> --start --end` | low-res mp4 of a source range (max 60 s) |

### Clips and cutting

| Command | What it does |
|---|---|
| `clip.create --name* --source-id* {start-unit, end-unit, expect-text, start, end, layout, layout-params, caption-style, captions}` | create a clip from unit ids (+ `--expect-text`) |
| `clip.add_segment <clip> --source-id* {start-unit, end-unit, start, end, expect-text, layout}` | append another part |
| `clip.list`, `clip.show <clip>` | clips; one clip with timeline, extras, renders |
| `edit.set_range <clip> {start-unit, end-unit, expect-text, segment-id}` | re-cut a segment by unit ids |
| `edit.trim <clip> {start-delta, end-delta, start, end, snap, segment-id}` | nudge or set segment ends (optional word snap) |
| `edit.refine <clip> {max-shift?, segment-id?}` | skip weak openers, end on a sentence end, land in silence |
| `edit.split <clip> <out_t>` | split the segment at an output time |
| `edit.delete_segment <clip> <seg>`, `edit.move_segment <clip> <seg> <to_index>` | remove / reorder segments |
| `edit.remove_pauses <clip> {min-gap?, keep?}` | cut long word gaps |
| `edit.remove_words <clip> {words?, word-indices?}` | cut fillers (um, uh, hmm…) by default, or listed words (neighbouring words as one span) |
| `edit.find_bad_takes <clip> {min-confidence?}` | read-only: restarts, stutters, false starts, repeated sentences with times, confidence and the `edit remove-words` command that keeps the cleaner take (exact word timings only) |
| `edit.insert_segment <clip> --source-id* {at-index \| at-out-t, start-unit, end-unit, start, end, expect-text, layout, base-revision}` | insert a range anywhere (at an out time the segment is split); layers move with their content |
| `edit.cold_open <clip> {start-unit, end-unit \| start, end, source-id?, expect-text?, flash? (default true), base-revision?}` | hook range first (cut out of where it was) + white flash at the cut |
| `clip.merge <clip>... --name* {transition?, transition-duration?}` | new clip = clips back to back with all their layers (times shifted, word anchors kept) |
| `clip.duplicate <clip> --name*` | copy a clip with every layer |
| `edit.speed <clip> --segment-id* --factor* {base-revision?}` | segment speed 0.5-2.0 (1 resets); captions/layers follow |
| `edit.freeze <clip> --out-t* --duration* {base-revision?}` | hold the frame at an edit time (silent) |
| `history.list <clip>`, `history.undo <clip>`, `history.restore <clip> <revision>` | revisions; undo/restore make a NEW revision and bring back the layers (extras) of that revision too |

`clip.create` also takes `--raw` (skip the smart defaults), `--template <name[@ver]>` (apply a template right after the
smart defaults) and reports `smart.decisions[]` (each automatic layout, framing, filler and pause decision with the
command that overrides it, plus `smart.undo_all`). Settings `clip.smart_defaults` and `template.default` change those defaults.

### Timeline document (whole clip)

| Command | What it does |
|---|---|
| `timeline.show <clip>` | the clip as one `rustler.timeline@2` document (every track: video, freezes, cutaways, images, masks, text, captions, audio, effects, transitions, progress bar, meta, extra) + `resolved` edit/rendered times of every item + `revision` |
| `timeline.schema` | JSON Schema of the document + the rules (edit-timeline times, anchors, rects, ids, patch paths) |
| `timeline.apply <clip> --base-revision* {patch \| document, dry-run?}` | JSON-Patch ops or a whole document, validated (errors list their paths), one new revision; stale base -> `revision_conflict`. Use `--input-json @patch.json` |

### Layout and framing

| Command | What it does |
|---|---|
| `clip.set_layout <clip> --preset* {params?, segment-id?}` | `center-crop`, `blur-fit`, `split-vertical` (or a saved custom preset via `layout apply-preset`) |
| `layout.list_presets`, `layout.presets` | built-in presets and the params each reads; `presets` also lists saved custom ones |
| `layout.detect_webcam <src> {clip-id?, samples?, at?}` | find a streamer webcam, return a ready split layout |
| `layout.show <clip> {segment-id?}` | regions with normalised and pixel rects, fit, z, warnings |
| `layout.from_frame_boxes <clip> --boxes* {split?, main-fit?}` | build a split or inset layout from rects drawn on a frame |
| `layout.edit_region <clip> <region> {src?, dst?, fit?, z?, nudge?}` | move, resize or nudge one region |
| `layout.add_region <clip> --src* --dst* {fit?, z?, id?}` | add a region (picture-in-picture) |
| `layout.remove_region <clip> <region>`, `layout.reorder <clip> --region-ids*` | remove; back-to-front draw order |
| `layout.save_preset <name> <from_clip>`, `layout.apply_preset <clip> <name>`, `layout.delete_preset <name>` | reuse a layout across clips |
| `crop.track_face <clip> {segment-id?, crop-w?}`, `crop.clear <clip>` | follow the speaker's face (center-crop segments) / undo |
| `crop.track_active <clip> {segment-id?, style?, switch?, look_room?}` | light active speaker (no diarization): the crop follows whoever's mouth moves while somebody speaks, smooth camera, reports the turns |
| `layout.plan <clip> {apply?}` | per-shot layout plan (one face: center-crop + smooth track; two or more: active speaker; no face or graphics: blur-fit or zoom-fit); proposes only, `--apply` saves it |

### Captions and text on screen

| Command | What it does |
|---|---|
| `caption.set <clip> {style?, enabled?, position-y?, max-words-per-line?, script?}` | caption settings (`clip.set_captions` is an alias) |
| `caption.list_styles`, `caption.styles` | preset names; presets + custom styles with scope, font, active-word kind |
| `caption.create_style <name> {base?, font-family?, primary?, active-color?, active-kind?, box-color?, outline?, size-factor?, scope?}` | custom style saved as an asset |
| `caption.export_srt <clip>` | `clips/<id>/captions.srt` |
| `caption.keywords <clip> {words? \| auto?, max?, color?}` | mark the keywords the `keyword-highlight` preset colours and enlarges (`--auto` proposes numbers, emphasis words, names; `--dry-run` previews) |
| `caption.word <clip> {source-id + word-i \| match, color?, scale?, emoji?, hide?}`, `caption.words_list <clip>`, `caption.words_clear <clip>` | style, list or clear single caption words (colour, size, emoji after the word for `emoji-karaoke`, hide); they follow re-cuts |
| `text.create_style <name> --font --size --color {...}`, `text.styles` | save / list text looks (font, size, colour, outline, box, shadow) for cards and overlays |
| `overlay.add_text <clip> <text> {style?, start?, end?, position-y?, box?, anim?, sfx?, sfx-volume-db?}` | hook title / lower third / plain text (`hook-top`, `lower-third`, `plain`, and the viral hook styles below); `--anim none\|pop\|zoom-in\|slide\|typewriter`, `--sfx` adds a whoosh at the hook's start |
| `overlay.hook_styles` | the viral hook styles `hook-big`, `hook-box-yellow`, `hook-outline`, `hook-pov`, `hook-question` with their entry animation, position and size |
| `overlay.edit <clip> <overlay> {text?, start?, end?, style?, ...}` | change one overlay |
| `overlay.list <clip>`, `overlay.remove <clip> <overlay>` | list with wrapped lines / remove |
| `overlay.progress_bar <clip> --enabled* {color?, height?, position?}` | bar that fills the width over the clip |

### Transitions and effects

| Command | What it does |
|---|---|
| `transition.set <clip> <after_segment_id> <kind> {duration?}` | `cut`, `fade`, `dissolve`, `wipeleft`, `wiperight`, `slideup`, `zoomin`, plus the pacing kinds `flash` (white, 0.1 s), `flashblack` (0.1 s) and `whip` (0.2 s) (frame-exact, shortens the clip by the overlap) |
| `transition.list <clip>`, `transition.clear <clip> {after-segment-id?}` | list with rendered windows and edit vs rendered duration / remove |
| `effect.add <clip> <kind> --start* --end* {amount?}` | `punch-in`, `zoom-slow`, `shake`, `flash` |
| `effect.list <clip>`, `effect.remove <clip> <effect_id>` | list (also shows the progress bar) / remove |

### Audio

| Command | What it does |
|---|---|
| `audio.add_music <clip> --path* {volume-db?, duck?, fade-in?, fade-out?}` | music bed ducked under speech; `--path asset:<id>` works |
| `audio.remove_music <clip>` | remove it |
| `audio.add_sfx <clip> <file> {at? \| at-word?, volume-db?}` | sound effect at a time or anchored to a transcript word; `asset:<id>` works |
| `audio.remove_sfx <clip> <sfx_id>`, `audio.list <clip>` | remove; list music, sfx (with current play times), denoise, volumes |
| `audio.denoise <clip> {strength?}` | `light`, `medium`, `strong`, `off` |
| `audio.volume <clip> --db* {segment-id?}` | gain for one segment, or the whole dialogue |

### Render, QC, fixes, export

| Command | What it does |
|---|---|
| `render.preview_frame <clip> --out-t*` | one composed frame (layout, captions, overlays, effects): look before the full render |
| `render.run <clip> {quality?, prefer-gpu?, two-pass-loudness?}` | one encode, 1080x1920 mp4 (`draft` or `final`) |
| `qc.run <clip> {render-id?, allow-fail?, faces?}` | fast checks + contact sheet (look at it); exit 7 when failed. `--faces` (or project `qc_faces`) adds the engine face checks, which are `pass` with `{"skipped": "off by default ..."}` otherwise |
| `fix.suggest <clip>` | QC report → ordered fixes with exact commands and an `auto_safe` flag |
| `fix.apply <clip> {check?, rerender?}` | apply the `auto_safe` fixes, re-render, re-run QC |
| `qc.compare <clip> <render_a> <render_b>` | side-by-side contact sheet + per-check diff |
| `export.clip <clip> {name?, render-id?, allow-fail?}` | copy a passing render + report + srt to `exports/` |

### Layers: images, B-roll, masks

Each has `add`, `edit`, `list`, `remove` (all take `<clip>`; times in seconds or anchored to transcript words with
`--start-word` / `--end-word`; `--base-revision N` guards concurrent edits). Layers follow their content on re-cuts.

| Command | What it does |
|---|---|
| `image.add <clip> --rect* {asset? \| file? \| url?, start?, end?, opacity?, z?, anim-in?, above-captions?}` | logo, sticker, PNG card or GIF in a canvas rect (animations fade, pop, slide_up) |
| `cutaway.add <clip> --mode* {source-id? \| asset? \| file? \| url?, src-start?, start?, end?, rect?, mute?}` | B-roll: another source, asset, file or URL, full-frame or picture-in-picture (speech keeps playing) |
| `mask.add <clip> --kind* --rect* {start?, end?, strength?, color?}` | hide part of the picture (pop-up, logo, burned-in subtitles) with a blur, solid box or pixelation |
| `image.edit`, `image.list`, `image.remove`, `cutaway.edit`, `cutaway.list`, `cutaway.remove`, `mask.edit`, `mask.list`, `mask.remove` | change / list with resolved and rendered times / remove |
| `asset.render_card <name> --spec*` | design a graphic (title card, badge, CTA, logo plate) from a JSON spec; saved as an image asset and shown to you; place it with `image.add` |

### Templates

| Command | What it does |
|---|---|
| `template.list`, `template.show <name[@ver]>` | project, global and built-in templates (`podcast-viral`, `streamer-split`); the JSON, versions, what it needs and what is missing here |
| `template.create <name> {from-clip? \| template?, include-text?, description?, scope?}` | save a clip's look (captions, layout, hook look, text looks, brand images, progress bar, audio, sfx/effect patterns, transitions) as version 1 |
| `template.update <name> {from-clip? \| template? \| patch?, description?}` | a new version (old versions stay); updating a starter saves your own shadowing version |
| `template.apply <name[@ver]> {clip-ids? \| all?, overrides?, dry-run?}` | restyle clips through `timeline.apply`: one undoable revision per clip, every clip validated first, re-applying is a no-op, a missing asset or font fails with `not_found` naming it |
| `template.diff <a> <b>`, `template.delete <name[@ver]>` | what differs / remove (starters cannot be deleted) |
| `template.export <name> --out*`, `template.import --file* {scope?}` | zip with the template and every font, style, image, music and sfx it needs / install it on another machine |

### Retention, cover, post copy and campaigns

| Command | What it does |
|---|---|
| `clip.retention <clip>` | retention map of the output (per-second visual change, speech, silence, loudness), first-frame hook, dead zones with exact fix commands, bad takes (`take_fixes`), ending type, 0-100 score; measured on the latest render when there is one |
| `clip.score_set <clip> <hook> <flow> <value> <trend> {notes?}` | record your 4-part score (0-25 each) for the report |
| `clip.cover <clip> {auto? \| out-t?}` | pick the cover frame of the latest render (sharp, face visible, not on a cut), save `clips/<id>/cover.jpg`, look at it; `export.clip` copies it as `<name>.cover.jpg` and names it in the post JSON |
| `clip.meta_context <clip>` | what you need to WRITE the posting copy: spoken words, hook, platform limits, campaign rules with a live check, brief, saved copy |
| `clip.meta_set <clip> --platforms* {campaign-id?}` | save the title, description, hashtags and mentions you wrote per platform (`shorts`, `reels`, `tiktok`); `export.clip` writes `<name>.post.json`; QC checks `meta_present` and `campaign_compliant` |
| `campaign.create <id> {name?, platform?, payout?, duration?, hashtags-required?, mentions-required?, rules?}` | one job's rules: machine rules Rustler checks and agent rules you judge |
| `campaign.list`, `campaign.show <id>`, `campaign.delete <id>` | list / show every requirement / delete |
| `campaign.analyze {campaign-id?}` | hard rules, duration window, payout model to strategy, recommended clips per source, checklist (reads `brief.json` when no campaign is named) |
| `campaign.mark <clip> <rule_id> --pass* --reason*` | record the outcome of an agent-checked rule (what you saw) |

Guides: `director-retention`, `campaigns-and-posting`, `templates-and-brand`, `editing-toolkit`.

### One-call pipeline

| Command | What it does |
|---|---|
| `clips.auto {source-id? \| url?, count?, min-len?, max-len?, platform?, template?, campaign?, ranges?, quality?, fixes?, raw?, cover?, include-intro?, pad?, dry-run?}` | picks moments (hook score for the first 2 s; skips the source's intro trailer), lists hook lines, a cold open and bad takes as suggestions, creates clips with smart defaults and a template, applies cheap retention fixes, renders, runs QC, picks the cover and exports with an EMPTY post JSON (you write the copy with `clip.meta_set`); never publishes. `--dry-run` shows the plan. Guide `clips-auto` |

### Briefs, variants, batches

| Command | What it does |
|---|---|
| `brief.set {platform, min-duration, max-duration, aspect, caption-style, layout, required-text, banned-words, credit-text, watermark-text, required-hashtags, required-mentions, creator, campaign, payout, notes, replace?}` | save the job rules in `brief.json` (merges; lists replace) |
| `brief.show`, `brief.clear` | read / delete it |
| `brief.check <clip>` | pass/warn/fail per rule with fix commands |
| `brief.description <clip>` | draft post text: hook line, credit, mentions, required hashtags |
| `variant.create <clip> {caption-styles?, layouts?, hooks?}` | clone per combination |
| `variant.list <clip>`, `variant.compare --clip-ids* {out-t?, cols?}` | list; labelled comparison grid (open the image) |
| `render.batch {all? \| clip-ids?, quality?, parallel?, only-missing?}` | render many; a failing clip never stops the rest |
| `export.batch {all? \| clip-ids?, require-qc?, require-brief?, allow-stale?}` | export the passing clips, list the skipped with reasons |

### Assets

| Command | What it does |
|---|---|
| `asset.list {kind?, tag?}`, `asset.search <query>`, `asset.show <id>` | fonts, music, sfx, images, caption styles, layouts with licenses; `show` makes a preview |
| `asset.add <path>\|--url <link>\|source:<id>@a-b --kind* {license?, name?, tags?, scope?, source-url?, attribution?}` | add anything: a file, a direct link, or the audio of an imported video; no license needed (recorded as "unspecified (added by user/agent)" with a warning) |
| `asset.remove <id>` | remove (built-ins cannot be removed) |
| `asset.render_card <name> --spec*` | design a card or badge from a JSON spec (see Layers above) |

### Speakers

| Command | What it does |
|---|---|
| `analyze.speakers <src> {num-speakers?, min-speakers?, max-speakers?}` | local diarization: who speaks when; labels transcript words S1, S2…; talk time and turns |
| `speaker.rename <src> <speaker> <name>` | display name for a speaker id (empty resets) |
| `caption.speaker_colors <clip> --enabled* {colors?}` | colour captions per speaker (`{"S1":"#FFE000"}`) |
| `crop.track_speaker <clip> {segment-id?}` | center-crop follows the ACTIVE speaker's face, per turn |

Guides: `captions-and-style` (colours), `layouts-and-framing` (track-speaker), `streamer-clips`.

## 8. Good habits

- `commands describe` before first use of any command in a session.
- `--dry-run` before anything destructive or expensive; `project status` and `clip show` to re-orient.
- Keep IDs and decisions in your notes; the project's `events.ndjson` records every action, so
  another agent (or the human in the GUI) can see and continue your work.
