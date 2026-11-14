# Progress Log

## ★ STATUS AT END OF THE 5-HOUR AUTONOMOUS BUILD (2026-11-15) — read this first

**Working now (tested for real on this laptop):**
- `rustler setup` / `rustler doctor` → installs/detects FFmpeg (libass), yt-dlp, uv + Python engine (CUDA),
  whisper `small`, fonts into `~/.rustler`; doctor = ready, NVENC/QSV/libx264.
- Full clip pipeline through the CLI (e2e passed): project → import (file or URL) → analyze quick
  (transcript + units, scenes, loudness/silences, highlight signals) → transcript get/search → see
  frame/tiles/preview → clip create by **unit ids + --expect-text** → layouts (center-crop, blur-fit,
  split-vertical) → 4 caption styles (bold-karaoke, clean-white, boxed-highlight, hormozi-pop) + SRT →
  music bed with ducking → face-tracked crop (`crop track-face`) → render (one NVENC encode) → QC (12
  checks + contact sheet) → export (mp4 + report.json + srt).
- YouTube **scan-first**: `source info` (chapters, most-replayed peaks), `source fetch-transcript`
  (captions without download), `source import --sections` (downloads only chosen minutes). Verified on a
  20-min TED talk.
- Edit commands with undo: trim, split, delete/move segment, set-range, remove-pauses, remove-words,
  history list/undo/restore.
- Hinglish: Devanagari → Latin romanization ("क्या scene है" → "kya scene hai").
- **MCP server** `rustler mcp serve`: 55 tools, 8 workflow prompts, 14 resources (guides, commands,
  project state); frames come back as images for vision agents.
- **Knowledge book**: 13 guides + 8 prompts (`rustler guide list|show|search`, `rustler prompt list|show`).
- **Simple GUI** `rustler gui`: projects, sources, clips, renders (player + QC + contact sheet), activity,
  generic command runner. Verified in a browser.
- Agent onboarding: `skills/rustler/SKILL.md` (Claude Code), `skills/rustler/AGENTS.md` (others),
  `docs/agent-integration.md`.
- Tests: last full Rust run 308 passed / 1 failed (an outdated test, since replaced; CLI suite then 26/26); Python engine 85 passed. Fixes made after that run were re-tested per crate only. Run `cargo test --workspace` to re-confirm.
- Binary: `target\debug\rustler.exe` (debug build). The release build was stopped by Claude Code because the laptop ran low on memory — run `cargo build --release -p rustler-cli` when the machine is free. The debug binary predates the last knowledge-book wording fix (rebuild to embed it).

**How to try it (PowerShell):**
```
cd C:\Users\krishna\Desktop\rustler
.\target\debug\rustler.exe doctor
.\target\debug\rustler.exe help
.\target\debug\rustler.exe guide show start-here
.\target\debug\rustler.exe --project $env:TEMP\rustler-e2e gui     # opens the demo project in the browser
```
Demo clip from the e2e run: `%TEMP%\rustler-e2e\exports\jfk-hook.mp4`.
Give Claude Code the skill: copy `skills\rustler` to `%USERPROFILE%\.claude\skills\rustler`, then add MCP:
`claude mcp add rustler -- <path>\rustler.exe mcp serve`.

**Known gaps / next steps (in order):**
1. Two-pass loudnorm (single pass lands ~-15.9 LUFS on half-silent clips; QC warns).
2. Live test of `crop track-face` on a real talking-head video (unit-tested; engine path tested separately).
3. Linux run (cloud agents like Muse) — code paths exist, not executed on Linux yet.
4. Not built (deferred by the 5-hour scope): Tauri Premiere-style editor, highlight helpers, briefs/variants,
   asset library, transitions/effects, SFX, speakers/diarization. Multi-region layouts exist via presets +
   layout params; free-form region editing commands are not built yet.
5. GitHub release binaries + one-line installer for agents on other machines.

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
