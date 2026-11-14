# Architecture Context

## Stack

| Layer | Technology | Role |
| ----- | ---------- | ---- |
| Core and CLI | Rust, edition 2024 (clap 4.6.7, tokio 1.54.1, serde_json, thiserror) | Registry, project store, render orchestration, CLI, MCP and GUI servers |
| Engine | Python 3.11 (`requires-python >=3.11,<3.13`), managed by uv | Transcription, scene detection, face tracking, transliteration |
| Media | FFmpeg and ffprobe (static BtbN GPL build with libass), run as subprocesses | Probe, proxy, frames, filtergraph render, QC measurements |
| Download | yt-dlp, run as a subprocess | Public YouTube and URL sources |
| Captions | ASS written by Rust, burned in by libass through FFmpeg | Caption presets, karaoke highlight |
| Web GUI | axum server in `rustler-gui`; plain HTML, CSS and JS page | Local view of projects, sources, clips, renders, activity |
| Hosting | The user's Windows 11 machine (RTX 4060, NVENC); not a server | Personal tool, local only |

Verified versions (from manifests, not memory): `Cargo.toml` (clap 4.6.7, tokio 1.54.1, reqwest 0.13.5,
serde_json 1.0.154); `engine/pyproject.toml` (faster-whisper >=1.2.1, opencv-python-headless >=5.0.0.93,
scenedetect >=0.7.3, numpy >=2.4.6, indic-transliteration >=2.3.82). Exact resolved versions are in `Cargo.lock`
and `engine/uv.lock`.

## System Boundaries

- `crates/rustler-core` — models, ids, errors, events, `ProjectStore` (atomic JSON I/O, revisions), `TimeMap`, units, layout presets. Must not call FFmpeg or Python.
- `crates/rustler-registry` — `Command` trait, `typed()`, specs, envelope, progress sink. Must not hold domain logic.
- `crates/rustler-tools` — setup, doctor, tool installers, subprocess helper. Must not own domain commands.
- `crates/rustler-engine-bridge` — supervises the Python worker; JSON-RPC over stdio. Must not interpret ML results beyond typing them.
- `crates/rustler-media` — probe, proxy, frames and tiles, audio analysis, facepath, compile, render, QC. Runs FFmpeg and ffprobe as subprocesses (`proc.rs`, `proc_render.rs`).
- `crates/rustler-captions` — presets, ASS text, chunking. Burn-in happens in `rustler-media` through the render graph.
- `crates/rustler-commands` — every registry command (`*_cmds.rs`). Glue only: validates input, calls core, media, captions, or the engine.
- `crates/rustler-cli` — the `rustler` binary; generic front over the registry. Intercepts `mcp serve` and `gui`.
- `crates/rustler-mcp` — MCP over stdio; tools from the registry, prompts and resources from the knowledge book.
- `crates/rustler-gui` — local web server and static page; every action goes through `POST /api/run`.
- `crates/rustler-knowledge` — embeds `knowledge/`; no runtime I/O.
- `engine/` — Python package `rustler_engine`; stdio JSON-RPC worker. Contract in `engine/PROTOCOL.md`.
- `knowledge/` — agent guides and prompts (content, not code).
- `docs/` — all project documentation.

## Data Model

Project folder layout (written by `ProjectStore`):

| File | Holds | Notes |
| ---- | ----- | ----- |
| `project.json` | Project id, settings (for example `default_whisper_model`), permissions (network, downloads, fs roots) | Atomic write |
| `source.json` | Imported sources: `Source`, `SourceOrigin`, `MediaProbe`, `TimeRange` | Display dims after rotation |
| `transcript.json`, `units.json` | `Transcript`, `Word`, `Unit`, `UnitKind` | Times from word data only |
| `scenes.json`, `audio_events.json` | `Scenes`, `AudioEvents`, `LoudnessPoint`, `Silence` | Analysis outputs |
| `clip.json` | `Clip`, `Segment`, `Layout`, `Region`, `Rect`, `Fit`, `Background`, `CaptionTrack`, `AudioSettings`, `Canvas` | Current revision |
| `history/<rev>.clip.json` | Snapshot of a clip at a revision | Used by undo and restore |
| `extras.json` | Music bed settings and similar per-clip extras | Written by `audio.add_music` |
| `report.json` | `QcReport`, `QcCheck`, `QcStatus` | Written by `qc.run` and export |
| `events.ndjson` | Append-only event log | One line per mutation |

IDs are prefixed ULIDs (`SourceId`, `ClipId`, `RenderId`, `ProjectId`).

## Data Access Layer

- Commands read and write project data through `ProjectStore` in `rustler-core`; no command writes JSON by hand.
- The GUI reads and writes only through `POST /api/run` and `GET /api/file`; it has no separate data path.
- Engine data (transcripts, scenes, face boxes) arrives over the bridge and is stored by the commands.

## Auth and Authorization Model

- Identity: none. Single local user; no accounts, no cloud.
- GUI: server bound to `127.0.0.1`, random port by default, random token per launch, token required on API routes (`token_required` test).
- MCP and CLI: local process only; no network listener (stdio).
- Authorization for side effects: project permissions (network, downloads, filesystem roots) checked in commands
  (for example `require_network` in `source_cmds.rs`). Destructive commands take `--yes`.

## Storage Model

- Project data: project folder on disk, as in the table above.
- Tools and engine: `RUSTLER_HOME` (default `~/.rustler`), with `bin/` for ffmpeg, ffprobe, yt-dlp, uv, and the engine venv.
- Derived media (proxies, extracted audio): inside the project folder; exact paths are defined in `crates/rustler-core/src/store.rs`.
- Knowledge book: embedded in the binary.
- No database. No remote storage.

## Invariants

1. Every capability is a registry command; CLI, MCP and GUI never call a handler directly.
2. Times used for cuts come from word-level data; the agent never supplies model-invented times.
3. `project.json` and `clip.json` are written atomically (temp file, then rename).
4. Exit code 0 is not success: `qc.run` must pass before a render is reported as done.
5. No AI API calls and no provider layer (user decision).
6. The GUI server binds to `127.0.0.1` only.
7. Frozen contracts (`rustler-core` models, ids, errors, events; `rustler-registry`) change only with a recorded decision.
8. Nothing is published or uploaded automatically.

## Known Risks

| Risk | Impact | Mitigation |
| ---- | ------ | ---------- |
| Single-pass loudnorm landed near -15.9 LUFS on half-silent clips | Loudness missed target on quiet clips | Fixed: `render.run` measures first (two-pass, default true); half-silent clip now -14.0 LUFS (f17) |
| Face tracking has no recorded live run | Reframe quality unknown on real footage | Live check with `crop.track_face` (f14 task) |
| Edit operations (f12) have no recorded test run | Undo and edit behaviour unconfirmed in the log | Run `cargo test -p rustler-commands --test edit` and log it |
| Windows linker clash in Git Bash | Build fails from bash | Run cargo from PowerShell |
| Single-platform verification (Windows 11 laptop) | Linux and macOS paths untested | Test on Linux before relying on it there |
| MSRV: deps need rustc >= 1.89 (progress log) | Older toolchains fail to build | Keep rustup stable current |
