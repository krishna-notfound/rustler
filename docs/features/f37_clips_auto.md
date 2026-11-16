# f37 — clips.auto (one-call pipeline over existing commands)

**Status:** Built  |  **Depends on:** f05, f20, f22, f31, f34, f35, post JSON (v2 Task 1.7)  |  **Owner:** Opus agent (branch `worktree-agent-aba989e64829b7b64`)

## Purpose

Design v2 §5.7: "`clips.auto {url|source, count, template}` optional wrapper over existing commands (agent
remains free to do any step itself)". One call goes from a URL or an imported source to rendered, QC'd and
exported clips with a report the agent reads and corrects. It is a convenience, not a replacement for the
agent's judgement: every decision in the report carries the exact command that changes it.

## Scope

**In:** command `clips.auto`; whisper fallback for a URL without captions (audio-only download + whisper base
to rank; sections as usual); ranking = `highlights.candidates` + a first-2-seconds hook score; non-overlap
(with each other and with clips that already cover the same moment); URL path (one cached `source.info`,
`source.fetch_transcript`, `source.import --sections` per pick with padding, `analyze.quick`); local path
(`analyze.quick` when there is no transcript); `clip.create` (smart defaults + `--template`); campaign attach
(`clip.meta_set --campaign-id`); `clip.retention` and the cheap retention fixes; `render.batch` with `eta_s`;
`clip.retention` on the render; `qc.run`; `clip.cover --auto` (best effort, `cover: false` skips); `export.clip` (post JSON skeleton); dry-run plan; `ranges` (the
agent's own moments, the rest automated); NDJSON progress per stage; per-clip failure isolation.
**Out (explicitly):** writing titles/descriptions/hashtags (Rustler never writes copy, `post_cmds.rs`);
publishing/uploading; the agent's 4-part score (`clip.score_set` stays the agent's); creative retention fixes
(text pops, cutaways, hook titles: listed, never applied); **zooms of any kind and colour grades** (never added by
clips auto; retention punch-ins are only suggested — user rule 2026-11-17).

## User-facing Behavior

`rustler clips auto --source-id <src> [--count 5] [--min-len 25] [--max-len 60] [--template T] [--quality draft|final]
[--campaign C] [--platform shorts|reels|tiktok] [--raw true] [--fixes false] [--dry-run]`
`rustler clips auto --url <url> [--lang L] [--pad 3] [--download-quality 720] ...`
`rustler clips auto --source-id <src> --input-json @ranges.json` (`{"ranges": [{"start", "end", "note"}]}`)

Progress (stderr NDJSON): `clips.auto.resolve`, `.analyze`, `.rank`, `.import` (URL, 2 sections at a time, `eta_s`),
`.create`, `.fix`, `.render` (with `eta_s`), `.qc`, `.done`.

Result: `source` (with `transcript_via` captions | whisper | source | none and `transcript_s`), `settings` (with
`download_quality`, default 720), `ranking` (`formula`, `candidates_considered`, `excluded` with reasons), `clips[]`
(rank, status `planned|done|failed`, `failed_stage`, `error`, name `auto-<n>-<MMmSSs>`, clip_id, source_id,
start/end/mmss/duration, `section` (URL), `why` (chosen_by, candidate_score, hook_2s + notes, first_2s_text,
rank_score, reasons, opening/closing text), `retention` (before_fixes, score, measured_on, dead_zones_left),
`fixes_applied` (command, ok, undo), `fixes_suggested` (creative ones), `render`, `qc` (passed, failed, warned,
verdict), `export` (video, post, report, needs_copy), `plan` (dry run), `change[]` (decision → command),
`warnings`), `summary` (requested, picked, created, rendered, qc_passed, exported, failed), `elapsed_s`, `next`.

## Routes / Files

**Owned files:**
- `crates/rustler-commands/src/auto_cmds.rs`
- `crates/rustler-commands/src/cover_cmds.rs` (follow-ups)
- `crates/rustler-commands/tests/auto.rs`, `tests/trial2_fixes_auto.rs`, `tests/auto_followups.rs`
- `knowledge/guides/clips-auto.md`
- `docs/features/f37_clips_auto.md`

**Shared edits (minimal):** `crates/rustler-commands/src/lib.rs` (module + `register`).

**Read-only dependencies:** `highlights_cmds` (`load_data`, `features`, `tokens_of`, `weak_opener_len`),
`source_cmds` (`load_source_checked`, `pending_id`), `campaign_cmds::load_campaign`, `post_cmds::platform_key`,
`template_cmds::{prepare, punch_config, BY_TEMPLATE, PlaceAt}`, `timeline_doc::{assemble, punch_input, next_item_id,
DocEnv}`, `rustler_media::punchin::plan`, `transcript_cmds::analysis_rel`, `clip_cmds::log_event`; every pipeline step is a
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
   `remove_pauses` (re-measured after each; rolled back when hook/ending get worse). Never a zoom: `punch_in` goes to
   `fixes_suggested`. After the silence fixes the template's own punch-ins (tagged `by_template`) are checked
   against the new cuts with the punch-in planner's rules (>= `min_cut_gap_s` 0.5 s from every cut, not across one,
   not on blocked spans); if one breaks them, all template punch-ins are re-planned with the template's pattern via
   `timeline.apply` (`fixes_applied` action `template_punch_ins`, undo `history restore <clip> <rev>`).
8. No captions for a URL (`source.fetch_transcript` validation "no captions", or yt-dlp wrote none): `source.import
   --audio-only true` of the whole URL + `analyze.quick --transcribe always --model base`; rank on that source. A dry
   run does not download: it warns and plans nothing. Network/tool errors still stop with the manual hint.
9. URL sections: `--download-quality` default 720; 2 imports at a time (each writes only its own source folder).
   Render: `render.batch --prefer-gpu true` (the probed GPU encoder, libx264 otherwise).
   Each section is analysed with `analyze.quick --transcribe always` (whisper: exact timings for the silence
   fixes, no caption markup such as `[ Laughter ]` / `-` speaker dashes in burned captions); when whisper does not
   run, it falls back to the fetched captions with a clip warning.
10. Cover (`clip.cover --auto`): 16 evenly spread render frames from one ffmpeg call (or the 12 tiles of the
    render's QC contact sheet when it exists), one engine `faces.track` call; frames with a mask shown or the face
    >= 25 % under a mask/card/text are left out when other frames are free; layer areas never count toward sharpness.
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
(3), `cargo test -p rustler-knowledge`; follow-ups: `cargo test -p rustler-commands --test auto_followups` (6; whisper and
cover parts need the engine / ffmpeg and skip otherwise), `--lib cover_cmds` (9).

## Status Log

- 2026-11-16 — Built. `--test auto` 5 passed (e2e 96 s: 2 clips, retention 40→77 and 34→75 after punch-ins,
  QC pass with `meta_present` warn, exports + post.json). CLI run with `--template podcast-viral`: 73 s, 2/2
  exported, ETA in render progress, frames looked at (blur-fit wide shot, center-crop close-up, template captions
  and progress bar). URL path not run (no network in tests).
- 2026-11-16 — Renamed f36 → f37 (main took f36), merged main, added best-effort `clip.cover --auto` before export
  (`cover: false` skips it; failure is a clip warning). `--test auto` 5 passed after the merge; the e2e picked a cover
  for both clips (post JSON `cover` set) and took 233 s (was 96 s without the cover step, run alongside another test).
