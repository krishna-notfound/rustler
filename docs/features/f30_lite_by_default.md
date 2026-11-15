# f30 - Lite by default, heavy features opt-in

**Status:** Built (tests pass; sizes measured, see the progress log)  |  **Depends on:** f01, f06, f21, f23, f07b, f29

## Purpose

Rustler must be lightweight for cloud agents. The agent already has vision (`see frame`, `see tiles`) and decides
itself which section to use, so heavy local analyzers (OCR, YAMNet, diarization, face-track QC, CUDA wheels) are
optional helpers, never implicit costs. Nothing is removed: every capability stays, behind an explicit opt-in.
Music and sound are open: the agent can add any file, link or source audio.

## Scope

**In**
- **Install profiles.** `engine/pyproject.toml`: base = faster-whisper, opencv-python-headless, scenedetect, numpy,
  indic-transliteration; extras `ocr` (rapidocr + onnxruntime), `audio-events` (onnxruntime), `speakers`
  (sherpa-onnx), `cuda`, `full` (ocr + audio-events + speakers). `setup run` = **lite** (base engine, whisper `base`,
  no heavy-model prefetch, no SFX pack); `--profile full` = everything (cuda only with an NVIDIA GPU and
  `--gpu auto`); `--with ocr,audio-events,speakers,cuda,sfx-pack` adds pieces; `--no-models` skips downloads.
  `uv sync --inexact` + a manifest `install` record (extras, sfx_pack) mean a lighter run never removes anything.
- **Graceful opt-in errors.** Engine `RpcError` carries `hint` + `extra`; missing extras raise `dependency_missing`
  with ``run `rustler setup run --with <extra>` ...``; the bridge surfaces the hint (exit 4). `audio.events` and
  `speakers.diarize` in `auto` mode keep falling back to their heuristic and put the hint in `warnings`.
  `RUSTLER_HIDE_EXTRAS=ocr,speakers` simulates a lite install (tests).
- **doctor** reports `profile`, `install` and `optional_features[]` (`available` + exact `enable` command).
- **QC is cheap.** `qc run --faces` / project `qc_faces: true` start the engine face checks; otherwise they are
  `pass` with `{"skipped": "off by default — look at the contact sheet, or run qc run --faces"}`. `fix suggest|apply --faces`.
- **analyze quick** `--transcribe auto|always|never` (auto reuses fetched captions of a URL source, no whisper).
- **Open music/assets.** `asset add`: license optional (stored as "unspecified (added by user/agent)" + warning),
  `--url`, `source:<id>[@a-b]`; `audio add-music|add-sfx` accept path, `asset:<id>`, `http(s)` URL, `source:<id>[@a-b]`
  (`media_ref.rs`; downloads/extractions cached in `<project>/assets/_cache/`; URL needs network+downloads permission).
- Default project whisper model is now `base` (core `ProjectSettings` default; existing projects keep their value).

**Out:** removing any analyzer; AI API calls; scraping page URLs (only direct links); image overlay layers (not built).

## Owned files

`engine/pyproject.toml`, `engine/uv.lock`, `engine/rustler_engine/{rpc,ocr,audio_events,speakers,transcribe}.py`,
`engine/tests/test_extras.py`, `crates/rustler-tools/src/{setup,engine_install,assets,doctor}.rs`,
`crates/rustler-engine-bridge/src/protocol.rs`, `crates/rustler-commands/src/{setup_cmds,qc_cmds,fix_cmds,analyze_cmds,asset_cmds,media_ref,audio_cmds,clip_cmds,project_cmds}.rs`,
`crates/rustler-media/src/qc.rs`, tests `crates/rustler-commands/tests/{lite,asset,analyze,fix}.rs`.

## Status Log

- 2026-11-15: built; tests in the files above; sizes and the implicit-engine audit are in `docs/progress-log.md`.
