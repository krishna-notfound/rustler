# f37 — clips.auto (one-call pipeline over existing commands)

**Status:** Built  |  **Depends on:** f05, f20, f22, f31, f34, f35, post JSON (v2 Task 1.7)  |  **Owner:** Opus agent (branch `worktree-agent-aba989e64829b7b64`)

## Purpose

Design v2 §5.7: "`clips.auto {url|source, count, template}` optional wrapper over existing commands (agent
remains free to do any step itself)". One call goes from a URL or an imported source to rendered, QC'd and
exported clips with a report the agent reads and corrects. It is a convenience, not a replacement for the
agent's judgement: every decision in the report carries the exact command that changes it.

## Scope

**In:** command `clips.auto`; ranking = `highlights.candidates` + a first-2-seconds hook score; non-overlap
(with each other and with clips that already cover the same moment); URL path (one cached `source.info`,
`source.fetch_transcript`, `source.import --sections` per pick with padding, `analyze.quick`); local path
(`analyze.quick` when there is no transcript); `clip.create` (smart defaults + `--template`); campaign attach
(`clip.meta_set --campaign-id`); `clip.retention` and the cheap retention fixes; `render.batch` with `eta_s`;
`clip.retention` on the render; `qc.run`; `export.clip` (post JSON skeleton); dry-run plan; `ranges` (the
agent's own moments, the rest automated); NDJSON progress per stage; per-clip failure isolation.
**Out (explicitly):** writing titles/descriptions/hashtags (Rustler never writes copy, `post_cmds.rs`);
publishing/uploading; the agent's 4-part score (`clip.score_set` stays the agent's); creative retention fixes
(text pops, cutaways, hook titles: listed, never applied); a whisper fallback for URLs without captions.

## User-facing Behavior

`rustler clips auto --source-id <src> [--count 5] [--min-len 25] [--max-len 60] [--template T] [--quality draft|final]
[--campaign C] [--platform shorts|reels|tiktok] [--raw true] [--fixes false] [--dry-run]`
`rustler clips auto --url <url> [--lang L] [--pad 3] [--download-quality 720] ...`
`rustler clips auto --source-id <src> --input-json @ranges.json` (`{"ranges": [{"start", "end", "note"}]}`)

Progress (stderr NDJSON): `clips.auto.resolve`, `.analyze`, `.rank`, `.create`, `.fix`, `.render` (with `eta_s`),
`.qc`, `.done`.

Result: `source`, `settings`, `ranking` (`formula`, `candidates_considered`, `excluded` with reasons), `clips[]`
(rank, status `planned|done|failed`, `failed_stage`, `error`, name `auto-<n>-<MMmSSs>`, clip_id, source_id,
start/end/mmss/duration, `section` (URL), `why` (chosen_by, candidate_score, hook_2s + notes, first_2s_text,
rank_score, reasons, opening/closing text), `retention` (before_fixes, score, measured_on, dead_zones_left),
`fixes_applied` (command, ok, undo), `fixes_suggested` (creative ones), `render`, `qc` (passed, failed, warned,
verdict), `export` (video, post, report, needs_copy), `plan` (dry run), `change[]` (decision → command),
`warnings`), `summary` (requested, picked, created, rendered, qc_passed, exported, failed), `elapsed_s`, `next`.

## Routes / Files

**Owned files:**
- `crates/rustler-commands/src/auto_cmds.rs`
- `crates/rustler-commands/tests/auto.rs`
- `knowledge/guides/clips-auto.md`
- `docs/features/f37_clips_auto.md`

**Shared edits (minimal):** `crates/rustler-commands/src/lib.rs` (module + `register`).

**Read-only dependencies:** `highlights_cmds` (`load_data`, `features`, `tokens_of`, `weak_opener_len`),
`source_cmds` (`load_source_checked`, `pending_id`), `campaign_cmds::load_campaign`, `post_cmds::platform_key`,
`template_cmds::prepare`, `transcript_cmds::analysis_rel`, `clip_cmds::log_event`; every pipeline step is a
registry dispatch of an existing command.

## Data and Interfaces

- `AutoIn` / `AutoOut` in `auto_cmds.rs` (schemas via `rustler commands describe clips.auto`).
- Pure helpers (unit-tested): `hook_2s(&Data, start) -> (score, notes, text)`, `select(pool, count, existing)`,
  `parse_cli(&Registry, "rustler effect add ...") -> (name, input JSON)` (runs the fix commands that
  `clip.retention` prints, positionals mapped through the command spec).
- Rank formula (also in the output): `candidate score (0-100) + 20 x hook_2s`; hook_2s over the first 2 s:
  +0.15 speech within 0.3 s, +0.30 emphasis word, +0.25 question, +0.15 number, +0.15 "you"/aap/tum,
  +0.10 loudness peak >= 3 LU over the median, +0.10 speech rate >= 1.15 x median, -0.20 weak opener; 0..1.

## Rules and Edge Cases

1. Exactly one of `url` / `source_id`; `count` 1..20; `0 < min_len < max_len`; ranges valid and non-overlapping;
   an explicit `template` must resolve — all checked before anything is written.
2. Campaign: its duration fills `min_len`/`max_len` and its platform fills `platform` when not given.
3. Dry run: ranks and returns `plan` per clip; no import, no clip, no render (a URL's captions are still fetched
   to rank; a source without a transcript gets a warning instead of `analyze.quick`).
4. Fixes applied only when `clip.retention` suggests them and the command exists: `trim_head`, `trim_tail`,
   `remove_pauses` first; when one ran, retention is measured again before `punch_in`s are placed. Undo:
   `effect remove <clip> <fx_id>` for effects, `history undo` for edits.
5. Export when QC passes, or when the only failure is `meta_present` (the copy the agent writes) —
   `needs_copy: true`. Any other QC failure: clip `failed` at `qc` with `fix suggest` in the hint.
6. A failure at import / analyze / create / render / qc / export is recorded on that clip; the others go on.
7. URL sections: picks padded by `pad` (default 3 s), one `source.import` per pick (the info lookup is cached);
   `clip.create` times are relative to the section.

## Tasks

- [x] `auto_cmds.rs` + registration
- [x] Tests: dry-run plan, count + non-overlap, existing-clip avoidance, bad input, failure isolation, real e2e
- [x] Guide `clips-auto`, feature index row, progress log

## Acceptance Criteria

- [x] Dry run returns `count` non-overlapping planned clips and writes no clip.
- [x] A failing range does not stop the other clips (they render, pass QC and export with ffmpeg).
- [x] Real e2e on `~/.rustler/test-media/boyle_twoshot_105s.mp4`: 2 clips created, rendered, QC passed, exported.
- [x] Render progress carries `eta_s`.

## Verification

`$env:CARGO_TARGET_DIR="D:\rustler-build\auto"; cargo test -p rustler-commands --test auto` (5 tests; the e2e needs
ffmpeg + uv + engine and skips with a printed reason otherwise), `cargo test -p rustler-commands --lib auto_cmds`
(3), `cargo test -p rustler-knowledge`.

## Status Log

- 2026-11-16 — Built. `--test auto` 5 passed (e2e 96 s: 2 clips, retention 40→77 and 34→75 after punch-ins,
  QC pass with `meta_present` warn, exports + post.json). CLI run with `--template podcast-viral`: 73 s, 2/2
  exported, ETA in render progress, frames looked at (blur-fit wide shot, center-crop close-up, template captions
  and progress bar). URL path not run (no network in tests).
