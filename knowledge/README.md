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
| 6 | `layouts-and-framing` | center-crop / blur-fit / split-vertical, webcam rect math, face tracking, safe zones |
| 7 | `captions-and-style` | 4 presets, position_y per layout, words per line, script, readability |
| 8 | `audio` | −14 LUFS / −2 dBTP, music bed −18 dB + ducking, copyright, silence |
| 9 | `quality-checklist` | QC checks and fixes, contact sheet review, fix loop, variants, export |
| 10 | `hinglish` | Language settings, romanisation, search, captions and titles for Hinglish |
| 11 | `platform-specs` | Format, durations, lengths that perform, titles/descriptions/hashtags, report template |
| 12 | `troubleshooting` | Exit codes, setup, revision conflicts, blocked downloads, resumable jobs, Windows |
| 13 | `cli-reference-conventions` | Grammar, flags, envelope, MCP naming, images, command map |

## Prompts (MCP workflows)

| name | Arguments | Use |
|---|---|---|
| `make_viral_clips` | source*, count, platform, style_notes | End-to-end clips from a URL or file |
| `clip_from_stream` | url*, streamer_layout, count | Stream VOD → split-vertical reaction clips |
| `quick_scan_source` | url* | Judge a long video without downloading; list windows |
| `hinglish_podcast_clips` | source*, count | Hindi-English podcast clips with romanised captions |
| `batch_from_brief` | brief* | Campaign/client brief → checked batch + compliance report |
| `review_clip` | clip_id* | Senior-editor critique + fixes |
| `fix_qc_failures` | clip_id* | Map QC failures/warnings to edits until it passes |
| `restyle_clip` | clip_id*, style* | Change captions/layout/music to a requested look |

`*` = required.

## Conventions used in these files

- Commands are written in CLI form (`rustler clip create …`); the MCP tool is `group_action`
  (`clip_create`).
- Commands marked "(if available — check `rustler commands list`)" belong to later slices and may
  not exist in every build.
- Example hooks, transcripts and numbers marked illustrative are invented for teaching.
- Selection and pacing heuristics are adapted from MIT-licensed research repos (openshorts, autoclip,
  bridgeclip, opensource-clipping); ideas from other repos are rephrased, not copied.
