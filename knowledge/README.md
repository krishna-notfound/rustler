# Rustler agent knowledge book

Knowledge for any AI agent (Muse, Claude Code, Antigravity, Grok, …) that uses Rustler's CLI or MCP
server to make short-form vertical clips. Rustler calls no AI APIs: the agent does the thinking,
Rustler does the work. These files are loaded by Rustler and exposed as guides (CLI `guide` commands,
MCP) and MCP prompts.

## Format (frozen)

- `guides/<id>.md`: YAML frontmatter `id`, `title`, `summary` (one line), `order` (int), `tags`
  (list), then a markdown body.
- `prompts/<name>.md`: YAML frontmatter `name` (snake_case), `title`, `description`, `arguments`
  (list of `{name, description, required}`), then the prompt text with `{{arg_name}}` placeholders.

## Guides (reading order)

| # | id | What it covers |
|---|---|---|
| 1 | `start-here` | What Rustler is, your role, discovery, the editing loop, non-negotiable rules |
| 2 | `workflow-youtube-longform` | Scan-first flow for long videos/streams: info, heatmap, captions, sections |
| 3 | `workflow-local-file` | Import, understanding pack, read/look, plan, build, report |
| 4 | `finding-hooks` | What makes a moment work, 0-100 rubric, signals, lengths, counts, red flags |
| 5 | `cut-points` | Start/end units, weak openers (EN + Hinglish), endings, pauses, montages |
| 6 | `layouts-and-framing` | center-crop / blur-fit / split-vertical, regions, webcam rect math, face and speaker tracking, safe zones |
| 7 | `captions-and-style` | 4 presets, position_y per layout, words per line, script, speaker colours, readability |
| 8 | `audio` | −14 LUFS / −2 dBTP, music bed −18 dB + ducking, copyright, silence |
| 9 | `quality-checklist` | QC checks and fixes, contact sheet review, fix loop, variants, export |
| 10 | `hinglish` | Language settings, romanisation, search, captions and titles for Hinglish |
| 11 | `platform-specs` | Format, durations, lengths that perform, titles/descriptions/hashtags, report template |
| 12 | `troubleshooting` | Exit codes, setup, revision conflicts, blocked downloads, resumable jobs, Windows |
| 13 | `cli-reference-conventions` | Grammar, flags, envelope, MCP naming, images, the complete command map (178 commands by area) |
| 14 | `streamer-clips` | Stream VOD to webcam-top / gameplay-bottom clip: detect-webcam, split, regions, presets, speakers |
| 15 | `editing-toolkit` | timeline document, edit.* (cold open, insert, speed, freeze), clip merge/duplicate, hook titles and styles, transitions (incl. flash/whip), effects, progress bar, image/cutaway/mask layers, keyword captions, polish order |
| 16 | `briefs-and-batches` | brief set/check/description, variants, render/export batch, project status/pack |
| 17 | `assets` | asset list/search/add with licenses, `asset:<id>`, bundled CC0 SFX, custom caption styles |
| 18 | `campaigns-and-posting` | campaign rules (machine and agent), per-platform copy from `clip meta-context`, post JSON, cover frame |
| 19 | `director-retention` | retention, hook formulas, dead zones, `clip retention`, directing a whole job |
| 20 | `learn-style-from-videos` | `reference import/analyze`: measure example shorts, LOOK at the sheets, write the style down |
| 21 | `templates-and-brand` | `template.*`, starters `podcast-viral` and `streamer-split`, apply to many clips, export/import bundles |

## Prompts (MCP workflows)

| name | Arguments | Use |
|---|---|---|
| `make_viral_clips` | source*, count, platform, style_notes | End-to-end clips from a URL or file |
| `clip_from_stream` | url*, streamer_layout, count | Stream VOD → split-vertical reaction clips |
| `streamer_split_clip` | url_or_path*, webcam_corner | ONE webcam-top / gameplay-bottom clip, end to end |
| `polish_clip` | clip_id* | Hook title, SFX on emphasis, transitions, effects, progress bar, fix loop |
| `quick_scan_source` | url* | Judge a long video without downloading; list windows |
| `hinglish_podcast_clips` | source*, count | Hindi-English podcast clips with romanised captions |
| `batch_from_brief` | brief* | Campaign/client brief → checked batch + compliance report |
| `review_clip` | clip_id* | Senior-editor critique + fixes |
| `fix_qc_failures` | clip_id* | Map QC failures/warnings to edits until it passes |
| `restyle_clip` | clip_id*, style* | Change captions/layout/music to a requested look |
| `direct_campaign` | source_id*, campaign_id | Brief to strategy to diverse moments to hooks to one look to retention-checked batch |
| `hook_lab` | clip_id* | Compare five hook variants for a moment |
| `learn_style_from_folder` | folder*, name*, notes | Import example shorts, measure, look, write the style spec |
| `write_clip_meta` | clip_id*, platform | Write per-platform title/description/hashtags and save them with `clip meta-set` |
| `apply_template_batch` | clip_ids*, template, from_clip | Dry-run and apply one template to many clips, write each hook, look, QC, export |

`*` = required.

## Conventions used in these files

- Commands are written in CLI form (`rustler clip create …`); the MCP tool is `group_action`
  (`clip_create`).
- Every command named in these files exists in the registry (178 at the time of writing); verify any
  command with `rustler commands describe <name>`. `rustler commands list` is the authority.
- On Windows PowerShell 5.1, JSON inside flags loses its double quotes: use `--input-json @file.json`
  (see `troubleshooting`).
- Example hooks, transcripts and numbers marked illustrative are invented for teaching.
- Selection and pacing heuristics are adapted from MIT-licensed research repos (openshorts, autoclip,
  bridgeclip, opensource-clipping); ideas from other repos are rephrased, not copied.
