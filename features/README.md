# Features

One file per feature: `fNN_<about_it>.md`, numbered in build order (`f01_`, `f02_`, ...).
Copy `_feature-template.md` to start a new one. **A feature file must exist and be filled in
before any code for that feature is written.**

Design source: `docs/superpowers/specs/2026-11-15-rustler-design.md` (§ refs below point there).
Research source for each feature: `findings/00-comparison-matrix.md` §2 (best source per capability).

## Index

| ID | File | Area | Slice | Depends on | Status |
| -- | ---- | ---- | ----- | ---------- | ------ |
| f01 | `f01_workspace_and_bootstrap.md` | Cargo workspace, uv engine project, `setup`, `doctor` | 1 | — | Planned |
| f02 | `f02_command_registry_and_cli.md` | Registry, JSON envelope, exit codes, discovery, jobs | 1 | f01 | Planned |
| f03 | `f03_project_store_and_timeline.md` | Project dir format, timeline/layout model, revisions, events | 1 | f01 | Planned |
| f04 | `f04_python_engine_bridge.md` | JSON-RPC stdio worker + Rust supervisor | 1 | f01 | Planned |
| f05 | `f05_sources_and_probe.md` | Import local/URL (yt-dlp), probe, proxy, audio extract | 1 | f02, f03 | Planned |
| f06 | `f06_transcription_and_units.md` | faster-whisper / STT endpoint, words, units | 1 | f04, f05 | Planned |
| f07 | `f07_understanding_pack_v1.md` | see.frame/tiles/filmstrip/preview, scenes, audio events | 1 | f04, f05, f06 | Planned |
| f08 | `f08_render_compiler_v1.md` | clip.json → single ffmpeg graph; basic layouts; 1 caption preset; loudnorm | 1 | f03, f05 | Planned |
| f09 | `f09_qc_v1.md` | Integrity, loudness, black/freeze/silence, duration, contact sheet | 1 | f08 | Planned |
| f10 | `f10_mcp_server.md` | rmcp server generated from registry; images; hardening | 2 | f02 | Planned |
| f11 | `f11_agent_skill_and_docs.md` | SKILL.md, AGENTS.md, workflow guide | 2 | f10 | Planned |
| f12 | `f12_edit_ops_and_history.md` | Full typed op set, undo/redo, word-anchored projection | 2 | f03 | Planned |
| f13 | `f13_provider_layer.md` | OpenAI-compatible LLM/vision/STT/TTS, keys, JSON recovery, cost | 2 | f02 | Planned |
| f14 | `f14_face_tracking_and_reframe.md` | YuNet tracks, One-Euro CropPath, sendcmd | 3 | f04, f08 | Planned |
| f15 | `f15_layout_region_system.md` | Custom multi-region layouts, keyframes, presets (stream cam/gameplay) | 3 | f08, f12 | Planned |
| f16 | `f16_caption_engine_v2.md` | Template schema, layered ASS, karaoke, metrics, face-aware placement, SRT/VTT | 3 | f08, f14 | Planned |
| f17 | `f17_audio_post.md` | 2-pass loudnorm, denoise, music ducking, SFX, voice-over | 3 | f08, f12 | Planned |
| f18 | `f18_overlays_hooks_transitions.md` | Hook titles, text/sticker layers, xfade, effects enum | 3 | f08, f12 | Planned |
| f19 | `f19_cut_refinement_and_pacing.md` | Sentence snap, silence trough, fillers/pauses, TimeMap | 3 | f06, f12 | Planned |
| f20 | `f20_highlight_helpers.md` | Signals, LLM two-pass candidates, rerank, judge | 4 | f07, f13, f19 | Planned |
| f21 | `f21_quality_loop_v2.md` | Caption/face/hook checks, vision review, fix hints | 4 | f09, f13, f16 | Planned |
| f22 | `f22_brief_variants_exports.md` | Brief schema + compliance, variants, export reports | 4 | f21 | Planned |
| f23 | `f23_asset_library.md` | Fonts/presets/layouts/SFX/music with license manifest | 4 | f01, f16, f17 | Planned |
| f24 | `f24_gui_shell_and_live_sync.md` | Tauri shell, dashboard, live events, agent activity | 5 | f02, f03 | Planned |
| f25 | `f25_gui_timeline_editor.md` | Multi-track timeline, trim/split/ripple, word snap | 5 | f24, f12 | Planned |
| f26 | `f26_gui_program_monitor_canvas.md` | Preview + draggable regions, crop keyframes, safe zones | 5 | f24, f15 | Planned |
| f27 | `f27_gui_captions_assets_render.md` | Caption editor, asset browser, render queue, QC view | 5 | f24, f16, f23 | Planned |
| f28 | `f28_gpu_throughput.md` | CUDA/NVENC tuning, parallel renders, proxies | later | f08, f14 | Planned |
| f29 | `f29_speakers_active_speaker.md` | Diarization, active speaker framing, speaker captions | later | f06, f14 | Planned |

Status values: `Planned` → `Specced` → `In progress` → `Built` → `Verified` (or `Deferred`).
