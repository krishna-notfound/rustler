# Vision & Requirements (source of truth for the user's intent)

Captured 2026-11-15 from the user's master prompt + chat instructions. Full master prompt sections are
summarized here; when in doubt, this file wins over memory.

## One-line vision
An autonomous AI clipping & editing environment: give any agent (Claude Code, opencode, Antigravity,
Muse, …) access and it discovers sources, picks moments, edits, captions, reframes, mixes audio,
renders, checks quality, fixes problems, and delivers finished short-form clips. Human review optional.

**The agent is the editor. The software is its complete editing environment.**

## Hard requirements
1. **Research first** (repos in `repos/`), inspect real code, comparison matrix per category, license inventory.
2. **Best-of-breed synthesis** — no single repo is the foundation; extract best algorithms/assets/patterns.
3. **Rust core** (clap, tokio, serde, anyhow/thiserror, tracing). FFmpeg/FFprobe orchestrated, not rewritten.
4. **Agent-agnostic interfaces** on one shared core: CLI (JSON output, exit codes, help, discoverable),
   MCP server (typed schemas, high-level + granular tools), agent skills/docs. Optional Python SDK / REST.
5. **Tauri GUI** on the same core — dashboard, source browser, preview, timeline, subtitle preview,
   asset/template browser, render status, agent activity history. Never a separate engine.
6. **Full agent control, iterative** — not a fixed pipeline. Agent can revise cuts, reposition captions,
   re-crop, re-render, compare variants.
7. **Agent decides, tools execute**: the agent tells the system *where* to cut / what style; the tool
   runs FFmpeg etc. Agents should not need to hand-write FFmpeg commands.
8. **Pluggable AI providers**: user can configure custom **STT, TTS, LLM** endpoints (OpenAI-compatible
   APIs, local models) for tasks that need them.
9. **Quality loop**: plan → execute → evaluate (hook, context, sync, caption readability/position,
   face visibility, crop, audio, export integrity) → correct → iterate. Exit code 0 ≠ good video.
10. **Creative asset library** with metadata/tags + licenses: caption presets, fonts, pairings,
    animations, backgrounds, templates, transitions, SFX, music, layouts.
11. **Portable modular project format** — resumable, hand-off-able between agents, history kept.
12. **Security**: permissioned filesystem/network/download/publish; no DRM/access-control bypass;
    never auto-publish.
13. **Spec-driven development**: Research → Architecture → Specs → Plan → Implement (vertical slice
    first) → Integrate → Agent testing → Optimize. No big implementation before specs approved.

## Process instructions from the user (chat)
- Repos cloned to `repos/`; one analysis agent per repo; results in `docs/research/`.
- Use `rea` for findings (see `docs/tooling-notes.md` for its status and scope).
- Use `/superpowers:brainstorming` to turn findings into the `docs/features/` folder, then
  `/superpowers:writing-plans` for implementation plans.
- Model tiering: Opus for high-complexity work, Sonnet for medium, Haiku for minor/mechanical tasks.
- Keep everything documented in `docs/` so work never gets lost between sessions.

## Confirmed by user (brainstorming)
- 2026-11-15: Core playback confirmed. **Use existing system-level tools (FFmpeg, etc.); do not build
  our own media engine.** The system wraps proven tools so agents can drive them; agents make the
  decisions, the tools do the work.

- 2026-11-15: **Full video understanding, not transcript-only.** The agent must be able to see frames,
  watch the video, know what is happening, and find the best hooks from complete understanding
  (visual + audio + speech). Content type not restricted to podcasts.
  Proposal (pending playback confirmation): per-source "understanding index" = transcript with word
  times + scene/shot list + keyframes & contact sheets + audio events (loudness spikes, laughter,
  music, silence) + faces/speakers + on-screen text (OCR); agent can request any frame or short
  preview on demand. Agents without vision use a configurable vision-LLM endpoint.

- 2026-11-15: **Purpose: the user clips to earn money.** The tool runs on the user's own machine
  (commercial use, not hosted for others). Claude's call: still reimplement ideas from AGPL/GPL/
  modified-Apache repos instead of copying code, so the tool can be sold or shared later without
  license problems.

- 2026-11-15: **Lightweight, portable, agent-installable.** GPU is optional (use it when present,
  CPU fallback otherwise). Must run inside cloud agents' own machines too (e.g. Muse in its sandbox).
  An agent must be able to install/bootstrap the whole toolkit by itself.
  Claude's proposal (pending confirmation): one Rust binary per OS + a `setup` command that fetches
  FFmpeg static build and models into a local tools dir; no required Python stack — STT via
  whisper.cpp (CPU/CUDA) or any STT endpoint; face detection via small ONNX model (e.g. YuNet, MIT);
  LLM/vision via configured endpoints or the agent's own vision.
- Payment model (campaign views vs per-clip fee) not answered; Claude's call: the per-job brief
  supports both.

- 2026-11-15: **Online agent workflow.** An online/cloud agent finds source videos and clip
  opportunities itself, installs ("ingests") the toolkit, and has full control over the video.
  Claude's call (Muse machine details not given): design for the worst case — Linux x86_64, no GPU,
  internet available; Windows/macOS and GPU are bonuses. Offline bundle deferred.
- Per-job **brief** (creator, platform, clip length, caption style, required text/tags, avoid-list)
  read before editing; every clip checked against it before "done". Supports campaign-per-view and
  per-clip-fee work.

- 2026-11-15: **Approach B chosen** (Rust core + Python engine), personal tool. GUI = Premiere-style
  editor incl. multi-region layouts (webcam/gameplay split); agent and software are one.
- 2026-11-15: **Languages: English + Hinglish.** **CLI is the complete primary surface** — Muse uses CLI
  only; locally-running agents (Antigravity) may use the GUI; GUI and CLI have the same capabilities.

## User's machine (checked 2026-11-15)
- Intel i7-14650HX (16 cores), 16 GB RAM, NVIDIA RTX 4060 Laptop GPU (8 GB VRAM) → NVENC encoding
  and local faster-whisper are realistic; large local vision LLMs are not (use endpoints).
- Rust (cargo) and Python installed. **FFmpeg not on PATH** — needs installing at build time.

- 2026-11-15: "discord" was a typo for **discover** — confirmed. MCP/CLI must let agents discover
  all commands themselves.
- Agents that will use it: Muse (cloud agent), computer-use agents (may drive the **GUI**), later a
  Grok bot — each on **its own machine**. Toolkit is installed and run where the agent lives.
  Claude's call: delivery is the agent's job — the toolkit writes finished clips + a per-clip report
  (score, brief check, notes) to the project's `exports/` folder; upload/delivery adapters deferred.
- 2026-11-15: **No AI API integrations.** Rustler calls no LLM/vision/TTS/STT APIs. The agent using it
  (Muse, Claude Code, Antigravity, …) does all the thinking — choosing moments, hooks, styles, judging
  quality by looking at frames/contact sheets. Rustler = local tools only (FFmpeg, yt-dlp, local
  faster-whisper, OpenCV). Provider layer (f13) and LLM-based helpers are dropped from scope.
- 2026-11-15 (02:40): **Goal: complete everything** — the full CLI for every tool in the design, then
  the full GUI (Premiere-style editor, built in the web GUI). CLI must be complete. No 5-hour cap.
  Plan: `docs/superpowers/plans/2026-11-15-complete-rustler.md`.
