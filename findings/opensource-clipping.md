# Findings: opensource-clipping

Repo: `repos/opensource-clipping` (NaufalRizqullah/opensource-clipping, version tags up to v1.15.7 at repos/opensource-clipping/CHANGELOG.md:11). Single squashed git commit (ac29455). Reviewed against `docs/research-methodology.md`. Citation form: `repos/opensource-clipping/<path>:<line>`. Anything not read in source is marked **UNVERIFIED**.

---

## 1. Summary

A Python CLI plus FastAPI/React GUI that turns a long YouTube/TikTok/IG/GDrive video into N vertical clips. Pipeline: yt-dlp download -> faster-whisper word timestamps -> ONE Gemini (or NVIDIA NIM) call over the whole transcript returns clip times, hook, typography plan, B-roll queries, BGM mood, keep-segments, and platform metadata -> OpenCV frame loop face-tracks and crops to 9:16 and pipes raw frames to FFmpeg -> libass `.ass` karaoke subtitles burned in -> sidechain-ducked BGM -> `.ts` concat.

Verdict: a good **idea/prompt/heuristic mine** (the single mega-prompt with a strict JSON schema, the ASS word-animation generator, the sidechaincompress BGM filter, the dead-zone face-camera, the story-recipe EDL). Poor as code to port: global-state Python, shell-out-heavy, temp files in CWD, no agent interface (CLI flags + REST only), no QC, no real LLM abstraction, no SFX, no loudness or denoise. Quality is hobby/creator-tool grade, with real engineering effort in the Kaggle/Studio backend and logging.

---

## 2. Stack and architecture

- Python 3.10+ (repos/opensource-clipping/pyproject.toml:6). Deps: yt-dlp, faster-whisper, google-genai, mediapipe, opencv-python, Pillow, ultralytics (YOLO), pyannote.audio, torch, fastapi, edge-tts (repos/opensource-clipping/requirements.txt:1-21).
- `openai` is imported by the NVIDIA path (repos/opensource-clipping/clipping/engine.py:931) but is **not** in `requirements.txt` or `pyproject.toml` deps; it appears only in `requirements-kaggle.txt:9`. A fresh `pip install -r requirements.txt` breaks the NVIDIA provider (verified by absence).
- FFmpeg is a **subprocess** everywhere; OpenCV decodes frames in Python and pipes raw BGR to an FFmpeg process (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:225-268).
- Web: FastAPI job server (repos/opensource-clipping/web/api/app.py:55-60), in-memory job store with JSON persistence (repos/opensource-clipping/web/api/store.py:1-8,30), single worker thread (`MAX_CONCURRENT_JOBS = 1`, repos/opensource-clipping/web/api/worker.py:28,34), React 19 + Vite dashboard (repos/opensource-clipping/web/dashboard/package.json), plus a static GitHub-Pages copy `docs/studio/*.html` that connects to a remote backend exposed via ngrok on Kaggle/Colab (repos/opensource-clipping/notebooks/kaggle-studio-server.ipynb cells 8-9).

```
CLI main.py / REST POST /api/jobs
        |
  clipping/runner.py:run_pipeline
   1 engine.download_video (yt-dlp/gdown)
   2 engine.transcribe_video (faster-whisper)  | or YouTube json3 subs
   3 engine.analyze_with_ai (Gemini | NVIDIA)  -> gemini_response.json
   4 metadata.normalize_and_validate
   5 diarization (pyannote) [only split/camera-switch]
   6 studio.proses_klip per clip:
        hook render -> main render (face-crop frames -> ffmpeg pipe)
        -> ASS burn (+BGM) -> VO intro -> concat -> edge glow -> thumbnail
   7 render_manifest.json
```
Orchestrator: repos/opensource-clipping/clipping/runner.py:15-267. Per-clip renderer: repos/opensource-clipping/clipping/studio/core.py:108-1142 (one ~1000-line function).

---

## 3. Repo map (important only)

| Path | Role |
|---|---|
| `main.py` | CLI entry, batch over URLs |
| `clipping/config.py` (971 lines) | defaults, argparse (~100 flags), builds a `SimpleNamespace` cfg (repos/opensource-clipping/clipping/config.py:234-805) |
| `clipping/engine.py` (1332) | download, Whisper, the analysis prompt, Gemini + NVIDIA calls |
| `clipping/runner.py` | pipeline orchestration |
| `clipping/studio/core.py` | `proses_klip` per-clip render |
| `clipping/studio/render_hybrid.py` | face-tracked crop + B-roll frame loop |
| `clipping/studio/render_split_screen.py` (1193), `render_camera_switch.py` (770) | podcast layouts |
| `clipping/studio/subtitles.py` | ASS generator (karaoke + kinetic typography) |
| `clipping/studio/audio_bgm.py` | BGM picker + ducking filter |
| `clipping/studio/ffmpeg_utils.py` | encoder detection, progress parser |
| `clipping/studio/transitions.py`, `effects.py`, `v2_helpers.py`, `edge_glow.py` | glitch, flash, glow, unused transition pool |
| `clipping/voiceover.py` | Gemini commentary + edge-tts |
| `clipping/story/*`, `story_recipe.json`, `sources.json` | multi-source recipe (EDL) mode |
| `youtube_uploader/`, `facebook_uploader/`, `youtube_tracker/` | publishing; out of scope for us |
| `web/api/*`, `web/dashboard/`, `docs/studio/` | job server + GUI |
| `notebooks/*.ipynb` | 3 Colab/Kaggle notebooks (thin wrappers) |
| `tests/` (15 py + 1 cjs) | mostly API/logging/bootstrap tests; zero tests of clip selection, captions or FFmpeg graphs |

Large assets: `.git` 75 MB; `assets/bgm/` 15 mp3 ~76 MB total (list in section 11); `assets/images/` logos < 140 KB each. No fonts, no models, no SFX bundled (fonts/models are downloaded at runtime).

---

## 4. Capabilities by category

### A. AI clipping (quality 3)

- One LLM call per video over the **full transcript** with `[start - end] text` lines (repos/opensource-clipping/clipping/engine.py:427, 1135-1148). Long videos hit context/timeout limits: `REQUEST_TIMEOUT_MS = 15 min` (repos/opensource-clipping/clipping/engine.py:494); no chunking/map-reduce (UNVERIFIED beyond this file; grep found no chunking).
- Output is a ranked JSON array: `viral_score` 1-100, `start_time`, `end_time`, `hook_start/end_time`, `bgm_mood`, `typography_plan`, `broll_list`, `hook_v2`, `keep_segments`, metadata, account classification (JSON skeleton in the prompt at repos/opensource-clipping/clipping/engine.py:873-925; Gemini schema at :1164-1330).
- Scoring is **prompt-only** ("internal" 5x20 rubric, repos/opensource-clipping/clipping/engine.py:681-694); the model is told not to emit the sub-scores, so no inspectable breakdown.
- Duration limits 20-179 s live **only in the prompt** (repos/opensource-clipping/clipping/engine.py:593-594, 646). No code validates duration, clamps times to the transcript, de-overlaps clips, or snaps boundaries to word/sentence edges. `metadata.normalize_and_validate` only normalizes aliases/strings (repos/opensource-clipping/clipping/metadata.py:104-215, timing at :130-133).
- Good prompt ideas: hook inside the clip, shift start to strongest sentence, "slow closing" padding of +0.10..0.85 s after the last word (repos/opensource-clipping/clipping/engine.py:751-752), anti-duplicate clips, "standalone clarity".
- Weakness: prompts are Indonesian, with Indonesia-specific account classification baked in (`TARGET_ACCOUNTS` of 4 hard-coded accounts, repos/opensource-clipping/clipping/engine.py:460-481). Must be stripped/generalized.
- Hook V2 / Segment trimming: LLM returns 2-6 micro-hooks (0.5-2 s) and `keep_segments` (cut boring parts). `--silence-trim` is only an extra prompt sentence ("AGRESIF buang ... silence"), not audio silence detection (repos/opensource-clipping/clipping/engine.py:621-629; grep for `silencedetect`/`silenceremove` returns nothing).

### B. Transcription and subtitles (quality 3)

- faster-whisper `large-v3`, cuda/float16 defaults, `beam_size=5, word_timestamps=True`, no `vad_filter`, no language param, no initial prompt (repos/opensource-clipping/clipping/engine.py:361-407; defaults repos/opensource-clipping/clipping/config.py:148-150).
- Words are chunked into groups of `max_words_per_subtitle` (default 5) per Whisper segment (repos/opensource-clipping/clipping/engine.py:429-452).
- Alternative: parse YouTube `json3` auto-captions (`--use-dlp-subs`) to skip Whisper (repos/opensource-clipping/clipping/engine.py:250-358; branch at repos/opensource-clipping/clipping/runner.py:55-77). Cleans `[Music]`, `>>` markers, HTML tags.
- Whisper model can be injected and reused across batch jobs (repos/opensource-clipping/clipping/engine.py:361-390). Only the local faster-whisper engine; no OpenAI-compatible STT endpoint.
- Speaker diarization is a separate pyannote 3.1 pass used only for layout; speaker labels are **not** attached to words or captions (repos/opensource-clipping/clipping/diarization.py:73-190).
- Emoji in captions: absent (grep found none; UNVERIFIED beyond grep).

### C. Creative design (quality 3 for captions, 2 for everything else)

Captions are generated as **libass ASS files** and burned with `subtitles=...:fontsdir=...` (repos/opensource-clipping/clipping/studio/core.py:694; escaping helper repos/opensource-clipping/clipping/studio/helpers.py:23-33).

- Three modes in `buat_file_ass` (repos/opensource-clipping/clipping/studio/subtitles.py:55-389):
  1. **Plain** (`use_advanced_text` false, the default per repos/opensource-clipping/clipping/config.py:40): per-word Dialogue events; karaoke = current word yellow `&H00FFFF&` (repos/opensource-clipping/clipping/studio/subtitles.py:185-197); non-karaoke = "typewriter reveal" via `\alpha&HFF&` on future words.
  2. **Advanced/kinetic**: PIL `ImageFont.truetype(...).getlength` measures each word (repos/opensource-clipping/clipping/studio/subtitles.py:218-224, 253-263), words wrapped manually to `PlayResX - 2*margin` with `TIGHTNESS = 0.95` (repos/opensource-clipping/clipping/studio/subtitles.py:231), every word becomes its **own `\pos`-positioned Dialogue event** spanning the whole segment (repos/opensource-clipping/clipping/studio/subtitles.py:373-379). Words from the LLM's `typography_plan` get a bigger scale, the second font, and animation.
  3. `rasio == "split"`: vertically centered stack (repos/opensource-clipping/clipping/studio/subtitles.py:136-139,296-297).
- Animation tags actually emitted (repos/opensource-clipping/clipping/studio/subtitles.py:358-383):
  - bounce_pop: `\alpha&HFF&\fscx{0.7*S}\fscy{0.7*S}\t(t0,t0,\alpha&H00&)\t(t0,t0+80,\fscx{1.15*S}...)\t(t0+80,t0+150,\fscx{S}...)`
  - stagger_up: `\move(x,y+30,x,y,t0,t0+150)` plus alpha reveal
  - karaoke+advanced: `\t(start,start,\c&H00FFFF&)\t(end,end,\c&HFFFFFF&)`
- Scale levels: 1 -> 110%, 2 -> avg(base,100), 3 -> `scale_kata_khusus` (= font + 120) (repos/opensource-clipping/clipping/studio/subtitles.py:120-126; repos/opensource-clipping/clipping/config.py:115,121).
- Font presets (4 pairs: main + "khusus" emphasis font): DEFAULT Montserrat Black/Medium, STORYTELLER Inter/Lora, HORMOZI Montserrat/Anton (default style per repos/opensource-clipping/clipping/config.py:50), CINEMATIC Roboto/Bebas Neue (repos/opensource-clipping/clipping/config.py:52-109). Fonts are **downloaded at runtime** from raw.githubusercontent.com / jsDelivr fontsource with a Firefox User-Agent and retry loop (repos/opensource-clipping/clipping/studio/typography.py:46-144), then copied into `~/.local/share/fonts` + `fc-cache` on Linux (repos/opensource-clipping/clipping/studio/typography.py:147-187). Resolution is `Fontname` by family name, with `\b` bold flag.
- Font size scaling: baseline 1920 high for vertical (repos/opensource-clipping/clipping/studio/subtitles.py:130-134), `ASS_FONT_916 = 90`, margin-v 450 (repos/opensource-clipping/clipping/config.py:112-115).
- Dead caption-follow: `get_x_func` moves captions with the camera only when `cfg.dev_mode` (repos/opensource-clipping/clipping/studio/subtitles.py:320).
- Visual effects: TV-glitch hook transition (downloaded YouTube clip, fallback `noise=alls=100:allf=t+u,rgbashift=rh=20:bv=20`, repos/opensource-clipping/clipping/studio/effects.py:50-154), white flash (`color=c=white`) and RGB-shift glitch lavfi clips for Hook V2 (repos/opensource-clipping/clipping/studio/v2_helpers.py:12-58), "edge glow" ambient animated gradient (numpy/OpenCV generated MP4 blended with `blend=all_mode=screen`, repos/opensource-clipping/clipping/studio/edge_glow.py:23-173; repos/opensource-clipping/clipping/studio/core.py:1082), B-roll alpha crossfade of 0.3 s and 10% Ken-Burns zoom (repos/opensource-clipping/clipping/studio/render_hybrid.py:338-339, 453-480), watermark engine (text/image, 9 anchors, opacity; repos/opensource-clipping/clipping/studio/watermark.py:442), thumbnail = frame at 5 s + 50% black overlay + wrapped Montserrat-Black text (repos/opensource-clipping/clipping/studio/thumbnail.py:46-107, frame time at :73).
- **Dead code**: `TMP_TRANSITION_POOL` (8 YouTube "film burn" overlays) plus `get_random_transition`/`prepare_transition_clip` are defined and re-exported but never called (grep shows only re-export lines at repos/opensource-clipping/clipping/studio/core.py:66-68). `JITTER_THRESHOLD` is computed but unused (repos/opensource-clipping/clipping/studio/render_hybrid.py:109, only occurrence).
- No gradients on text, no per-template JSON, no generic transition set (xfade) outside story mode, no emoji, no stickers/progress bar.

### D. Video processing and rendering (quality 2-3)

- Encoder auto-detection: tests `h264_nvenc` (two presets), `h264_amf`, `h264_vaapi`, falls back to libx264 by actually running a 1 s synthetic encode (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:43-71, 82-174). Auto bitrate by height 4M/8M/12M/20M (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:74-79). Defaults CQ 23 / CRF 20 (repos/opensource-clipping/clipping/config.py:151-152). The bitrate string parser breaks on non-"M" values (`float(target_bitrate.replace('M',''))`, repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:105-110).
- Intermediate format is **MPEG-TS**, joined with the concat protocol and `-c copy` (repos/opensource-clipping/clipping/studio/core.py:1012-1025, h264_mp4toannexb at :349). Progress is parsed from `-progress pipe:2` `out_time_ms` (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:272-345).
- Every clip re-encodes **3-4 times** (silent crop -> subtitle/BGM pass -> optional VO intro -> glow pass), with the video going OpenCV decode -> raw pipe -> x264 -> TS -> x264 again. Hundreds of temp files in CWD named by rank, cleaned in `finally` (repos/opensource-clipping/clipping/studio/core.py:1132-1142). Parallel renders of two jobs in one CWD would collide (names are `h_{rank}.ts`, `m_{rank}.ts`, repos/opensource-clipping/clipping/studio/core.py:214-217).
- Renders video-only silent MP4 through the frame loop, then muxes audio from the original source with `-ss/-to` placed on the **second input** (repos/opensource-clipping/clipping/studio/core.py:703-705). Risk of A/V drift on VFR sources: UNVERIFIED (not tested).
- Output dims: `_get_render_dims` makes `render_height` the short side for vertical ratios; even-dimension rounding; ratios 9:16, 16:9, 1:1, 3:4, 4:5 (repos/opensource-clipping/clipping/studio/utils.py:121-170).
- A "dev mode" renders a director's-console view (dimmed full frame + bright crop rect + face box + HUD text) for debugging the camera (repos/opensource-clipping/clipping/studio/render_hybrid.py:394-470). Useful as a QA-overlay concept.
- No preview render, no proxy, no batch of FFmpeg graph (everything is imperative commands), no thumbnails of candidate frames.

### E. Reframing (quality 3)

Hybrid (single-speaker) mode (repos/opensource-clipping/clipping/studio/render_hybrid.py:54-533):
- Phase 1: sample a face every `track_step=0.25 s` with seeks (`cap.set(CAP_PROP_POS_MSEC)`, one decode per sample), choose the **largest** face via MediaPipe BlazeFace short-range (min conf 0.5) or YOLOv8-face (repos/opensource-clipping/clipping/studio/render_hybrid.py:97,176-232; detector repos/opensource-clipping/clipping/studio/face_detection.py:48-101).
- Phase 2: camera filter. Start at median of first 5 samples; if |face - cam| > snap_px -> **hard cut** to the face; else outside a dead-zone of `0.15 * crop_w` move a fraction `0.30` of the overshoot per sample (repos/opensource-clipping/clipping/studio/render_hybrid.py:234-264). Vertical eased the same way.
- Quirk: the `--track-snap` default 0.25 is silently overridden: `temp_snap = SNAP_THRESHOLD if SNAP_THRESHOLD < 0.1 else 0.08` (repos/opensource-clipping/clipping/studio/render_hybrid.py:246-247), so the effective snap is 8% of source width unless the user passes < 0.1.
- Phase 3: render each frame: crop `crop_w = height*9/16` centred on interpolated `cx` (linear interpolation between 0.25 s samples, repos/opensource-clipping/clipping/studio/render_hybrid.py:266-282), resize (OpenCV interp from `video_scale_algo`), optional B-roll overlay, watermark, write raw BGR to FFmpeg (repos/opensource-clipping/clipping/studio/render_hybrid.py:340-505).
- Landscape 16:9 output letterboxes with black bars (repos/opensource-clipping/clipping/studio/render_hybrid.py:366-395). Square/4:5 ratios may use `--static-crop` (center) (repos/opensource-clipping/clipping/studio/render_hybrid.py:166).
- No tracking IDs, no active-speaker selection in single mode (just the biggest face), no body/object tracking, no scene-cut awareness in hybrid mode (only snap threshold).
- Podcast modes (diarization-driven): `--split-screen` (top/bottom panels, `np.vstack`, repos/opensource-clipping/clipping/studio/render_split_screen.py:973) and `--camera-switch` (cuts to the active speaker, optional blended pillarbox). Speaker->face mapping is heuristic: faces sorted left->right, speaker canonical X from the median of frames where a single face + single speaker exist, then nearest-canonical assignment (repos/opensource-clipping/clipping/studio/render_split_screen.py:211-318,367-470; repos/opensource-clipping/clipping/studio/render_camera_switch.py:152-340). Duplicate-box merge via IoU 0.2 (repos/opensource-clipping/clipping/studio/render_split_screen.py:244-258), layout majority-vote window 12 frames and scene-cut threshold 18 (repos/opensource-clipping/clipping/studio/render_split_screen.py:742-749). Needs `HF_TOKEN` and accepting gated pyannote terms (repos/opensource-clipping/.env.sample:13-16). Speaker count can be estimated by sampling 20 frames for max faces (repos/opensource-clipping/clipping/studio/face_detection.py:104-190). Real-world robustness: UNVERIFIED (not run); the logic is the most complicated code in the repo (about 2000 lines) and is stateful and closure-heavy.

### F. Audio (quality 2)

- BGM only: local `assets/bgm/<mood>/*.mp3`, **random pick** (repos/opensource-clipping/clipping/studio/audio_bgm.py:11-33), mood from the LLM's enum `chill|epic|sad|upbeat|suspense` (repos/opensource-clipping/clipping/engine.py:746-749), fallback to `chill`, else no BGM (repos/opensource-clipping/clipping/studio/core.py:574-583). No beat/energy matching; the mp3s are looped with `-stream_loop -1` and always start at 0:00, no fade in/out, no trim to a good section.
- Ducking filter (the key reusable bit), verbatim from repos/opensource-clipping/clipping/studio/audio_bgm.py:36-67:
  ```
  [1:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,volume=1.2[voc];
  [2:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,volume={0.25}[bgm];
  [voc]asplit=2[voc_sc][voc_mix];
  [bgm][voc_sc]sidechaincompress=threshold=0.08:ratio=5.0:attack=100:release=1000[bgm_ducked];
  [voc_mix][bgm_ducked]amix=inputs=2:duration=first[a_out]
  ```
  `background` mode is the same minus the sidechain: `[voc][bgm]amix=inputs=2:duration=first[a_out]`. Defaults: `BGM_BASE_VOLUME = 0.25`, mode `ducking` (repos/opensource-clipping/clipping/config.py:140-142). `amix` without `normalize=0` will halve levels in newer FFmpeg: UNVERIFIED which FFmpeg version the author targeted.
- Applied to the **main** part only; the hook segment keeps original audio (BGM added after concat only to main, repos/opensource-clipping/clipping/studio/core.py:588-600,718-727).
- **No SFX at all** (grep for `sfx|whoosh|sound effect` finds one YouTube video label, repos/opensource-clipping/clipping/studio/transitions.py:95). No loudness normalization (no `loudnorm`), no denoise, no real silence removal, no limiter. Audio codec is plain AAC 48 kHz stereo (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:187-196).
- TTS voice-over: Gemini writes a commentary script (4 styles x 3 lengths prompt, repos/opensource-clipping/clipping/voiceover.py:167-218), edge-tts synthesizes mp3 and yields WordBoundary cues grouped 3 words/segment for captions (repos/opensource-clipping/clipping/voiceover.py:74-130). The VO intro is a freeze frame + dimmed + animated edge glow + `showwaves` audio spectrum, then the main clip follows (repos/opensource-clipping/clipping/studio/core.py:745-960). VO volume/orig volume flags exist (repos/opensource-clipping/clipping/config.py:694-705).

### G. Agent integration (quality 1-2)

- No MCP server, no skill file, no tool schemas, no structured stdout. Closest things: (a) the CLI (`main.py`, ~100 flags, repos/opensource-clipping/clipping/config.py:234-805); (b) REST jobs API with Pydantic models (repos/opensource-clipping/web/api/models.py:18-234, routes at repos/opensource-clipping/web/api/routes/jobs.py:65-176); (c) file artifacts: `gemini_response.json`, `metadata_preview.json`, `render_manifest.json` (repos/opensource-clipping/clipping/runner.py:85-107,258-264); (d) `--load-gemini-json` to replay the LLM output (repos/opensource-clipping/clipping/runner.py:85-91).
- Idea worth stealing: the pipeline already has a **plan artifact** (LLM JSON) that can be re-fed, edited, and re-rendered without redoing transcription/LLM calls. That is exactly the agent hand-off point we want, but it's a flat dict with no versioning.
- `AGENTS.md` only holds contributor workflow rules and references `docs/architecture.md` and `docs/modules.md`, which do not exist in the repo (repos/opensource-clipping/AGENTS.md:5-9; listing in section 3 shows none).
- Story mode: a declarative JSON "recipe" (clips -> hook/highlight -> scenes `{source_id,start,end,label}` + `transition`) that an agent can fully author (repos/opensource-clipping/story_recipe.json:1-45; docs repos/opensource-clipping/docs/STORY_CLIP.md:60-100). It is the only edit-decision-list in the repo.

### H. GUI (quality 3 for a hobby tool)

- React dashboard (Dashboard / NewJob / JobDetail / Settings; ~850 lines, repos/opensource-clipping/web/dashboard/src/pages/*.jsx) and a static clone at `docs/studio` that talks to the Kaggle ngrok backend. Features: job form mapping to CLI flags (repos/opensource-clipping/web/api/models.py:68-110), progress statuses `queued|downloading|transcribing|analyzing|rendering|completed|failed|cancelled` (repos/opensource-clipping/web/api/models.py:18-26), settings (keys), file download. Upload endpoint with filename validation (repos/opensource-clipping/web/api/models.py:111-123).
- No timeline, no subtitle editor, no template/asset browser, no in-browser preview of caption styles, no clip trimming UI. It is a job launcher, not an editor.
- Readiness checks and structured JSONL logging with correlation IDs are well done (repos/opensource-clipping/web/api/readiness.py, repos/opensource-clipping/scripts/studio_logging.py; tests under tests/test_readiness.py, tests/test_studio_logging.py).

### I. Provider abstraction (quality 2)

- Two providers dispatched by `analyze_with_ai`: `gemini` (google-genai SDK, `response_schema`, JSON mime type, `v1beta`, 10-attempt retry with 60 s + 30 s*n back-off, then a fallback model) and `nvidia` (the `openai` SDK pointed at `https://integrate.api.nvidia.com/v1` with `extra_body.nvext.guided_json`, `chat_template_kwargs.thinking=False`) (repos/opensource-clipping/clipping/engine.py:527-590, 929-1148, 1151-1330). On NVIDIA failure it silently falls back to Gemini (repos/opensource-clipping/clipping/engine.py:1135-1148).
- `base_url` is **hard-coded**; there is no `--base-url`, no generic "OpenAI-compatible" provider, no model-capability probing (grep: the only `base_url=` in the pipeline is repos/opensource-clipping/clipping/engine.py:939). Defaults: `gemini-3-flash-preview` primary, `gemini-2.5-flash` fallback, NVIDIA `deepseek-ai/deepseek-v4-pro` (repos/opensource-clipping/clipping/config.py:160-162).
- Cross-provider hygiene: `metadata.py` contains alias maps for non-Gemini output quirks (`peringkat`, `timing_klip_start`, DeepSeek `hook` string, repos/opensource-clipping/clipping/metadata.py:125-146), which shows real-world messy JSON from open models. Good to learn from.
- Voice-over uses Gemini unconditionally (repos/opensource-clipping/clipping/voiceover.py:221-266); STT is local-only; TTS = edge-tts (unofficial Microsoft Edge online voices; ToS risk; UNVERIFIED legal position, see section 11).

---

## 5. Pipeline walkthrough

1. `main.py` builds cfg (`clipping/config.py:build_config`) and loops URLs (repos/opensource-clipping/main.py:109-277). Output isolation per URL slug (repos/opensource-clipping/clipping/config.py:171-194).
2. `engine.download_video` uses yt-dlp with a format selector that excludes AV1 (`[vcodec!*=av01]`, repos/opensource-clipping/clipping/engine.py:20-57), optional cookies (browser or file, repos/opensource-clipping/clipping/engine.py:124-132), `gdown` for Drive (repos/opensource-clipping/clipping/engine.py:72-87).
3. Transcribe (json3 shortcut for YouTube), then `analyze_with_ai`; raw LLM JSON saved (repos/opensource-clipping/clipping/runner.py:85-107).
4. Normalize metadata and print preview (repos/opensource-clipping/clipping/runner.py:110-115).
5. Optional diarization: extract 16 kHz mono WAV with FFmpeg (repos/opensource-clipping/clipping/diarization.py:40-62), pyannote 3.1 (repos/opensource-clipping/clipping/diarization.py:119), merge same-speaker gaps < 0.5 s (repos/opensource-clipping/clipping/diarization.py:188,199-217).
6. Choose encoder; build glitch asset (repos/opensource-clipping/clipping/runner.py:151-174).
7. Optional per-clip VO script + TTS (repos/opensource-clipping/clipping/runner.py:186-231).
8. `proses_klip` per clip, ordered by rank (repos/opensource-clipping/clipping/runner.py:232-256): hook render (v1 teaser or Hook V2 micro-cuts joined by white-flash/glitch clips, repos/opensource-clipping/clipping/studio/core.py:268-359), main render (single pass or per `keep_segment` with hard-cut `concat`, repos/opensource-clipping/clipping/studio/core.py:472-548), ASS burn + BGM (repos/opensource-clipping/clipping/studio/core.py:690-730), VO intro (repos/opensource-clipping/clipping/studio/core.py:745-960), final TS concat order = hook, glitch, VO intro, main (repos/opensource-clipping/clipping/studio/core.py:996-1025), edge glow overlay pass (repos/opensource-clipping/clipping/studio/core.py:1031-1100), thumbnail (repos/opensource-clipping/clipping/studio/core.py:1103-1106).
9. `render_manifest.json` with status per clip (repos/opensource-clipping/clipping/studio/core.py:177-210).

Notable pipeline decisions: the hook is a duplicate of a hook sentence pulled from inside the clip (so it plays twice: teaser then in context) - a retention trick from the prompt (repos/opensource-clipping/clipping/engine.py:706-714, "HOOK (WAJIB)").

---

## 6. Prompts and heuristics (quoted)

Main analysis prompt: `get_analysis_prompt` (repos/opensource-clipping/clipping/engine.py:596-926). Shared by both providers. Key blocks, translated/condensed from Indonesian:

- Persona (:634): "Kamu adalah Art Director, Editor Video, dan Strategist Metadata Short-Form Content untuk TikTok, Reels, dan YouTube Shorts." ("You are an Art Director, Video Editor and short-form Metadata Strategist.")
- Task: find `{jumlah_clip}` most compelling, shareable, viral moments; order by viral_score; clip length `{MIN_CLIP_DURATION}-{MAX_CLIP_DURATION}` s (20-179).
- Retention rules (:656-680): first 3 s must grab; ideal structure `hook -> context singkat -> tension/insight -> payoff`; "if the start is slow, shift start_time to a stronger sentence"; avoid intros/filler; clip must stand alone; goal list: stop scroll, watch to end, comment, share, save, "ini gue banget".
- Timing rules: start at the first strong moment; end after the payoff/punchline; don't cut mid-sentence; merge two close strong moments only if still within length; different angles = separate clips.
- Internal scoring (:681-694): hook 1-20 + emotion 1-20 + shareability 1-20 + standalone clarity 1-20 + payoff 1-20 = 1-100; bands 90-100 fyp-worthy, 80-89 good, 70-79 standard; "do not pick below 70 unless few good moments"; do **not** add sub-score fields to JSON.
- Hook: "Ambil 1 kalimat paling punchy yang ADA DI DALAM klip", about `durasi_hook` s (default 3 s).
- Typography plan (:715-745): 3-6 single words, spelled exactly as in the transcript, with `scale_level` 1/2/3, `style` utama|khusus, `animasi` bounce_pop|stagger_up.
- B-roll (:728-745): 1-3 moments of 3-7 s, `search_query` short English, never at the hook second, empty array if not helpful. Plus 2-5 "visual B-roll hook" ideas for manual editors.
- BGM mood: one of `[chill, epic, sad, upbeat, suspense]` (:746-749).
- Slow closing: end_time padded +0.10..+0.85 s after the last word (:751-752).
- Metadata block (:782-871): Indonesian title (<=100 chars), English title (<=100), 2-3 hashtags (avoid #fyp), 1-sentence hook and context, 5-8 keyword tags, TikTok EN caption (<=140 chars) and ID title/caption, plus account classification with confidence and alternative account.
- Optional blocks injected by flags: Hook V2 `{_hook_v2_items}` micro-hooks 0.5-2 s with on-screen text of 2-5 words and `transition.type` white_flash|glitch (:605-621); segment trimming `keep_segments` chronological non-overlapping (:622-632).

System message for NVIDIA: "You are a professional video editor and strategist. Return JSON only. Follow the provided JSON schema exactly." with `temperature=0.5, top_p=1, max_tokens=16384` (repos/opensource-clipping/clipping/engine.py:1089-1102). Gemini uses `response_mime_type=application/json` + a `response_schema` mirroring the same fields; note the Gemini schema does **not** enforce enums (e.g. `bgm_mood` free STRING) while the NVIDIA schema does (repos/opensource-clipping/clipping/engine.py:1242-1330 vs :929-1090). Downstream code compensates: unknown mood -> `chill` (repos/opensource-clipping/clipping/studio/core.py:566-568).

Voice-over prompt: narrator persona, no greetings, no repeating the transcript, only the script text; styles analysis|reaction|lesson|summary; lengths 1-2 / 3-5 / 5-7 sentences (repos/opensource-clipping/clipping/voiceover.py:167-218).

Numeric heuristics worth recording:

| Heuristic | Value | Source |
|---|---|---|
| clip duration (prompt) | 20-179 s | repos/opensource-clipping/clipping/engine.py:593-594 |
| default clips | 7 | repos/opensource-clipping/clipping/config.py:28 |
| hook duration | 3 s | repos/opensource-clipping/clipping/config.py:33 |
| face sample step | 0.25 s | repos/opensource-clipping/clipping/studio/render_hybrid.py:97 |
| camera dead-zone / smoothing / snap | 15% of crop / 0.30 / 8% of source width | repos/opensource-clipping/clipping/studio/render_hybrid.py:100-247 |
| BGM volume / duck | 0.25; threshold 0.08, ratio 5, attack 100 ms, release 1000 ms | repos/opensource-clipping/clipping/studio/audio_bgm.py:65 |
| flash transition | 0.12 s | repos/opensource-clipping/clipping/studio/v2_helpers.py:12-16 |
| B-roll fade / zoom | 0.3 s / 1.10 | repos/opensource-clipping/clipping/studio/render_hybrid.py:338-339 |
| diarization merge gap | 0.5 s | repos/opensource-clipping/clipping/diarization.py:188 |
| ASS word pop timing | in 80 ms, settle 150 ms | repos/opensource-clipping/clipping/studio/subtitles.py:340-342 |
| retry back-off | 60 s + 30 s*(n-1), 10 attempts, codes 408/429/5xx | repos/opensource-clipping/clipping/engine.py:492-497,556 |
| upload safety | 1 per run, 2 per day, 24 h interval | repos/opensource-clipping/upload_safety.json:3-6 |

---

## 7. FFmpeg commands/filtergraphs actually used

All via `subprocess` with list args (no shell). Standard encode tail `std_p` = encoder args + `-pix_fmt yuv420p -r 30 -c:a aac -ar 48000 -ac 2 -f mpegts` (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:177-201). NVENC example (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:101-108): `-c:v h264_nvenc -preset p1 -rc vbr -cq 25 -b:v 8M -maxrate 12M -bufsize 16M`.

1. **Raw frame pipe writer** (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:225-268):
   `ffmpeg -hide_banner -loglevel error -y -f rawvideo -pix_fmt bgr24 -s WxH -r FPS -i - <encoder args> -pix_fmt yuv420p -r FPS -movflags +faststart -an out.mp4`
2. **Main clip: burn ASS + optional sharpen + BGM ducking in one graph** (repos/opensource-clipping/clipping/studio/core.py:694-727, 702-707):
   ```
   ffmpeg -hide_banner -loglevel verbose -y -i m_silent.mp4 -ss S -to E -i SOURCE [-stream_loop -1 -i BGM.mp3]
     -filter_complex "[0:v]subtitles=<ass>:fontsdir=<dir>,unsharp=5:5:0.5:5:5:0.0[v_out]; <BGM filter above>"
     -map [v_out] -map [a_out] -shortest <std_p> -progress pipe:2 -nostats m.ts
   ```
   No BGM: `-map 0:v:0 -map 1:a:0 -vf "subtitles=...,unsharp=..."`.
3. **Per keep-segment** (repos/opensource-clipping/clipping/studio/core.py:508-536): same sans BGM, `-map 0:v:0 -map 1:a:0`; join with `-i "concat:a.ts|b.ts" -c copy m.ts` (:547-550); BGM pass afterwards on the joined TS (:585-600). No crossfade or audio fade at segment joins, so cuts can click.
4. **Hook V2 transitions**: `ffmpeg -y -f lavfi -i color=c=white:s=WxH:d=0.12:r=30 -f lavfi -i anullsrc=r=48000:cl=stereo -t 0.12 -c:v libx264 -pix_fmt yuv420p -c:a aac -b:a 128k out.mp4`; glitch variant adds `-vf "noise=alls=100:allf=t+u,rgbashift=rh=20:bv=20"` on a black `color` source (repos/opensource-clipping/clipping/studio/v2_helpers.py:12-58). Converted with `-c copy -bsf:v h264_mp4toannexb -f mpegts` (repos/opensource-clipping/clipping/studio/core.py:345-352).
5. **Final concat**: `ffmpeg -y -i "concat:hook.ts|glitch.ts|vo.ts|main.ts" -c copy -bsf:a aac_adtstoasc final.mp4` (repos/opensource-clipping/clipping/studio/core.py:1012-1025). Output has no `+faststart`.
6. **Voice-over intro graph** (repos/opensource-clipping/clipping/studio/core.py:905-960):
   ```
   [0:v]scale=W:H:force_original_aspect_ratio=increase,crop=W:H,colorchannelmixer=rr=0.3:gg=0.3:bb=0.3[v_bg];
   [2:v]scale=W:H[glow_scaled];
   [v_bg][glow_scaled]blend=all_mode=screen:shortest=1[v_glowed];
   [1:a]asplit=2[vo_a][vo_wave_in];
   [vo_wave_in]lowpass=f=300,showwaves=s=800x260:mode=cline:colors=0x00FFFF:rate=30:scale=sqrt,
     format=rgba,colorkey=0x000000:0.1:0.1,colorchannelmixer=aa=0.65[wave_v];
   [v_glowed][wave_v]overlay=(W-w)/2:(H-h)/2:shortest=1[v_over];
   [v_over]subtitles=<ass>:fontsdir=<dir>[v_out];
   [3:a]volume=0.25[bgm_vol]; [vo_a]volume=1.0[vo_loud];
   [bgm_vol][vo_loud]amix=inputs=2:duration=first:dropout_transition=2[a_out]
   ```
   inputs: `-loop 1 -framerate 30 -i frame.jpg`, VO mp3, glow mp4 (looped), BGM. VO duration = ffprobe(mp3) + 0.5 s (repos/opensource-clipping/clipping/studio/core.py:766-774). Note: this BGM branch does **not** duck (plain amix).
7. **Edge-glow overlay**: `-filter_complex "[1:v]scale=W:H[glow]; [0:v][glow]blend=all_mode=screen:shortest=1[v_out]" -map [v_out] -map 0:a? -c:a copy -t DUR` (repos/opensource-clipping/clipping/studio/core.py:1082-1087).
8. **Glitch asset**: `-ss 0.2 -t 1 -i glitch_raw.mp4 -vf "crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=W:H:flags=lanczos,setsar=1"` (repos/opensource-clipping/clipping/studio/effects.py:100-130).
9. **Story mode**: trim `-ss S -i in -t D -c copy -avoid_negative_ts make_zero` (stream copy; imprecise at GOP boundaries, docstring admits it, repos/opensource-clipping/clipping/story/assembler.py:35-88); normalize `scale=W:H:force_original_aspect_ratio=decrease,pad=W:H:(ow-iw)/2:(oh-ih)/2:color=black,fps=30,format=yuv420p` with libx264 crf 18, aac 192k (repos/opensource-clipping/clipping/story/assembler.py:96-142); xfade fallback `[0:v][1:v]xfade=transition=fade:duration=0.5:offset=0[v];[0:a][1:a]acrossfade=d=0.5[a]` (repos/opensource-clipping/clipping/story/assembler.py:234-236). **Bug**: `offset=0` makes the fade start at the first frame instead of at `len(A) - fade`, and pairwise chaining re-encodes N-1 times (repos/opensource-clipping/clipping/story/assembler.py:244-268). Text overlay uses `drawtext` (repos/opensource-clipping/clipping/story/assembler.py:277-327).
10. **Diarization audio**: `ffmpeg -y -i v -vn -acodec pcm_s16le -ar 16000 -ac 1 out.wav` (repos/opensource-clipping/clipping/diarization.py:40-62).
11. **Encoder probe**: lavfi `color=c=black:s=640x360:r=30:d=1` into the candidate encoder with `-f null -` (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:43-71).

---

## 8. Dependencies and external services

- Gemini API key required (`GOOGLE_API_KEY`); NVIDIA build.nvidia.com key optional; Pexels key (B-roll); HuggingFace token + accepted gated terms for pyannote (repos/opensource-clipping/.env.sample:6-23). Meta/YouTube OAuth for uploaders.
- Runtime downloads: fonts (GitHub/jsDelivr), MediaPipe `blaze_face_short_range.tflite` (Google storage, repos/opensource-clipping/clipping/config.py:136), YOLO face weights from HuggingFace `Bingsu/adetailer` (repos/opensource-clipping/clipping/config.py:836), a glitch video from YouTube (repos/opensource-clipping/clipping/config.py:132), faster-whisper model weights.
- Deno/Node needed for yt-dlp JS challenges (repos/opensource-clipping/Dockerfile comment; notebook labels Deno "used internally by subtitle rendering modules", which is misleading; UNVERIFIED actual use).
- GPU: Whisper large-v3 ~10 GB VRAM per the notebook table (repos/opensource-clipping/notebooks/Quick_Start.ipynb cell 4); NVENC optional.

---

## 9. Performance notes

- Face detection is sampled (4 Hz) but **every output frame** goes through Python (decode, numpy slice, resize, optional alpha blend, `tobytes`, pipe): CPU-bound, no batching or hardware decode. Three decode passes of the same span for hook/main, plus a seek per detection sample (`cap.set` is slow on long-GOP media). YOLO runs per sample on CPU unless CUDA.
- `-loglevel verbose` main pass floods stderr (repos/opensource-clipping/clipping/studio/core.py:702). Progress parsing is fine.
- 3-4 generations of x264 re-encode per clip (section 4D). `.ts` intermediates avoid container finalisation but cost bitrate.
- Per-job single worker thread (repos/opensource-clipping/web/api/worker.py:28-34); batch mode reuses a loaded Whisper (repos/opensource-clipping/main.py:204 passes `whisper_model_instance`).
- No benchmarks in repo; **UNVERIFIED** speed claims (README says 5-15 min per video on Colab T4, repos/opensource-clipping/notebooks/Quick_Start.ipynb cell 8).

---

## 10. Weaknesses, bugs, limitations

1. **LLM output not validated**: durations, overlap, out-of-range timestamps, hook-in-clip, `keep_segments` inside clip bounds are not checked (repos/opensource-clipping/clipping/metadata.py:104-215). A hallucinated timestamp goes straight to FFmpeg.
2. **Whole transcript in one prompt**, no chunking, no second-pass rerank, no user-supplied criteria beyond count. Prompt embeds Indonesian brand account taxonomy.
3. **Temp files in CWD, global singletons** (`_FACE_DETECTOR`, `USED_PEXELS_IDS`, repos/opensource-clipping/clipping/studio/face_detection.py:45, repos/opensource-clipping/clipping/studio/broll.py:50), `importlib` loading hacks duplicated in every module (repos/opensource-clipping/clipping/studio.py:19-37). Not safe for concurrency.
4. **Config via env var side-effects**: `OSC_VIDEO_SCALE_ALGO` (repos/opensource-clipping/clipping/runner.py:159-161).
5. `--track-snap` default silently overridden (see 4E); `JITTER_THRESHOLD` dead; transition pool dead; `--silence-trim` is prompt-only; `hook_v2` item timestamps are LLM-chosen and not verified to fall in the clip.
6. **Story xfade offset bug**, stream-copy trims not frame-accurate (see 7.9).
7. BGM: random file, starts at 0:00, no fades, ducking not applied in VO intro, moods mislabeled (a "hanuman-devotional-song" and "baby-sleep-lullaby" sit in `upbeat/`, `nature-ambient` in `upbeat/`; see filenames under assets/bgm/upbeat/).
8. Uploaders have an "approval/daily limit" safety removed and parked in `youtube_uploader/Safety.md` (repos/opensource-clipping/youtube_uploader/Safety.md:3-5) while `upload_safety.json` still exists (repos/opensource-clipping/upload_safety.json), confusing.
9. Hard-coded private Kaggle dataset path for cookies in all notebooks (`/kaggle/input/datasets/muhammadnaufal/tiktok-secret-cookies/youtube_cookies.txt`, repos/opensource-clipping/notebooks/Quick_Start.ipynb cell 9); credential-hygiene smell.
10. Missing dependencies in `requirements.txt` (openai; gdown (engine.py:74) and tqdm (engine.py:414) are imported; gdown is only in pyproject, tqdm in neither list).
11. Tests don't cover the product core: no assertions on prompt output parsing, ASS content, filtergraphs, or camera math (list in section 3). Only API, logging, Kaggle bootstrap, diarization wrapper, face-detector construction.
12. `README` License section says "Open source. Feel free to use, modify, and distribute." (repos/opensource-clipping/README.md:722-724) which is looser than the MIT `LICENSE` file; MIT wins as the legal text.
13. Hard-codes `yt-dlp` downloading of third-party YouTube videos as overlay assets (glitch, film burns) with unknown rights (repos/opensource-clipping/clipping/studio/transitions.py:48-109).

---

## 11. License and reuse notes

- **Repo code: MIT**, (c) 2026 Muhammad Naufal Rizqullah (repos/opensource-clipping/LICENSE:1-3). Verified. No NOTICE, no third-party license file.
- **BGM mp3s (15 files, ~76 MB): no license file or per-file attribution in the repo.** `assets/bgm/README.md` tells users to bring their own royalty-free tracks from Pixabay / YouTube Audio Library / Incompetech (repos/opensource-clipping/assets/bgm/README.md:25-37), and `.gitignore` has the mp3 exclusion **commented out** (repos/opensource-clipping/.gitignore `# *.mp3`), so they were committed. Filenames are `<author>-<title>-<numeric id>.mp3`, which matches Pixabay's naming pattern (UNVERIFIED). The MIT license does **not** clearly cover them: if from Pixabay they fall under the Pixabay Content License (use allowed, standalone redistribution not allowed; I could not verify, UNVERIFIED), which does not permit republishing as a pack. Several titles look like third-party songs or covers (e.g. `tomomi_kato-silhouette-2026ver`, `hanuman-devotional-song`); rights UNVERIFIED. **Do not copy or ship these files.** Treat as unlicensed.

  | File (assets/bgm/...) | Bytes |
  |---|---|
  | chill/alex-morgan-background-music-545525.mp3 | 5,111,808 |
  | chill/alex-morgan-peaceful-cafe-jazz-relaxing-coffee-shop-541031.mp3 | 4,245,504 |
  | epic/alex-morgan-rock-rock-music-545492.mp3 | 1,029,888 |
  | epic/jonasblakewood-adventure-trailer-528060.mp3 | 4,320,862 |
  | epic/trailer-score-room-epic-cinematic-thriller-538890.mp3 | 5,111,040 |
  | sad/soulprodmusic-room-beautiful-melancholy-calm-piano-improvisation-126608.mp3 | 5,761,149 |
  | sad/tomomi_kato-silhouette-2026ver-486531.mp3 | 4,467,983 |
  | sad/vifotofreesounds-1800s-ambient-mood-486673.mp3 | 6,706,573 |
  | suspense/the_mountain-cinematic-mood-129193.mp3 | 5,857,280 |
  | suspense/the_mountain-epic-mood-129192.mp3 | 4,024,111 |
  | suspense/virtualvisualviews-dueling-music-box-horror-373912.mp3 | 9,506,899 |
  | upbeat/alex-morgan-gentle-baby-sleep-lullaby-dream-548647.mp3 | 6,296,064 |
  | upbeat/alex-morgan-hanuman-devotional-song-548645.mp3 | 5,979,648 |
  | upbeat/alex-morgan-nature-ambient-548640.mp3 | 5,687,040 |
  | upbeat/tolsetmusic-talking-about-love-romantic-piano-background-music-542958.mp3 | 4,119,405 |

  These are **music beds only; there are no SFX files in the repo** (the answer to "are they SFX or music": music/ambient).
- Logos: `assets/images/opensource-clipping-*.{png,svg}` (< 140 KB each); license not stated separately, presumed MIT, branding of the project; do not reuse.
- Fonts (runtime-downloaded, not vendored): Montserrat, Anton, Bebas Neue, Lora, Inter (SIL OFL 1.1) and Roboto (Apache-2.0) per my general knowledge; **the repo states none**, UNVERIFIED. Safe to bundle if we verify OFL files. Source URLs are in repos/opensource-clipping/clipping/config.py:52-109.
- Models/deps: faster-whisper (MIT) + Whisper weights (MIT, UNVERIFIED here); MediaPipe (Apache-2.0, UNVERIFIED); **ultralytics YOLO (AGPL-3.0)** and the community `face_yolov8*.pt` weights from `Bingsu/adetailer` (AGPL per the YOLOv8 lineage; UNVERIFIED, check model card) -> avoid in a closed or permissive product; pyannote 3.1 (MIT code, gated HF model requiring terms acceptance + token); edge-tts (LGPL-3 package, UNVERIFIED) wraps Microsoft's unofficial Edge read-aloud endpoint (ToS risk); yt-dlp (Unlicense).
- Pexels B-roll: Pexels license permits free use, no attribution needed (UNVERIFIED here); API terms apply. YouTube-sourced overlay videos (glitch, film burns, from creators like "Think Make Push", comment at repos/opensource-clipping/clipping/studio/transitions.py:30) have **unknown/third-party rights**; do not reuse.
- Output-use risk: the tool downloads and re-uploads third-party videos (YouTube/TikTok) and ships auto-uploaders; copyright/ToS exposure is the user's. It appends `Source: <url>` to descriptions (repos/opensource-clipping/clipping/metadata.py:76).
- Prompts are MIT-covered original text and are safe to adapt (we will rewrite in English anyway).

---

## 12. Steal list (ranked)

1. **One-shot structured clip-plan schema** (reimplement). A strict JSON schema from the LLM containing clip times, hook, keep_segments, typography words, B-roll queries, BGM mood, title/caption (repos/opensource-clipping/clipping/engine.py:929-1090). In Rust: `serde` + `schemars` JSON Schema, sent as OpenAI-compatible `response_format: json_schema`; store it as the editable "plan" artifact that agents/MCP read and patch. Fix what they omit: server-side validation (duration bounds, snap to word boundaries, de-overlap, keep_segments within clip, hook inside clip).
2. **Retention-oriented selection prompt** (port text, translate, drop account taxonomy). Hook-in-clip, shift start to strongest sentence, 5x20 rubric, slow-closing padding, standalone clarity, anti-duplicate (repos/opensource-clipping/clipping/engine.py:634-760). Improve by emitting the sub-scores so agents can reason/rerank (they intentionally suppress them, :681).
3. **Sidechain-ducked BGM filter** (reimplement as FFmpeg graph builder). Exact parameters in section 4F/7; add `normalize=0` on `amix`, fades, start-offset, loudnorm afterwards, and duck inside the VO intro (repos/opensource-clipping/clipping/studio/audio_bgm.py:36-67).
4. **ASS word-level generator** (reimplement in Rust, `ass` string builder). Per-word `\pos` events, bounce_pop/stagger_up tag recipes, scale levels, font-metric line wrapping via `ab_glyph`/`rusttype` instead of PIL, emphasis words from the plan (repos/opensource-clipping/clipping/studio/subtitles.py:55-389). Make styles data-driven templates (JSON) rather than 4 hard-coded font pairs; bundle OFL fonts; pass `fontsdir`. Keep the caption event model but use one event per line with `\k`/`\kf` karaoke instead of N events per word where possible.
5. **Face-follow camera smoother** (port algorithm). Dead-zone + proportional follow + snap-cut threshold + median init (repos/opensource-clipping/clipping/studio/render_hybrid.py:234-264) is a sound minimal camera model. In our system: run detection via an ONNX model (`ort`) at 4 Hz, emit a **crop-path keyframe list** (JSON) and apply with FFmpeg `crop` using time expressions or `sendcmd`/`zoompan` rather than piping frames through the host process; agent can inspect/override keyframes. Fix their "largest face = speaker" with diarization-aware picking.
6. **Speaker->face assignment heuristics** (reimplement later, lower priority): canonical X per speaker from solo frames, sort-left-to-right, IoU duplicate merge, layout majority-vote window (repos/opensource-clipping/clipping/studio/render_split_screen.py:211-470).
7. **`render_manifest.json` + `--load-gemini-json` replay pattern** and per-stage status/error trace (repos/opensource-clipping/clipping/runner.py:85-107, repos/opensource-clipping/clipping/studio/core.py:96-105,1108-1130). Formalize as resumable job state with step artifacts on disk (transcript.json, plan.json, crop_path.json, captions.ass, render report).
8. **Story-recipe EDL** (repos/opensource-clipping/story_recipe.json) -> generalize into our timeline/EDL JSON (sources, scenes, transitions, labels). Use `xfade` correctly (`offset = cumulative_len - fade`) in a single filtergraph, not iterative pairwise re-encodes.
9. **Encoder auto-probe** (reimplement): list `-encoders`, then run a 1 s synthetic encode to verify NVENC/AMF/VAAPI/QSV/VideoToolbox actually works, cache result (repos/opensource-clipping/clipping/studio/ffmpeg_utils.py:24-71,82-174).
10. **Robust LLM JSON handling**: retry classes (408/429/5xx/JSON decode), back-off, fallback model, code-fence stripping, key alias healing for non-Gemini models (repos/opensource-clipping/clipping/engine.py:496-590, 1113-1118; repos/opensource-clipping/clipping/metadata.py:125-146). Our provider layer should add `base_url`, model list, per-model capability flags (json_schema vs json_object vs prompt-only).
11. **Readiness/health + JSONL structured logging with correlation IDs** (repos/opensource-clipping/web/api/readiness.py, repos/opensource-clipping/scripts/studio_logging.py) for our daemon/MCP server.
12. **Dev-mode camera overlay** (dimmed frame, bright crop rect, face box, HUD) as a QA render type that an agent can request to "see" reframing decisions (repos/opensource-clipping/clipping/studio/render_hybrid.py:394-470).
13. Small ideas: AV1-avoiding yt-dlp format selector (repos/opensource-clipping/clipping/engine.py:20-57), json3 YouTube caption shortcut (repos/opensource-clipping/clipping/engine.py:250-358), white-flash/RGB-shift glitch lavfi recipes (repos/opensource-clipping/clipping/studio/v2_helpers.py), `showwaves` + `blend=screen` ambient overlay graph (section 7.6).

Do not adopt: Python frame-pipe rendering, `.ts` temp-file chain, the 1000-line `proses_klip`, edge-tts, YOLO/Ultralytics (AGPL), bundled mp3s, YouTube-hosted overlay assets.

---

## 13. Gaps (what we must build ourselves)

- Agent/MCP surface: tool schemas for find_highlights, set_cut, set_caption_style, set_reframe_path, add_music/sfx, render, qc; resumable step state; JSON I/O everywhere; read-only "describe" tools (frame grabs, waveform/energy, transcript windows) so the agent can inspect, not just generate.
- Chunked/long-video highlight detection with candidate -> rerank, audio-energy/laughter/scene-change signals, boundary snapping to words/sentences/silence, de-dup and overlap rules, per-platform duration targets.
- Pluggable OpenAI-compatible LLM, STT (`/v1/audio/transcriptions` with word timestamps) and TTS endpoints with config for base URL, model, schema mode; local Whisper (whisper.cpp / faster-whisper sidecar) fallback.
- Caption template system (JSON-driven styles, fonts, gradients, outlines, emoji, highlight color per speaker/word, safe-area presets per platform) with live GUI preview; subtitle editor with word-level timing.
- Audio: loudness normalization (`loudnorm` two-pass), denoise (`afftdn`/RNNoise), true silence removal via `silencedetect`, SFX library with licenses and placement (whoosh on transitions/hook, impact on emphasis), music selection by metadata/energy, beat-aware cuts, fade in/out, ducking in all stages.
- Reframing: tracking with IDs, active-speaker from audio-visual correlation, multi-person layouts, smooth virtual camera with zoom, scene-cut detection, subject/object tracking, safe area for captions.
- Quality checks (nothing exists): duration within target, hook within first 3 s, caption overflow/safe-zone, audio clipping/LUFS, black/frozen frames, A/V sync, face-visible ratio, subtitle-word coverage vs transcript; produce a machine-readable QC report for the agent loop.
- Single-pass FFmpeg filtergraph rendering with HW accel and previews/proxy, deterministic temp directories, concurrency safety, cancellation.
- Timeline/EDL data model (tracks, clips, effects, keyframes) shared by CLI/MCP/GUI; undo/redo.
- Licensing hygiene: bundled assets manifest with license per file; no third-party downloads at runtime.
