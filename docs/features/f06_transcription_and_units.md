# f06 — Transcription and Units

**Status:** Verified  |  **Depends on:** f04, f05  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

Speech becomes data an agent can cite. Local faster-whisper produces word-level timestamps; words are grouped
into numbered **units** (sentence-like chunks). An agent chooses cut points by unit id and never invents
times: times come from word data. Empty or wrong-language speech is handled without failure.

## Scope

**In:**
- `analyze.transcribe` (full transcript, resumable chunks) and `analyze.quick` (fast pass for a first look).
- `transcript.get` and `transcript.search` (find a quote, returns unit ids and times).
- Units: `build_units()` and `find_quote()` in `units.rs`; `units.json` and `transcript.json` in the project.
- Devanagari to Latin transliteration via the engine method `translit.to_latin`.
- Transcription model comes from project settings (`default_whisper_model`).

**Out (explicitly):**
- Speaker diarization (f29, later).
- Word-level edits and sentence snapping (f19, planned; trim snapping to words is in f12).
- LLM summaries or highlight picking (no AI API integrations by user decision).

## User-facing Behavior

- `transcript.search "<quote>"` returns matching unit ids with start and end times.
- `analyze.quick` on a 55 s demo gave 110 words and 15 units.
- A transcript with no speech saves fine with 0 words and an empty `units` list.
- Rerunning transcription resumes from saved chunk parts (`transcribe_resumes_from_parts`).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/transcript_cmds.rs`
- `crates/rustler-commands/src/analyze_cmds.rs` (shared with f07: f06 owns `analyze.transcribe` and `analyze.quick`)
- `crates/rustler-core/src/units.rs`, `crates/rustler-core/src/model/transcript.rs`
- `engine/rustler_engine/transcribe.py`, `engine/rustler_engine/translit.py`

**Read-only dependencies:**
- `crates/rustler-engine-bridge` — calls `transcribe.run` and `translit.to_latin` (f04).
- `crates/rustler-core/src/store.rs` — atomic writes of transcript files (f03).

## Data and Interfaces

- Engine: `transcribe.run {audio_path, model, language: auto|en|hi, device: auto|cpu|cuda, parts_dir, chunk_seconds: 600, max_seconds}`; `translit.to_latin {texts: [...]}`.
- Files: `transcript.json`, `units.json`, `transcript.raw.json`.
- Types: `Transcript`, `Word`, `Unit`, `UnitKind` (`model/transcript.rs`).

## Rules and Edge Cases

1. Times come from word data only; never from model text output.
2. Empty speech yields an empty `units` list; `clip.create --start-unit` then errors with a hint to use `--start/--end` (`create_without_units_hints_transcribe`).
3. Interrupted transcription resumes from saved parts (`transcribe_resumes_from_parts`).
4. The engine `language` option takes `auto`, `en` or `hi` (`engine/PROTOCOL.md`).

## Tasks

- [x] Units builder and quote finder (`units.rs`)
- [x] Transcribe, quick analyze, get and search commands
- [x] Engine transcription and transliteration (`transcribe.py`, `translit.py`)

## Acceptance Criteria

- [x] Units and quote finder (`crates/rustler-core/tests/units.rs`, 10 tests)
- [x] Analyze commands (`crates/rustler-commands/tests/analyze.rs`: `transcribe_resumes_from_parts`, `empty_transcript_ok`)
- [x] Transcript commands (`crates/rustler-commands/tests/transcript.rs`, 5 tests)
- [x] Engine transcription tests (`engine/tests/test_transcribe.py`, 16 tests) and transliteration (`engine/tests/test_translit.py`, 6 tests)
- [x] Live: quick analyze, quote search "fellow Americans" on a 55 s demo, TED fetch-transcript giving 307 units (progress log)

## Verification

- `cargo test -p rustler-core --test units` and `cargo test -p rustler-commands --test analyze --test transcript` (PowerShell).
- `uv run --project engine pytest -q engine/tests/test_transcribe.py engine/tests/test_translit.py`.
- Pass = tests green; `transcript.search` returns unit ids on a real clip.

## Status Log

- 2026-11-15 — Transcription and units merged. End-to-end: quick analyze (110 words, 15 units) and quote search passed on the demo video; TED talk fetch-transcript produced 307 units.
