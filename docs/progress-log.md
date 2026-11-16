# Progress Log

## 2026-11-15 - Lite by default, heavy features opt-in (f30)

User feedback: local OCR / audio-event / diarization / face-track analysis stresses cloud agents; the agent has
vision and decides itself; music must be fully open. Nothing was removed: everything heavy is now explicit opt-in.

**What changed** (details in `docs/features/f30_lite_by_default.md`):
- `setup run` = **lite**: base engine (faster-whisper, opencv-headless, scenedetect, numpy, indic-transliteration), whisper
  `base`, no yunet/yamnet/diarization/ocr prefetch, no SFX pack, no CUDA wheels. `--with ocr,audio-events,speakers,cuda,sfx-pack`
  adds pieces, `--profile full` adds all (cuda only with an NVIDIA GPU), `--music-pack` is the separate opt-in FreePD pack,
  `--no-models` skips downloads. `uv sync --inexact` + manifest `install` record: a lighter run never removes anything.
  `doctor run` reports `profile` and `optional_features[]` with the exact enable command.
- Missing extras fail `dependency_missing` (exit 4) with ``run `rustler setup run --with <extra>` `` (engine `RpcError.hint/extra`,
  bridge surfaces it). `audio.events`/`speakers.diarize` auto mode still fall back to heuristics, with the hint in `warnings`.
- `qc run --faces` / project `qc_faces` for the engine face checks; default = `pass` + `{"skipped": "off by default ..."}`;
  `fix suggest|apply --faces`. `analyze quick --transcribe auto|always|never` (auto reuses fetched captions, no whisper).
- Open music/assets: `asset add` license optional (+ `--url`, `source:<id>@a-b`), `audio add-music|add-sfx` accept path, `asset:`,
  `http(s)`, `source:<id>[@a-b]` (`media_ref.rs`, cached in `<project>/assets/_cache/`).
- Fixed on the way: `cuda_available()` was true on any NVIDIA machine even without cublas, which would have broken a lite install
  there (CPU is now used until `--with cuda`).

**Decisions / deviations to review**
1. `ProjectSettings` (core, "frozen") changed: `default_whisper_model` default `small` -> `base` (so lite's prefetched model is the one used)
   and a new `qc_faces: bool` (default false, serde default). Existing projects keep their saved values.
2. `uv.lock` regenerated; `onnxruntime` is also a dependency of faster-whisper (VAD), so the `audio-events` extra adds no size
   (kept as a named extra so the feature is reported/enabled uniformly).

**Measured** (Windows, temp `RUSTLER_HOME`, `target\debug\rustler.exe setup run`, ffmpeg/yt-dlp/uv/fonts pre-copied):
| | engine venv | models | python | wall time |
|---|---|---|---|---|
| lite (`setup run`) | 0.37 GB | 0.14 GB (whisper base) | 0.07 GB | 4 min 23 s (PyPI cache cold) |
| lite + `--with ocr` | 0.42 GB | + OCR dev. model | | 45 s |
| full without cuda (`--profile full --gpu none`) | 0.47 GB | 0.64 GB (base+small+yunet+yamnet+diarization+ocr) | 0.07 GB | 25 min (slow network; model + SFX downloads) |
| previous install on this laptop (full + CUDA wheels) | 2.5 GB | | | |
Lite over an existing full install: extras kept, venv size unchanged (nothing removed). The CUDA wheels (about 2 GB) are the
real weight; they are now only installed with `--with cuda` (or `--profile full` on an NVIDIA machine).

**Where the engine is started** (`EngineClient::spawn` via `spawn_engine`; all user-invoked, none implicit):
`analyze.transcribe`, `analyze.quick` (only when it must run whisper or detect scenes; skipped transcript with `--transcribe never` or reused captions),
`analyze.scenes`, `analyze.ocr`, `analyze.events`, `analyze.faces`, `analyze.deep` (OCR+events+faces), `analyze.speakers`,
`crop.track_speaker`, `crop.track_face`, `layout.detect_webcam`, `source.fetch_transcript` (only to romanize Devanagari captions),
`setup.run` (model prefetch), `doctor.run` (capabilities), and `qc.run` / `fix.suggest` / `fix.apply` / `export.clip` / `qc.compare`
**only with `--faces` or `qc_faces: true`** (before: every QC run). `source.import`, `render.*`, `see.*`, `highlights.*` never start it.


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

**Known limits / next ideas (updated after the gap-closing pass):**
1. ~~detect-webcam sliver~~ — fixed: edge-snapped facecam box (within 1 px); agents can also read boxes off `see frame --grid true`.
2. ~~Devanagari OCR~~ — fixed: `analyze ocr --lang devanagari|auto` (PP-OCRv5 Devanagari).
3. ~~No music~~ — agents bring any music (file / URL / audio from an imported source); optional CC0 pack via `setup run --music-pack`.
4. Speakers: verified on real two-person White House footage (speaker in crop 97 % / 100 %). Still: the auto speaker count is a guess (pass `--num-speakers 2`), the first second of a turn can stick to the previous speaker, and very wide shots make faces small in 9:16.
5. Windows PowerShell 5.1 strips quotes in JSON args → use `--input-json @file.json`.
6. ~~Desktop app / installers~~ — done: `rustler desktop` native window; release v0.1.0 with Windows + Linux builds and one-line installers (private repo → needs gh or a token).
7. Linux desktop window needs WebKitGTK (`linux-webview` feature); Linux agents use the CLI/MCP/web GUI.
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
- Merged **native desktop app**: `rustler desktop` / `rustler-desktop.exe` — native "Rustler" window (wry +
  tao, WebView2; Tauri's engine without its build tooling) hosting the same editor, native menus (File:
  Open project / Open Recent / Quit; Edit: Undo/Redo; View: Reload/Zoom; Help), folder picker, saved
  window state, server bound to 127.0.0.1 + token. 12 tests; release build 27.9 MB. Screenshots
  .playwright-mcp/desktop-*.png. Linux window behind `linux-webview` feature (needs WebKitGTK).
- Running: release binaries + installers + CC0 music (FreePD) [Sonnet]; real-footage speaker test [Sonnet].
- Added `see frame --grid true` (labelled 0–1 coordinate grid, 1280 wide) and `--regions-of <clip>`
  (draws the clip's region boxes with numbers). Recommended path for vision agents: look at the grid,
  read the webcam/gameplay box edges, `layout from-frame-boxes`, verify with `--regions-of`. No model
  runs. Verified on the stream demo (facecam read off the grid as x 0.775–0.985, y 0.02–0.50; the
  regions view exposed the old box cutting the facecam's bottom). Guides updated.
- Merged **release v0.1.0 + installers**: https://github.com/krishna-notfound/rustler/releases/tag/v0.1.0
  (private) — rustler-v0.1.0-windows-x64.zip (rustler.exe + rustler-desktop.exe + skills + docs) and
  rustler-v0.1.0-linux-x64.tar.gz (CLI, glibc ≥ 2.30 → Ubuntu 20.04+), SHA256SUMS. `scripts/install.ps1`
  / `install.sh` (gh or RUSTLER_TOKEN/GITHUB_TOKEN; sha256 check; ~/.rustler/app; PATH; skill copy;
  setup) tested in temp dirs on Windows + WSL (118 commands). docs/install.md.
- FreePD closed in 2025 → opt-in `setup run --music-pack` pulls 11 CC0 tracks from the Internet Archive
  capture (license sentence verified, sha256 pinned). **Not installed by default** (agent brings any music).

## 2026-11-15 (~10:00) - speaker framing on real two-person footage (branch `speaker-real-footage`)
- **Media (no login, public domain, `docs/test-media.md`):** two White House fireside chats on Wikimedia Commons (`PD-USGov-POTUS`):
  "Vice President Vance Holds a Fireside Chat" (95 s excerpt: wide two-shot with faces 4-6 % of the width + cuts to a close-up) and
  "...joins Breitbart's Matthew Boyle..." (105 s excerpt: medium/very wide shots with slow zooms, close-ups at the end).
- **Real CLI run:** `source import` -> `analyze quick` -> `analyze speakers --num-speakers 2` -> `clip create` (center-crop) ->
  `crop track-speaker` -> `caption speaker-colors` -> `render run --quality draft` -> `qc run` (passed) -> contact sheets and frames looked at.
- **What broke (all reproduced with the old code, then fixed):**
  1. Diarization put host and guest in one cluster (S1 = 96 % of the time, auto count 7 speakers). Fix: windowed-embedding re-segmentation
     with debris folding, Viterbi smoothing and quiet-moment snapping (`engine/rustler_engine/speakers.py`). Hand-labelled Vance excerpt: 91.5 % of
     speech time correctly attributed (old 70 %); Boyle excerpt turns within 0.3 s of the transcript; auto count 2 on both excerpts (JFK+FDR test: 2;
     200 s guest monologue: 1). Cost: about +0.2 s per second of audio on CPU.
  2. Faces were tracked by position; the close-up got a different id than the same person in the wide shot, and five "faces" appeared for two
     people. Fix: SFace (OpenCV Zoo, Apache-2.0, 38 MB, sha256-checked, optional) person identity across cuts (`engine/MODELS.md`).
  3. Mouth movement is noise on 4-6 % wide faces (left/right pick in the wide shot 60-78 % right, the host's 22 s turn at chance). Fix: only trusted on
     faces >= 7 % wide; speaker -> person comes from close-ups (`face-id`) or elimination.
  4. After a cut the crop kept a position from the old shot. Fix: follow the largest visible face (`speaker_face_samples`).
  5. **Crop drift bug** (also in `crop track-face`): the path only emitted a key when it moved and the renderer interpolates linearly, so the
     crop slid for seconds toward the next position (speaker out of the crop in 14.8-15.8 s of the Vance excerpt). Fix: `SmoothParams::hold_keys`
     (additive; on for `crop track-speaker`, off by default so `crop track-face` is unchanged: its owner can switch it on).
- **Numbers** (1 s samples inside hand-labelled speaking spans; speaker's face centre inside the crop window): Vance excerpt old track-speaker 33 %
  -> new 97 % (84/87), `crop track-face` 70 %; Boyle excerpt old track-speaker 95 % (lucky: wrong for the first turn), new 100 % (102/102),
  `crop track-face` 63 %. Captions: S1 yellow / S2 cyan checked on frames in both renders.
- Remaining: the first seconds of a turn can land on the previous speaker when the diarizer's windows straddle a switch (Vance excerpt 5.9-7.6 s: the
  host's one-line intro attributed to the guest); faces < 4 % of the frame width are small in the crop; the auto speaker count needs clearly different voices.
- Tests: `cargo test -p rustler-commands --lib speaker` (15, incl. 3 fixture tests from the real runs), `cargo test -p rustler-media --test facepath`
  (14), engine `pytest` (122 passed, 1 skipped).
- **YouTube speed fix** (user found it slow): yt-dlp now spends ~3 min per YouTube lookup solving a JS
  challenge (Deno), measured 191–398 s per call; network itself is fine (~0.74 MB/s, YouTube page 1.4 s).
  Fix: one cached lookup per video (4 h) shared by `source info`, `fetch-transcript` and
  `import --sections` via `--load-info-json`. Measured: fetch-transcript 398 s → 10.6 s, section import
  235 s → 15.8 s, repeat info 0.1 s; YouTube part of a flow ~14 min → ~3.7 min.
- New `transcript import <src|url> --file …` (VTT/SRT/json3/words/lines): agents bring transcripts from
  their own tools; 0.24 s; section time shift verified on the TED section. Registry: 119 commands.
- Merged lightweight-by-default (lite setup 0.37 GB venv + 0.14 GB models vs ~2.5 GB before; heavy
  extras opt-in with `--with`; QC face checks opt-in; analyze quick --transcribe auto reuses captions;
  music/SFX from any file/URL/source). Pushed.
- **docs/MUSE.md**: how to give Rustler to Muse (token, install.sh, guides, copy-paste prompt). User is
  travelling; Muse will operate Rustler. Running: release v0.1.1 build (Windows + Linux) and the real
  5-clip trial on the Vinh Giang × Raj Shamani podcast → GitHub release `clips-trial-1`.

### 2026-11-16 - Fixes found while making the real 5-clip trial (Vinh Giang x Raj Shamani)

- **Crop paths lost after edits** (`extras.rs` `carry_crop_paths`, called from `edit_cmds::finish` and `edit.refine`):
  `crop_paths` are keyed by segment id with segment-relative times, but `edit.split` / `remove-pauses` / `remove-words`
  mint new ids and `trim` / `set-range` / `refine` / history restore move a segment start. The face path stayed on a dead
  id and the pieces silently rendered the stale static crop (on the trial clip the window was 0.56-0.88 while the face
  was at 0.5: face out of frame). Now new pieces inherit the parent's path (shifted, cut to the piece, interpolated
  ends); a moved start shifts the keys (all kept, so undo is lossless); a warning names segments that now reach outside
  the tracked range. Test: `tests/edit.rs::crop_paths_follow_edits`.
- **`crop track-face <clip>` failed on mixed layouts**: without `--segment-id` it errored on the first blur-fit segment;
  it now tracks the center-crop segments and lists the skipped ones as warnings (all non-crop still fails).
  Test: extended `tests/clip.rs`.
- **Still crop path broke render/preview**: a path whose x never changes (single key, common for short segments)
  wrote an empty sendcmd file; ffmpeg rejects it ("No commands were specified"). Now a plain `crop` at that x.
  Test: `tests/compile.rs::still_crop_path_uses_plain_crop`.
- **`>>` burned into captions**: YouTube caption speaker-change markers are timed words in captions-based transcripts.
  `chunk_words` now skips tokens made only of ASCII punctuation (`>>`, `-`). Test: `tests/ass.rs::speaker_markers_are_not_captioned`.
- Observed, not changed: `crop track-face` centres the face box, so tight close-ups of a profile speaker leave the nose
  at the crop edge and the smoothed path lags quick leans (worked around with split + static `focus_x`); caption-derived
  word timings stretch words over pauses, so `edit remove-pauses` finds nothing (re-transcribing with whisper fixed it);
  hormozi-pop at 0.076 renders ~45 px cap height (used a custom 0.115 style).

## 2026-11-16
- v0.1.1 released (Windows + Linux, install.sh verified in WSL; ~500 tests green crate by crate).
- Real trial: 5 clips from Vinh Giang × Raj Shamani → release `clips-trial-1`, all QC pass; 4 bugs found and
  fixed (crop path lost on split/trim, track-face failing with blur-fit parts, static crop crash, ">>" markers in
  captions), merged + pushed. Gaps seen: face centring without look-room, small hook text, no way to hide source
  pop-ups/logos, missed scene cuts in dark footage.
- Muse's 23-item list triaged; OpusClip researched (`docs/research/opusclip.md`); v2 design drafted
  (`docs/superpowers/specs/2026-11-16-rustler-v2-design.md`), builder check running; awaiting user approval.
- Note: git push over SSH is blocked on this network → push over HTTPS with the gh token.

## 2026-11-16
- v0.1.1 released (Windows + Linux; install.sh verified in WSL; ~500 tests green crate by crate).
- Real trial: 5 clips (Vinh Giang × Raj Shamani) → release `clips-trial-1`, all QC pass; user: "clips are good".
  4 bugs found + fixed (crop path lost on split/trim, track-face with blur-fit parts, static crop crash, ">>" in
  captions). Gaps: face centring without look-room, small hook text, can't hide source pop-ups, dark scene cuts.
- Muse's 23-item list → `docs/research/muse-live-test-2026-11-15.md`; OpusClip analysis → `docs/research/opusclip.md`.
- v2 design `docs/superpowers/specs/2026-11-16-rustler-v2-design.md`: approach A approved; added director layer
  (campaign analysis, retention map/score, playbook, hook lab, cold open) and smooth camera; builder-check
  questions answered as Claude's calls. Awaiting final approval of the design doc.
- Git: default branch renamed master → **main** (GitHub default updated, master deleted); origin now HTTPS
  (SSH port blocked on this network).

## 2026-11-16 — PAUSED (user at college) — how to resume
Merged to `main` and pushed: wave A of the v2 plan — 0.1 (720p default, `--quality`, `--audio-only`), 0.2 (Deno
JS-runtime install, `--insecure-tls`, `doctor run --network`), 0.3 (`render batch` parallel 2, ETA in progress),
0.4 (mobile-size captions/hooks, `effect add --words`). Branch is `main` (master renamed). v0.1.2 NOT yet released.

Paused mid-task (work saved as WIP commits, each branch pushed to GitHub; worktrees kept locally):
| Branch | Task | State when paused |
|---|---|---|
| `worktree-agent-a9576d299e649e986` | 1.3 layers (images, masks, cutaways, caption word overrides) — Opus | layers module committed; was wiring render_cmds.rs |
| `worktree-agent-ac9af68683ae375d1` | smooth camera planner + motion QC — Opus | planner in progress; WIP commit; guide pending |
| `worktree-agent-acb178ee097543b7a` | 0.6 settings + 1.7 posting JSON/campaigns — Sonnet | WIP commit **may not compile**: a bad regex edit on `.push(` calls was being repaired |
| `worktree-agent-a6bde39ca2a13759d` | director: clip.retention, score_set, campaign.analyze, guides — Opus | retention analysis module committed; was running tests |
Resume: for each branch, re-launch an agent on its worktree with "continue Task X from the WIP commit; run its tests;
`git merge main`", then merge to main in this order: settings/posting → layers → camera → director. Then release v0.1.2,
then wave C (timeline doc + insert/cold-open/merge/duplicate/speed/freeze, cards, references) per
`docs/superpowers/plans/2026-11-16-rustler-v2.md` FAST PARALLEL SCHEDULE.

## 2026-11-17 — resumed
- Merged **0.6 settings** (`config.get/set/unset/list`, project overrides global; used by source.import quality,
  render.batch parallel, qc.faces) and **1.7 posting JSON + campaigns** (`campaign.create/show/list/delete/mark`,
  `clip.meta_context`, `clip.meta_set`; export writes `<name>.post.json`; QC `meta_present`, `campaign_compliant`;
  guide campaigns-and-posting + prompt write_clip_meta). 333 tests passed on the branch.
- Disk was down to 2.3 GB (a link error) → removed old `target/release` → 33.7 GB free. Keep an eye on disk:
  each worktree build is several GB.
- Still running: layers (1.3, Opus), smooth camera (Opus), director/retention (Opus).
- Merged **director layer** (f31): `clip.retention` (per-second retention map of the output clip, dead zones with
  exact fix commands, 0–100 retention score), `clip.score_set` (agent's hook/flow/value/trend score; in export
  report + post.json), `campaign.analyze` (rules, duration, payout strategy, clip counts by source length), guide
  `director-retention` + prompts `direct_campaign`, `hook_lab`. Workspace tests on the branch: 581 passed, 0 failed.
  Real run on the 5 trial clips: retention scores 43–63 (all have hooks; low pacing/dead zones). Applying the 5
  suggested punch-ins on clip 04 raised it 43 → 80. Registry: 122 commands.
### 2026-11-16 — Smooth camera planner + motion QC (v2 §5.10, branch `worktree-agent-ac9af68683ae375d1`)
- New `crates/rustler-media/src/camera.rs`: offline trajectory over the whole segment. Primary face (median-filtered,
  so 1–2-sample detection jumps never move the camera) → look-room target (nose vs eye midpoint from the YuNet
  landmarks, ±10 % of the crop width, smoothed per shot, capped so a big close-up keeps the back of the head inside)
  → shots split at source cuts (face jump, size change, a head-direction flip in one sample = reaction shot, and
  `analysis/<src>/scenes.json` when present) and at speaker switches → **hold-or-move**: locked while the subject stays
  inside ±18 % of the crop width; a move only when it stays out 0.5 s (or would leave the frame), to where the subject
  settles, as a minimum-jerk ease (≤ 0.35 frame widths/s, ≤ 0.8 /s²), starting with the lean's onset (+0.1 s, at most
  0.5 s before the box crossing; at least 0.15 s). Keys at 30/s during moves only; nothing while holding.
- `crop.track_face` / `crop.track_speaker`: `style: locked|smooth|follow` (default smooth), `look_room` (default true;
  `--no-look-room`); track-speaker `switch: auto|cut|pan` (auto: cut on the turn start when the reframe is > 40 % of
  the frame, else an eased pan ≤ 0.6 s centred on it). Results report `moves`, `cuts`, `look_room_shift`. Track-speaker's
  look-room costs one extra `faces.track` pass. `facepath::crop_keys` is kept (old tests; comparison).
- Engine `faces.track` now returns `lm` (5 landmarks) per face (PROTOCOL.md updated, additive). **The installed engine
  in `~/.rustler/engine` must be updated (`rustler setup`) for look-room**; without landmarks the camera centres the face.
- QC `camera_motion` (engine-free, from `extras.crop_paths`): moves/min, direction changes/min, jitter (wasted
  back-and-forth per second), moves starting at full speed, max velocity/accel/jerk; warns with a `crop track-face
  --style …` fix hint. `QcExtras.crop_paths` added (one-line fill in `qc_cmds::gather_extras`).
- **Real check** (temp copy of `rustler-real1`, engine with the new `faces.py`, frames looked at): old paths QC:
  play-the-instrument pass (drifting), free-vs-three warn (9.6 direction changes/min). First planner run found three
  problems, all fixed and re-checked: (1) a 1.5 s reaction shot of Raj was *panned* across (face moved only 0.03, head
  direction flipped) → now 2 crop cuts, Raj locked with room on his right; (2) a quick lean touched the right edge
  (move began at the box crossing) → move now starts at the lean onset, face stays in frame; (3) full look-room on a
  profile close-up (face 44 % of the crop) cut the back of the head → capped. Final: play-the-instrument 2 moves + 2 cuts
  in 45 s, free-vs-three 2 moves in 37 s, both `camera_motion` pass (jitter 0, 0 abrupt starts, max 0.19 frame widths/s);
  look-room puts the space on the side each person faces (Vinh left, Raj right).
- Tests: `cargo test -p rustler-media --test camera` (12), qc unit `camera_motion_warns_on_jitter_and_passes_planned_paths`,
  speaker real-fixture planner tests (2 recorded excerpts: speaker in crop ≥ 90 % / ≥ 95 %, no wobble, no abrupt
  starts), engine `test_faces.py` (6, landmarks).
- Not done: §5.10 "contact strip of the crop path" image; push-in/punch-in moves (separate tasks).
- Merged **smooth camera** (`crates/rustler-media/src/camera.rs`): hold-or-move with a ±18% safe box, minimum-jerk
  eased moves (≤ 0.35 widths/s, ≤ 0.8/s²), anticipation from lean onset, outlier rejection, look-room from YuNet
  landmarks (capped for big faces), cut-with-shot on source cuts; `crop track-face|track-speaker --style
  locked|smooth|follow`, `--no-look-room`, `--switch auto|cut|pan`; QC `camera_motion` (jitter, moves/min, abrupt
  starts). Real trial clips: 2 moves + 2 cuts in 45 s, jitter 0. Needs `rustler setup run` with this build for the
  engine's landmark output (look-room). 380 tests passed on the branch.
- Merged **layers** (1.3): `image.add/list/edit/remove`, `mask.add/list/edit/remove` (blur/box/pixelate — hide source
  pop-ups/logos), `cutaway.add/list/edit/remove` (full or PiP B-roll), `caption.word/words_list/words_clear` (per-word
  colour/size/emoji/hide); all in the single render graph, word-anchored items follow re-cuts. Workspace tests on the
  branch: 0 failures. Registry now 134 commands. Note: `history undo` still covers clip.json only (layers via their
  own remove) — timeline doc task will extend undo to all layers.

## 2026-11-17 — PAUSED again (user closing laptop) — how to resume
On `main` (pushed): wave A, settings + posting/campaigns, director/retention, smooth camera, layers. 134 commands.
Not yet released: v0.1.2 (Muse still on v0.1.1).
Paused mid-task (WIP commits pushed):
| Branch | Task | State |
|---|---|---|
| `worktree-agent-a5f8e3d0367d3ae9b` | 1.4 agent graphics (`asset.render_card`, `text.create_style`) — Sonnet | code + tests committed; was rendering 2 real cards |
| `worktree-agent-a03c0c8f29f4f51ee` | 1.6 style references (`reference.import/analyze`) — Sonnet | WIP; was running measures on trial clip proxies |
Not started (stopped right at launch, nothing saved): **timeline doc + timeline edit commands (Opus)** and
**smart defaults + light active speaker (Opus)** — relaunch with the same briefs (plan wave C/D).
Resume order: finish cards + references → merge; launch timeline + smart defaults; then templates (Opus, needs the
timeline doc), hooks/captions presets, cleanup/cover; then knowledge refresh, `clips.auto`, release v0.2.0 + trial.
Before releasing: run `rustler setup run` with the new build so the engine has the landmark output (look-room).
- 2026-11-17: Merged **agent graphics** (f1.4): `asset.render_card` (PNG cards from a JSON spec: gradients, rounded
  boxes, shadows, measured text wrap/fit, images) and `text.create_style` / `text.styles`. Real POV hook card + lower
  third rendered and checked. Running: style references (1.6), timeline doc + edit commands (Opus), smart defaults +
  light active speaker (Opus). Next after them: templates (Opus), hook/caption presets, cleanup/cover, knowledge,
  clips.auto, release v0.2.0.

## 2026-11-16 - Style references (Task 1.6, f32)

- Built `reference.import/list/show/analyze` (4 commands; registry now +4), engine method `reference.measure`
  (`engine/rustler_engine/reference.py`, light: OpenCV + numpy, YuNet when available), guide `learn-style-from-videos`, prompt
  `learn_style_from_folder`. A set lives in `<project>/references/<name>/` (proxies 540 px short side 15 fps, 16 kHz wavs, sheets,
  `profile.json`); originals are never touched. Output `image` lists contact sheets then first-3-second filmstrips (MCP shows them).
- Tests: engine `test_reference.py` (10: known cuts at 3/6/9 s, caption band y~0.75 + words per line + yellow fill, hook gone by ~2.5 s,
  zoom-in 1.0 -> 1.5, cut in dark low-contrast footage, flat-colour cut, fade is not a cut, no caption in plain video, music heuristic
  speech vs speech+music, bad params) all pass; command tests `tests/reference.rs` (synthetic captioned 12 s clip with cuts at 4/8 s,
  `.srt` sidecar 120 wpm, zoom clip, skipped broken file, replace, errors) + 5 unit tests pass; `cargo test -p rustler-knowledge` ok.
- Real run on copies of the 5 trial exports (210 s for `reference analyze`, ~10 s of measuring per 40 s video, rest is sheets/loudness):
  cuts/min 6.5, 16.7, 9.4, 4.8, 14.4 (median 9.4; median shot 3.8 s); caption band y 0.69-0.73 in all 5 (matches the sheets: bold
  uppercase white with green highlight word, 2-3 words, black outline; colours measured #e8e6df/#4df873 style); hook box in the first
  ~3.2 s in 5/5 (matches); loudness -14.0 LUFS in all (the render target); speech 159-254 wpm from the sidecars; layouts: face_closeup
  34 of 38 shots, the chart insert in clips 02/04 shows as `multi_face`. Zoom: dark footage leaves only 0-13 percent of frame pairs
  measurable, so zoom is reported as under-counted (0 events; one in clip 05), not as "no zooms". Known misses: a head turn adds one
  spurious cut in clip 02 (34.3 s), animated charts can look like caption text (the caption pick prefers the edge-dense band that
  changes within shots; verified correct on all 5 after tuning).
- Decisions: punch-in jump cuts (same scene, new scale) are reported as zoom events, not cuts; speech rate only from `.srt/.vtt`
  sidecars (no transcription of references); `reference.import` keeps only the first 180 s of each video (`--max-seconds`).
- Open: templates (Task 1.5) should consume `profile.json` (`based_on {reference_set}`); the engine copy in `~/.rustler/engine` needs
  `rustler setup` to pick up `reference.py`.

## 2026-11-17 — PAUSED (3rd) — resume steps
On `main` (pushed): everything through agent graphics + style references (f32). Not released yet (v0.1.2/v0.2.0).
Paused mid-task, WIP pushed: timeline doc + edit commands (Opus), smart defaults + light active speaker (Opus),
hook presets / keyword+emoji captions / flash+whip / cover frame (Sonnet) — branches `worktree-agent-*` listed by
`git branch -r | grep worktree-agent`. Resume: SendMessage each agent (or relaunch with the same brief on its branch),
merge, then templates (Opus), knowledge refresh, clips.auto, `rustler setup run` (new engine), release v0.2.0, trial.
### 2026-11-17 — Timeline v2 document + timeline edit commands (f33, branch `worktree-agent-ae7f9cafc65fc59fc`)
- `rustler.timeline@2` (`rustler_core::Timeline`, item types in `rustler_core::timeline`): every extras key maps 1:1
  (`overlays` -> `tracks.text`, `crop_paths`/`segment_speed` -> `tracks.video[].crop_path/speed`, `audio_fx`/`music`/
  `sfx` -> `tracks.audio`, `caption_word_overrides`/`speaker_colors` -> `tracks.captions`, `meta`/`post`/`variant_*`
  -> `meta`, ...); unknown keys kept in `tracks.extra`, unknown item fields in each item's `extra`.
- `timeline_doc` (assemble / disassemble / validate / resolve / diff / content-following re-cut helpers);
  commands `timeline.show` (document + resolved edit and rendered times + revision), `timeline.schema`,
  `timeline.apply` (JSON-Patch or document, base_revision -> `revision_conflict`, errors listed by JSON path,
  dry run, human-readable `changes`). History now snapshots extras.json too (`history/<rev>.extras.json`):
  `history undo/restore` bring back every layer (edits saved without a snapshot warn).
- New commands: `edit.insert_segment`, `edit.cold_open` (hook range first, cut out of where it was, flash effect at
  the cut), `clip.merge` (all layers, times shifted, anchors kept, transition between clips), `clip.duplicate`,
  `edit.speed` (0.5-2.0, `setpts`+`atempo`), `edit.freeze` (`loop` + silence). Registry: +9 commands.
- `TransitionMap::with_retime` (compile.rs): edit time -> speeds/freezes -> transitions. `fx_cmds::transition_map`
  and `apply_to_compile` read speeds and freezes, so captions, layers, preview frames, QC `duration`, retention and
  `render.run` use the effective duration.
- Tests: core `timeline` 4, commands `timeline` 9 + `timeline_edit` 8 (incl. real render: cold open + speed 1.5 +
  freeze 0.6 + dissolve, QC duration pass, frozen frames identical); `cargo test -p rustler-media -p rustler-core
  -p rustler-commands` all green on the branch.
- Real check (temp copy of rustler-real1): all 5 clips assemble and dry-run-apply clean. duplicate ten-vs-three ->
  cold open on unit 5 ("You're as good as you can communicate") -> merge with free-vs-three (dissolve 0.25) ->
  speed 1.1 on one segment -> freeze 0.5 s at the hook cut. Draft render 60.79 s vs effective 60.77 s, QC duration
  pass. Frames looked at: hook first with its title, white flash over the frozen frame, clip restarts at "think
  about", second clip's hook title appears right after the merge point, crop paths intact.
- Found on real data and fixed: crop-path keys before the segment start are normal (kept for undo), validation no
  longer rejects them; `clip merge` takes clip ids as positionals; a cold open now keeps a hook-top title that opened
  the clip at the start (it used to follow its words to after the hook).
- Not done / open: v1 layer commands (overlay/image/mask/...) still write extras without a revision bump, so undo
  takes them together with the previous revision; `cut_cmds` (edit refine) saves without an extras snapshot.

### 2026-11-17 — Templates (Task 1.5, f35)
- Built `template.create/update/show/list/delete/apply/diff/export/import` (+9 commands) in `template_cmds.rs`.
  A template (`rustler.template@1`) is a clip's look without content: captions, layout, hook look, text looks,
  brand images, progress bar, loudness/denoise/music, sfx + effect patterns (start / cuts / every N s / time),
  transitions, canvas, fonts. Storage `templates/<name>@<ver>.json` (project) or `<home>/templates/`.
- `template.apply` builds a JSON-Patch per clip and goes through `timeline.apply` (all clips dry-run validated
  first; one revision each; `history undo` reverts). Added items carry `by_template`, so re-applying is a no-op.
  Missing asset/font/style/file -> `not_found` naming it (`template show` lists `missing`).
- Starters `podcast-viral` and `streamer-split` (embedded JSON, trial learnings: hormozi-pop, hook <= 7 words
  at font 0.10/0.09, punch-ins, progress bar, split for streamers; no assets needed).
- `clip.create --template` and config `template.default` (default that cannot apply = warning).
- Export/import zip bundles the template with caption style, fonts, assets and plain files.
- Tests: `--test templates` 5 passed (+3 unit); clip, config, timeline, timeline_edit, variant suites green;
  `cargo check --workspace --tests` clean; `cargo test -p rustler-knowledge` ok. Not done: a real render of a
  templated clip; spec fields layout_rules/cutaway_rules/cleanup/keyword_highlight/export/post_defaults (out of scope,
  rejected as unknown fields).
- Small shared edits: `timeline_cmds::apply` is now `pub`; `ClipCreateIn` gained `template`.
