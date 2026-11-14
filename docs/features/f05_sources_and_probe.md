# f05 — Sources and Probe

**Status:** Verified  |  **Depends on:** f02, f03  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

An agent can bring a source video into a project from a local file or a YouTube URL, and inspect it
before downloading. Scan-first: read chapters, heatmap peaks and captions for a URL without downloading the
video, then download only the chosen section. Every imported source is probed with ffprobe and gets a
proxy and an audio track for later analysis.

## Scope

**In:**
- `source.import` for a local path or URL, with `--sections MM:SS-MM:SS` (or `H:MM:SS-H:MM:SS`) to download only part of a video.
- `source.info` (metadata, chapters, heatmap peaks for URLs), `source.fetch_transcript` (YouTube captions, json3), `source.list`, `source.show`, `source.search`.
- ffprobe probe with display dimensions after rotation; proxy and audio extraction.
- Permission checks: network and downloads flags on the project (`require_network`).

**Out (explicitly):**
- Transcription (f06) and scene/audio analysis (f07).
- Multi-source timelines and source-side editing (f12, f25).
- Downloading DRM or paywalled content. Only public sources yt-dlp can fetch.

## User-facing Behavior

- `source.import` returns a source id; a section import keeps its offset (for example `--sections 2:00-2:40` keeps a 120 s offset).
- `source.info` on a URL returns chapters and heatmap peaks without downloading.
- `source.fetch_transcript` returns captions without downloading the video; a video with no captions reports "this video has no captions"; an unknown language lists the available ones.
- Audio-only sources import successfully; `clip.create` later rejects them with "source has no video".

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/source_cmds.rs`
- `crates/rustler-media/src/probe.rs`, `proxy.rs`, `proc.rs`
- `crates/rustler-core/src/model/source.rs`

**Read-only dependencies:**
- `crates/rustler-tools/src/ytdlp_install.rs`, `run.rs` — yt-dlp and subprocess helper (f01).
- `crates/rustler-core/src/store.rs` — writes `source.json` (f03).

## Data and Interfaces

- Commands: `source.import`, `source.info`, `source.fetch_transcript`, `source.list`, `source.show`, `source.search`.
- Project file: `source.json` holds `Source`, `SourceOrigin`, `MediaProbe`, `TimeRange`.
- Probe reports display dimensions (after rotation); outputs always use even dimensions.

## Rules and Edge Cases

1. Import succeeds for a video without an audio track; render adds a silent AAC track later (f08).
2. Rotated portrait sources report display dims (`probe_reports_display_dims_for_rotated`).
3. Audio-only import is accepted (`probe_audio_only`).
4. Network access and downloads are refused unless the project permissions allow them.
5. Awkward file names (spaces, brackets, Devanagari) are supported.

## Tasks

- [x] Probe, proxy and audio extraction (`crates/rustler-media`)
- [x] Import, info, fetch-transcript and search commands (`source_cmds.rs`)
- [x] Scan-first YouTube flow

## Acceptance Criteria

- [x] Probe tests (`crates/rustler-media/tests/probe.rs`, 6 tests)
- [x] Source command tests (`crates/rustler-commands/tests/source.rs`, 14 tests)
- [x] Live: `source info` on a 20-minute TED talk (10 chapters, 8 heatmap peaks); `fetch-transcript` (3,170 words, no download); `import --sections 2:00-2:40` downloaded 40 s (progress log)

## Verification

- `cargo test -p rustler-media --test probe` and `cargo test -p rustler-commands --test source` (PowerShell).
- Live: `rustler source info <url>`, then `rustler source fetch-transcript <url>`; pass = chapters and words returned, no video file written.

## Status Log

- 2026-11-15 — Import, probe, proxy and source commands merged. Live YouTube scan-first verified (info, fetch-transcript without download, section import with offset kept). Big Buck Bunny correctly reports no captions.
