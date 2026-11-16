# Features

One file per feature: `fNN_<about_it>.md`, numbered in build order (`f01_`, `f02_`, ...).
Copy `_feature-template.md` to start a new one. **A feature file must exist and be filled in
before any code for that feature is written.**
Design source: `docs/superpowers/specs/2026-11-15-rustler-design.md` (§ refs below point there).
Research source for each feature: `docs/research/00-comparison-matrix.md` §2 (best source per capability).
## Index
Status reflects `docs/progress-log.md` (2026-11-15) and the docs pass that ran the commands against a demo project. "Not written" means no feature file exists yet; f13 is dropped (no AI APIs) and f28 is not started.
| ID | File | Area (as built) | Slice | Depends on | Status |
| -- | ---- | ---- | ----- | ---------- | ------ |
| f01 | `f01_workspace_and_bootstrap.md` | Cargo workspace, uv engine project, `setup`, `doctor` | 1 | — | Verified |
| f02 | `f02_command_registry_and_cli.md` | Registry, JSON envelope, exit codes, `commands list/describe`, generic CLI | 1 | f01 | Verified |
| f03 | `f03_project_store_and_timeline.md` | Project dir format, clip/layout model, atomic writes, revisions, events | 1 | f01 | Verified |
| f04 | `f04_python_engine_bridge.md` | JSON-RPC stdio worker + Rust supervisor | 1 | f01 | Verified |
| f05 | `f05_sources_and_probe.md` | Import local/URL (yt-dlp), scan-first info/transcript, probe, proxy, audio extract | 1 | f02, f03 | Verified |
| f06 | `f06_transcription_and_units.md` | faster-whisper transcription, words, units, quote search | 1 | f04, f05 | Verified |
| f07 | `f07_understanding_pack_v1.md` | see.frame/tiles/preview, scene and audio analysis | 1 | f04, f05, f06 | Verified |
| f07b | `f07b_understanding_v2.md` | analyze.ocr/events/faces/deep: on-screen text, audio events, face timeline (local, no AI API) | 4 | f07 | Verified |
| f08 | `f08_render_compiler_v1.md` | clip → single ffmpeg graph; crop / blur-fit / split-vertical; NVENC; loudnorm | 1 | f03, f05 | Verified |
| f09 | `f09_qc_v1.md` | 12 QC checks with fix hints; export clip | 1 | f08 | Verified |
| f10 | `f10_mcp_server.md` | MCP server over stdio, generated from registry; images; prompts and resources | 2 | f02 | Verified |
| f11 | `f11_agent_skill_and_docs.md` | Knowledge book (17 guides, 10 prompts), guide/prompt commands, agent skill and AGENTS.md (`skills/rustler/`) | 2 | f10 | Built |
| f12 | `f12_edit_ops_and_history.md` | Edit ops (trim, split, delete, move, pauses, words) and history undo/restore | 2 | f03 | Built |
| f13 | — (not written) | OpenAI-compatible LLM/vision/STT/TTS, keys, JSON recovery, cost | 2 | f02 | Dropped (no AI APIs — user decision) |
| f14 | `f14_face_tracking_and_reframe.md` | YuNet tracks, smoothed crop path, crop.track_face / crop.clear (`crop.track_speaker` is in f29) | 3 | f04, f08 | Built |
| f15 | `f15_layout_region_system.md` | Region layouts, picture-in-picture, presets, `layout.detect_webcam` streamer split (keyframes not built) | 3 | f08, f12 | Verified |
| f16 | `f16_caption_engine_v2.md` | 4 caption presets (ASS via libass) + SRT export; template schema, metrics, VTT not built | 3 | f08, f14 | Verified |
| f17 | `f17_audio_post.md` | Music bed with ducking, 2-pass loudnorm, denoise, word-anchored SFX, volume (voice-over not built) | 3 | f08, f12 | Verified |
| f18 | `f18_overlays_transitions_effects.md` | Hook titles / lower thirds, 7 transitions (xfade), 4 camera effects, progress bar (no stickers) | 3 | f08, f12 | Built |
| f19 | `f19_cut_refinement.md` | `edit.refine`: weak opener skip, sentence end, silence trough (plus trim --snap word, remove pauses/words) | 3 | f06, f12 | Verified |
| f20 | `f20_highlight_helpers.md` | highlights.signals / candidates (deterministic, no LLM), transcript.read_window | 4 | f07, f19 | Verified |
| f21 | `f21_quality_loop_v2.md` | caption_over_face, face_visible_ratio, face_cut_off, hook_text_present, duration_for_platform; fix.suggest / fix.apply / qc.compare | 4 | f09, f16 | Built |
| f22 | `f22_brief_variants_exports.md` | Brief schema + compliance check, post description, variants + compare grid, render.batch / export.batch | 4 | f21 | Built |
| f23 | `f23_asset_library.md` | Fonts, caption styles, layouts, SFX with license manifest; `asset:<id>`; custom caption styles | 4 | f01, f16, f17 | Built |
| f24 | `f24_gui_shell_and_live_sync.md` | Simple local web GUI (`rustler gui`); Tauri shell deferred to f25–f27 | 5 | f02, f03 | Verified |
| f25 | `f25_gui_editor.md` | GUI editor in the web GUI: multi-track timeline, trim/split/word snap (also covers f26, f27; no Tauri) | 5 | f24, f12 | Built |
| f26 | `f25_gui_editor.md` | Program monitor + draggable regions (built, documented in f25) | 5 | f24, f15 | Built |
| f27 | `f25_gui_editor.md` | Inspector: captions, overlays, audio, QC + fixes, render queue, brief, variants, history (asset browser partial) | 5 | f24, f16, f23 | Built |
| f28 | — (not written) | CUDA/NVENC tuning, parallel renders, proxies | later | f08, f14 | Planned |
| f29 | `f29_speakers.md` | analyze.speakers (diarization), speaker.rename, caption.speaker_colors, crop.track_speaker | later | f06, f14 | Built |
| f30 | `f30_lite_by_default.md` | Lite install by default (base engine, whisper base), heavy analyzers opt-in (`setup run --with ...`), QC face checks opt-in, `analyze quick --transcribe`, open music/assets (`asset add --url`, `source:<id>` audio) | later | f01, f21, f23 | Built |
| f31 | `f31_director_retention.md` | Director layer: `clip.retention` (retention map, dead zones + fix commands, 0-100 retention score), `clip.score_set` (agent hook/flow/value/trend), `campaign.analyze` (rules, payout strategy, clip count, checklist); guide `director-retention`, prompts `direct_campaign`, `hook_lab` | v2 | f18, f20, f21, f22 | Built |
| f32 | `f32_style_references.md` | Style references: `reference.import/list/show/analyze` (proxies, cuts/min, caption band + colours, hook text, zoom rate, face layout, LUFS, music, speech rate, contact sheets + first-3-s filmstrips); guide `learn-style-from-videos`, prompt `learn_style_from_folder` | v2 | f06, f07b | Built |
| f33 | `f33_timeline_v2.md` | Timeline v2 document (`timeline.show/schema/apply`, JSON-Patch, history of every layer) + `edit.insert_segment`, `edit.cold_open`, `clip.merge`, `clip.duplicate`, `edit.speed`, `edit.freeze` | v2 | f03, f18, f19, f29, f31 | Built |
| f34 | `f34_smart_defaults.md` | Smart defaults at `clip.create` (per-shot layout + framing, filler and pause trim; `--raw`, setting `clip.smart_defaults`; every decision with its override command), `crop.track_active` (light active speaker, lite install), `layout.plan`, engine `faces.active` (landmark mouth motion, dark-safe cut scan, graphics stats) | v2 | f14, f19, f29 | Built |
| f35 | `f35_templates.md` | Templates (`rustler.template@1`): `template.create/update/show/list/delete/apply/diff/export/import`, apply = JSON-Patch through `timeline.apply` (one undoable revision, `by_template` items replaced on re-apply), starters `podcast-viral` / `streamer-split`, v2 fields layout_rules, keyword/emoji captions, cutaway_rules, music by tag/mood, brand logo/CTA/outro, export, post_defaults (2026-11-16), `clip.create --template` + `template.default`, zip bundles with fonts/styles/assets; guide `templates-and-brand` | v2 | f23, f33 | Built |
| f36 | `f36_hooks_captions_pacing_cover.md` | Hook styles (hook-big/box-yellow/outline/pov/question, animations, whoosh), `keyword-highlight` + `emoji-karaoke` captions, `caption.keywords`, transitions flash/flashblack/whip, `clip.cover` (+ export `<name>.cover.jpg`) | v2 | f09, f10, f18, f23 | Built |
| f37 | `f37_clips_auto.md` | `clips.auto`: optional one-call pipeline over existing commands (URL: cached info + captions + padded sections; or a source), ranking = highlights.candidates + first-2-s hook, non-overlapping, smart defaults + template, cheap retention fixes, render.batch with ETA, QC, export with post JSON skeleton, per-clip report with change commands, dry-run plan, per-clip failure isolation; guide `clips-auto` | v2 | f20, f22, f31, f34, f35 | Built |
| f38 | `f38_color_grading.md` | Colour grading the agent controls (never automatic): extras `grade` (clip + per-segment overrides), 8 presets, exposure/contrast/saturation/vibrance/temperature/tint/blacks, shadows/midtones/highlights colour, `.cube` LUT (asset kind `lut`) + strength, intensity; `grade.set/show/presets/clear/preview/measure`; compiled per segment on the source picture before layout and captions; history undo; template field `grade`; guide `color-grading` | v2 | f10, f18, f23, f33, f35 | Built |
| f39 | `f39_content_suggestions.md` | Content suggestions (never applied): `edit.find_bad_takes` (stutters, restarts, false starts, repeated sentences with the `edit remove-words` command; also `clip.retention` `take_fixes` and `clips.auto` `fixes_suggested`), intro-trailer skip in `highlights.candidates` / `clips.auto` (`include_intro`, originals ranked), `clips.auto` hook-title (≤ 7 words, hook-big) and cold-open suggestions, caption hyphen-fragment fix, `edit.remove_words` neighbour spans | v2 | f12, f16, f20, f31, f33, f37 | Built |
The 5-hour build (2026-11-15) delivered f01–f10, f11, f12, f14, f16, f17 and f24; the "complete everything" phase then added f07b, f15, f17 (rest), f18–f23, f25–f27 and f29 (118 registered commands). The original 5-hour scope is in
`docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` (section "5-HOUR SCOPE").
Status values: `Planned` → `Specced` → `In progress` → `Built` → `Verified` (or `Deferred`).
`Verified` means the progress log records an end-to-end run or real tests passing; `Built` means merged
without a recorded test or live run.
| f14 | `f14_face_tracking_and_reframe.md` | YuNet tracks + landmarks, offline camera planner (styles locked/smooth/follow, look-room, eased moves, crop cuts), crop.track_face / crop.clear, QC camera_motion (`crop.track_speaker` is in f29) | 3 | f04, f08 | Built |
