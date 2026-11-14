# Features

One file per feature: `fNN_<about_it>.md`, numbered in build order (`f01_`, `f02_`, ...).
Copy `_feature-template.md` to start a new one. **A feature file must exist and be filled in
before any code for that feature is written.**

Design source: `docs/superpowers/specs/2026-11-15-rustler-design.md` (§ refs below point there).
Research source for each feature: `docs/research/00-comparison-matrix.md` §2 (best source per capability).

## Index

Status reflects `docs/progress-log.md` (2026-11-15). "Not written" means no feature file exists yet.

| ID | File | Area (as built) | Slice | Depends on | Status |
| -- | ---- | ---- | ----- | ---------- | ------ |
| f01 | `f01_workspace_and_bootstrap.md` | Cargo workspace, uv engine project, `setup`, `doctor` | 1 | — | Verified |
| f02 | `f02_command_registry_and_cli.md` | Registry, JSON envelope, exit codes, `commands list/describe`, generic CLI | 1 | f01 | Verified |
| f03 | `f03_project_store_and_timeline.md` | Project dir format, clip/layout model, atomic writes, revisions, events | 1 | f01 | Verified |
| f04 | `f04_python_engine_bridge.md` | JSON-RPC stdio worker + Rust supervisor | 1 | f01 | Verified |
| f05 | `f05_sources_and_probe.md` | Import local/URL (yt-dlp), scan-first info/transcript, probe, proxy, audio extract | 1 | f02, f03 | Verified |
| f06 | `f06_transcription_and_units.md` | faster-whisper transcription, words, units, quote search | 1 | f04, f05 | Verified |
| f07 | `f07_understanding_pack_v1.md` | see.frame/tiles/preview, scene and audio analysis | 1 | f04, f05, f06 | Verified |
| f08 | `f08_render_compiler_v1.md` | clip → single ffmpeg graph; crop / blur-fit / split-vertical; NVENC; loudnorm | 1 | f03, f05 | Verified |
| f09 | `f09_qc_v1.md` | 12 QC checks with fix hints; export clip | 1 | f08 | Verified |
| f10 | `f10_mcp_server.md` | MCP server over stdio, generated from registry; images; prompts and resources | 2 | f02 | Verified |
| f11 | `f11_agent_skill_and_docs.md` | Knowledge book (13 guides, 8 prompts), guide/prompt commands, agent skill and AGENTS.md (`skills/rustler/`) | 2 | f10 | Built |
| f12 | `f12_edit_ops_and_history.md` | Edit ops (trim, split, delete, move, pauses, words) and history undo/restore | 2 | f03 | Built |
| f13 | — (not written) | OpenAI-compatible LLM/vision/STT/TTS, keys, JSON recovery, cost | 2 | f02 | Dropped (no AI APIs — user decision) |
| f14 | `f14_face_tracking_and_reframe.md` | YuNet tracks, smoothed crop path, crop.track_face / crop.clear | 3 | f04, f08 | Built |
| f15 | — (not written) | Custom multi-region layouts, keyframes, presets (stream cam/gameplay) | 3 | f08, f12 | Planned |
| f16 | `f16_caption_engine_v2.md` | 4 caption presets (ASS via libass) + SRT export; template schema, metrics, VTT not built | 3 | f08, f14 | Verified |
| f17 | `f17_audio_post.md` | Music bed with speech ducking; 2-pass loudnorm, denoise, SFX, voice-over not built | 3 | f08, f12 | Verified |
| f18 | — (not written) | Hook titles, text/sticker layers, xfade, effects enum | 3 | f08, f12 | Planned |
| f19 | — (not written) | Sentence snap, silence trough, fillers/pauses, TimeMap | 3 | f06, f12 | Planned |
| f20 | — (not written) | Signals, LLM two-pass candidates, rerank, judge | 4 | f07, f13, f19 | Planned |
| f21 | — (not written) | Caption/face/hook checks, vision review, fix hints | 4 | f09, f13, f16 | Planned |
| f22 | — (plan C1 in superpowers/plans) | Brief schema + compliance, variants, batch render/export, export reports | 4 | f21 | Built (branch, tests pass) |
| f23 | `f23_asset_library.md` | Fonts/presets/layouts/SFX/music with license manifest | 4 | f01, f16, f17 | Built (branch, tests pass) |
| f24 | `f24_gui_shell_and_live_sync.md` | Simple local web GUI (`rustler gui`); Tauri shell deferred to f25–f27 | 5 | f02, f03 | Verified |
| f25 | — (not written) | Multi-track timeline, trim/split/ripple, word snap | 5 | f24, f12 | Planned |
| f26 | — (not written) | Preview + draggable regions, crop keyframes, safe zones | 5 | f24, f15 | Planned |
| f27 | — (not written) | Caption editor, asset browser, render queue, QC view | 5 | f24, f16, f23 | Planned |
| f28 | — (not written) | CUDA/NVENC tuning, parallel renders, proxies | later | f08, f14 | Planned |
| f29 | — (not written) | Diarization, active speaker framing, speaker captions | later | f06, f14 | Planned |

The 5-hour build (2026-11-15) delivered f01–f10, f11, f12, f14, f16, f17 and f24 in part; scope is in
`docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` (section "5-HOUR SCOPE").

Status values: `Planned` → `Specced` → `In progress` → `Built` → `Verified` (or `Deferred`).
`Verified` means the progress log records an end-to-end run or real tests passing; `Built` means merged
without a recorded test or live run.
