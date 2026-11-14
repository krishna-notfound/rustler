# Rustler — Agent-Native Clipping & Editing Studio — Design

Status: **APPROVED by user 2026-11-15** · Date: 2026-11-15
Inputs: `docs/00-vision-and-requirements.md` (user intent, confirmed decisions), `docs/research/00-comparison-matrix.md`
(research synthesis; every "COPY/IDEA" source below is detailed there), `docs/research/<repo>.md`.

---

## 1. Intent — the why

The user clips videos to earn money (campaign views or per-clip fees). They want **any AI agent** they use —
Muse (cloud agent), Claude Code, opencode, Antigravity, computer-use agents, later a Grok bot — to install one
toolkit on whatever machine it lives on and then work as a **professional video editor**: find source videos,
watch and understand them (frames, audio, speech), pick the best hooks, cut, lay out, caption, mix, render,
check quality, fix, and deliver finished clips. The human only reviews if they want to.

"The agent and the software are one thing": every capability the software has — including everything a human
can do in its Premiere-style GUI — is a command the agent can discover and call. The agent decides; the tools
execute. Agents never hand-write FFmpeg.

## 2. Goals, non-goals, anti-goals

**Goals**
1. One toolkit (`rustler`) that any agent can install itself and fully drive via CLI or MCP.
2. Agent can *see and hear* the video: frames, labelled frame grids, scenes, audio events, on-screen text, faces.
3. Every editing operation is a typed, discoverable command: same set for CLI, MCP and GUI.
4. Multi-region layouts (e.g. streamer: webcam top half, gameplay bottom half), per-shot layout changes,
   keyframed crops — all editable by agent or human.
5. Premiere-style GUI editor (timeline, canvas with draggable regions, captions, assets, render queue) that
   shows agent work live and lets a human (or computer-use agent) edit the same project.
6. Pluggable OpenAI-compatible endpoints for LLM, vision, STT, TTS; local faster-whisper as default STT.
7. Quality loop: every clip is rendered, *watched* (QC report + contact sheet), checked against the job brief,
   and fixed before it counts as done.
8. Runs on a cloud Linux box with no GPU; uses GPU (NVENC/CUDA) when present (user laptop: RTX 4060 8 GB).

**Non-goals (this version)**
- Auto-posting/uploading to platforms (publishing stays manual — the agent delivers files).
- Hosting Rustler as a service for other people (it is a personal tool).
- Rewriting FFmpeg or any media codec; training our own ML models.
- Bypassing logins, paywalls or DRM when downloading sources.

**Anti-goals (would be failure even if it "works")**
- A fixed one-shot pipeline the agent can't step into, override or revise.
- GUI-only features the agent can't reach, or agent features the GUI can't show.
- An install that needs a human (interactive prompts, manual downloads, admin rights) on a cloud agent box.
- "Exit code 0" treated as success without the output being checked.
- Hallucinated timestamps: the agent/LLM must never invent times; times come from word-level data.

## 3. Constraints

- **Rust** core (clap, tokio, serde, anyhow/thiserror, tracing). **FFmpeg/FFprobe as subprocesses**, static
  builds fetched by `setup`.
- **Python engine** for ML/perception (approach B, chosen by user), managed by **uv** (self-contained Python,
  no admin rights). Default install is **CPU-only and torch-free** so cloud sandboxes stay light; GPU/"full"
  extras are optional.
- Windows 11 (user laptop) + Linux x86_64 (cloud agents) must both work; macOS best-effort.
- Licenses: personal tool, but **Claude's call**: copy code only from MIT repos (openshorts, autoclip,
  bridgeclip, opensource-clipping) with attribution; AGPL/GPL/modified-Apache repos (supoclip, ViralCutter,
  hypit) are ideas-only. Avoid ultralytics/YOLO (AGPL), InsightFace weights (non-commercial), unlicensed
  media from the repos. Every bundled asset has a license manifest entry.
- Never auto-publish. Downloads only from public sources the agent is permitted to fetch (yt-dlp).

## 4. Approaches considered

| | Approach | Trade-off | Decision |
|---|---|---|---|
| A | Rust-native: Rust + FFmpeg + whisper.cpp + ONNX, everything reimplemented | Lightest install; slowest to first clip; can't reuse Python code | Rejected by user |
| **B** | **Rust core + Python engine**: Rust owns CLI/MCP/project/render/GUI; Python worker does transcription, face tracking, scene/audio/OCR analysis, reusing MIT Python code | Reuses proven MIT code and best Python ML libs (faster-whisper, OpenCV); heavier install — mitigated with uv + CPU-only default | **Chosen by user** |
| C | Fork bridgeclip/openshorts and add agent tools | Fastest start; Electron/Python-heavy, locked into their pipeline design | Rejected — conflicts with "agent controls everything" |

## 5. Architecture

```
            agents (Muse, Claude Code, opencode, Grok, computer-use)          human
                 │ CLI (JSON)        │ MCP (stdio / loopback HTTP)        │ GUI (Tauri)
                 ▼                   ▼                                    ▼
        ┌──────────────────────────────────────────────────────────────────────────┐
        │                COMMAND REGISTRY  (single source of truth)                │
        │  every command: name, JSON-Schema in/out, side-effects, annotations,     │
        │  help, examples. CLI subcommands, MCP tools and GUI actions are          │
        │  generated from / dispatched through it.                                 │
        └──────────────────────────────────────────────────────────────────────────┘
                 │
        ┌────────┴──────────────────────────────────────────────────────────────────┐
        │ rustler-core: project store · timeline model · edit ops · history · jobs  │
        │ rustler-media: ffprobe · filtergraph compiler · render · previews · QC    │
        │ rustler-captions: templates → ASS · font metrics · placement              │
        │ rustler-providers: OpenAI-compatible LLM/vision/STT/TTS · keys · cost     │
        │ rustler-engine-bridge: supervises Python worker (JSON-RPC over stdio)     │
        └────────┬───────────────────────────────┬──────────────────────────────────┘
                 │ subprocess                     │ JSON-RPC stdio
           ffmpeg / ffprobe / yt-dlp     python engine (uv venv): faster-whisper, OpenCV YuNet
           (static, in tools dir)        face tracking, PySceneDetect, audio events (ONNX),
                                         OCR (RapidOCR), optional WhisperX/diarization
```

**Event bus:** core emits project events (`edit.applied`, `job.progress`, `render.done`…) to an append-only
`events.ndjson` in the project + a local websocket/IPC channel, so the GUI updates live while an agent works.

### 5.1 Repository layout (Claude's call)
```
rustler/
  Cargo.toml                 workspace
  crates/
    rustler-core/            project model, timeline, edit ops, history, events, errors
    rustler-registry/        command registry, schemas, dispatcher
    rustler-media/           ffmpeg/ffprobe wrappers, filtergraph compiler, render, QC
    rustler-captions/        caption templates, ASS writer, font metrics, placement
    rustler-providers/       endpoint config, OpenAI-compatible clients, JSON recovery, keyring
    rustler-engine-bridge/   python worker supervisor + typed RPC client
    rustler-cli/             `rustler` binary (clap) — thin over registry
    rustler-mcp/             MCP server (rmcp) — thin over registry
  engine/                    python package `rustler_engine` (uv project, pyproject.toml)
  apps/gui/                  Tauri app (Rust backend reuses crates) + React/TypeScript/Vite frontend
  assets/                    asset manifests (fonts, presets, layouts, sfx, music) — binaries fetched by setup
  skills/rustler/            SKILL.md + references for agents; AGENTS.md for non-Claude agents
  docs/ docs/features/ docs/research/ repos/ (research, read-only)
```

### 5.2 Command registry & CLI contract
- Every command is declared once: `name` (`project.create`, `source.import`, `edit.cut`, `layout.set`, …),
  input/output types (Rust structs with `schemars` → JSON Schema), annotations (read-only / destructive /
  costs money / network), examples.
- Discovery: `rustler commands list`, `rustler commands describe <name>` (schema + examples),
  `rustler help <topic>`. MCP `tools/list` is generated from the same registry.
- Output envelope (stdout, always JSON with `--json`, default when not a TTY):
  `{ "ok": true, "schema": "rustler.<cmd>@1", "data": {...}, "warnings": [] }` or
  `{ "ok": false, "error": { "code", "stage", "message", "hint", "retryable" } }`.
- Progress: NDJSON on stderr (`{"event":"progress","job":..,"pct":..,"stage":..}`).
- Exit codes: 0 ok · 2 usage/validation · 3 not found · 4 dependency missing · 5 provider error ·
  6 media/ffmpeg error · 7 QC failed · 8 cancelled · 10 internal.
- Long work runs as **jobs** (`job.start/status/wait/cancel`), resumable after crash (stage artifacts +
  input hashes).
- `--dry-run` on every mutating command: validates and shows what would change/cost.

### 5.3 Project format (portable, modular, resumable)
```
<project>/
  project.json            id, name, revision, settings, source list, brief ref
  brief.json              job brief (creator, platform, durations, caption style, required/forbidden, payout model)
  sources/<id>/           source.json (path/url, probe), proxy.mp4, audio.wav
  analysis/<source-id>/   transcript.json (words+times+speaker), units.json, scenes.json, audio_events.json,
                          faces.json (tracks), ocr.json, tiles/, frames/   ← the "understanding pack"
  clips/<clip-id>/        clip.json (timeline: tracks, items, layouts, captions, audio, overlays),
                          renders/, qc/<render-id>.json, contact sheets
  exports/                finished files + <clip>.report.json (score, brief check, QC summary, notes)
  history/                ops.ndjson (append-only typed ops, undo/redo), snapshots
  events.ndjson           activity feed (agent + human actions)
```
Saves are **revision-checked** (op carries `base_revision`; conflict → error with current revision), so an
agent and a human in the GUI can edit the same project safely.

### 5.4 Timeline & layout model (the heart of "Premiere-style" + agent control)
- **Clip timeline** = ordered **segments** (keep-ranges from one or more sources, in source time, referenced by
  transcript unit/word ids where possible) → output time computed via a TimeMap.
- **Layout** per segment (or per shot) = list of **regions**: `{ id, source_rect (normalized 0–1, keyframable),
  dest_rect (normalized on output canvas), z, fit: cover|contain|blur-fill, border, radius, mask }`.
  Presets: `single-crop`, `blur-fit`, `split-vertical` (webcam top / gameplay bottom), `split-2speaker`,
  `screencast+inset`, `pip`. Agents/humans can create custom layouts and save them as presets.
- **Crop tracking**: a region's `source_rect` can be driven by a **CropPath** (keyframes from face tracking,
  smoothed One-Euro + dead zone + speed cap; editable).
- **Tracks**: video regions, captions (word-anchored events — they move with their words on re-cut),
  text/overlay layers (hook title, stickers, progress bar), audio (source, music bed with ducking, SFX cues,
  voice-over).
- **Edit ops** (typed, each a registry command): `edit.trim`, `edit.split`, `edit.cut_words`,
  `edit.remove_pauses`, `edit.remove_fillers`, `layout.set/apply_preset/edit_region`, `crop.set_keyframe`,
  `crop.track_face`, `caption.set_style/edit_words/move`, `overlay.add/edit/remove`, `audio.add_music/duck/sfx/
  denoise`, `transition.set`, … plus `history.undo/redo`.

### 5.5 Understanding pack (agent eyes & ears)
Built by `analyze.*` commands (each runnable separately; `analyze.all` runs them as a job):
transcript+units (faster-whisper CPU int8 / CUDA; endpoint STT alternative; optional WhisperX alignment),
scenes (PySceneDetect), faces & tracks (OpenCV YuNet + tracker), audio events (loudness curve, silences, peaks;
laughter/applause/music via an ONNX audio classifier — model choice in feature spec), on-screen text
(RapidOCR on keyframes). Inspection commands return **images** the agent can look at (MCP image content; CLI
returns file paths): `see.frame`, `see.tiles` (grid labelled with timestamps + spoken words, `--around "<phrase>"`),
`see.filmstrip`, `see.preview` (short low-res clip), `see.contact_sheet` (of a render).
Agents without vision call `see.describe` → configured vision endpoint.

### 5.6 Highlight finding (optional helpers, never mandatory)
Agent can pick clips itself from the understanding pack, or call helpers: `highlights.signals` (deterministic:
loudness peaks, laughter, questions, speaker turns), `highlights.candidates` (LLM two-pass over windows →
shortlist → rerank, returns **unit ids + verbatim opening-words quote**, code verifies), `highlights.judge`
(yes/no rubric). Each stage is its own command so the agent can replace any of them.

### 5.7 Render & quality loop
- Compiler turns `clip.json` → one `ffmpeg -filter_complex` (regions, crops via `sendcmd`, ASS captions,
  overlays, audio graph, loudnorm) → **one encode per deliverable**. Encoder tiers: NVENC/QSV/AMF/VideoToolbox
  probed with a real 1-frame encode, CPU libx264 fallback.
- `render.preview` (single frame or low-res, same libass path → preview matches export).
- `qc.run`: integrity (ffprobe, duration, streams), loudness (ebur128), black/freeze/silence detect, caption
  safe-zone/overflow, caption-over-face, face-visible ratio, hook-in-first-3 s, brief compliance → JSON report +
  contact sheet the agent must look at. `qc.review` (vision LLM rubric, optional). Failed checks carry
  `fix_hint`s that map to edit ops.

### 5.8 Providers
`rustler config provider add` — named endpoints `{kind: llm|vision|stt|tts, base_url, model, key_ref,
capabilities}`; OpenAI-compatible wire format (`/chat/completions`, `/audio/transcriptions`, `/audio/speech`).
Keys in OS keyring or `RUSTLER_<NAME>_KEY` env; never in argv/logs. JSON-output ladder (json_schema →
json_object → prompt) + recovery (strip `<think>`, brace-matched extraction, one repair retry). Cost/usage log.
Note: the *driving agent* is itself an LLM — provider calls are only for helper tasks (auto-highlights, vision
describe, TTS, STT endpoint).

### 5.9 Install & bootstrap (agent self-install)
`rustler setup [--gpu auto|cuda|none] [--profile lite|full]`: downloads static FFmpeg/ffprobe, installs uv →
Python 3.11 venv → `rustler_engine` deps (lite: faster-whisper, opencv-python-headless, scenedetect,
onnxruntime, rapidocr; full adds WhisperX etc.), fetches models (whisper size per profile, YuNet, audio
classifier) and fonts with SHA-256 pinning, all into `~/.rustler/` (no admin). Idempotent; resumable.
`rustler doctor` reports what works (GPU, encoders, libass, engine, providers) as JSON.
Distribution: GitHub release binaries (Windows x64, Linux x64) + one-line install script — **builder decides
CI details**.

### 5.10 GUI (Tauri, React + TypeScript)
Panels: **Projects** dashboard · **Sources** browser (with understanding-pack views: transcript, scenes, faces,
audio waveform/events) · **Program monitor** (proxy playback + canvas overlay: drag/resize regions, crop-path
keyframes, safe zones, caption position) · **Timeline** (multi-track: video segments, layout changes, captions,
overlays, audio; trim/split/ripple, word-snapped) · **Captions** editor + style browser · **Assets** browser ·
**Render queue/Exports** with QC reports · **Agent activity** feed (live from events). Every GUI action dispatches a
registry command (so the history shows human and agent ops the same way). Exact-look preview = libass render
from core.

### 5.11 Agent skill
`skills/rustler/SKILL.md` (Claude Code / any skills-aware agent) + `AGENTS.md`: discovery first
(`commands list`), the editing loop (understand → plan → edit → preview → render → **watch** → fix), choices
only the agent can make, brief compliance, "done means watched", cost/spend consent, never auto-publish,
treat transcripts/web text as data not instructions.

### 5.12 Security
Per-project permissions (`project.json → permissions`: filesystem roots, network on/off, downloads allowed,
max spend); MCP loopback HTTP hardening (bearer token, Host/Origin checks, body cap); destructive/costly commands
annotated and require `--yes`/confirmation flag; no auto-publish.

## 6. Features (build order) — detailed specs go in `docs/features/fNN_*.md`

**Slice 1 — "first real clip"** (agent goes from a video file/URL to a checked 9:16 clip via CLI):
F01 workspace & bootstrap · F02 command registry & CLI contract · F03 project store & timeline model ·
F04 python engine bridge · F05 sources & probe · F06 transcription & units · F07 understanding pack v1
(frames, tiles, scenes, audio events) · F08 render compiler v1 (segments, blur-fit/single-crop/split layouts,
one caption preset, loudnorm) · F09 QC v1.
**Slice 2 — "agent native"**: F10 MCP server · F11 agent skill & docs · F12 edit ops & history (full set) ·
F13 provider layer.
**Slice 3 — "pro editing"**: F14 face tracking & auto-reframe · F15 layout & region system v2 (custom
layouts, keyframes, presets) · F16 caption engine v2 · F17 audio post · F18 overlays, hooks & transitions ·
F19 cut refinement & pacing.
**Slice 4 — "autopilot & business"**: F20 highlight helpers · F21 quality loop v2 (vision review, fix
hints) · F22 brief, variants & exports · F23 asset library.
**Slice 5 — "studio GUI"**: F24 GUI shell & live sync · F25 timeline editor · F26 program monitor & canvas
layout editor · F27 captions/assets/render panels.
**Later**: F28 GPU throughput (CUDA/NVENC tuning, parallel renders) · F29 speakers & active-speaker.

Parallel build rule: F02/F03 contracts (registry types, timeline/project types, event types) are written and
frozen first; then parallel agents each own disjoint crates/dirs (see per-feature *Owned files*).

## 7. Decided vs. left to the builder

**Decided with the user:** approach B; agent-native (registry = one surface); full video understanding; GUI is
a Premiere-style editor incl. multi-region layouts; runs on each agent's own machine; GPU optional; personal tool
for earning; brief per job; downloads respect platform rules; no auto-publish.

**Claude's calls (user can override):** name `rustler`; crate layout above; React+TS for GUI frontend; uv for
Python; lite/full install profiles; faster-whisper default STT; YuNet faces; ASS/libass captions (no Remotion);
unit-id + quote contract for time; one encode per deliverable; MIT-only code copying; delivery = `exports/` +
report (agent delivers onward); Linux-no-GPU as the baseline target.

**User answers to the builder check (2026-11-15):**
- **Languages: English + Hinglish.** Claude's call: Hinglish captions default to **Latin script**
  (romanized), with `caption.script = latin | devanagari | as-transcribed`; transcription uses a multilingual
  whisper model; Hindi words that come back in Devanagari are transliterated to Latin (Python
  `indic-transliteration` or LLM, chosen in f06 spec); Devanagari fonts (e.g. Noto Sans Devanagari, Mukta —
  OFL, verify) available; filler/weak-opener lists for English + Hinglish.
- **CLI is the complete, primary surface.** Muse works CLI-only on its own machine and never opens the GUI.
  Agents running locally (e.g. Antigravity) may use the GUI. Rule: **every capability exists as a CLI command
  first**; GUI and MCP are views over the same commands — nothing is GUI-only. No remote-GUI access needed;
  project format still uses relative paths so a project folder can be copied between machines.

**Claude's calls from the builder check (2026-11-15):**
- *Unknown cloud-agent sandbox limits* → no daemon required. Every long command runs in the foreground by
  default, saves stage artifacts, and is **resumable** (rerun continues); `--max-seconds N` lets an agent work
  in chunks under per-call timeouts; `--detach` is optional where background processes survive. Lite profile
  budget target ≤ 1.5 GB incl. models; default CPU whisper model `small` (int8), configurable.
- *Long sources (hours)* → two-depth analysis: a **cheap pass** over the whole source (audio loudness/silence,
  scenes on low-res proxy, fast transcript with small model) and a **deep pass** (faces, OCR, accurate
  transcript, frames) only on windows the agent selects. Partial results saved per window.
- *Quick scan for online sources (user idea, 2026-11-15)* → **scan before download**: `source.info` +
  `source.fetch_transcript` use yt-dlp with no media download to get existing captions (uploaded or
  auto-generated; json3/vtt with timings), chapters, **"most replayed" heatmap**, title/description and
  (optionally) timestamped comments. Agent picks windows from these, then `source.import --sections
  "00:47:00-00:49:30"` downloads **only those parts**; whisper re-transcribes just those parts for exact word
  timing (YouTube timing is approximate). No captions available → download audio only + fast whisper pass.
  Local files: audio loudness/silence + scenes on low-res proxy + fast small-model transcript.
- *Finding sources* → the agent uses its own web search/browsing; Rustler provides `source.import` (file/URL)
  plus `source.search` (yt-dlp search of public results) and `source.info` (metadata without download).
- *Logins/cookies* → opt-in: the user may supply their **own** cookies file per project (`permissions.cookies`),
  stored outside the project, redacted from logs. Never shared or used to bypass DRM/paywalls. Local files
  remain the primary way in when URL import fails on cloud IPs.
- *Plan scope* → F01–F09 specs (Slice 1) are written and approved first; the implementation plan covers
  Slice 1 in detail including the frozen F02/F03 contracts; later slices get specs + plans slice by slice.

**Left to the builder:** internal module structure within crates; exact Rust/Python library versions (verify
current); CI/release mechanics; audio-classifier and OCR model choice (license-verified); GUI component library
and styling; exact QC thresholds (start from matrix values, tune on real clips).
