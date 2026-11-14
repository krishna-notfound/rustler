# Complete Rustler — Remaining Features Plan (Slices 2–5)

> For agentic workers: follow `docs/agent-brief.md`. Each wave item is one agent in its own worktree,
> owning the files listed. TDD; real-tool tests; LOOK at rendered images. `$env:CARGO_BUILD_JOBS=2`
> (laptop memory is tight — max 4 agents at once).

**User instruction (2026-11-15):** "fully complete it with full GUI and CLI — CLI first, then implement
everything, all working." No AI APIs (the driving agent thinks; Rustler = local tools).

**Spec:** `docs/superpowers/specs/2026-11-15-rustler-design.md` (§5, §6 feature list). Built so far:
see top of `docs/progress-log.md`.

**GUI decision (Claude's call):** the full Premiere-style editor is built in the existing local web GUI
(`crates/rustler-gui`, plain JS, no build step) — timeline, canvas region editor, caption editor, assets,
render queue, live agent activity. Reason: same capability as a Tauri app, no heavy WebView/Node
toolchain build on a 16 GB laptop; a Tauri shell can wrap the same page later (backlog).

## Global rules
- Every capability is a registry command first (CLI/MCP get it automatically); GUI calls the same commands.
- `clips/<id>/extras.json` holds clip data the frozen `Clip` model lacks: `music`, `crop_paths`,
  `overlays`, `audio_fx`, `sfx`, `transitions`, `effects`, `progress_bar`. Read-modify-write without
  clobbering other keys (use one shared helper `rustler_commands::extras::{load, save}` — created by A1).
- Project-level libraries: `<project>/layouts/<name>.json` (custom layout presets), `<project>/brief.json`,
  `~/.rustler/assets/` (global asset library with `manifest.json`).
- Each new command: typed schemas, 1–2 CLI examples, dry-run, events, revision-checked clip saves,
  `next` suggestions where useful, fix hints in errors.
- Update `knowledge/guides/*` and `skills/rustler/*` mentions when a "(if available)" command becomes real.

## Wave A (running/now)
- **A1 Render quality + overlays** (Opus): two-pass loudnorm; text overlays (hook-top, lower-third);
  `overlay.add_text|list|edit|remove`. Owns compile.rs/render.rs, overlay_cmds.rs, extras helper.
- **A2 Layout region system (f15)** (Sonnet): `layout.add_region|edit_region|remove_region|reorder`,
  `layout.save_preset|list_presets (incl. custom)|apply_preset|delete_preset`, `layout.detect_webcam`
  (find a face-containing corner box in a stream frame via engine faces.track over a few samples →
  suggested normalized rect + split-vertical params), `layout.show`. Region fields: src/dst rect, z, fit.
  Owns new layout_cmds.rs (+ additive fns in rustler-core/src/layout.rs).
- **A3 Highlights + cut refinement (f19/f20)** (Sonnet): `highlights.signals {source_id}` (per-window
  deterministic features: loudness peaks, heatmap peaks if URL source, speech rate, question/exclamation,
  pause-before, scene cuts) and `highlights.candidates {source_id, count, min_len, max_len}` (sentence-
  aligned windows scored by those features; returns unit ranges + reasons — agent decides);
  `edit.refine {clip_id}` (snap to sentence ends, silence trough, weak-opener rewind for EN+Hinglish).
  Owns highlights_cmds.rs, cut_cmds.rs.

## Wave B (after A1 merges compile.rs)
- **B1 Audio post + transitions/effects (f17/f18)** (Opus): `audio.denoise {strength}` (afftdn/anlmdn),
  `audio.volume {segment?, db}`, `audio.add_sfx {at_out_t|at_word, file|asset, volume}`,
  `audio.list`; `transition.set {between: seg_a→seg_b, kind: cut|fade|xfade-<type>, duration}`
  (frame-exact xfade/acrossfade), `effect.add {kind: punch-in|zoom-slow|shake, start, end, amount}`,
  `effect.list|remove`, `overlay.progress_bar {on, color}`. Owns compile.rs/render.rs + audio/fx cmds.
- **B2 Understanding pack v2** (Sonnet): engine `ocr.run` (RapidOCR on keyframes), `audio.events`
  (laughter/applause/music/speech via a small license-checked ONNX classifier, e.g. YAMNet; if no
  safe model, energy+spectral heuristics labelled as such); commands `analyze.ocr`, `analyze.events`,
  `analyze.faces` (face presence timeline), folded into `analyze.quick --deep` and `highlights.signals`.
- **B3 Quality loop v2 (f21)** (Sonnet): QC adds `caption_over_face` (faces.track on render vs caption
  band), `face_visible_ratio` (center-crop/track layouts), `hook_text_present` (overlay or caption in
  first 3 s), `fix.suggest {clip_id}` → ordered list of exact commands to run; `qc.compare {clip_id,
  render_a, render_b}` side-by-side contact sheet.
- **B4 Full GUI editor (Slice 5)** (Opus): clip editor page: program monitor (proxy video + canvas
  overlay with draggable/resizable region rectangles bound to layout.edit_region; crop-path keyframes
  shown), multi-track timeline (segments with trim handles → edit.trim, split at playhead, delete/move,
  word-snapped; caption track; overlay track; audio/music/sfx track), caption editor (words list, style
  picker with live preview_frame), layout preset picker incl. custom + detect-webcam button, overlays
  panel, audio panel, render queue (render.run with progress via polling events), QC/fix panel
  (fix.suggest buttons), asset browser, brief panel, variants panel; live sync (poll events, reload
  clip on revision change); keyboard shortcuts (space play, S split, Del delete, Ctrl+Z undo).
  Only calls registry commands. Owns crates/rustler-gui/**.

## Wave C
- **C1 Brief, variants, batch (f22)** (Sonnet): `brief.set|show|check {clip_id?}` (schema: creator,
  platform, min/max duration, aspect, caption style, required text/hashtags/mentions, banned words,
  watermark/credit text, payout model, notes); `brief.check` over transcript/overlays/duration/QC;
  `variant.create {clip_id, styles?:[], layouts?:[]}` (clones clips), `variant.render_all`,
  `variant.compare` (contact-sheet grid); `render.batch {clip_ids|all, parallel: n}`; export report
  includes brief compliance.
- **C2 Asset library (f23)** (Sonnet): `asset.list|search|show|add {path, kind, license, tags}|remove`,
  kinds font/music/sfx/image/caption-style/layout; `setup --assets` fetches a CC0 SFX pack (e.g. Kenney
  audio, CC0 — verify) with manifest entries; custom caption styles `caption.create_style|list_styles`
  stored as asset JSON consumed by rustler-captions (additive preset loading).
- **C3 Project & jobs** (Sonnet): `project.pack {out}` / `project.unpack {archive, dir}` (zip with
  relative paths, optional media), `project.list` (recent projects registry in ~/.rustler), `job.*`
  only if needed by GUI render queue (status from events).
- **C4 Speakers (f29, optional)** (Sonnet): `analyze.speakers` via sherpa-onnx diarization (license-
  checked models) → word speaker labels; speaker-coloured captions option; active-speaker crop using
  lower-face motion vs speaker turns. Skip cleanly if models' licenses don't check out.

## Wave D — integration
- Full `cargo test --workspace` + engine pytest; e2e scripts for: stream split-layout clip with
  detect-webcam + hook title + music + sfx + transition; Hinglish clip; brief check + variants.
- Update knowledge book, SKILL.md, AGENTS.md, docs/features, progress log. Release build when memory allows.
