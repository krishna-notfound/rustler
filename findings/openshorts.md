# Findings: OpenShorts (`repos/openshorts`)

How this was done: I read the source and grepped the large files (`app.py` is 10,256 lines, `main.py` 2,991). Nothing was run. Every claim cites `repos/openshorts/<path>:<line>`. **UNVERIFIED** marks anything not confirmed in code. Licences marked *(external knowledge)* come from upstream projects and are not stated in this repo. The clone is shallow (1 commit, HEAD `8af4a48`, 2026-11-12).

---

## 1. Summary

OpenShorts is a production SaaS-grade Python (FastAPI) + React pipeline. It turns a long video (YouTube URL or upload) into 9:16 shorts:
- transcription (faster-whisper / Parakeet);
- two-pass LLM moment ranking (Gemini, or any OpenAI-compatible server for the text passes);
- scene-aware reframing with 6+ layouts (track, blur-pad, split, screencast, inset, wide);
- word-level ASS karaoke captions, PIL hook overlays, AI effects as an EDL;
- ElevenLabs dubbing, and publishing through Upload-Post;
- an MCP server (HTTP + stdio), a CLI, a Claude skill and a ChatGPT app.

It also ships `osrender`, a **Rust + CUDA single-pass renderer** (NVDEC → CUDA compositor → NVENC) that reads the same FFmpeg filtergraphs.

**Verdict:** this is the strongest reference so far for **clip selection (A)**, **ASS captions (B)**, **layout geometry (E)** and **agent surface design (G)**. It is weak on camera smoothing, audio, and output QC. Its provider abstraction is Gemini-centric. Licence traps: YOLOv8 (AGPL), `cloud/` (commercial), Remotion.

## 2. Stack and architecture

- **Backend:** Python 3.11, FastAPI/uvicorn (`requirements.txt:15-16`), an in-memory job dict plus `asyncio.PriorityQueue` (`app.py:893-906`).
  - Each job runs as a child process: `python -u main.py -u <url> -o <dir> --format ...`, with options passed as env vars (`app.py:5575-5717`).
  - The child talks back through stdout markers (`CLIP_READY <i> <file>`, `SOURCE_READY`, …) (`app.py:4435-4505`).
- **ML:**
  - faster-whisper 1.2.1, onnx-asr Parakeet, TransNetV2 + PySceneDetect, MediaPipe 0.10.14, Ultralytics 8.4.46 (YOLOv8n), torch 2.11 (`requirements.txt:1-12`).
  - google-genai 1.75 (`requirements.txt:10`).
- **Render:** Debian `ffmpeg` called as a subprocess through the wrapper `bin/ffmpeg`, which adds thread caps and an NVENC slot flock (`bin/ffmpeg:23-106`, `Dockerfile:80-81,136`).
  - Optional `osrender` Rust binary built in a separate Docker stage (`Dockerfile:57-67`).
  - Optional Remotion (Node) render service plus in-browser `@remotion/web-renderer`.
- **Frontend:** React 18 + Vite 4 + Tailwind 3 + Remotion player (`dashboard/package.json:12-37`). `App.jsx` is a single 2,770-line component.
- **Agent surfaces:** `mcp_server.py` (hand-rolled JSON-RPC over FastAPI, no SDK), `mcp_stdio.py`, `cli/openshorts_cli.py` (stdlib only), `skills/openshorts/SKILL.md`, `integrations/chatgpt-app/`, `examples/n8n/`.

```
dashboard (React) ─┐                       ┌─ main.py child per job (ASR → LLM pick → cut → reframe_v2 → captions/hook)
CLI / MCP / n8n  ──┼─ HTTP ─ app.py (FastAPI, queue, webhooks, publish) ──┤        └─ osrender (Rust/CUDA) | ffmpeg
MCP stdio ─────────┘   (MCP tools call REST in-process via ASGITransport)  └─ remote_worker (CPU host, pull-based)
```

## 3. Repo map (important files only)

| Path | Role |
|---|---|
| `main.py` | Pipeline child: ingest, transcription glue, `get_viral_clips` / `get_visual_clips`, `SmoothedCameraman`, `SpeakerTracker`, v1 reframe fallback, hook/caption burn |
| `clip_selection.py` | Windows, batching, shortlist and count targets, word snap, dedupe, vision segments |
| `gemini_worker.py` | All prompt templates plus pydantic schemas, JSON repair, upload, block detection, cost |
| `llm_backend.py` | OpenAI-compatible chat backend, used for the two text passes only |
| `transcribe_backends.py`, `subtitles.py` | STT (whisper/Parakeet), SRT/ASS generation, caption presets, burn |
| `reframe_v2.py`, `split_layout.py`, `screencast_layout.py`, `camera_inset.py`, `panel_layout.py`, `active_speaker.py`, `punch_in.py`, `layout_picker.py`, `layout_ranges.py` | Reframing and layouts |
| `gpu_render.py`, `gpu_decode.py`, `osrender/src/{main.rs,kernels.cu}` | Filtergraph → layer interpreter, Rust/CUDA renderer |
| `ffmpeg_utils.py`, `bin/ffmpeg` | Encoder tiers, cut, loudnorm, filter escaping, NVENC probe/slots |
| `hooks.py`, `hook_placement.py`, `hook_grounding.py` | Hook text overlays |
| `editor.py`, `edit_builder.py`, `recut.py` | LLM effects EDL → safe filter; manual segment recut |
| `translate.py`, `saasshorts.py`, `thumbnail.py` | ElevenLabs dubbing; AI UGC ad generator; thumbnail studio |
| `mcp_server.py`, `mcp_stdio.py`, `mcp_ui.py`, `cli/`, `skills/` | Agent access |
| `app.py` | REST API, queue, resume/drain, webhooks, Upload-Post publishing |
| `remotion/`, `render-service/`, `dashboard/src/remotion/` | Remotion compositions (two copies that have drifted apart) |
| `cloud/` | Auth, billing, metering, autopilot: **commercial licence** |
| `tests/` | 133 files, including a GPU/ffmpeg SSIM equivalence test |

## 4. Capabilities by category (A–I)

### A. AI clipping — quality **4**

**Two-pass, rank-everything design** (`main.py:2096-2286`):
1. **Windows.** The transcript is cut into windows aligned to Whisper segments, each `max(90, 1.5·max_secs)` s long with a 30 s overlap (`clip_selection.py:228-283`, `main.py:2137-2139`).
2. **Score pass.** Every window gets a 0–100 score, in near-equal batches: 8 per call for Gemini, 3 for a local LLM, or `LLM_SCORE_BATCH` (`main.py:2084-2093`, `clip_selection.py:114-130`).
3. **Global shortlist.** The top N across all windows, `N = max(3, min(10, dur//90 + 2))` (`clip_selection.py:71-82`, `main.py:2183-2185`).
4. **Detail pass.** One call over the shortlist returns exact start/end, a predicted score, a hook, a title and platform descriptions (`main.py:2198-2213`).
5. **`min_clips` floor enforced in code.** If too few clips come back, the windows that produced none are asked again for the missing count (`main.py:2221-2235`).
6. **Ceiling.** Excess clips are trimmed by `predicted_score` (`clip_selection.py:133-165`).
7. **Word snap.** Start and end move to word boundaries within ±1.5 s, with padding into the silence of `min(0.35, gap/2)` before and `min(0.45, gap/2)` after. The duration is then repaired to fit the band (`clip_selection.py:328-390`).
8. **Dedupe.** A clip is dropped if it overlaps a better one by ≥ 25% of the shorter clip's length (`clip_selection.py:286-325`).

Other paths and signals:
- **Duration band:** 15–60 s by default (`CLIP_MIN_SECONDS`/`CLIP_MAX_SECONDS`, clamped 5–180) (`clip_selection.py:192-213`).
- **Silent or sparse speech:** fewer than 8 words, or fewer than 5 words/min (`main.py:2295-2304`), switches to the Gemini **video** path.
  - The video is split into ≤ 2400 s stretches (at most 8). Each stretch is sent as a 360p / 1 fps / CRF 30 proxy (`main.py:2311-2340`, `clip_selection.py:168-189`).
- **Twitch prior:** viewer-made clips from Helix are mapped to windows by midpoint and passed into the prompt as `viewers_clipped`. Up to `target//3` shortlist slots are reserved for them (`twitch_signals.py:85-141,177-245`). Chat spikes are not used.
- **Early pick:** with `EARLY_AUDIO`, ASR and selection run on the audio track while the video is still downloading (`main.py:2634-2665`).
- **Render order:** clips render best-first, 6 at a time (`CLIP_WORKERS`), with one retry each (`main.py:1952-1960,2938-2973`).

Missing:
- No audio-energy, laughter or visual signal in the main text pick.
- Scene cuts are **not** fed to selection; they are only used by reframing (`main.py:1715`, `reframe_v2.py:669`).

### B. Transcription and subtitles — quality **4.5** (STT) / **4** (captions)

**STT engines** (`transcribe_backends.py:732-759`). Both run locally; there is no cloud STT.
- **Whisper:** faster-whisper, default `small` / `cpu` / `int8`; production uses `large-v3-turbo` / `cuda` / `float16` (`subtitles.py:16-25`, `.env.example:25-30`).
  - Decode parameters: `beam_size=5, vad_filter=True, condition_on_previous_text=False, word_timestamps=True` (`subtitles.py:30-35`).
- **Parakeet:** `TRANSCRIBE_BACKEND=parakeet` uses parakeet-tdt-0.6b-v3 through onnx-asr with Silero VAD forced onto the CPU (`transcribe_backends.py:42,305-322,346`).
  - It falls back to whisper when it finds 0 words, the language is outside its 25 supported ones, or speech is under 0.2 words/s (`:695-709`).
  - Language comes from `py3langid`; texts under 20 characters default to `en` (`:678-692`).
  - **Wrong-alphabet repair:** a segment written in an unexpected alphabet (for example English in Cyrillic letters) is re-transcribed with whisper forced to the file's language (`:573-675`).
- **Word model:** each word has a leading space; fragments without one are merged into the previous word (`subtitles.py:38-61`).
  - Parakeet word ends are set to the next word's start, capped at last token + 0.6 s (`transcribe_backends.py:482-514`).
- **VRAM:** `release_models()` takes every slot of the lock, then unloads the ctranslate2 weights and empties the CUDA cache (`:416-466`).
  - GPU concurrency is limited per process (`ASR_GPU_CONCURRENCY`) and across the host with lock files (`ASR_HOST_SLOTS`) (`:54-121`).
- **Not present:** diarization and forced alignment.

**Caption generation:**
- **Blocks:** a block closes at `max_chars` (default 20, 16 for auto captions) or `max_duration` (2.0 s, 1.4 auto) (`subtitles.py:152-201,252-253`). Each block is a single line, `WrapStyle 0` (`:468`).
- **Per-font line budget:** `max(6, round(chars_at_44 · 44/size))`, where Anton fits 16 characters at size 44 and Montserrat ExtraBold 9 (`:292-298`).
- **ASS karaoke** is one Dialogue event per word. Every event repeats the whole block with the active word wrapped in overrides, so there are no `\k` tags (details in §6).
- **Presets** (server side): default, hormozi, pill, lime, oneword, clean (`subtitles.py:261-288`). The dashboard adds 11 more that exist only in the browser (`dashboard/src/components/SubtitleModal.jsx:91-101`).
- **Emoji:** none in captions apart from the Noto Color Emoji fallback font (`Dockerfile:91-92`). Hooks do handle emoji (`hooks.py:43-91`).
- **Existing burned-in subtitles** in the source are detected by Gemini on 12 frames. If present in ≥ 40% of frames, sitting in the 75–97% height band, the bottom is cropped off before reframing (`burned_subtitles.py:34-93`).

### C. Creative design — quality **3.5**

- **Fonts:** Montserrat ExtraBold, Anton and Noto Serif Bold are bundled (`fonts/`). A fontconfig alias file maps UI font names to real fonts: Verdana/Arial/Helvetica → Liberation Sans, Impact → Anton (`fonts/openshorts-fontmap.conf:11-15`).
- **Hook styles:** pill (default), classic, dark, yellow, red, outline, outline_yellow (`hooks.py:207-224`).
  - Rendered with PIL to an RGBA PNG, then composited with `overlay=...:enable='between(t,0,dur)'` (`hooks.py:656-668`).
  - Emoji are drawn from a colour-emoji font at its native bitmap size, then rescaled (`hooks.py:43-91`).
- **Active-word effects:** pop, glow, box, highlight (an opaque-box style) and none (`subtitles.py:429-462`).
- **AI effects** (EDL types): zoom_in, punch_in, zoom_pulse, color_pop, bw_moment, flash, vignette (`edit_builder.py:15-23`).
- **Remotion overlays:** spring pop, glow, karaoke box, hook entrance animations (`remotion/src/compositions/Subtitles.tsx:167-211`).
- **Not present:** transitions between clips, gradient/motion templates, a template marketplace, a brand kit.

### D. Video processing and rendering — quality **4**

- **Encoder tiers:** x264 and NVENC, with the rule NVENC `cq ≈ crf + 7` (`ffmpeg_utils.py:26-52,223-236`). Commands are in §7.
- **NVENC selection:** `FFMPEG_ENCODER` is `x264`, `nvenc` or `auto`. `auto` probes NVENC once with a 256×256 lavfi test encode (`ffmpeg_utils.py:157-220`).
- **Per-scene segments:** one segment per scene, then a stream-copy concat and a metadata scrub (`reframe_v2.py:550-639`, `ffmpeg_utils.py:63-64`).
- **TRACK crop:** `sendcmd` changes the crop x per frame, writing a line only when x changes (`reframe_v2.py:81-92`).
- **Sparse analysis decode:** an ffmpeg `select=` expression decodes only the frames the detectors need. Terms are combined as a balanced tree because ffmpeg's expression parser fails past about 100 terms (`gpu_decode.py:213-224`, `reframe_v2.py:310-347`).
- **`osrender` (Rust):** one pass on the GPU, NVDEC → CUDA bicubic resample + separable Gaussian + overlay blend → NVENC (`osrender/src/main.rs:285-508`, `osrender/src/kernels.cu:5-80`).
  - Python's `gpu_render.layers_from_graph` interprets the same filtergraph strings into `{src,dst,blur}` layers, so the GPU path and the FFmpeg path cannot drift apart (`gpu_render.py:232-298`).
  - An SSIM ≥ 0.97 equivalence test covers every layout (`tests/test_gpu_render_equivalence.py:23,58-179`).
- **Lazy hooks:** with `HOOK_LAZY`, the PNG and its position are stored and the hooked file is encoded only on demand (`hooks.py:652-740`).
- **Remote CPU worker:** a second host pulls jobs and renders them (`worker.py:1-37`, `remote_worker.py`).
- **Weakness:** each clip goes through 3–4 lossy encodes: cut → reframe → hook/captions → watermark (`main.py:2873,1510,1619`; `reframe_v2.py:554`).

### E. Reframing — quality **3** (camera) / **4** (layouts)

- **Detectors:**
  - MediaPipe BlazeFace full-range model, confidence 0.5, every 4th frame on a 640 px copy (`main.py:101-102,416-436`).
  - YOLOv8n persons every 8th frame, with the top 40% of the box used as a stand-in face (`main.py:97,479,497-500`).
  - TransNetV2 shots at threshold 0.5; scenes under 0.4 s are merged (`scene_detection.py:46,120,142-168`).
- **Camera ("Heavy Tripod", `SmoothedCameraman`, `main.py:116-269`):**
  - Dead zone of `0.25·crop_w` (`main.py:143`).
  - A jump must be confirmed by 3 consecutive detections that agree with each other (`main.py:107,204-218`).
  - Constant pan speed of **3 px/frame**, or 15 px/frame when the gap is more than half the crop width (`main.py:236-243`). There is no EMA, Kalman filter or easing.
  - **Horizontal only**; y is pinned (`main.py:266-267`).
  - Snaps to the first target after a scene cut (`main.py:170-202`).
- **SpeakerTracker** picks the face by **area**, not by who is speaking. The current target's score gets ×3 hysteresis, and a switch needs a 30-frame cooldown (`main.py:271-411`, `reframe_v2.py:815`).
- **Scene classifier:** average faces < 0.5 or > 1.2 → GENERAL (blur-pad), otherwise TRACK. Scenes under 2 s are smoothed to match their neighbours (`main.py:621-637`).
- **Layout upgrades,** each with measured thresholds (full table in §6):
  - SPLIT: two faces stacked on top of each other (`split_layout.py:38-203`).
  - SCREENCAST: screen content above, presenter below (`screencast_layout.py:45-156,637-749`).
  - WIDE / focus-crop, centred on motion (`screencast_layout.py:434-607`).
  - INSET: corner camera box (`camera_inset.py:25-455`).
  - PANEL: 3–4 people — **not wired in** (`panel_layout.py`, only tests import it).
  - Manual per-scene overrides (`reframe_v2.py:192-271`).
- **Gemini layout picker:** 12 frames at 1024 px with a closed choice of `none` / `screencast` / `split` (`layout_picker.py:55-61`, `gemini_worker.py:142-171`).
- **Active speaker** (only on SPLIT scenes, off by default):
  - Mouth-region frame-difference energy in 0.4 s windows, gated by audio RMS ≥ 0.18 of the scene's peak.
  - Normalised per speaker to the 20th–80th percentile range; a window needs a 0.15 margin to count.
  - A new speaker must win 3 windows in a row; each speaker needs a 0.2 minimum share.
  - Output is hard cuts (`active_speaker.py:27-218`).
- **Punch-in** (off by default): zoom ×1.12 with a smoothstep envelope of 0.25/1.30/0.55 s, triggered on audio RMS peaks at least 18 s apart (`punch_in.py:30-186`).
- **Already-vertical sources** skip the classifier and pass through (`reframe_v2.py:69-78,689-698`).

### F. Audio — quality **2**

- **Loudness:** a single `loudnorm=I=-14:TP=-2.0:LRA=11` pass at the cut (`ffmpeg_utils.py:93,371`) and in recut (`recut.py:194`).
  - The comment explains why the true-peak is −2 dB: AAC adds inter-sample peaks, and `alimiter` or two-pass `linear=true` made things worse (`ffmpeg_utils.py:79-92`).
  - Every later pass uses `-c:a copy`.
- **Not present:** silence removal, music, SFX, ducking, `amix`, `sidechaincompress`, `afade`, denoising (grep over `*.py`).
- **TTS:** ElevenLabs only, in two places:
  - dubbing (`translate.py:95-209`);
  - SaaSShorts narration with `eleven_multilingual_v2` (`saasshorts.py:788-848`).

### G. Agent integration — quality **5** (best reference so far)

- **MCP tools.** 8 coarse tools (`mcp_server.py:64-339`):
  - `process_video`, `create_upload`, `get_job_status`, `list_clips`, `get_quota`, `add_subtitles`, `recut_clip`, `publish_clip`;
  - each has readOnly/destructive/idempotent/openWorld annotations (`:344-357`).
- **MCP transport and auth.**
  - Tools proxy to REST in-process through `httpx.ASGITransport` (`:9-14,363-370`).
  - stdio swaps stdout for stderr before importing the app (`mcp_stdio.py:29-31,93-114`).
  - OAuth access tokens are ordinary scoped `osk_` API keys (`cloud/scopes.py:26-40`).
- **Outputs and long jobs.**
  - Results carry both text and `structuredContent` (`mcp_server.py:690-705`).
  - Every media URL is signed and absolute (`:463-510`).
  - Jobs are async: the client polls every 30–60 s or supplies an HMAC webhook (`:46-48`; `app.py:3688-3777`).
- **MCP Apps widget:** a clip-picker UI with a content-hashed resource URI (`mcp_ui.py:9-234`).
- **Recut works in source-time coordinates.** `list_clips` returns `source_segments`, and `recut_clip` accepts `[{start,end}]` in source seconds (`mcp_server.py:275-310,452-460`).
- **Skill:** "always async", "the choices the API cannot make for you", "three responses that are not failures", consent before publishing (`skills/openshorts/SKILL.md:19-113`).
- **CLI:** stdlib only, global `--json`, `--wait`/`--watch` poll and stream new log lines, exit code 1 = API error, 2 = server unreachable (`cli/openshorts_cli.py:50-125,192-228`). It is a subset of what MCP offers (no upload, recut or subtitle commands).
- **Gaps:** progress is free-text log lines parsed with regex (`log_view.py:25-40`), there are no fine-grained edit tools (caption text edit, per-scene layout set, font upload), and REST and MCP default `auto_hook` differently (`mcp_server.py:400-406`).

### H. GUI — quality **4**

- **Stack and tabs:** React. Tabs are Clip Generator, Autopilot, AI Shorts, AI Agent, UGC Gallery, YouTube Studio, History, Settings (`dashboard/src/App.jsx:1352-1361`).
- **`ClipEditor.jsx`** (1,614 lines) is a segment editor, not a multi-track NLE (`ClipEditor.jsx:11-90,315-433,1585`):
  - a source track and a clip track;
  - drag to trim, move and paint segments;
  - undo/redo through a reducer;
  - a transcript panel that snaps to words.
- **`ReframeEditor.jsx`:** manual crop per scene, plus a two-region split.
- **Caption and hook editing:** `SubtitleModal` and `HookModal` show a live Remotion preview.
- **Other:** a schedule calendar (`ScheduleWeekModal.jsx`) and in-browser WebCodecs export (`dashboard/src/lib/renderInBrowser.js:1-58`).
- **Preview drift:** the browser dims inactive words with `0.35+0.65·opacity`, the ASS burn with `0.5+0.5·opacity` (`dashboard/src/remotion/compositions/Subtitles.tsx:187` vs `subtitles.py:344`).

### I. Provider abstraction — quality **2.5**

- **LLM backends:** `llm_backend.py` supports Gemini or any OpenAI-compatible `/chat/completions` server.
  - Selected with `LLM_PROVIDER`, or by `LLM_BASE_URL`; also `LLM_MODEL` (default `llama3.1:8b`), `LLM_API_KEY` and `LLM_TIMEOUT` (600 s) (`llm_backend.py:33-78`).
  - JSON is enforced by downgrading the response format `json_schema` → `json_object` → none on a 400/422 rejection, then validating with pydantic. Temperature is fixed at 0.2 (`llm_backend.py:86-151`).
- **Coverage:** this covers **only the two transcript passes**. These hard-code `genai.Client`:
  - layout picker, screencast detector, vision path;
  - hook grounding, editor, thumbnails, SaaSShorts, burned-subtitle detection.
  
  (`llm_backend.py:9-14`)
- **STT:** local only (whisper/Parakeet). There is no OpenAI-compatible STT endpoint.
- **TTS:** ElevenLabs only.
- **Image generation:** Gemini image or fal.ai (Flux 2 Pro, Kling).
- **Retries:** 3 attempts on transient errors. A policy-blocked batch is bisected until the offending window is found and dropped (`main.py:2014-2081`).

## 5. Pipeline walkthrough (URL → published short)

1. **Submit.** `POST /api/process` (`app.py:5441`) validates the request and runs an SSRF check (`app.py:5531-5537`), queues the job, and later spawns `main.py` with env options (`app.py:5599-5717`).
2. **Ingest.** yt-dlp runs with explicit clients and a PO-token provider, trying routes in order direct → static proxy → paid proxy, with rotating cookie accounts (`yt_clients.py:96-123`, `main.py:924-1124`, `yt_account.py`).
3. **Early audio.** ASR and the clip pick can start on the audio track before the video finishes downloading (`main.py:2634-2665`).
4. **Transcribe.** `transcribe_backends.transcribe` (`transcribe_backends.py:732-759`), then `release_models()` (`main.py:1983-1990`).
5. **Select.** `get_viral_clips` (`main.py:2096-2286`) on the text path, or `get_visual_clips` (`main.py:2420-2517`) for silent footage.
6. **Layout pick.** `layout_picker`, only if `AUTO_LAYOUT` is set (`main.py:2717-2734`).
7. **Cut.** Each clip is cut with loudnorm (`ffmpeg_utils.py:365-392`).
8. **Reframe.** `reframe_v2.render` (`reframe_v2.py:669-936`):
   - TransNetV2 scenes;
   - classify each scene, then upgrade to SPLIT / SCREENCAST / INSET / WIDE;
   - TRACK analysis on sparsely decoded frames;
   - osrender single pass, or per-scene ffmpeg plus concat.
   
   If v2 fails, it falls back to the v1 OpenCV loop (`main.py:1680-1700`).
9. **Hook grounding** on screen content, using 3 frames (`hook_grounding.py:144-185`). Then hook placement, a 1-D search that avoids faces (`hook_placement.py:91-180`).
10. **Burn.** Auto captions (ASS) and the hook PNG are burned in one ffmpeg pass into a `hooked_` derivative (`main.py:1407-1469`).
11. **Report.** The child prints `CLIP_READY` after each clip (`main.py:2929`). The parent writes metadata and runs delivery outside the GPU slot: S3/R2 upload, signed webhook, emails (`app.py:3152-3163,3697-3777`).
12. **After the job (optional):**
    - `/api/edit` for AI effects (`editor.py`);
    - `/api/clip/rerender` to recut;
    - `/api/subtitle` to restyle captions;
    - `/api/translate` for dubbing;
    - `/api/social/post` to publish through Upload-Post (`app.py:8131-8234`).

## 6. Prompts and heuristics

**Score prompt** (`gemini_worker.py:254-294`, core quoted):
```
You are a senior short-form video strategist.
RANK these candidate windows by how well each would work as a standalone short.
- Score EVERY window in this batch: exactly one entry per input window ... Do not drop the weak ones
- `score` must be an integer from 0 to 100 ... use the whole range instead of clustering. Most windows
  of a normal video are not clippable, so reserve 70+ for the ones that pass the test below, and put weak
  filler, housekeeping, outros, rambling transitions and low-signal padding under 30 ...
- THE 2-SECOND TEST is the main criterion: would the first 2 seconds of this moment force a cold viewer
  (no context) to keep watching? Windows that only work with prior context score low.
- Prefer windows with strong hooks, conflict, surprise, outrage, emotion, novelty, big numbers, or a clear payoff.
- A window may carry `viewers_clipped` / `viewer_clip_views` ... strong evidence the moment lands ...
TRANSCRIPT_LANGUAGE / VIDEO_DURATION_SECONDS / WINDOWS_JSON
Return only: {"windows":[{"id","start","end","score":<0-100>,"reason"}]}
```

**Detail prompt** (`gemini_worker.py:296-372`, key rules):
```
CLIP RULES: Each clip must be {min_secs}-{max_secs} seconds ... Stay within the candidate window boundaries.
- THE 2-SECOND RULE: the clip MUST open on its strongest moment ...
- Do not cut in the middle of a word or phrase. No generic intros/outros unless they are the hook.
- STANDS ALONE: ... If it opens on a pronoun, a "that", a "so anyway", or an answer whose question was asked
  earlier, move the start back ... Fix this by moving the START earlier, never by cutting the ending short.
- HOW MANY: return {min_clips} to {max_clips} clips. Work through EVERY candidate window ...
- DIVERSITY: never return two clips that make the same point, tell the same story, or land the same joke ...
HOOK PLAYBOOK (viral_hook_text, max 10 words): Open question / Hot take / Number shock / Story loop / POV
- ABOUT THIS MOMENT, NOT THE VIDEO: the hook and the title name the concrete thing that happens inside this clip
COPY RULES: all text in TRANSCRIPT_LANGUAGE; descriptions 1-2 sentences + 3-5 hashtags; title <=100 chars;
predicted_score honest 0-100; why <=20 words.
```
The output schema is `DetailClipModel{start,end,source_window_id,predicted_score,video_description_for_tiktok,video_description_for_instagram,video_title_for_youtube_short,viral_hook_text,why}` (`gemini_worker.py:34-50`).

**Other prompts:**
- **Vision prompt** (`gemini_worker.py:70-87`): "This video has NO speech/audio — judge it purely by what you SEE … action, reveals, transformations … Cut on visual scene changes, never mid-motion".
- **Grounded hook** (`gemini_worker.py:99-128`): it names the on-screen app or document, and the hook "MUST mention the thing you named in `on_screen` … keep the strongest concrete fact".
- **Layout picker** (`gemini_worker.py:142-171`): a closed choice of none/screencast/split; "\"none\" is by far the most common correct answer … If you are unsure, answer \"none\"".
- **Editor EDL** (`editor.py:104-132`): "You do NOT write FFmpeg — you return an edit decision list, and a deterministic renderer applies it safely". Rules are 2–6 edits per 30 s, no overlapping zooms, and no zooms when the clip has captions.
- **Thumbnails** (`thumbnail.py:85-163`): two calls, brainstorm 25 titles then a "ruthless critic" that scores and picks.
- **Dead code:** `GEMINI_PROMPT_TEMPLATE` (`main.py:55-92`) and `gemini_worker.main()` are unused. `GEMINI_THINKING_SCORE` and the per-stage temperatures are therefore dead knobs (`gemini_worker.py:537-642`).

**Key numeric heuristics:**

| Item | Value | Ref |
|---|---|---|
| Shortlist N | `max(3,min(10,dur//90+2))` | `clip_selection.py:82` |
| Clip count | low `max(2,min(6,n//2+2))`, high `min(12,max(4,2n))` | `clip_selection.py:52-68` |
| Snap | ±1.5 s search; lead ≤ 0.35 s, tail ≤ 0.45 s; +0.2 s on repair | `clip_selection.py:329-390` |
| Dedupe | ≥ 25% of the shorter clip | `clip_selection.py:286` |
| Camera | dead zone 0.25·crop_w; 3 / 15 px/frame; 3-frame jump confirm | `main.py:107,143,236-243` |
| SPLIT | coexistence 0.5, separation 0.20, min face width 0.045, scene ≥ 2.5 s, crop y = `cy-0.42·crop_h`, IoU 0.30 | `split_layout.py:38-68,174` |
| SCREENCAST | width 0.5–0.85, presenter in ≥ 3/6 samples, content side ≥ 0.30, band ≤ 0.55·H | `screencast_layout.py:45-72` |
| INSET | subject height ≤ 0.35, offset ≥ 0.18, spread ≤ 0.05, inset 0.10–0.38·H | `camera_inset.py:25-71` |
| GENERAL foreground | 0.42·H; WIDE zoom `min(1.4, 0.41·out_h/full_h)` | `reframe_v2.py:130`; `screencast_layout.py:512-539` |
| Speaker | 0.4 s windows, audio floor 0.18, margin 0.15, hold 3, min share 0.2 | `active_speaker.py:27-48` |
| Hook placement | top 0.06–0.80; bans caption band 0.70–0.88 (seam 0.42–0.58 on SPLIT); cost = face coverage + 0.05·\|y−0.20\|; shrink 1.0/0.85/0.72 | `hook_placement.py:33-180` |
| Caption safe margin | `MarginV=43` of PlayResY 288 (~15%) | `subtitles.py:225-230` |

**ASS generation** (`subtitles.py:464-510`). Header:
```
[Script Info]
ScriptType: v4.00+
PlayResY: 288
WrapStyle: 0
ScaledBorderAndShadow: yes
Style: Default,{font},{size*0.85},{primary},{primary},{outline},{back},1,0,0,0,100,100,0,0,{border_style},{outline_w},{shadow},{align},10,10,{margin_v},1
```
How the events work:
- Each word becomes one event, lasting from its start to the next word's start.
- The full block text is repeated in every event; the active word gets `{<prefix>}WORD{\r}`.
  - pop: `{\c<hl>\fscx90\fscy90\t(0,110,\fscx108\fscy108)}`
  - glow: `{\c&HFFFFFF&\3c<hl>\bord..\blur4}`
  - highlight: `{\rActive}`, a second style with BorderStyle 3; the text on the box turns black when the box luminance is > 0.6.
- With reveal, unspoken words get `{\alpha&HFF&}`, so the line does not reflow.
- Split-screen events get an `{\an5}` prefix to sit on the seam.
- Inactive words are dimmed by scaling RGB, not with alpha, so the outline does not bleed through (`subtitles.py:332-348`).
- `PlayResX` is unset; how libass scales horizontally in that case is UNVERIFIED.

**Caption presets** (`subtitles.py:261-288`):
- default: Anton 44, #FFE500 pop, border 4
- hormozi: Montserrat ExtraBold 44, #FFE500 pop, reveal, max 9 chars, shadow 2
- pill: Montserrat 44, #7C3AED highlight
- lime: Montserrat 44, #A3FF12 highlight
- oneword: Anton 70, 1 char/word
- clean: Montserrat 34, no effect, base opacity 0.7

## 7. FFmpeg commands and filtergraphs actually used

**Cut with loudness normalisation** (`ffmpeg_utils.py:365-373`). On the GPU it adds `-hwaccel cuda -hwaccel_output_format cuda` (`:382-384`).
```
ffmpeg -y -ss <start> -to <end> -i <src> <INTERMEDIATE> -af loudnorm=I=-14:TP=-2.0:LRA=11 -c:a aac <out>
```

**Encoder tiers** (`ffmpeg_utils.py:26-52`):

| Tier | x264 | NVENC |
|---|---|---|
| QUALITY | `-preset medium -crf 18` | `-preset p3 -tune hq -rc vbr -cq 25 -b:v 0 -spatial-aq 1 -temporal-aq 1 -pix_fmt yuv420p` |
| DELIVERY | `-preset fast -crf 22` | `-preset p3 -rc vbr -cq 29 -b:v 0 -spatial-aq 1` |
| INTERMEDIATE | `-preset fast -crf 18` | `-preset p1 -rc vbr -cq 19 -b:v 0` |

The x264 tiers set no `-pix_fmt`.

**TRACK** (`reframe_v2.py:917-921`). The sendcmd file holds lines like `0.1333 crop@c x 812;`, written only when x changes.
```
-filter_complex "[0:v]sendcmd=f='cmd_000.txt',crop@c=w=W:h=H:x=X0:y=Y,scale=OW:OH,setsar=1[v]"
```

**GENERAL (blurred backdrop)** (`reframe_v2.py:180-186`, `ffmpeg_utils.py:268-273`). The blur runs at quarter size; the comment reports SSIM 0.994–0.998 against a full-size blur and about 45% less CPU.
```
[0:v]split=2[bga][fga];
[bga]scale=-2:{oh/4},crop=w=min(iw\,{ow/4}):h={oh/4},gblur=sigma=3,scale={ow}:{oh}[bg];
[fga]scale=-2:{fg_h},crop=w=min(iw\,{ow}):h=ih[fg];
[bg][fg]overlay=x=(W-w)/2:y=(H-h)/2,setsar=1[v]
```

**SPLIT** (`split_layout.py:192-203`):
```
[0:v]split=2[ta][ba];[ta]crop=..,scale={ow}:{half_h}[top];[ba]crop=..,scale={ow}:{half_h}[bot];
[top][bot]vstack=inputs=2,pad={ow}:{oh}:0:0,setsar=1[v]
```

**SCREENCAST** (`screencast_layout.py:149-156`):
```
[0:v]split=3[bga][ca][sa];[bga]<blur σ14>[bg];[ca]crop=w=cw:h=H:x=cx:y=0,scale=sw:sh[content];
[sa]crop=..,scale={ow}:{speaker_h}[speaker];[bg][content]overlay=x=(ow-sw)/2:y=0[top];
[top][speaker]overlay=x=0:y=sh,setsar=1[v]
```

**Concat and mux, with metadata scrub** (`reframe_v2.py:621-639`):
```
ffmpeg -f concat -safe 0 -i concat.txt -i clip.mp4 -map 0:v:0 -map 1:a:0? -c:v copy -c:a copy \
  -map_metadata -1 -map_chapters -1 -map_metadata:s:v -1 -map_metadata:s:a -1 -movflags +faststart out.mp4
```

**Captions** (`subtitles.py:636-704`). `filename=` is named because FFmpeg 8 rejects a quoted positional value followed by more options. Free-plan jobs use `-filter_complex "[0:v]<vf>,split=2[s][sm];<watermark>"` to write the clean and watermarked files from one decode.
```
ffmpeg -y -i in.mp4 -vf "ass=filename='<p>':fontsdir='<repo>/fonts'" -c:a copy <QUALITY> <SCRUB> -movflags +faststart out.mp4
```

**Hook overlay** (`hooks.py:656-668`):
```
[0:v][1:v]overlay=x:y:enable='between(t,0,dur)'
```

**Watermark** (`main.py:1581-1585`):
```
[1:v]scale={0.30*vw}:-1,format=rgba,colorchannelmixer=aa=0.85[wm];[0:v][wm]overlay=x={0.05*vw}:y={0.40*vh}
```

**AI effects** (`edit_builder.py:94-155`):
- color_pop: `eq=contrast=1+0.15s:saturation=1+0.6s:enable='between(t,a,b)'`
- bw_moment: `hue=s=0`
- flash: `eq=brightness=0.35`
- vignette: `vignette=angle=PI/4.5`
- Zooms become one `zoompan=z='1+Σ…':x='iw/2-(iw/zoom)/2':y='ih*0.45-(ih/zoom)/2':d=1:fps=..:s=WxH`.

Execution checks (`editor.py:367-414,506-554`):
- The filter string must only use `_ALLOWED_FILTERS = {eq, zoompan, vignette, hue, setsar, format, scale, fps, null}` and must not contain `@ \ [ ] ;`.
- It is dry-run on 2 s with `-f null`, and the environment is scrubbed of credentials.
- One LLM self-repair attempt is allowed.

**Vision proxy** (`main.py:2329-2340`):
```
-ss/-to ... -vf scale=-2:360,fps=1 -c:v libx264 -preset veryfast -crf 30 -ac 1 -b:a 48k
```

**`bin/ffmpeg` wrapper** (`bin/ffmpeg:23-106`):
- Adds `-threads N` and `-filter_threads`/`-filter_complex_threads` from `FFMPEG_THREADS=2`.
- Limits NVENC sessions with a `flock` on `.nvenc-$i.lock`, waiting up to 600 s.

## 8. Dependencies and external services

- **Paid APIs:**
  - Gemini (`GEMINI_MODEL` default `gemini-3.1-flash-lite`; thumbnails `gemini-3.7-flash` + `gemini-3.1-flash-image`) (`main.py:2122`, `thumbnail.py:16-17`)
  - ElevenLabs: dubbing and TTS (`translate.py`, `saasshorts.py:788-848`)
  - fal.ai: Flux 2 Pro, Kling Avatar v2, Hailuo 2.3, veed/lipsync (`saasshorts.py:584-976`)
  - Upload-Post: TikTok, Instagram and YouTube publishing, analytics, scheduling (`app.py:8179-8636`)
  - Twitch Helix (`twitch_signals.py:177-245`)
  - AWS S3 / Cloudflare R2 (`s3_uploader.py`)
  - Paid per-GB proxy for YouTube downloads (`PROXY_URL`)
- **Local models** (none are committed to the repo):
  - `yolov8n.pt`, downloaded at image build (`Dockerfile:159`)
  - MediaPipe BlazeFace (bundled in the pip wheel)
  - faster-whisper weights, downloaded from Hugging Face at runtime
  - `istupakov/parakeet-tdt-0.6b-v3-onnx` (`transcribe_backends.py:346`)
  - TransNetV2 weights, from the `transnetv2-pytorch` wheel
  - Silero VAD, through onnx-asr
- **GPU:** optional. NVENC, NVDEC, CUDA (osrender needs `GPU=1`), and CUDA 12/13 user-space libraries in the GPU lock (`requirements-gpu.lock`). A job is admitted only when ≥ 4500 MB VRAM is free (`app.py:2038`).
- **Node:** Remotion 4 render service (Chromium) (`render-service/Dockerfile`), and Deno + the bgutil PO-token provider for yt-dlp (`Dockerfile:95-127`).
- **Platform:** POSIX-only. It uses `fcntl` and `os.nice` (`yt_account.py:41`, `app.py:4536`), and ASR host slots do nothing on Windows (`transcribe_backends.py:86-90`).

## 9. Performance notes

- **Sparse analysis decode:** frames are decoded only at the detection stride, at ≤ 640 px (`reframe_v2.py:286-347`).
- **Single-pass GPU render:** osrender removes the per-scene encode plus concat. It is gated on ≥ 2500 MB free VRAM and 8-bit h264/hevc 4:2:0 input (`gpu_decode.py:28-114`).
- **Kernels are not optimised.** Weights are recomputed for every pixel, there is no shared memory, and `__expf` is called per tap (`osrender/src/kernels.cu:31-60`).
- **Cheaper blur:** the quarter-resolution backdrop blur saves about 45% CPU (`ffmpeg_utils.py:263-266`).
- **Early audio** overlaps ASR and LLM work with the video download. Clips render best-first, 6 in parallel, and each is announced as soon as it is done.
- **Parakeet** is about 2× faster than whisper-turbo on GPU (claim in `.env.example:32-35`, not measured). Its VRAM was tuned from 6.1 to 4.6 GB with VAD batch 4 (`transcribe_backends.py:264-272`).
- **Gemini cost:** frame-based prompts send 12 × 1024 px stills instead of uploading the video. Per-call cost is tracked (`clip_selection.py:10-27`).
- **Hook placement** samples only the first 5 s at 6 frames (`hook_placement.py:54-57`).

## 10. Weaknesses, bugs and limitations

1. **Camera is crude:**
   - constant 3 or 15 px/frame in **source** pixels, so the pan speed depends on resolution and fps;
   - the subject is never re-centred inside the 25% dead zone;
   - there is no vertical tracking;
   - comments contradict the code ("20%" vs 0.25, "2 px" vs 3.0) (`main.py:141-143,236-243`).
2. **TRACK is not speaker-aware.** It picks the largest face. `stabilization_threshold`/`locked_counter` are dead code (`main.py:279-384`). Active-speaker detection runs only on SPLIT scenes and is off by default (`active_speaker.py:32-33`).
3. **No output QC at all.** There is no blackdetect, freezedetect, ebur128 or silence check. The only checks are a cut ≥ 1024 bytes and the osrender frame count (`ffmpeg_utils.py:392`, `gpu_render.py:434`). `quality_probe.py` is a pre-download yt-dlp height probe (`quality_probe.py:1-10`).
4. **Generation loss:** 3–4 lossy encodes per clip. The x264 tiers set no `-pix_fmt yuv420p` (`ffmpeg_utils.py:26-31`).
5. **Effects editor time base is wrong.** It receives the full source transcript in **absolute source times**, while the clip starts at 0, so effects land in the wrong place (`main.py:2845`, `app.py:6418-6424`, `editor.py:121`).
   - Also, Gemini uploads in the editor are never deleted (`editor.py:68-85`, `app.py:6405`).
6. **The clip-count floor can be undercut,** because dedupe runs after the floor (`main.py:2221-2258`). Duplicate window IDs from the model can take two shortlist slots (`main.py:2185`).
7. **Provider lock-in:** only 2 of about 12 LLM call sites can use OpenAI-compatible servers. STT is local-only, TTS is ElevenLabs-only.
8. **Monoliths and in-memory state:**
   - `app.py` is 10k lines and the `jobs` dict lives in memory;
   - `webhook_sent` and rate limits are in-memory too;
   - the code reaches into the private `job_queue._queue` (`app.py:902,955,3766,8095`).
9. **Progress has no structure.** stdout carries both logs and control markers, and progress is regex-parsed (`app.py:4435-4505`, `log_view.py:25-40`).
10. **Two Remotion copies have drifted apart:** `remotion/` vs `dashboard/src/remotion/`. `/api/render` has no UI caller, and the render service keeps jobs in memory (`render-service/src/server.ts:22`).
11. **PANEL layout is dead code** (not imported). On the ffmpeg path, punch-in may truncate segments, because crop sizes changed by sendcmd stop the stream. The repo's own test comment says so (`tests/test_gpu_render_equivalence.py:117-119`). UNVERIFIED by running.
12. **Agent and doc drift:**
    - SKILL.md says "seven tools" but the server has 8 (`skills/openshorts/SKILL.md:27-29`);
    - the n8n HMAC recipe cannot match, because Python's `json.dumps` spacing differs from `JSON.stringify` (`app.py:3776` vs `examples/n8n/README.md:28-33`);
    - `/api/social/post` turns every error into a 500 and leaks a server path (`app.py:8160-8234`).
13. **Audio is minimal:** no silence trimming, music or ducking. Dubbing blocks a thread for up to 10 minutes and cannot be resumed (`translate.py:255-280`).

## 11. License and reuse notes

- **Repo:** MIT, "Copyright (c) 2024 OpenShorts" (`LICENSE:1-3`).
- **`cloud/` is not MIT.** NOTICE carves it out under the "OpenShorts Commercial License" (`NOTICE:4-8`). That licence permits personal or internal use and forbids offering it as a hosted or paid service, or redistributing it for a fee (`cloud/LICENSE:1-40`). It covers auth, billing, metering, autopilot, OAuth for MCP and queue modes. **Do not copy anything from `cloud/`.**
- **Ultralytics YOLOv8 is AGPL-3.0** *(external knowledge; not stated in repo)*. `main.py:15` imports it unconditionally, and `yolov8n.pt` (AGPL weights) is fetched at build (`Dockerfile:159`). This is a copyleft trap: our Rust system must not use Ultralytics or its weights. Use a permissive person detector instead, or face-only detection.
- **MediaPipe:** Apache-2.0. **faster-whisper:** MIT. **TransNetV2:** MIT. **Parakeet-tdt-0.6b-v3:** CC-BY-4.0 (attribution required). **Silero VAD:** MIT. All *(external knowledge)*.
- **Remotion** (`remotion/package-lock.json:3381-3384`, `"license": "SEE LICENSE IN LICENSE.md"`) has its own licence: free for individuals and small companies, a paid company licence otherwise *(external knowledge)*. No licence key is configured. Avoid Remotion in our stack, or budget for the licence.
- **`osrender/Cargo.toml`** declares `license = "MIT"`. It links FFmpeg through `ffmpeg-sys-next`, so the FFmpeg build's LGPL/GPL status applies to the binary. Debian's ffmpeg is GPL-enabled *(external knowledge)*.
- **Font licences** (sizes in bytes):

| File | Size | Licence evidence |
|---|---|---|
| `fonts/Montserrat-ExtraBold.ttf` (+ `dashboard/public/fonts/`) | 455,468 | OFL 1.1, `fonts/Montserrat-OFL.txt` present |
| `fonts/Anton-Regular.ttf` (+ dashboard copy) | 170,812 | **No licence file** in repo (Anton is OFL upstream, *external knowledge*) |
| `fonts/NotoSerif-Bold.ttf` (+ `remotion/public/fonts/`) | 638,164 | **No licence file** (Noto is OFL upstream, *external knowledge*) |
| `dashboard/public/fonts/{geist,instrument-serif,jetbrains-mono,noto-serif}-*.woff2` | 7.8–65.7 KB each | No licence file. `dashboard/public/fonts.css:7` says "OFL-licensed WOFF2 subsets" |

  OFL requires the licence text to travel with the font, and the repo is out of compliance for everything except Montserrat. If we ship these fonts, take them from upstream with their OFL texts.
- **Large binary and media assets.** None are ML models; no `.onnx`, `.pt` or `.tflite` files are committed.

| Path | Size | Notes |
|---|---|---|
| `churchil_queen_vertical.gif` | 65.4 MB | demo GIF, README media |
| `screenshots/split-before-after.gif` | 6.2 MB | demo |
| `demo-openshorts.mp4` | 5.6 MB | demo |
| `churchil_queen_vertical_short.gif` | 2.7 MB | demo |
| `screenshots/*.png` | 0.26–2.9 MB | UI screenshots |
| `dashboard/public/demo/*.mp4` | 20 files, 88–552 KB | landing-page demos, including caption-style samples |
| `osrender/src/kernels.ptx` | 24.9 KB | compiled CUDA PTX (generated from `kernels.cu`) |
| `assets/watermark.png` | 66 KB | free-plan watermark |

  No licence is stated for the demo media. They appear to contain third-party footage (Churchill/Queen archival footage), so their rights status is **UNVERIFIED**. Do not reuse them. `.git` is 87 MB, the working tree 95 MB.

## 12. Steal list (ranked)

1. **Two-pass global-rank clip selection** (`clip_selection.py` + `main.py:2096-2286`) — **port to Rust.**
   - Segment-aligned overlapping windows → score every window in near-equal batches → global top-N shortlist → detail pass with a floor enforced in code (re-ask only the unused windows) → trim by score → word-snap with `min(cap, gap/2)` silence padding → dedupe by overlap share.
   - Copy the SCORE and DETAIL prompts almost verbatim: the 2-second test, "stands alone", diversity, and "about this moment, not the video".
   - Pure logic plus one JSON-schema LLM call per stage maps cleanly onto our OpenAI-compatible provider trait.
2. **Per-word ASS karaoke generator** (`subtitles.py:351-518`) — **port line for line.**
   - One event per word ending at the next word's start; the full block repeated in each event; active-word override + `{\r}`; `\alpha&HFF&` reveal; a second `Active` style with BorderStyle 3 for pill/box plus the luminance contrast check; dimming by scaling RGB; per-event `{\an5}` on split seams.
   - Also the presets table as data, the per-font line budget, and the ASS escaping and font-name whitelist.
   - Burn with `ass=filename=..:fontsdir=..` plus a fontconfig alias file (`fonts/openshorts-fontmap.conf`).
3. **Layer model + `osrender` architecture** (`gpu_render.py:232-326`, `osrender/src/main.rs`, `kernels.cu`) — **reimplement.**
   - Express every layout as `(src_rect, dst_rect, blur?)` layers. Generate the FFmpeg filtergraph *and* the GPU plan from the same layers, so the two renderers cannot diverge. Keep an SSIM equivalence test.
   - The osrender code is already Rust and MIT. Its NVDEC → CUDA bicubic (B=0, C=0.6) + separable Gaussian → NVENC loop, with PTX embedded and `libcuda` loaded at runtime via `libloading`, is a direct template for a later GPU backend. Ship FFmpeg-only first.
4. **Agent surface design** (`mcp_server.py`, `SKILL.md`, `cli/openshorts_cli.py`) — **reimplement in rmcp + clap.**
   - One engine behind all surfaces. Coarse outcome-level tools with annotations and a `hint` for the next step.
   - Async jobs with poll or webhook, signed absolute media URLs, typed "not a failure" outcomes (`needs_confirmation`, quota, 429), and recut in **source-time segments**, so the same EDL serves the agent and the GUI.
   - A skill that teaches judgement ("the choices the API cannot make for you").
   - Improve on it: typed NDJSON progress events instead of log regexes, plus finer-grained edit tools.
5. **Layout heuristics and constants** (`split_layout.py`, `screencast_layout.py`, `camera_inset.py`, `active_speaker.py`, `reframe_v2.py`) — **port the constants and geometry; replace the camera.**
   - Port: SPLIT coexistence, separation and face-width guards with crop y = `cy-0.42·crop_h`; screencast and inset detection thresholds; the motion-centred WIDE crop; the quarter-resolution blur backdrop.
   - Port the active-speaker pipeline: mouth-ROI frame difference, audio RMS gate 0.18, per-speaker 20/80 percentile normalisation, margin 0.15, 3-window hold.
   - Keep the jump-confirmation (3 frames) and snap-on-cut behaviour from `SmoothedCameraman`. Replace its constant-speed pan with critically damped motion normalised to resolution and fps, with re-centring.
6. **LLM EDL → deterministic filter compiler with allowlist and dry-run** (`edit_builder.py`, `editor.py:367-414`) — **reimplement as a typed Rust enum.** Drop the free-text "repair the filter" path, and fix the time-base bug by passing clip-relative transcripts.
7. **Hook placement as 1-D optimisation** (`hook_placement.py:91-180`) — **port.** Face-coverage cost plus a move penalty, banned caption bands, shrink steps. Combine with frame-grounded hook rewriting for screen content (`hook_grounding.py`).
8. **Encoder tiers and ops tricks** — **copy as config:**
   - NVENC `cq ≈ crf+7` mapping and the p1/cq19 intermediate tier;
   - single-pass loudnorm `I=-14:TP=-2:LRA=11` applied once at the cut;
   - sparse `select=` decode built as a balanced expression tree;
   - NVENC slot flock;
   - sendcmd lines written only on change;
   - metadata scrub plus `+faststart`.
9. **STT fallback rules** (`transcribe_backends.py:573-709`) — **port the logic.** Run Parakeet first and fall back to whisper on 0 words, an unsupported language or < 0.2 words/s, plus the wrong-alphabet segment repair. Map onto whisper-rs or a sidecar plus an OpenAI-compatible STT endpoint.
10. **Burned-subtitle band detection and crop** (`burned_subtitles.py:34-93`), the **Twitch viewer-clip prior** (`twitch_signals.py:85-141`), and the **thumbnail text-fit loop** (`thumbnail.py:459-516`) — smaller ports.

## 13. Gaps (what OpenShorts lacks that we need)

- **Provider abstraction:** we need OpenAI-compatible LLM, vision, STT and TTS at every call site. OpenShorts is Gemini-only for vision, ElevenLabs-only for TTS, and local-only for STT.
- **Smooth camera:** easing or critically damped motion, vertical tracking, re-centring, and speaker-aware TRACK (active speaker beyond SPLIT, more than 2 people).
- **Output QC:** blackdetect, freezedetect, ebur128 loudness verification, caption-overflow and safe-zone checks, frame-count and A/V-sync checks, and LLM/vision review of the rendered clip. None of these exist.
- **Audio:** silence and filler-word removal, music beds with ducking (`sidechaincompress`), SFX, denoising.
- **Creative:** template/brand-kit system, transitions, B-roll insertion (only SaaSShorts generates it), animated emoji and keyword emphasis in captions, multi-line wrapping.
- **Fine-grained agent tools:** set the layout per scene, edit caption words and timings, choose fonts, preview a frame, run QC, and structured progress events. Today an agent can only process, recut, restyle and publish.
- **Desktop and offline:** OpenShorts is a Linux, Docker, server-centric SaaS. There is no single-binary local app, and Windows support is broken (`fcntl`, `os.nice`).
- **Avoiding encode generations:** compose captions, hooks and effects in the single render pass instead of re-encoding 3–4 times.
- **Diarization** for multi-speaker captions and speaker labels.
