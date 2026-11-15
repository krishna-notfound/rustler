---
id: troubleshooting
title: "Troubleshooting: errors, exit codes and recovery"
summary: "What each exit code and error means and what to do, missing dependencies, revision conflicts, blocked downloads on cloud IPs, resumable long jobs and Windows path quirks."
order: 12
tags: [errors, exit-codes, recovery, setup, download, windows]
---

# Troubleshooting

Every failure returns `{"ok": false, "error": {"code", "stage", "message", "hint", "retryable"}}` and
a non-zero exit code. **Read `hint` first**; it usually names the exact command to run. Never repeat a
failing command unchanged unless `retryable` is true.

## 1. Exit codes

| Exit | `code` | Meaning | What to do |
|---|---|---|---|
| 0 | — | success | still read `warnings` |
| 2 | `usage` / `validation` | bad flags or input that doesn't make sense | `rustler commands describe <cmd>`; fix input; read `message` (e.g. "opening words do not match") |
| 3 | `not_found` | unknown command, project, source, clip or render id | `commands list`, `source list`, `clip list`; check `--project` |
| 4 | `dependency_missing` | ffmpeg, yt-dlp, uv, engine, model or font missing, **or an optional heavy analyzer that is not installed** | follow the `hint`: `rustler setup run`, or `rustler setup run --with ocr\|speakers\|audio-events\|cuda`; then `rustler doctor run` |
| 5 | `provider` | network/download error (yt-dlp, model download) | check `retryable`; retry once after a pause; see §4 |
| 6 | `media` | ffmpeg/ffprobe failed (corrupt file, unsupported codec, bad filter) | read the stderr tail in `message`; `see frame` the source; re-import; try `--prefer-gpu false` |
| 7 | `qc_failed` | QC ran and at least one check failed | open the report; follow `quality-checklist` |
| 8 | `cancelled` | the run was cancelled or hit a limit | rerun; long commands resume |
| 9 | `revision_conflict` | project/clip changed since you loaded it (someone else edited) | reload (`clip show`), re-apply your change once |
| 10 | `internal` | bug or engine crash | if `retryable`, retry once; otherwise report the full error to the user |

## 2. Setup and dependencies

```
rustler doctor run       # ready, missing[], encoders[], libass, engine, fonts, nvidia_gpu
rustler setup run        # idempotent: re-running only fills gaps
rustler setup run --force          # reinstall everything if files are corrupt
rustler setup run --gpu none       # cloud box without GPU
rustler setup run --with ocr,speakers   # add heavy analyzers later (lite is the default; nothing is removed)
rustler setup run --profile full   # everything: all analyzers, models, SFX pack (+ cuda when an NVIDIA GPU is found)
```

- **Lite vs full**: `setup run` installs the lightweight base (whisper `base`, scenes, face tracking).
  OCR, YAMNet, diarization, CUDA libraries and the SFX pack are opt-in (`--with ...`). `doctor run` shows
  `profile` and `optional_features[]` with the `enable` command for each. Asking for a feature that is not
  installed exits 4 with the exact command in `hint`; heavy models download on first use or with the same flag.
- The default whisper model is `base` (fast, small). For Hindi, accents or noisy audio use a larger one:
  `rustler analyze transcribe <src> --model small` (downloads on first use) or
  `rustler project set-settings --default-whisper-model small`.
- QC is fast by default: the engine face checks (`caption_over_face`, `face_visible_ratio`, `face_cut_off`)
  are `pass` with `{"skipped": "off by default ..."}`. Look at the contact sheet, or run `rustler qc run <clip> --faces`
  (or `rustler project set-settings --qc-faces true`). `fix suggest --faces` / `fix apply --faces` do the same.

- `ready: false` → `missing` lists what to fix. Re-run `setup run`.
- `libass: false` → captions cannot render; setup fetched a wrong ffmpeg build. `setup run --force`.
- Setup downloads from GitHub and Hugging Face. If those are blocked, report it: the user must allow
  them or provide the tools; do not fetch binaries from random mirrors.
- `RUSTLER_HOME` changes where tools live (default `~/.rustler`). A different home = a fresh setup.
- GPU: NVENC is used only after a real 1-frame test encode. If GPU renders fail, add
  `--prefer-gpu false` to `render run` (CPU libx264 always works).

## 3. Revision conflicts (exit 9)

Projects and clips carry a `revision`. If a human in the GUI (or another agent) saved a change after
you loaded, your write is refused instead of silently overwriting theirs.

1. `rustler clip show clp_X` (or `project show`) to get the current state.
2. Check what changed. If it conflicts with your intent (they moved the captions on purpose), keep
   their change, or ask.
3. Re-apply your edit once. Don't loop.

## 4. Downloads blocked on cloud machines

Symptoms: `provider` error with yt-dlp messages like "Sign in to confirm you're not a bot", HTTP
403/429, "This video is unavailable" from a cloud IP.

What to do, in order:

1. Retry once later (`retryable: true` only).
2. Scan without media: `source info` and `source fetch-transcript` sometimes work when media download
   doesn't.
3. Ask the user for **their own** cookies file (exported from their browser). They set it in the
   project permissions (`permissions.cookies_file`); it stays outside the project and is redacted
   from logs. Never use cookies from anyone else.
4. Ask the user for a local copy of the video, or a different machine (their laptop) to run the
   download.

Never: use proxies/VPN rotation to evade a ban, bypass DRM, paywalls, age gates or private videos,
or download content the user has no right to use.

Other download errors:

| Message | Cause | Action |
|---|---|---|
| validation: network/downloads not permitted | project permissions | ask the user to enable |
| format not available | live stream still running, premiere, or region lock | wait for VOD; try another source |
| sections empty / wrong time | section beyond video end, wrong format | use `MM:SS-MM:SS` or `H:MM:SS-H:MM:SS` |

## 5. Long jobs and timeouts

Agents often have per-call time limits. Long commands are **resumable**:

```
rustler analyze transcribe src_A --max-seconds 240
# → {"complete": false, "resume": "rerun the same command"}
rustler analyze transcribe src_A --max-seconds 240      # continues from saved parts
```

- Transcription saves finished chunks (`transcript.parts/`); a rerun skips them.
- Proxy/wav creation is skipped when the output is newer than the source.
- If a render is interrupted, the partial file is deleted; just run `render run` again.
- Prefer section imports (`--sections`) over hour-long full downloads on slow machines.

## 6. Transcript problems

| Symptom | Fix |
|---|---|
| 0 words / empty units | music or silence, or wrong language → `--language`; silent footage → use `--start/--end` from scenes |
| "units missing" on `clip create` | run `analyze transcribe <src>` (or `analyze quick`) first |
| "opening words do not match" | you chose the wrong unit or misquoted; re-read `transcript get` around it |
| Translated instead of transcribed | wrong `--language` (see `hinglish`) |
| Timing slightly off on URL transcripts | YouTube captions are approximate; import the section and transcribe it |

## 7. Media problems

| Symptom | Fix |
|---|---|
| "source has no video" | audio-only file; get the video |
| Wrong orientation | probe uses display dims after rotation; if it still looks wrong, report the file |
| Variable-frame-rate stream → av_sync fail | re-render; trim segment ends 0.1 s |
| Black frames at the start | section began on a fade; move start unit later |

## 8. Windows quirks

- Quote paths with spaces: `rustler source import "C:/Users/me/My Videos/ep 1 (final).mp4"`.
  Forward slashes work everywhere.
- Unicode/Devanagari file names work; if a shell mangles them, pass the path via
  `--input-json @input.json` (file saved as UTF-8).
- **Windows PowerShell 5.1 strips the double quotes inside arguments passed to native programs**, so
  `--layout-params '{"top":{"x":0.7}}'` reaches Rustler as `{top:{x:0.7}}` and fails validation (exit 2,
  "invalid input" / "expected struct ..."). Fix it one of two ways: write the JSON to a file and use
  `--input-json @file.json` (preferred, also for any array or object flag such as `--boxes`,
  `--caption-styles`, `--at-word`, `--colors`), or escape the inner quotes as `\"`:
  `--layout-params '{\"focus_x\":0.4}'`. bash, zsh and cmd.exe are unaffected.
- `detect-webcam` suggests a rect that can include a sliver of gameplay (or overlay border) around the
  facecam. Always run `render preview-frame` and nudge with `layout edit-region <clip> top --nudge
  '{"dw":-0.02,"dh":-0.02}'` (same quoting rules).
- Long paths (> 260 chars) can fail in some tools: keep project dirs short (`C:/clips/ep212`).
- Antivirus can lock freshly downloaded `.exe` tools during `setup`; rerun `setup run`.

## 9. When to stop and ask the user

- Permissions (network, downloads, filesystem roots, cookies) are needed.
- The same error repeats after one sensible fix.
- The source is unusable (no rights, blocked, corrupt) and there's no alternative in the brief.
- A brief requirement can't be met (e.g. min length 60 s but no moment is that long).
