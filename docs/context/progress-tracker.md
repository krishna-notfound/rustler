# Progress Tracker

**The authoritative record is `docs/progress-log.md`** (dated, newest entries at the bottom). This file is the
short current-state pointer for agents. If this file and the progress log disagree, the progress log wins; then
fix this file.

Per-feature status: `docs/features/README.md`. Plan and scope: `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md`
("5-HOUR SCOPE" section at the end).

## Current Phase

- Slice 1 ("First Real Clip") plus the 5-hour extras are built and were run end to end on 2026-11-15.
- Phase status per `docs/README.md`: implementation in progress under the 5-hour build window.

## Current Goal

- Not set in the progress log. Ask the user before starting new feature work.

## Completed (verified in the progress log)

- Setup and doctor on the laptop (f01).
- Registry and CLI; end-to-end run through the real CLI (f02).
- Project store, clip model, atomic writes, revisions (f03).
- Engine bridge; real engine tests (f04).
- Source import, scan-first YouTube, probe (f05).
- Transcription and units; quote search; TED fetch-transcript (f06).
- See commands: tile grids and frames inspected (f07).
- Render compiler with NVENC and Devanagari-named source (f08).
- QC v1 12/12 and export (f09).
- MCP over stdio: 55 tools, 8 prompts, 14 resources (f10).
- GUI in a real browser: Renders page, QC table, contact sheet (f24).
- Captions: four presets and SRT (f16); music bed with ducking measured -3.9 dB (f17).

## Built, not yet verified in the log

- Knowledge book and guide commands (f11). Agent skill and AGENTS.md committed (`skills/rustler/`, commit 8e23db6); no agent run recorded.
- Edit operations and history (f12). No test run recorded.
- Face tracking and crop path (f14). No live run recorded.

## Next Up

- Log a test run for f12 (`cargo test -p rustler-commands --test edit`).
- Live check of `crop.track_face` on a talking-head clip (f14).
- Two-pass loudnorm (f17).
- Next planned feature units by index: f15 (custom layouts), f18 (overlays and transitions), f19 (cut refinement). Feature files for these are not written yet; write one before coding (see `docs/features/README.md`).

## Open Questions

> Anything unknown or ambiguous goes here. Mark answered ones with the answer and date.

- [ ] Upgrade Node 24 for the rea plugin (rea MCP failed: Node 24.21.0 is older than the required 24.22.1). Needs user OK; system change (from `docs/progress-log.md`, 2026-11-15).
- [ ] GUI: `#view` reload fix committed (d77e638); re-check in a browser and log it.
- [ ] Guide-command tests (the log records 5) do not appear under `crates/*/tests`; location unverified (f11).

## Architecture Decisions

> Date, decision, reason. Newest first.

- 2026-11-15 — Rustler is a personal tool; no hosted service — the user's stated use.
- 2026-11-15 — No provider layer, no AI API calls (f13 dropped) — user decision; the driving agent does all reasoning.
- 2026-11-15 — GUI is a simple local web page (`rustler gui`), not Tauri, for the 5-hour window — Claude's call with user's "keep GUI simple, CLI strong" constraint. Tauri editor deferred to f25–f27.
- 2026-11-15 — Rust core plus Python engine (approach B) — user's choice; reuses MIT Python code and best Python ML libraries.
- 2026-11-15 — Copy code only from MIT repos; reimplement ideas from AGPL, GPL and modified-Apache repos — license correction in the progress log.
- 2026-11-15 — Knowledge book lives in `knowledge/` (product content, embedded in the binary), indexed from `docs/README.md`.
- 2026-11-15 — All documentation lives in `docs/` and is updated after every merge — user rule.
- 2026-11-15 — Single-pass loudnorm for Slice 1; two-pass deferred — the -15.9 LUFS result on half-silent clips is recorded as an open issue.

## Deferred Work

- Provider layer (f13) — dropped by user decision.
- Highlight helpers (f20), brief and variants (f22), asset library (f23) — later slices.
- Transitions and hook titles (f18), custom layouts (f15), cut refinement (f19) — not started.
- Tauri timeline editor, program monitor, caption editor (f25–f27) — the simple GUI stands in.
- Diarization (f29), GPU throughput (f28).
- MCP loopback HTTP transport with bearer token (spec §5.12) — stdio only is built.

## Session Notes

- Resume by reading `docs/progress-log.md` (newest entries last), then `docs/features/README.md`.
- Build and test from PowerShell on Windows (Git Bash `link.exe` clash).
- Tool binaries live in `~/.rustler/bin`; run `rustler doctor` before any render work.
