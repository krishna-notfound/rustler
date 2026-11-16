# Progress Tracker

**The authoritative record is `docs/progress-log.md`** (dated, newest entries at the bottom). This file is the
short current-state pointer for agents. If this file and the progress log disagree, the progress log wins; then
fix this file.

Per-feature status: `docs/features/README.md`. Plan and scope: `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md`
("5-HOUR SCOPE" section at the end).

## Current Phase

- Slice 1 ("First Real Clip") plus the 5-hour extras are built and were run end to end on 2026-11-15.
- The "complete everything" phase added (all merged): layout regions and webcam detection (f15), overlays/hook titles, transitions, effects, progress bar (f18), cut refinement (f19), highlight helpers (f20), understanding v2 (f07b), QC v2 and fix loop (f21), brief/variants/batches (f22), asset library (f23), the full GUI editor (f25-f27), speakers (f29), project pack/status, audio post (denoise, SFX, two-pass loudnorm). The registry has 179 commands; the knowledge book has 22 guides and 15 prompts (v2 added config, campaigns and posting, director/retention, style references, timeline v2, smart defaults, templates, hooks/captions/cover).
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
- MCP over stdio: 55 tools, 8 prompts, 14 resources at the time of the check (f10); the registry now has 179 commands, so 179 tools and 15 prompts.
- GUI in a real browser: Renders page, QC table, contact sheet (f24).
- Captions: four presets and SRT (f16); music bed with ducking measured -3.9 dB (f17).

## Built, not yet verified in the log

- Knowledge book and guide commands (f11). Agent skill and AGENTS.md committed (`skills/rustler/`, commit 8e23db6); no agent run recorded.
- Edit operations and history (f12). No test run recorded.
- Face tracking and crop path (f14). No live run recorded.

## Next Up

- Log a test run for f12 (`cargo test -p rustler-commands --test edit`).
- Live check of `crop.track_face` on a talking-head clip (f14).
- Not yet run live: `analyze speakers` / `crop track-speaker` on real two-person footage (f29); `rustler setup run` refreshes the engine with the speaker models.
- Remaining planned units: GPU throughput (f28). The Tauri shell for the GUI editor (f25) is deferred.

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
- GPU throughput and parallel renders (f28).
- Tauri desktop shell for the GUI editor (f25–f27 are built in the web GUI).
- Dedicated GUI panels for speaker colours, custom caption styles and a full asset browser.
- MCP loopback HTTP transport with bearer token (spec §5.12) — stdio only is built.

## Session Notes

- Resume by reading `docs/progress-log.md` (newest entries last), then `docs/features/README.md`.
- Build and test from PowerShell on Windows (Git Bash `link.exe` clash).
- Tool binaries live in `~/.rustler/bin`; run `rustler doctor` before any render work.
