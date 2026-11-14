---
id: cli-reference-conventions
title: "CLI and MCP conventions"
summary: "Command grammar, flags, --input-json, global flags, the JSON envelope, exit codes, progress on stderr, MCP tool naming, images for vision agents, and the command map."
order: 13
tags: [cli, mcp, reference, json, conventions]
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
  `clip_id`). Every positional can also be given as a flag (`--clip-id clp_X`).
- `--input-json '{"clip_id":"clp_X","preset":"blur-fit"}'` or `--input-json @input.json` supplies the
  whole input; explicit flags override its fields. Use it for complex inputs or awkward quoting.

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
Parse stdout only; never parse stderr for results.

Exit codes: 0 ok · 2 usage/validation · 3 not found · 4 dependency missing · 5 provider/network ·
6 media/ffmpeg · 7 QC failed · 8 cancelled · 9 revision conflict · 10 internal
(details in `troubleshooting`).

## 4. IDs, times and paths

- IDs: `prj_…`, `src_…`, `clp_…`, `seg_…`, `rnd_…` (prefix + ULID). Copy them from outputs.
- Times are seconds (float) in **source time**, unless the field starts with `out_` (clip/output
  time). Transcript lines print `HH:MM:SS.s` for reading; commands take seconds.
- Section sources start near 0; `original_offset` maps back to the original URL's timeline.
- Paths inside the project are stored relative to it, so a project folder can be copied to another
  machine and continued.

## 5. Discovery

```
rustler commands list                    # {"commands":[{name, summary, annotations}]}
rustler commands describe clip.create    # input_schema, output_schema, annotations, examples
rustler help                             # list + the start-here chain
rustler guide list / guide show <id>     # knowledge guides (if available)
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
  (`data.image`); open that file.
- **Prompts** = ready workflows: `make_viral_clips`, `clip_from_stream`, `quick_scan_source`,
  `review_clip`, `fix_qc_failures`, `restyle_clip`, `batch_from_brief`, `hinglish_podcast_clips`.
- **Guides** (this knowledge book) are exposed by the server too; read `start-here` first.

## 7. Command map

Core (Slice 1):

| Group | Commands |
|---|---|
| setup/doctor | `setup.run`, `doctor.run` |
| project | `project.create {name, dir?}`, `project.show` |
| source | `source.import {input, sections?, name?}`, `source.info {url}`, `source.fetch_transcript {url, lang?}`, `source.search {query, limit?}`, `source.list`, `source.show {source_id}` |
| analyze | `analyze.quick`, `analyze.transcribe {source_id, model?, language?, device?}`, `analyze.scenes`, `analyze.audio` |
| transcript | `transcript.get {source_id, start?, end?}`, `transcript.search {source_id, query, limit?}` |
| see | `see.frame {source_id, t, width?}`, `see.tiles {source_id, start?, end?, around?, window?, count?}`, `see.preview {source_id, start, end}` (≤ 60 s) |
| clip | `clip.create {name, source_id, start_unit?, end_unit?, start?, end?, expect_text?, layout?, layout_params?, captions?}`, `clip.add_segment`, `clip.set_layout {clip_id, segment_id?, preset, params?}`, `clip.set_captions {clip_id, enabled?, preset?, position_y?, max_words_per_line?, script?}`, `clip.show`, `clip.list` |
| render | `render.run {clip_id, quality?: final\|draft, prefer_gpu?}`, `render.preview_frame {clip_id, out_t}` |
| qc/export | `qc.run {clip_id, render_id?}`, `export.clip {clip_id, render_id?, name?}` |

Extras (if available — check `rustler commands list`): `edit.trim`, `edit.split`,
`edit.delete_segment`, `edit.remove_pauses`, `edit.remove_fillers`, `history.undo`,
`crop.track_face`, `caption.list_styles`, `audio.add_music`, `mcp serve`, `gui`; later:
`see.contact_sheet`, `highlights.signals`, `overlay.*`, `qc.review`, brief/variant commands.

## 8. Good habits

- `commands describe` before first use of any command in a session.
- `--dry-run` before anything destructive or expensive.
- Keep IDs and decisions in your notes; the project's `events.ndjson` records every action, so
  another agent (or the human in the GUI) can see and continue your work.
