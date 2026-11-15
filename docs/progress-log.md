# Progress Log

## ★ CURRENT STATUS — COMPLETE BUILD (2026-11-15, morning) — read this first

**Everything in the design is implemented and working** (CLI first, then the full GUI), except the
items explicitly dropped by the user (AI API integrations / provider layer). Windows (this laptop) and
Linux (WSL Ubuntu, no-GPU path) both verified.

**CLI — 118 commands** (`rustler commands list`), grouped:
- Setup & discovery: `setup run`, `doctor run`, `help`, `commands list|describe`, `guide list|show|search`, `prompt list|show`
- Projects: `project create|show|open|list|status|pack|unpack|set-permissions|set-settings`
- Sources: `source import (file|URL|--sections)|info|fetch-transcript|search|list|show`
- Understanding: `analyze quick|transcribe|scenes|audio|deep|ocr|events|faces|speakers`, `transcript get|search|read-window`, `see frame|tiles|preview`, `highlights signals|candidates`, `speaker rename`
- Clips & editing: `clip create|add-segment|set-layout|set-captions|show|list`, `edit trim|split|delete-segment|move-segment|set-range|remove-pauses|remove-words|refine`, `history list|undo|restore`
- Layout & framing: `layout show|add-region|edit-region|remove-region|reorder|presets|list-presets|save-preset|apply-preset|delete-preset|detect-webcam|from-frame-boxes`, `crop track-face|track-speaker|clear`
- Captions & overlays: `caption set|styles|list-styles|create-style|export-srt|speaker-colors`, `overlay add-text|list|edit|remove|progress-bar`
- Audio & FX: `audio add-music|remove-music|add-sfx|remove-sfx|denoise|volume|list`, `transition set|list|clear`, `effect add|list|remove`
- Output & quality: `render run|preview-frame|batch`, `qc run|compare`, `fix suggest|apply`, `export clip|batch`
- Business: `brief set|show|clear|check|description`, `variant create|list|compare`
- Assets: `asset list|search|show|add|remove`
- Servers: `mcp serve` (119 MCP tools, 10 prompts, guides as resources, images to vision agents), `gui`

**GUI** (`rustler gui`): Premiere-style editor — source panel with draggable region boxes (webcam/gameplay
split, detect webcam), 9:16 live program + exact libass frame, multi-track timeline (V/FX/CAP/TXT/A1,
trim with word snap, split, reorder, transitions on seams, effect ranges, SFX markers), inspector tabs
(Clip, Layout, Captions + style creator, Overlays, Audio, Effects, QC + fixes, Render queue, Brief,
Variants, History), Assets page, Brief page, Project dashboard (status/pack/unpack), Sources (transcript,
highlights, understand: OCR/events/faces/speakers), Activity feed, generic Commands runner. Live sync with
agent edits. Every action verified in a browser and confirmed with CLI reads.

**Knowledge for agents:** 17 guides + 10 workflow prompts (`knowledge/`), embedded in the binary and served
over CLI and MCP; `skills/rustler/SKILL.md` (Claude Code) and `AGENTS.md` (others); `docs/agent-integration.md`.

**Tests (final code, every crate, 2026-11-15 ~06:00):** **478 Rust tests passed, 0 failed** (core 47, registry 5,
tools 18, engine-bridge 11, captions 22, knowledge 9, media 92, mcp 10, gui 9, cli 36, commands 75 unit + 144
integration across 21 test files); Python engine 116 passed, 1 skipped. Linux (WSL): 477 Rust + 114 engine passed.
**Release build:** `target\release\rustler.exe` (28 MB) built; `doctor run` -> ready.
Linux (WSL): 477 Rust tests passed, engine 114 passed; setup/doctor/full flow/MCP/GUI verified — see `docs/linux.md`.

**Real end-to-end runs (all through the CLI, contact sheets inspected):**
- Talk clip: import → analyze → search → tiles → clip by unit ids + expect-text → blur-fit + karaoke → NVENC render → QC 12/12 → export.
- YouTube scan-first on a 20-min TED talk: info (chapters, most-replayed peaks) → captions without download → import 40 s section.
- Streamer clip: detect-webcam (facecam top-right) → split-vertical → hook title → Kenney SFX via `asset:` → denoise → progress bar → punch-in → render → QC pass → brief check (correctly caught missing credit) → export.
- Transitions: split + dissolve → render → QC duration check passes with the shortened timeline.

**How to use it now (PowerShell):**
```
cd C:\Users\krishna\Desktop\rustler
.\target\release\rustler.exe doctor run
.\target\release\rustler.exe guide show start-here
.\target\release\rustler.exe --project $env:TEMP\rustler-stream gui    # streamer demo project in the editor
```
Give Claude Code the skill: copy `skills\rustler` to `%USERPROFILE%\.claude\skills\rustler`; MCP:
`claude mcp add rustler -- C:\Users\krishna\Desktop\rustler\target\release\rustler.exe mcp serve`.
Demo outputs: `%TEMP%\rustler-e2e\exports\jfk-hook.mp4`, `%TEMP%\rustler-stream\exports\stream-hook.mp4`.

**Known limits / next ideas:**
1. detect-webcam's suggested box can include a sliver of gameplay beside the facecam → nudge the region.
2. OCR can't read Devanagari on-screen text (RapidOCR model limit).
3. No CC0 music library (no verifiable login-free source) — add your own with `asset add --license`.
4. Speaker colours / active-speaker crop tested on synthetic two-face video, not real podcast footage yet.
5. Windows PowerShell 5.1 strips quotes in JSON args → use `--input-json @file.json`.
6. Tauri desktop wrapper and published GitHub release binaries / one-line installer are not done (local release build exists; the web GUI runs locally).

---

## 2026-11-15
- Cloned 7 repos (shallow) into `repos/`: hypit (Apache-2.0, TS), openshorts (MIT, Py),
  supoclip (MIT, Py+TS), autoclip (MIT, Py), ViralCutter (MIT, Py), bridgeclip (MIT, Py+TS),
  opensource-clipping (MIT, Py).
- **License correction (verified by reading LICENSE files):** supoclip = **AGPL-3.0**, ViralCutter =
  **GPL-3.0**, hypit = **modified Apache-2.0** (no multi-tenant/SaaS or commercial redistribution
  without a commercial license), openshorts `cloud/` has its own separate LICENSE. My first quick scan
  wrongly said MIT (it matched "mit" inside "permitted"). Details: `docs/research/00-license-and-dependency-inventory.md`.
  Consequence: copy code only from MIT repos; from AGPL/GPL/modified-Apache repos, study ideas and
  reimplement independently (unless the user decides the whole product is GPL/AGPL and personal-use).
  ultralytics (YOLO) is AGPL-3.0 — avoid as a dependency, or isolate it.
- Wrote `docs/research-methodology.md` (shared evaluation template, categories A–I).
- Started 7 per-repo analysis agents + 1 license/dependency inventory agent → `docs/research/`.
- rea MCP fails: Node 24.21.0 too old (needs >= 24.22.1). See `tooling-notes.md`.

- All 7 per-repo reports + license inventory landed in `docs/research/`. Cross-repo comparison matrix
  being written by an Opus agent → `docs/research/00-comparison-matrix.md`.
- Comparison matrix done: `docs/research/00-comparison-matrix.md` (scores, best source per capability,
  key choices, gaps, asset shortlist, draft feature list f01–f20). Claude's note: pull frame
  viewing/contact sheets (draft f05) into the first slice — "agent sees the video" is a core ask.
- Brainstorming decisions recorded in `docs/00-vision-and-requirements.md` (confirmed section).

- **User chose approach B: Rust core + Python engine** (personal tool). GUI must be a Premiere-style
  editor with multi-region layouts (e.g. webcam top / gameplay bottom); everything the GUI can do,
  the agent can do ("agent and software are one").
- Design doc drafted: `docs/superpowers/specs/2026-11-15-rustler-design.md`; feature index f01–f29 in
  `docs/features/README.md`. Builder check (Opus reviewer) running.
- Rust: rustup had no toolchain → installed stable (rustc 1.100.0). Linking failed: MSVC C++ build
  tools missing → installing VS 2022 Build Tools (VCTools workload) via winget.

## Open questions
1. Upgrade Node for rea? (system change — needs user OK; unanswered)
2. (resolved) Design approved 2026-11-15.

## Resolved
- "discord" = discover (typo). Content type: any — full visual+audio+speech understanding.
- Use: user clips to earn, tool runs on each agent's own machine; reimplement non-MIT ideas.

## 2026-11-15 (later)
- Design approved by user. Slice 1 implementation plan written:
  `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` (13 tasks in 4 waves; Opus for contracts,
  render compiler and e2e; Sonnet for feature crates; Haiku for docs/feature files).
- Added quick-scan design (user idea): fetch YouTube transcript/heatmap/chapters without downloading,
  then download only chosen sections (`source.info`, `source.fetch_transcript`, `source.import --sections`).
- VS 2022 Build Tools MSVC 14.44 present (linker for Rust).
- Waiting: user review of the plan before implementation.
- 2026-11-15: User set a **5-hour autonomous build window** (user asleep, no questions). Scope = plan's
  "5-HOUR SCOPE" section. Defaults decided: public-domain/CC test media, local installs in ~/.rustler,
  priority CLI pipeline → MCP → edit ops → face reframe → caption styles → music → simple web GUI.
- Private GitHub repo created: github.com/krishna-notfound/rustler (remote `origin`, branch master).
  `repos/` (third-party clones) is gitignored and never pushed.
- Wave 0 done (commits 246c040, 9a0a224, 086bc63): workspace + frozen contracts, 13 tests green.
  Notes: real MSRV is ≥1.89 (deps); `tiles` label fn needs `+ Send + Sync`; rustler-tools cannot call
  the engine (bridge depends on tools) → engine calls for setup/doctor live in rustler-commands.
- Tools pre-installed at ~/.rustler/bin (ffmpeg BtbN GPL w/ libass, ffprobe, yt-dlp, uv); NVENC OK.
- Wave 1 launched in worktrees: T3 core impl, T4 setup/doctor, T6 media basics, T7 captions (4 presets +
  SRT), T8 CLI, T9 render compiler (+music bed, crop sendcmd), MCP server crate, simple web GUI crate.
  Python engine agent still running in main tree.
- Merged: MCP server (stdio, tools from registry, images to vision agents), captions (4 presets, SRT;
  switched to static Poppins ExtraBold — libass can't pick weights from variable Montserrat), Python
  engine (80 tests; CUDA works), core store/timemap/units/layout (40 tests), GUI crate (axum, token, plain
  JS), CLI front (hand-written parser, 36 tests), media basics (15 tests). `rustler mcp serve` and
  `rustler gui` wired in main.rs.
- User added: MCP + CLI must be a full **knowledge book** (MCP prompts + resources + `rustler guide`).
  Opus writing `knowledge/`; Sonnet building loader + MCP prompts/resources + guide commands.
- Running: T4 setup/doctor, T9 render compiler, bridge, knowledge content+loader, faces, edit ops,
  10a/10b commands, T11 QC.
- User rule (2026-11-15): **all documentation lives in `docs/` and is kept updated after every merge.**
  Planned move (after the knowledge-content agent finishes, since it reads `docs/research/` right now):
  `docs/research/` → `docs/research/`, `docs/features/` → `docs/features/`, `context/` → `docs/context/`; update all
  path references + CLAUDE.md. `knowledge/` stays at repo root (it's product content embedded in the binary,
  not project docs) but is indexed from `docs/README.md`.
- Merged engine bridge (Rust<->Python JSON-RPC; 11 tests vs real engine; process-tree kill on cancel).
- Knowledge book committed (`knowledge/`: 13 guides, 8 MCP prompts).
- **Docs consolidated into `docs/`**: `findings/`→`docs/research/`, `features/`→`docs/features/`,
  `context/`→`docs/context/`; all path references updated; `docs/README.md` rewritten as the index.
- Merged face tracking: engine faces.track (YuNet, MIT model, auto-download) + Rust facepath::crop_keys (median+One-Euro+dead zone+speed cap, ported from autoclip MIT). 85 py + 13 rust tests. TODO: unify CropKey with render compiler's type; crop.track_face command still to wire.
- Merged face tracking: engine faces.track (YuNet, MIT model, auto-download) + Rust facepath::crop_keys (median+One-Euro+dead zone+speed cap, ported from autoclip MIT). 85 py + 13 rust tests. TODO: unify CropKey with render compiler type; wire crop.track_face command.
- Merged setup/doctor (live setup into temp home passed: ffmpeg, yt-dlp, uv, engine(cuda), fonts; doctor ready=true, NVENC+QSV ok) and render compiler (26 tests incl. NVENC, awkward Devanagari filenames, music ducking measured -3.9 dB, crop pan verified by frame colour). Unified CropKey (facepath re-exports compile::CropKey). MIT attributions added to THIRD_PARTY_NOTICES.md. Open: single-pass loudnorm hits -15.9 LUFS on half-silent clips → two-pass later.
- Merged knowledge loader: crate rustler-knowledge (embeds knowledge/), commands guide.list/show/search + prompt.list/show, MCP prompts/list+get, resources/list+read+templates (rustler://guides/<id>, rustler://commands, rustler://project/state), initialize instructions. Tests: knowledge 9, guide cmds 5, mcp 10.
- Merged QC v1 + export: 12 checks with fix_hint commands, qc.run (exit 7 on fail, allow_fail), export.clip (mp4 + report.json + srt). 19 tests.
- **Real `rustler setup` on this laptop: OK** (engine lite+cuda, whisper small fetched, fonts Anton/Poppins/NotoSansDevanagari; existing ffmpeg/yt-dlp/uv adopted). `rustler doctor`: ready=true, libass, CUDA engine, encoders h264_nvenc/hevc_nvenc/h264_qsv/libx264 OK.
- Merged see/clip/render/caption/layout/audio/crop commands (clip.*, caption.*, layout.list_presets, audio.add_music/remove_music via extras.json, crop.track_face/clear, see.frame/tiles/preview, render.run/preview_frame). Real render of Devanagari-named source with captions+music verified 1080x1920.
- Merged project/source/analyze/transcript commands (live: source.info + heatmap peaks OK; TED video
  fetch-transcript → 307 units; Big Buck Bunny correctly reports "no captions").
- **END-TO-END PASSED via real CLI** on a 55 s demo video named `jfk talk (demo) – टेस्ट.mp4`:
  project create → source import → analyze quick (110 words, 15 units) → transcript search "fellow
  Americans" → see tiles --around → clip create --start-unit/--end-unit --expect-text, blur-fit,
  hormozi-pop → render run (h264_nvenc) → qc run (12/12 pass) → export clip. Contact sheet and tile grid
  inspected visually: layout, karaoke captions and labels correct.
- Fixes from that inspection: hormozi pop scale 115%→108/112% (active word touched neighbour);
  see.tiles auto-fits columns to count.
- Full workspace test run: **308 passed, 1 failed** ? the failure was an outdated test expecting gui/mcp "not built yet"; replaced with a real `mcp serve` stdio test (26/26 CLI tests pass).
- Bug found by real use + fixed: `rustler --project <dir> gui` treated the dir as the command.
- MCP checked over stdio like an agent: 55 tools, 8 prompts, 14 resources; `prompts/get make_viral_clips` renders.
- YouTube scan-first verified live on a 20-min TED talk: `source info` (10 chapters, 8 heatmap peaks), `fetch-transcript` (3,170 words, no download), `import --sections 2:00-2:40` (40 s downloaded, offset 120 s kept).
- GUI verified in a real browser on the e2e project: Project page, Renders page plays the NVENC render, QC table (12 pass, -14.3 LUFS), contact sheet; no console errors. Known nit: reloading with a `#view` hash lands on Project.

## 2026-11-15 (02:40) — "complete everything" phase
- Stopped early at ~1.5 h of the 5 h window (user called it out) → resumed. New goal from user: complete
  CLI for every tool, then full GUI. Plan: `docs/superpowers/plans/2026-11-15-complete-rustler.md`.
- Release build was killed by low memory → max 4 parallel agents, CARGO_BUILD_JOBS=2.
- Running: A1 render quality (2-pass loudnorm) + text overlays [Opus], A2 layout region system +
  detect-webcam [Sonnet], A3 highlights signals/candidates + edit.refine [Sonnet], B2 understanding v2
  (OCR, audio events, faces timeline, analyze deep) [Sonnet].
- Queued: B1 audio post + transitions/effects, B3 QC v2 + fix.suggest, B4 full GUI editor, C1 brief/
  variants/batch, C2 asset library, C3 project pack/list, C4 speakers (optional).
- Merged A2 layout region system: layout.show/add_region/edit_region(+nudge)/remove_region/reorder/save_preset/apply_preset/delete_preset/presets/detect_webcam/from_frame_boxes. detect-webcam found a corner face 4/4 samples and produced a working split layout (preview checked visually). Guide layouts-and-framing.md updated.
- Merged A3: highlights.signals, highlights.candidates (sentence-aligned, scored, non-overlapping, weak-opener skip, ready create_cmd), transcript.read_window, edit.refine (weak-opener rewind + silence-trough + sentence end). Real run on demo found 'ask not what your country can do for you' as #1. TODO: source.import (URL) should save source_info.json so heatmap feeds signals.
- Merged A3: highlights.signals, highlights.candidates (sentence-aligned, scored, non-overlapping, weak-opener skip, ready create_cmd), transcript.read_window, edit.refine. Real run on demo ranked 'ask not what your country can do for you' #1. TODO: URL import should save source_info.json so heatmap feeds signals.
- Merged A1: two-pass loudnorm (half-silent clip -15.9 ? -14.0 LUFS, TP -2.2), text overlays/hook titles in the same encode, overlay.add_text/list/edit/remove, shared extras helper. Fixed ClipExtras clobbering other extras keys (verified: overlay survives audio.add_music).
- Merged B2 understanding v2: engine ocr.run (RapidOCR, Apache), audio.events (YAMNet ONNX Apache, sha-pinned; heuristic fallback), commands analyze.ocr/events/faces/deep. Models + licenses in engine/MODELS.md. Note: OCR can't read Devanagari.
- Registry now has 77 commands.
- C1 brief, variants, batch (f22) built in a worktree branch: `brief.set|show|clear|check|description`, `variant.create|list|compare`, `render.batch`, `export.batch` (10 new commands).
  - Brief = `<project>/brief.json` (merge; lists replace; `replace:true` overwrites). `brief.check` rules: duration (TimeMap minus extras `transitions_total`, else the sum of non-cut `transitions[].duration`), aspect, caption_style, layout, required_text, banned_words (whole word, case/accent/punctuation-insensitive, over transcript + raw transcript + caption words + overlays), credit_text/watermark_text (must be in an overlay), hashtags/mentions as `info` post items, qc of the latest render. Saved to `clips/<id>/brief_check.json`; the `export.clip` report now has a `brief` key (`brief_cmds::brief_status`).
  - Variants: clones with `extras.variant_of` (root clip id) + `extras.variant_info`; layouts applied via `layout.apply_preset` (custom presets work; layouts are preflighted on the base with a dry run so nothing is half-created); hooks via `overlay.add_text` (replaces the base's hook-top overlay). `variant.compare` composes draft preview frames at the same out_t into a labelled grid under `<project>/variants/`.
  - `render.batch`: JoinSet + semaphore, default parallel 1, capped at 2 on NVENC (else 4); continues on failure; `only_missing` skips clips whose latest render of that quality is newer than the clip/extras files (qc.run re-saves clip.json, so the reference time is max(render, qc report)). `export.batch`: skips clips with no render, a stale render, failing QC (runs it when missing) or a failing brief, with reasons + fix commands.
  - Tests: tests/brief.rs 4/4, tests/variant.rs 2/2 (real ffmpeg + NVENC; 2x2 variants with extras copied, compare grid viewed, render.batch with a failing clip, export.batch skipping a brief-failing clip). Not verified: Hindi/Devanagari banned words beyond what the saved transcripts contain (matching runs on the romanized transcript, the raw transcript and caption words).
- Merged C1: brief.set/show/clear/check/description, variant.create/list/compare (grid image checked), render.batch, export.batch; export report includes brief status.
- Merged B3: QC checks caption_over_face, face_visible_ratio, face_cut_off, hook_text_present, duration_for_platform (engine checks degrade to warn); fix.suggest (ordered, auto_safe flags), fix.apply (safe fixes + re-render + re-QC), qc.compare (side-by-side sheet). Workspace tests green on the branch.
- Merged B4 **full GUI editor** (Premiere-style, in the web GUI): source panel with draggable/resizable region boxes (layout.edit_region), detect-webcam + draw-2-boxes streamer split, 9:16 live program preview + exact libass frame, multi-track timeline (V/CAP/TXT/A1: trim with word snap, split, delete, reorder, overlay retime, zoom), inspector tabs (Clip, Layout, Captions, Overlays, Audio, QC+fixes, Render queue with background jobs/progress, Brief, Variants, History), keyboard shortcuts, live sync with agent edits. 14 JS modules, no build step. Verified in Playwright at 1440/1280 against real projects; screenshots .playwright-mcp/b4-*.png.

- C4 speakers (f29) built in a worktree branch: `analyze.speakers`, `speaker.rename`, `caption.speaker_colors`, `crop.track_speaker` (4 commands) + engine `speakers.diarize` / `speakers.active`.
  - Diarization = sherpa-onnx 2.0.1 (Apache-2.0) + pyannote segmentation-3.0 ONNX (MIT, LICENSE read in the release tarball) + WeSpeaker ResNet34 (Apache-2.0); URLs + sha256 in `engine/MODELS.md`, downloaded to `~/.rustler/models/diarization/`. A numpy MFCC+clustering fallback is labelled `model: "heuristic"` (weaker: it split a single speaker in two on the JFK sample).
  - `analyze.speakers` writes `analysis/<src>/speakers.json` (turns S1.., `labels` map) and sets `speaker` on `transcript.json` words by max overlap (`transcript.raw.json` untouched). Real run on the 55 s JFK demo: 1 speaker, 110/110 words labelled, 7.3 s. Engine test: JFK + FDR (Wikimedia, public domain) + JFK = 2 speakers, boundaries correct.
  - `caption.speaker_colors` stores `extras.speaker_colors`; `render` writes speaker-coloured ASS through the new `rustler_captions::write_ass_colored` (additive: `CaptionWord` was NOT changed, adding a field would break literals in clip_cmds/qc_cmds/tests; one-line wiring in render_cmds). Looked at preview frames: yellow S1 / cyan S2 words. Caveat: the preset active-word colour is unchanged, so bold-karaoke/clean-white yellow highlight looks like S1 yellow; the guide says to pick another S1 colour.
  - `crop.track_speaker`: `speakers.active` (YuNet boxes + lower-face frame-difference score normalized per face track) correlated with the diarized turns (per-turn mouth score, speaker-to-face map, largest-face fallback flagged `dominant`), then `facepath::crop_keys` on the chosen face with faster smoothing; stored in `extras.crop_paths` like `crop.track_face`. Real run on a synthetic two-Lincoln video (mouth flicker alternating L/R/L): faces 0,1,0 and crop keys left/right/left. Not tested on real two-person footage (none available offline).
- Merged C3: project.pack (+media)/unpack/list/open/status; URL imports save source_info.json (heatmap feeds highlights.signals — live-verified); source.info --source-id offline.
- Merged B1 audio post + transitions/effects: audio.denoise (measured noise floor; hiss -12/-20/-30 dB by strength), audio.volume, audio.add_sfx (word-anchored, sample-exact), audio.remove_sfx/list, transition.set/list/clear (frame-exact xfade+acrossfade, caption/overlay times remapped), effect.add/list/remove (punch-in, zoom-slow, shake, flash), overlay.progress_bar. 434 workspace tests green on branch. Fixed QC duration check for transitions (CLI verified).
- Registry now has 77 commands.
- C1 brief, variants, batch (f22) built in a worktree branch: `brief.set|show|clear|check|description`, `variant.create|list|compare`, `render.batch`, `export.batch` (10 new commands).
  - Brief = `<project>/brief.json` (merge; lists replace; `replace:true` overwrites). `brief.check` rules: duration (TimeMap minus extras `transitions_total`, else the sum of non-cut `transitions[].duration`), aspect, caption_style, layout, required_text, banned_words (whole word, case/accent/punctuation-insensitive, over transcript + raw transcript + caption words + overlays), credit_text/watermark_text (must be in an overlay), hashtags/mentions as `info` post items, qc of the latest render. Saved to `clips/<id>/brief_check.json`; the `export.clip` report now has a `brief` key (`brief_cmds::brief_status`).
  - Variants: clones with `extras.variant_of` (root clip id) + `extras.variant_info`; layouts applied via `layout.apply_preset` (custom presets work; layouts are preflighted on the base with a dry run so nothing is half-created); hooks via `overlay.add_text` (replaces the base's hook-top overlay). `variant.compare` composes draft preview frames at the same out_t into a labelled grid under `<project>/variants/`.
  - `render.batch`: JoinSet + semaphore, default parallel 1, capped at 2 on NVENC (else 4); continues on failure; `only_missing` skips clips whose latest render of that quality is newer than the clip/extras files (qc.run re-saves clip.json, so the reference time is max(render, qc report)). `export.batch`: skips clips with no render, a stale render, failing QC (runs it when missing) or a failing brief, with reasons + fix commands.
  - Tests: tests/brief.rs 4/4, tests/variant.rs 2/2 (real ffmpeg + NVENC; 2x2 variants with extras copied, compare grid viewed, render.batch with a failing clip, export.batch skipping a brief-failing clip). Not verified: Hindi/Devanagari banned words beyond what the saved transcripts contain (matching runs on the romanized transcript, the raw transcript and caption words).
- Merged C1: brief.set/show/clear/check/description, variant.create/list/compare (grid image checked), render.batch, export.batch; export report includes brief status.
- Merged B3: QC checks caption_over_face, face_visible_ratio, face_cut_off, hook_text_present, duration_for_platform (engine checks degrade to warn); fix.suggest (ordered, auto_safe flags), fix.apply (safe fixes + re-render + re-QC), qc.compare (side-by-side sheet). Workspace tests green on the branch.

- Merged B4 **full GUI editor** (Premiere-style, in the web GUI): source panel with draggable/resizable region boxes (layout.edit_region), detect-webcam + draw-2-boxes streamer split, 9:16 live program preview + exact libass frame, multi-track timeline (V/CAP/TXT/A1: trim with word snap, split, delete, reorder, overlay retime, zoom), inspector tabs (Clip, Layout, Captions, Overlays, Audio, QC+fixes, Render queue with background jobs/progress, Brief, Variants, History), keyboard shortcuts, live sync with agent edits. 14 JS modules, no build step. Verified in Playwright at 1440/1280 against real projects; screenshots .playwright-mcp/b4-*.png.
- Merged C3: project.pack (+media)/unpack/list/open/status; URL imports save source_info.json (heatmap feeds highlights.signals — live-verified); source.info --source-id offline.
## 2026-11-15 - C2 Asset library
- New: `asset.list|search|show|add|remove`, `caption.create_style`, `caption.styles` (crates/rustler-commands/src/asset_cmds.rs); `rustler_tools::assets` (manifest, built-ins, setup fetch); `rustler_captions::{CustomStyle, load_custom_presets, write_ass_with, find_preset_in, list_presets_with}` (write_ass unchanged).
- Library = `~/.rustler/assets/` + `<project>/assets/` merged by id. `rustler setup` (`assets: true` by default, `--no-assets` skips) registers 7 built-ins (4 caption styles + 3 layouts), the installed OFL fonts, and 26 files from Kenney Interface Sounds + Impact Sounds (both CC0; License.txt verified at download). Fetch flow: kenney.nl/download-link/<hex of assets/<slug>> with a Referer header returns a signed zip URL (the site's own "continue without donating" call; it answers 403 without Referer/User-Agent).
- Wiring: clip.create / clip.set_captions / variant.create accept custom styles (asset_cmds::check_caption_style); caption.list_styles includes custom ones; render stages library fonts and uses write_ass_with; audio.add_music accepts `asset:<id>` (asset_cmds::resolve_asset_path). audio.add_sfx (other agent) should call the same helper.
- Tests: tests/asset.rs 7 passed + 1 ignored network test passed (26 SFX downloaded, license + sha256 verified, ogg duration via ffprobe). Previews viewed: font specimen, custom caption style (box colour visible), render.preview_frame with the custom style.
- Not done: no music (no verifiably CC0 login-free source found); GUI asset browser; layout assets are only the 3 built-ins (project layouts stay in `<project>/layouts`).
- Merged C2 asset library: asset.list/search/show/add(license required)/remove, caption.create_style +
  caption.styles (custom styles render), setup registers fonts/built-in styles/layouts + 26 Kenney CC0 SFX
  (license text verified). No CC0 music source found without login — users add their own with a license.
  audio.add_sfx / add_music accept `asset:<id>`. Registry: **114 commands**.
- Fixed: progress-log had cp1252 bytes from PowerShell Add-Content → re-encoded UTF-8 (use bash heredocs
  for docs from now on).
- Running (integration): GUI integration pass (assets page, audio/effects/brief/variants/project panels
  verified against real commands) [Opus]; docs/knowledge/skill refresh for the full command set [Sonnet];
  C4 speakers [Sonnet].
- Merged C4 speakers: analyze.speakers (sherpa-onnx + pyannote-seg-3.0 MIT + WeSpeaker Apache; heuristic
  fallback), speaker.rename, caption.speaker_colors, crop.track_speaker. 477 Rust + 116 Python tests green
  on branch. Registry: **118 commands**.
- **Streamer e2e via CLI** on a synthetic stream (gameplay + Lincoln facecam top-right + speech):
  analyze quick → highlights candidates (#1 "ask not what your country…", score 46.9) → layout
  detect-webcam (top-right) → clip create split-vertical + hormozi-pop → overlay add-text hook →
  audio add-sfx asset:kenney click → denoise → progress bar → punch-in effect → render (h264_nvenc,
  18.3 s) → qc passed → brief check correctly FAILED on missing credit text → export. Contact sheet
  inspected: facecam top / gameplay bottom, hook title first 3 s, karaoke captions, progress bar.
  Known: detect-webcam rect includes a sliver of gameplay beside the facecam (nudge region); hook
  title sits over the facecam forehead at default y — agents should adjust position_y.
- Windows PowerShell 5.1 strips quotes in JSON args → use `--input-json @file.json` (docs updated).
- Merged docs refresh: knowledge book now 17 guides + 10 prompts covering all 118 commands (script-
  verified, README quick start run end to end), SKILL.md/AGENTS.md/agent-integration refreshed, feature
  files for f07b, f15, f18–f22, f25, f29.
- `setup` now pre-fetches whisper + YuNet + YAMNet + diarization models; re-ran setup on this laptop:
  engine updated, assets registered (7 built-in, 4 fonts, 26 SFX); `analyze speakers` works with
  sherpa-onnx (pyannote-seg-3.0 + WeSpeaker).
- Running: GUI integration pass [Opus], Linux/WSL validation [Sonnet].
- Merged GUI integration pass: new Assets page (browse/search/preview/add with required license/remove,
  use-in-clip), Audio tab (music, denoise, segment volume, SFX markers draggable on A1), Effects tab +
  FX track (transitions on V1 seams with overlap shown, punch-in/zoom/shake/flash ranges, progress bar),
  caption style creator with exact preview, Brief page (form, check with fix buttons, post text),
  Variants (create/compare/batch render/export), Project dashboard (status badges, recent projects,
  pack/unpack), QC (fix.apply, qc.compare), Speakers (analyze/rename/colours/track active speaker).
  Every action verified in Playwright and confirmed with a CLI read; 0 console errors; screenshots
  .playwright-mcp/gi-*.png.

## 2026-11-15 (~06:10) — closing the known limits
- Stop-hook review: gaps remained against the agreed scope (Tauri/desktop app was in the design; known
  limits listed). Now running: native desktop app wrapping the editor [Opus]; webcam edge-snap +
  Devanagari OCR [Sonnet]; real two-person footage speaker-framing test + fixes [Sonnet].
- Next: release binaries (Windows + Linux) on the private GitHub repo + one-line installers; CC0 music
  (FreePD public-domain library) in setup assets.
- Merged webcam edge-snap + Devanagari OCR: engine `overlay.find_box` (edge-supported rectangle around the
  face; stream_demo facecam now x1490..1890 y30..546 vs true 1490..1890/30..550 — was 1459..1920 with a
  gameplay sliver); `layout detect-webcam` reports rect_source edges|expansion. OCR `lang:
  latin|devanagari|auto` with PP-OCRv5 Devanagari ONNX (Apache-2.0, sha-pinned): "नमस्ते दोस्तों" read at
  0.95/1.00 confidence. Engine 131 passed. Known limits 1 + 2 closed.
