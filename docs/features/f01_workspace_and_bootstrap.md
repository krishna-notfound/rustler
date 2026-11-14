# f01 — Workspace and Bootstrap (setup / doctor)

**Status:** Verified  |  **Depends on:** — (foundation)  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

Let any agent with only a shell install every local dependency Rustler needs, with no admin rights and no
interactive prompts, and then report what works. After `rustler setup` the machine has FFmpeg, ffprobe,
yt-dlp, uv, the Python engine venv, the Whisper model and fonts under `~/.rustler/`. `rustler doctor`
reports readiness as JSON.

## Scope

**In:**
- Cargo workspace (`members = ["crates/*"]`) and the uv-managed Python engine project.
- `setup.run` (CLI `rustler setup`): installs/adopts ffmpeg, ffprobe, yt-dlp, uv, the engine venv (`engine` project), Whisper models and fonts into `RUSTLER_HOME` (default `~/.rustler`).
- `doctor.run` (CLI `rustler doctor`): reports tool presence, libass, CUDA engine, and video encoders (for example `h264_nvenc`, `libx264`).
- Download, archive and install helpers for each tool.

**Out (explicitly):**
- Provider/API key setup (f13 was dropped: no AI API integrations).
- Signed release binaries, CI and one-line install script (spec §5.9 lists them; not built).
- Linux and macOS install paths are not verified; the live run was on Windows 11.

## User-facing Behavior

- `rustler setup` runs without prompts (its module docs describe it as idempotent). On the laptop run, existing ffmpeg, yt-dlp and uv were adopted.
- `rustler doctor` prints a JSON envelope; `ready: true` means the render path can run.
- A lone `*.run` command in a group runs without an action word (`rustler setup`, `rustler doctor`); `--help` shows its schema.
- Tool-dependent tests skip with a printed reason when tools are missing (agent brief rule), never fail silently.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `Cargo.toml` (workspace), `engine/pyproject.toml`, `engine/uv.lock`
- `crates/rustler-tools/src/lib.rs`, `paths.rs`, `setup.rs`, `doctor.rs`, `run.rs`
- `crates/rustler-tools/src/download.rs`, `archive.rs`, `ffmpeg_install.rs`, `ytdlp_install.rs`, `uv_install.rs`, `engine_install.rs`, `fonts.rs`
- `crates/rustler-commands/src/setup_cmds.rs`

**Read-only dependencies:**
- `crates/rustler-registry/src/registry.rs` — command registration used by `setup_cmds.rs`.
- `engine/` — installed as the uv project by `engine_install.rs`.

## Data and Interfaces

- Commands: `setup.run`, `doctor.run` (registry names; see `rustler commands describe setup.run` for options).
- Environment: `RUSTLER_HOME` (default `~/.rustler`), tools in `~/.rustler/bin` (`ffmpeg`, `ffprobe`, `yt-dlp`, `uv`).
- Output: JSON envelope `{ok, schema, data, warnings}` (see f02).

## Rules and Edge Cases

1. Setup must not need admin rights; everything lives under `RUSTLER_HOME`.
2. Pre-existing ffmpeg/yt-dlp/uv are adopted (live run on the laptop adopted them).
3. Windows: run `cargo` from PowerShell; Git Bash `link.exe` resolves to GNU coreutils and linking fails.
4. Setup into a temp home is the test pattern (`setup_live.rs`), so the real `~/.rustler` is not touched by tests.

## Tasks

- [x] Workspace skeleton and member crates (`Cargo.toml`)
- [x] Tool installers (ffmpeg, yt-dlp, uv, engine venv, fonts)
- [x] `setup.run` and `doctor.run` registered in `setup_cmds.rs`

## Acceptance Criteria

- [x] `rustler setup` completes into a temp home (`crates/rustler-tools/tests/setup_live.rs`)
- [x] `rustler doctor` reports `ready=true` on the laptop with CUDA engine and NVENC (progress log)
- [x] Path resolution tests pass (`crates/rustler-tools/tests/paths.rs`)
- [x] Doctor output shape is tested (`crates/rustler-tools/tests/doctor.rs`)

## Verification

- `cargo test -p rustler-tools` (PowerShell). Live tests `setup_live.rs` are `#[ignore]`-gated for network use.
- `rustler setup` then `rustler doctor`; pass = `ready: true` and listed encoders present.
- Engine tests: `~/.rustler/bin/uv run --project engine pytest -q`.

## Status Log

- 2026-11-15 — Real `rustler setup` on the laptop passed (engine lite + cuda, whisper small, fonts Anton / Poppins / NotoSansDevanagari; existing ffmpeg/yt-dlp/uv adopted). `rustler doctor`: ready=true, libass, CUDA engine, h264_nvenc / hevc_nvenc / h264_qsv / libx264 OK.
