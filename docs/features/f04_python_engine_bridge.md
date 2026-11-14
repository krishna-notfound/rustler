# f04 — Python Engine Bridge

**Status:** Verified  |  **Depends on:** f01  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

Machine learning and perception work (Whisper transcription, scene detection, face tracking,
Devanagari to Latin transliteration) runs in a Python worker managed by uv. The Rust core supervises that
worker and talks to it with line-delimited JSON-RPC over stdio, so the Python side can be restarted or
killed without corrupting the project.

## Scope

**In:**
- Python worker `rustler_engine`: stdio JSON-RPC loop, framing, progress notifications.
- Rust supervisor and typed client (`rustler-engine-bridge`), with cancel that kills the process tree.
- Protocol document `engine/PROTOCOL.md` (methods and payloads).
- Methods: `engine.ping`, `engine.capabilities`, `transcribe.run`, `scenes.detect`, `translit.to_latin`, `models.fetch`, `faces.track` (see PROTOCOL.md).

**Out (explicitly):**
- Provider HTTP calls (LLM, vision, STT, TTS). No AI API integrations by user decision.
- Diarization / WhisperX (f29, later).
- Remote engine over the network. The worker is local only.

## User-facing Behavior

- `engine.capabilities` reports CUDA availability and package versions; `engine.ping` returns the engine version.
- Progress is reported as JSON-RPC `progress` notifications and surfaced as stderr progress events.
- Cancelling a command kills the Python process tree; no orphan workers remain.
- Protocol errors use `-32600` (invalid request) and `-32601` (method not found), both with `data.kind = "validation"`.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-engine-bridge/src/lib.rs`, `client.rs`, `protocol.rs`
- `engine/rustler_engine/__init__.py`, `__main__.py`, `rpc.py`
- `engine/PROTOCOL.md`

**Read-only dependencies:**
- `engine/pyproject.toml` — uv project; owned by f01.
- `crates/rustler-tools/src/engine_install.rs` — creates the venv that runs the worker (f01).

## Data and Interfaces

- Request: `{"jsonrpc":"2.0","id":N,"method":"...","params":{...}}`; `id` is int or string.
- Progress: `{"jsonrpc":"2.0","method":"progress","params":{"id":N,"stage":"...","pct":0.0-100.0|null}}`.
- `engine.capabilities {}` returns `{"cuda": bool, "packages": {...}}`.
- Client call shape in Rust: `engine.call(method, params, &progress, &cancel)`.

## Rules and Edge Cases

1. Method names and payloads are fixed by `engine/PROTOCOL.md`; change the doc first, then both sides.
2. Long methods emit progress notifications; the client must not treat silence as a hang.
3. Model downloads (`models.fetch`) report `stage: "models.fetch"` progress.
4. The worker speaks over stdio only; no network listener is part of this feature.

## Tasks

- [x] JSON-RPC loop and framing (`engine/rustler_engine/rpc.py`)
- [x] Rust supervisor and client with process-tree kill (`crates/rustler-engine-bridge`)
- [x] Protocol document (`engine/PROTOCOL.md`)

## Acceptance Criteria

- [x] Bridge tests against the real engine pass (`crates/rustler-engine-bridge/tests/bridge.rs`)
- [x] RPC framing and error codes (`engine/tests/test_rpc.py`, 13 tests)
- [x] Cancel stops the worker tree (covered by bridge tests; progress log, 2026-11-15)

## Verification

- `cargo test -p rustler-engine-bridge` (PowerShell) and `~/.rustler/bin/uv run --project engine pytest -q`.
- Pass = all bridge tests green and engine RPC tests green; a real transcription run reaches the worker.

## Status Log

- 2026-11-15 — Engine bridge merged (Rust to Python JSON-RPC; bridge tests run against the real engine; process-tree kill on cancel). Engine suite: 85 Python tests at the faces merge (progress log); CUDA works on the laptop.
