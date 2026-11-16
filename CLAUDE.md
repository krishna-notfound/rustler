# Rustler — Claude Code Instructions

> Rustler is a personal, local, agent-native clipping and editing studio: an AI agent discovers source videos,
> picks moments, edits, captions, reframes, renders and quality-checks short-form clips through one command
> registry exposed as a `rustler` CLI, an MCP server and a simple local web GUI.

This file is loaded into every session. It exists so each session starts with the same
verified knowledge and does not guess. **Read it fully, then follow the session start protocol.**

## 1. Session Start Protocol (do this first, every session)

1. Read the project documentation **in this order** (all of `docs/` is the documentation; the user rule is that
   all project documentation lives there and is kept current after every merge):
   1. `docs/README.md` — docs index, repository layout, phase status
   2. `docs/progress-log.md` — **current truth**: what is built, what was tested, problems, decisions, open questions
   3. `docs/00-vision-and-requirements.md` — the user's vision and every confirmed decision
   4. `docs/superpowers/specs/2026-11-15-rustler-design.md` — approved system design (architecture, contracts)
   5. `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` — implementation plan; read the
      "5-HOUR SCOPE" section at the end, which overrides the slice ordering
   6. `docs/features/README.md` — feature index with status
2. Read the context files in `docs/context/` (these are the working summaries; where they differ from the
   progress log, the progress log wins):
   1. `docs/context/project-overview.md` — what the product is, goals, scope, success criteria
   2. `docs/context/architecture-context.md` — stack, boundaries, data model, invariants
   3. `docs/context/ui-context.md` — the simple web GUI: theme, tokens, layout
   4. `docs/context/code-standards.md` — implementation rules, testing, definition of done
   5. `docs/context/ai-workflow-rules.md` — how work is scoped, split, verified, delivered
   6. `docs/context/progress-tracker.md` — pointer to the progress log, open questions, deferred work
3. Read **only** the feature file(s) for the task at hand: `docs/features/fNN_<name>.md`.
4. If `docs/context/AGENTS.md` applies, obey it too (it lists the read order for non-Claude agents).
5. State in one or two sentences what you understand the current task and phase to be
   before you change anything.

## 2. Source of Truth Hierarchy

When two sources disagree, the higher one wins:

1. The user's latest explicit instruction in this session
2. `docs/features/fNN_*.md` for the feature being built
3. `docs/progress-log.md` for what is actually built and tested
4. `docs/superpowers/specs/` and `docs/superpowers/plans/` for design and intent
5. `docs/context/*.md`
6. Code that already exists in the repo
7. Your own memory or training data (**lowest — never overrides the above**)

## 3. Anti-Hallucination Rules

- **Never invent** product behavior, command names, JSON fields, file names, env var names, package
  versions, or library APIs. If it is not in the docs, the feature file or the code, it is unknown.
- **Verify before you cite**: before naming a file, command, function, route, or flag, confirm it
  exists (Glob/Grep/Read). Command names are the `group.action` strings in `crates/rustler-commands/src/*_cmds.rs`
  (see `rustler commands list`). Quote paths as absolute paths or `path/to/file.rs:line`.
- **Verify library APIs against the pinned version**: Rust versions are in `Cargo.toml` and `Cargo.lock`;
  Python versions are in `engine/pyproject.toml` and `engine/uv.lock`. Do not rely on training data for APIs.
- **Unknown or ambiguous → write it down, do not guess**: add it to *Open Questions* in
  `docs/progress-log.md` (and `docs/context/progress-tracker.md`), ask the user if it blocks, otherwise continue.
- **Say what you did not verify.** "It compiles" is not "it works". Run it, look at the output, report the real result.
- **No fake functionality.** Every command, flag, button and status must do what it says. No demo numbers.
- **Do not expand scope.** Build what the active feature file says, nothing adjacent.

## 4. Working Rules

- Spec-driven: **no code for a feature until `docs/features/fNN_*.md` exists and is filled in.**
- One feature unit at a time; small, verifiable increments (see `docs/context/ai-workflow-rules.md`).
- Frozen contracts are not changed unless the task says so: `crates/rustler-core` models, ids, errors and
  events, and `crates/rustler-registry`. An agent that needs a contract change stops and reports it
  (`docs/agent-brief.md`).
- After each meaningful change: append a dated entry to `docs/progress-log.md`, update the feature file's
  *Status Log*, and update the index in `docs/features/README.md`. If architecture, scope or standards change,
  update the matching `docs/context/` file **before** continuing.
- Commit only when the user asks (or when the plan says a task ends in a commit). Never push without being asked.
- Windows: run `cargo` from PowerShell. In Git Bash, `link.exe` resolves to GNU coreutils and linking fails.

## 5. Parallel Work (worktrees + sub-agents)

- Each parallel agent gets its **own git worktree and branch** and owns a **disjoint set of
  files** listed in its feature file (`Owned files`). Agents never edit files they do not own.
- Shared contracts (core models, registry, `PROTOCOL.md`) are written and frozen **before** agents start.
  An agent that needs a contract change stops and reports it.
- Each agent uses its own `CARGO_TARGET_DIR` when building in parallel.
- Every agent reports: files created/changed, commands run + real results, open questions.
- The orchestrator reviews and merges; nothing merges without `cargo build --workspace` and `cargo test --workspace` passing.
- Model tiering (plan, execution notes): contracts, render compiler and end-to-end work by the strongest model;
  feature crates by a mid-tier model; docs and feature files by the cheapest model; all reviewed.

## 6. Commands

| Purpose | Command |
| ------- | ------- |
| Build | `cargo build --workspace` |
| Run the CLI | `cargo run -p rustler-cli -- <args>` or `target/debug/rustler.exe <args>` after building |
| Install local tools and engine | `rustler setup` (writes to `~/.rustler`, i.e. `RUSTLER_HOME`) |
| Health check | `rustler doctor` (JSON; `ready: true` means the render path can run) |
| Command list | `rustler commands list` (187 commands) and `rustler commands describe <group.action>` |
| Knowledge book | `rustler guide list`, `rustler guide show start-here`, `rustler prompt list` (embedded from `knowledge/`; `cargo test -p rustler-knowledge` checks it parses) |
| Docs check | after changing docs, verify every `rustler <group> <action>` you wrote against `commands list` |
| JSON args on Windows PowerShell 5.1 | quotes inside JSON flags are stripped: use `--input-json @file.json` |
| Local GUI | `rustler gui` (or `cargo run -p rustler-cli -- gui`); local page on `127.0.0.1` with a token |
| MCP server | `rustler mcp serve` (stdio) |
| Type check | `cargo check --workspace` |
| Test (Rust) | `cargo test --workspace` (one crate: `cargo test -p rustler-media`) |
| Test (engine) | `~/.rustler/bin/uv run --project engine pytest -q` |
| Lint | not configured in this repo |

Run cargo from PowerShell on Windows (see section 4). Tests that need ffmpeg, yt-dlp or uv skip with a printed
reason when those tools are missing; live and network tests are `#[ignore]`-gated and run on demand.

## 7. Repository Map

```text
docs/        all project documentation (index: docs/README.md)
  context/     working summaries for agents (this read order)
  features/    feature index (README.md) + per-feature files fNN_*.md
  research/    per-repo research reports, comparison matrix, license inventory
  superpowers/ approved design spec and implementation plan
crates/      Rust workspace (members = crates/*)
  rustler-core/         models, ids, errors, events, project store, timemap, units, layout
  rustler-registry/     command registry, typed() helper, envelope, progress sink
  rustler-tools/        setup, doctor, tool installers (ffmpeg, yt-dlp, uv, engine venv, fonts)
  rustler-engine-bridge/ JSON-RPC supervisor and client for the Python engine
  rustler-media/        probe, proxy, frames/tiles, audio analysis, facepath, compile, render, QC
  rustler-captions/     caption presets, ASS writer, chunking
  rustler-commands/     all 187 registry commands (`*_cmds.rs`: setup, project, config, auto (`clips.auto`), grade, source, analyze, understand, transcript, transcript_import, highlights, see, clip, edit, cut, timeline, timeline_edit, layout, smart, speaker, overlay, keyword, cover, layer (image/cutaway/mask), card (asset render-card, text styles), fx (transition/effect), audio, asset, template, reference, campaign, post, director, brief, variant, render, qc, fix, guide)
  rustler-cli/          `rustler` binary (generic CLI over the registry)
  rustler-mcp/          MCP server over stdio
  rustler-gui/          local web GUI and editor (axum server + static page, 14 JS modules, no build step)
  rustler-knowledge/    loader that embeds knowledge/ into the binary
engine/      Python worker `rustler_engine` (uv project): faster-whisper, scenes, YuNet faces, OCR, audio events, speaker diarization, translit
             PROTOCOL.md = the JSON-RPC contract
knowledge/   agent guides (23) and MCP prompts (15), embedded in the binary
repos/       cloned upstream repos studied in research (gitignored, read-only)
```

## 8. Project Snapshot

- **Stack:** Rust workspace (edition 2024; clap 4.6.7, tokio 1.54.1, serde_json) + Python 3.11 engine
  managed by uv (faster-whisper, OpenCV headless, PySceneDetect, indic-transliteration) +
  FFmpeg/ffprobe (static build with libass) and yt-dlp as subprocesses. Local machine only.
- **Current phase:** Slice 1 plus the 5-hour extras are built; end-to-end runs passed on 2026-11-15. The live
  record is in `docs/progress-log.md`; per-feature status is in `docs/features/README.md`.
- **Built since Slice 1** (all merged; per-feature status in `docs/features/README.md`): layout regions and webcam detection (f15),
  overlays/hook titles, transitions, effects, progress bar (f18), cut refinement (f19), highlight helpers (f20), understanding v2 (f07b),
  QC v2 and fix loop (f21), brief/variants/batches (f22), asset library (f23), GUI editor (f25-f27), speakers (f29), project pack/status,
  audio post (denoise, SFX, two-pass loudnorm). 187 registry commands; `rustler commands list` is the authority.
- **v2 (merged, v0.2.0 line):** lite-by-default install and `config.*` settings (f30), campaigns, posting JSON, director/retention
  (f31), style references and transcript import (f32), timeline document and timeline edits: `timeline.*`, cold open, insert, speed,
  freeze, merge/duplicate, image/cutaway/mask layers (f33), smart defaults at `clip.create`, `layout.plan`, `crop.track-active` (f34),
  templates with starters `podcast-viral` and `streamer-split` (f35), hook styles, keyword/emoji captions, flash/whip transitions and
  `clip.cover` (f36), `clips.auto` (f37), colour grading `grade.*` (f38, agent-decided, never automatic). Zooms are
  the agent's decision: nothing adds punch-ins on its own. Agent-facing manual: `knowledge/guides/start-here.md` (v2 recommended flow).
- **Deferred / out of scope right now:** AI API integrations (user decision), voice-over.

## 9. Do Not

- Do not commit `.env*` files, API keys or secrets. Do not add AI API provider calls (user decision: none).
- Do not change the stack, add major dependencies, or restructure folders without recording the
  decision in `docs/progress-log.md` first.
- Do not write FFmpeg command lines for agent workflows; agents call registry commands (`render.run`, `qc.run`).
- Do not publish, upload or post any output. Publishing is manual.
- Do not add a command or a GUI control that has no working handler behind it.
- Do not leave TODOs for core functionality.
- Do not move project documentation out of `docs/`.
