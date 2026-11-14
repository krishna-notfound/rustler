# Rustler

## Overview

Rustler is a personal, local, agent-native clipping and editing studio. The user clips videos to earn money
(campaign views or per-clip fees). An AI agent (Claude Code, or any MCP-capable or shell-capable agent) installs
Rustler on the user's machine and works as the video editor: it finds sources, understands them (frames, scenes,
audio, speech), picks moments by transcript unit, edits, reframes, captions, mixes music, renders, and checks the
result. The core rule is "the agent is the editor, the software is its complete editing environment": every
capability is a command in one registry, reachable from the CLI, the MCP server and the web GUI.

## Goals

1. One toolkit (`rustler`) that an agent can install itself and drive fully through CLI (JSON output) or MCP.
2. The agent can see and hear the source: frames, labelled tile grids, scenes, audio peaks, transcript with unit ids.
3. Every render is checked (`qc.run`) before it counts as done; exit code 0 alone is not success.

## Users and Roles

| Role | Who | Can do | Must never |
| ---- | --- | ------ | ---------- |
| Owner | The user, on their own machine | Set project permissions, run setup, review clips in the GUI or files | Expect Rustler to publish anything (publishing is manual) |
| Driving agent | Claude Code or another CLI/MCP agent | Call any registry command within project permissions | Auto-publish, invent timestamps, hand-write FFmpeg, treat exit 0 as good output |
| Human reviewer (optional) | The owner, using `rustler gui` | View Project, Sources, Clips, Renders, Activity, Commands | Expect GUI-only features; every GUI action is a registry command |

## Core User Flows

### Clip a local file

1. `rustler setup` once, then `rustler doctor` (ready: true).
2. `rustler project create`, `rustler source import <file>`.
3. `rustler analyze quick` (or `analyze transcribe`), then `rustler transcript search "<quote>"` for unit ids.
4. `rustler see tiles --around <t>` to look at frames.
5. `rustler clip create --start-unit ... --end-unit ...` plus layout and caption options (the e2e run used blur-fit and hormozi-pop; exact flag names: `rustler commands describe clip.create`).
6. `rustler render run`, `rustler qc run`, `rustler export clip`.

### Scan-first YouTube

1. `rustler source info <url>` (chapters and heatmap peaks, no download).
2. `rustler source fetch-transcript <url>` (captions, no download).
3. `rustler source import <url> --sections MM:SS-MM:SS` (download only the chosen part).
4. Continue with the local-file flow from step 3.

## Features

### Built (see `docs/features/README.md` for status)

- Setup, doctor and tool installers (f01)
- Command registry, JSON envelope, exit codes, generic CLI (f02)
- Project store, clip and layout model, atomic writes, revisions (f03)
- Python engine bridge (f04)
- Source import, probe, scan-first YouTube (f05)
- Transcription, units, quote search (f06)
- See and analysis: frames, tiles, scenes, audio peaks (f07)
- Render compiler: crop, blur-fit, split-vertical; NVENC; loudnorm (f08)
- QC v1 with 12 checks and export (f09)
- MCP server over stdio (f10)
- Knowledge book: guides, prompts, guide commands (f11)
- Edit operations and history (f12)
- Face tracking and crop path (f14)
- Four caption presets and SRT (f16)
- Music bed with ducking (f17)
- Simple local web GUI (f24) and the full editor on top of it (f25–f27)
- Layout regions, webcam detection (f15); overlays, transitions, effects (f18); cut refinement (f19); highlight helpers (f20)
- Understanding v2: OCR, audio events, faces (f07b); QC v2 and the fix loop (f21)
- Brief, variants, batches (f22); asset library (f23); speakers (f29); project pack and status

## Scope

### In Scope

- Local CLI, MCP and web GUI over one registry
- Local-file and YouTube sources (public, via yt-dlp)
- 9:16 short-form clips with captions, music bed and loudness normalisation
- Quality checks with fix hints

### Out Of Scope

- Auto-posting or uploading to platforms
- Hosting Rustler as a service for other people (personal tool)
- Rewriting FFmpeg or any codec; training own ML models
- Bypassing logins, paywalls or DRM when downloading

### Deferred (decided, not now)

- Provider layer for LLM, vision, STT and TTS endpoints (f13) — dropped by user decision: no AI API integrations
- Tauri desktop shell for the GUI editor (the editor itself is built into the web GUI)
- GPU throughput and parallel renders (f28)

## Business Rules

1. Rustler never publishes, uploads or posts output.
2. Times used in edits come from word-level data (transcript units), never from model-generated text.
3. Every render is checked with `qc.run` before it is reported as done; a failed check exits 7.
4. Downloads use only public sources yt-dlp can fetch; network and download access follow project permissions.
5. Copy code only from MIT-licensed studied repos (with attribution in `THIRD_PARTY_NOTICES.md`); reimplement
   ideas from AGPL, GPL and modified-Apache repos (see `docs/research/00-license-and-dependency-inventory.md`).
6. No AI API calls: all reasoning is done by the driving agent; Rustler runs local tools only.

## Success Criteria

1. An agent with only a shell can run `rustler setup`, import a local file or YouTube URL, transcribe it, look at
   frames, render a 9:16 clip with karaoke captions and loudnorm, and get a QC report with a contact sheet.
   (Observed end to end on 2026-11-15; see `docs/progress-log.md`.)
2. Every feature listed as Verified in `docs/features/README.md` has a recorded end-to-end run or real tests.
3. The same clip can be inspected in the web GUI (Renders view) without using the CLI.

## Glossary

| Term | Meaning |
| ---- | ------- |
| Source | An imported video or audio file, with probe data, proxy and optional transcript |
| Unit | A numbered sentence-like chunk of transcript words; the agent cuts by unit id |
| Clip | A named output with segments, a layout, captions, and an optional music bed |
| Segment | A time range of a source placed in a clip |
| Layout | How source frames fill the 9:16 canvas (for example crop, blur-fit, split-vertical) |
| Registry command | A named, schema-described operation (`group.action`) used by every front end |
| Revision | A version number of a clip; undo and restore use snapshots per revision |
| QC report | The result of `qc.run`: a list of checks with pass or fail and a fix hint for each failure |
| Proxy | A smaller copy of a source used for analysis and preview |
| Knowledge book | The guides and MCP prompts embedded in the binary for agents |
