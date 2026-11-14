# ViralCutter — Findings

Repo: `repos/ViralCutter` (RafaelGodoyEbert/ViralCutter per README badges, v0.8 Alpha: repos/ViralCutter/README_en.md:73). Evaluated per `docs/research-methodology.md`. Citations are `repos/ViralCutter/<path>:<line>`. Anything not verified in source is marked **UNVERIFIED**.

## 1. Summary
- Local, Python-only Opus-Clip clone: yt-dlp download -> WhisperX (transcribe + wav2vec2 forced alignment) -> chunked LLM prompt for "viral" segments -> FFmpeg cut -> per-frame OpenCV face-tracked 9:16 crop (InsightFace buffalo_l, MediaPipe/Haar fallback) -> ASS word-highlight captions burned with FFmpeg `subtitles=` filter. Gradio WebUI that shells out to the CLI; a separate CustomTkinter "Subtitle Studio"; a 3200-line JS subtitle timeline editor; Premiere XML export.
- **LICENSE IS GPLv3, NOT MIT** (repos/ViralCutter/LICENSE:1-2, "GNU GENERAL PUBLIC LICENSE Version 3, 29 June 2007"). No other license statement in README (grep for "licen" in README.md / README_en.md returned nothing). Code must not be copied into a non-GPL Rust codebase; ideas/algorithms/thresholds can be reimplemented.
- Verdict: valuable as a **reference for the data contract and the heuristics** (time-tagged transcript prompt, text-anchored timestamp alignment, face-count/dead-zone/crowd logic, ASS word-highlight recipe, presets). Engineering quality is low-to-medium (god-functions, monkeypatching, triple re-encode, dead/buggy code, no tests, secrets hygiene issues). Not a design to copy structurally.

## 2. Stack & architecture
Python 3.12 venv via `uv` (repos/ViralCutter/install_dependencies.bat:11-12), torch 2.5.1 cu124 + onnxruntime-gpu 1.20.1 (install_dependencies.bat:23-25). `requirements.txt`: whisperx, mediapipe, insightface, g4f[all], google-genai, google-generativeai, yt-dlp, ffmpeg-python, gradio, opencv-python, deep-translator, fastapi/uvicorn (repos/ViralCutter/requirements.txt:1-16). `llama-cpp-python` is optional (import guard at create_viral_segments.py:27-31; only the "advanced" installer pulls it, UNVERIFIED body of install_dependencies_advanced_LocalLLM.bat not read).

```
WebUI (webui/app.py, Gradio)  --subprocess + CLI flags + stdout parsing-->  main_improved.py (CLI orchestrator)
                                                                              |
 download_video -> transcribe_video -> create_viral_segments -> cut_segments -> edit_video(face crop)
   (yt-dlp)        (WhisperX)            (LLM + align)           (ffmpeg)       (cv2 + insightface + ffmpeg pipe)
        -> [translate_json] -> adjust_subtitles (JSON->ASS) -> burn_subtitles (ffmpeg subtitles=)
 All state = files in VIRALS/<project>/ : input.mp4, input.{srt,tsv,json}, viral_segments.txt,
 cuts/, final/, subs/, subs_ass/, burned_sub/, process_config.json, face_modes.json
```
Key architectural fact for us: **the WebUI drives the pipeline purely by spawning `main_improved.py` with ~40 CLI flags and scraping "Project Folder:" from stdout** (repos/ViralCutter/webui/app.py:214-335, 341-343). There is no library API, no JSON-RPC, no structured events. State is implicit in folder naming (`NNN_Title_*.mp4/json`) and is renamed in several places (edit_video.py:1234-1288, main_improved.py:594-623) — fragile.

## 3. Repo map (important only)
| Path | Role |
|---|---|
| main_improved.py (726 lines) | CLI + pipeline orchestration, subtitle default config, saves process_config.json |
| prompt.txt | LLM prompt template (the only prompt file) |
| api_config.json | selected_api, gemini key/model/chunk_size, g4f chunk_size |
| scripts/create_viral_segments.py (792) | chunking, LLM calls (gemini/g4f/llama.cpp/manual), JSON repair, text-anchored timestamp alignment, dedupe |
| scripts/transcribe_video.py (352) | WhisperX transcribe+align, SRT/VTT "alignment-only" path |
| scripts/cut_segments.py (167), cut_json.py (66) | ffmpeg cut; word-JSON slicing to clip-relative times |
| scripts/edit_video.py (1303) | the reframer (InsightFace/MediaPipe/Haar/center) + encoder detection + mux |
| scripts/face_detection_insightface.py, one_face.py, two_face.py | detectors + crop math |
| scripts/adjust_subtitles.py (263), burn_subtitles.py (107) | ASS generation, burn |
| scripts/translate_json.py (343) | caption translation via Google Translate scrape |
| scripts/export_xml_lib/*, old_export_xml_lib/* | Premiere XML pack (duplicated "old" tree is dead weight) |
| subtitle_studio.py (2391) | standalone CustomTkinter caption designer, presets, alpha/PNG-sequence render, Premiere graphics XML |
| webui/app.py (1088), subtitle_handler.py, library.py, subtitle_editor.py | Gradio UI, presets, project gallery |
| webui/advanced_editor/{index.html,script.js(3206),style.css} | JS subtitle timeline editor w/ waveform, served via FastAPI routes |
| i18n/ | en_US, pt_BR, tr_TR JSON locales |

## 4. Capabilities A–I (quality 1-5)

### A. AI clipping — 3
- Single LLM pass over chunked transcript with embedded time tags; the model returns text anchors, not timestamps; code resolves anchors to real transcript timestamps (the best idea in the repo). See §5/§6.
- Fields requested per segment: `start_text`, `end_text`, `start_time_ref`, `title`, `reasoning`, `score` (create_viral_segments.py:578-591). `hook` is just a copy of `title` (create_viral_segments.py:452).
- Duration clamp + dedupe: min extended / max truncated (create_viral_segments.py:433-443); overlap >5 s with a higher-scoring segment dropped (create_viral_segments.py:462-481); sort by score desc (create_viral_segments.py:345,462).
- **Bug**: user "themes" are never used when `prompt.txt` exists, because `prompt.txt` has no `{virality_instruction}` placeholder (repos/ViralCutter/prompt.txt:1-42) while the themes text is only injected via that variable (create_viral_segments.py:621-624, 635). Theme mode silently degrades to generic mode. Marked as verified by reading both files.
- Score is LLM self-reported 0-100 (example value 95 in template, create_viral_segments.py:590); no independent scoring, no audio/visual signals, no narrative/hook classification beyond the prompt.
- Cross-chunk selection: each chunk asks for `{amount}` segments, all are pooled, then globally sorted and truncated to N (create_viral_segments.py:487-490 region, process_segments output_count).

### B. Transcription & subtitles — 3
- WhisperX: `whisperx.load_model(model_name, device, compute_type, asr_options={"hotwords": None})`, `model.transcribe(audio, batch_size=16, chunk_size=10)` (transcribe_video.py:263-274); then `whisperx.load_align_model(language_code)` + `whisperx.align(..., return_char_alignments=False)` (transcribe_video.py:293-295). Default model `large-v3-turbo` (main_improved.py:112). compute_type float16 on CUDA else float32 (transcribe_video.py:206). Outputs SRT, TSV, JSON via whisperx writers (transcribe_video.py:330-337).
- **Fast path**: if YouTube subs were downloaded (`input.srt`/`input.vtt`), skip ASR and run alignment only on the parsed subtitle text (transcribe_video.py:217-258, 285-295). Language is **hard-forced to `en`** for this path (transcribe_video.py:245-247) — wrong for non-English videos (README sells PT/translation). YouTube VTT karaoke cleanup in download_video.py:242-326.
- Word timestamps drive captions; per-clip word JSON is made by slicing the full transcript JSON, shifting times to clip-relative (cut_json.py:4-44).
- No diarization/speaker labels, no emoji, no per-word confidence use (WhisperX `score` field is carried but unused), no STT provider abstraction (WhisperX only, local).
- Hack: monkeypatches `torch.load` to `weights_only=False` globally and `torchaudio.list_audio_backends` (transcribe_video.py:29-50) — security smell + version-pin fragility.
- Translation: Google Translate unofficial scrape through `deep_translator`, chunks <=4999 chars joined by a " ◌ " separator, 7 concurrent, infinite 30 s retry loop (translate_json.py:1-45, 171). Word-level highlight preserved after translation per changelog (repos/ViralCutter/changelog.md, "Tradução de legendas em JSON..."); mechanics in `adjust_segments` translate_json.py:120 (not deeply verified).

### C. Creative design — 3
- Caption styling is only ASS: font, base/highlight size+color, outline/shadow, bold/italic/underline/strike, uppercase, BorderStyle 1/3, alignment, margin-V, words-per-block, gap limit, modes `highlight | word_by_word | no_highlight` (main_improved.py:56-77, adjust_subtitles.py:164-183).
- Named presets: about 11 Python dict presets in webui/subtitle_handler.py:14-280 (e.g. "MrBeast Clean Hook", "Hormozi (Classic)": Montserrat-ExtraBold, 30/35 pt, white/green `#00FF00`, outline 3, uppercase, 2 words per block, vertical 200 — webui/subtitle_handler.py:40-62), and 7+ different fallback defaults in Subtitle Studio (the `subtitle_presets.json` it loads is not in the repo) (subtitle_studio.py:2142-2220: "MrBeast (Impacto)", "TikTok (Caixa Preta)", "Hormozi (Amarelo/Bold)", "Hacker (Terminal)", "Netflix"). Two disjoint preset systems.
- No animation (no `\t`, `\fad`, `\move`, pop/scale-in), no gradients, no emoji, no transitions, no B-roll, no hook overlays/title cards, no music. `highlight` is "redraw whole block per word with active word bigger+recolored" (see §7), which causes layout reflow on every word since `\fs` changes (inferred from adjust_subtitles.py:168-170).
- Fonts: **no font files shipped** (repo listing has none); default font name `Montserrat-Regular` must be installed on the system (main_improved.py:57); `subtitles=` filter has no `fontsdir` (burn_subtitles.py:20). Font availability unverified at runtime.

### D. Video processing & rendering — 2
- Cut: `ffmpeg -ss <start> -i in -t <dur> -c:v h264_nvenc|libx264 ...` re-encode (cut_segments.py:110-135). NVENC `-preset p1 -b:v 5M`; CPU `-preset ultrafast -crf 23`.
- Reframe: OpenCV decodes the cut, crops per frame in Python, writes raw frames into an `ffmpeg` stdin pipe in the fallback path (edit_video.py:117-135) but into `cv2.VideoWriter('mp4v')` in the face paths (edit_video.py:237-238, 392-393, 491-493); audio is extracted to AAC then muxed with a **second re-encode** at 5 Mb/s (edit_video.py:162-191); subtitles are burned in a **third re-encode** (burn_subtitles.py:17-28). So video is encoded up to 4x (cut, mp4v, mux, burn). Quality + speed loss; our design should do one filtergraph, one encode.
- Encoder auto-detect order: NVENC > AMF > QSV > VideoToolbox > libx264 via `ffmpeg -encoders` text match (edit_video.py:19-55). Burn step only tries NVENC then libx264 (burn_subtitles.py:33,40). Good small idea: capability probe + fallback chain.
- Output is fixed 1080x1920 (edit_video.py:111, one_face.py:33). No other aspect ratios (README roadmap lists "More framing formats" unchecked, README_en.md:~70).
- Batch: sequential per clip; no parallelism. Idempotence by file existence checks (transcribe_video.py:199-201; main_improved.py:530-546).
- Preview: Subtitle Studio streams `ffmpeg -re ... color=...,ass=...` into a Tk canvas via rawvideo pipe (subtitle_studio.py:729-738). Alpha overlay export (UT Video RGBA .avi, and QTRLE .mov per segment for Premiere) (subtitle_studio.py:1156-1165; export_xml_lib/rendering.py:36-41).

### E. Reframing — 3 (best part of repo; algorithm worth studying)
See §5.4 for full detail. Summary: detector = InsightFace `buffalo_l` (SCRFD det + landmarks) at det_size 640, ONNX CUDA/CPU (face_detection_insightface.py:55-56); fallbacks MediaPipe FaceDetection+FaceMesh+Pose, then OpenCV Haar, then center crop/pad (edit_video.py:1179-1229). Detect every ~0.17 s (1 face) / 1.0 s (2 faces) (main_improved.py:126; edit_video.py:937-955). Smoothing = dead-zone hold + short linear interpolation of bbox over 4 frames (edit_video.py:503, 877-931) — *no* Kalman/EMA (an `zoom_ema_bbox` variable is only ever reset, never used: edit_video.py:624). Multi-face: relative-area filter, 1-vs-2-face decision by 2nd/1st area ratio, 2 faces => vertical split-screen of two 1080x960 crops (two_face.py:81-103). Active speaker via mouth aspect ratio (68-pt landmarks) + motion score, marked experimental (edit_video.py:194-223, 627-706, 719-755).

### F. Audio — 1
No denoise, loudness norm, silence removal, music, SFX, ducking. Audio is just `-c:a aac -b:a 128k` at cut and 192k at mux (cut_segments.py:132-134; edit_video.py:176). README roadmap lists auto-duck as unchecked (README_en.md:~67).

### G. Agent integration — 2
- Closest thing to an agent surface is the CLI (`--skip-prompts`, explicit flags) (main_improved.py:103-141) and `process_config.json` written at the end (main_improved.py:676-711). Intermediate artifacts are JSON files on disk (`viral_segments.txt` is JSON despite `.txt`: save_json.py:31-33), which an agent could edit between steps: `--workflow 1|2|3` = Full / Cut-only / Subtitles-only (main_improved.py:121). Good property: **"Raw Segment Repair"** accepts hand/agent-written segments with only `start_text/end_text/start_time_ref` and re-aligns them (main_improved.py:492-518; changelog.md first section). 
- A `manual` backend prints the full prompt and asks the human to paste the JSON back (create_viral_segments.py:713-745 region) — effectively an agent-handoff protocol, though via stdin `input()` (blocking, unusable headless).
- No MCP, no structured stdout (logs are free text in Portuguese/English), no exit codes beyond 0/1, no progress events, no schema for outputs.

### H. GUI — 3
- Gradio dark UI: input source (YouTube URL / existing project / upload), AI backend + model dropdowns (models fetched live from Gemini API / g4f / local *.gguf dir) (webui/app.py:138-160, 431-607), face presets and "experimental" active-speaker presets (webui/app.py:33-47), subtitle preset dropdown + 20 style controls, project gallery (library.py), logs textbox streaming the subprocess stdout (webui/app.py:335-360).
- Advanced subtitle editor: standalone HTML/JS (3206 lines) with waveform canvas, zoom levels (word at 2.5x, char at 20x) (webui/advanced_editor/script.js:2-4, 41-42), reads/writes subtitles via FastAPI routes `api/subtitles`, `api/subtitles.vtt`, `api/subtitle-config` (script.js:680-704; webui/app.py:966-1056). Title says "DUB Editor" (index.html mobile header) — likely adapted from another project; provenance/license **UNVERIFIED**.
- CustomTkinter "Subtitle Studio" with live ASS preview via ffmpeg pipe (subtitle_studio.py:669-752).
- No timeline for the video itself, no clip trim UI, no reframe keyframe editor, no render queue. Premiere XML export (`/export_xml_api`) is the "editing" escape hatch (webui/app.py:944-960).

### I. Provider abstraction — 2
- Hard-coded `ai_mode` branches: `gemini` (google-generativeai, 5 retries with 429/"retry in Ns" parsing, create_viral_segments.py:199-230), `g4f` (gpt4free reverse-engineered free endpoints, 3 retries exp backoff, create_viral_segments.py:233-281), `local` (llama-cpp-python GGUF, `n_gpu_layers=-1, n_ctx=8192`, `temperature=0.7`, `max_tokens=4096`, system "You are a helpful assistant that outputs only JSON." create_viral_segments.py:690-697, 750-758), `manual` (copy/paste).
- **No OpenAI-compatible endpoint support at all** (no base_url/api_key/model triple); "GPT-4" in README is only reachable through g4f. README claims Gemini/GPT-4/local (README_en.md:~13) — GPT-4 via g4f is unofficial.
- Gemini API key passed on the command line (`--api-key`, main_improved.py:115, webui/app.py:273) => visible in process list; also a key slot lives in the tracked `api_config.json` (value empty in this checkout: length 0).
- Chunk sizes per provider differ: gemini 20000 (api_config.json), g4f 2000, local 3000 (create_viral_segments.py:540-559), but the WebUI always sends its own default `70000` (webui/app.py:476) overriding config.
- Only LLM is pluggable (poorly); STT and TTS are not; no TTS at all.

## 5. Pipeline walkthrough
1. **Download** `download_video.download()` (download_video.py:64): yt-dlp, format map `best=bestvideo+bestaudio/best`, `1080p=bestvideo[height<=1080]+bestaudio/best[height<=1080]` etc. (download_video.py:153-158); merge to mp4, `-movflags faststart`; also downloads `pt.*,en.*,sp.*` subs (manual+auto) converted to SRT (download_video.py:169, 184-188); cookies.txt or tries each browser's cookies (download_video.py:10-33); `force_ipv4`, hard-coded Chrome UA (download_video.py:171-176).
2. **Transcribe** `transcribe_video.transcribe()` -> `input.srt/.tsv/.json` (§4B).
3. **Segments** `create_viral_segments.create()`:
   - `load_transcript` reads TSV (ms) else SRT (create_viral_segments.py:284-330).
   - `preprocess_transcript_for_ai`: one string where a `(Ns)` tag is inserted after every segment whose end is >=4 s from the last tag (create_viral_segments.py:172-196).
   - Chunk by characters (default 15000 fallback / config), overlap `max(1000, 10%)`, break on spaces (create_viral_segments.py:593-616).
   - One prompt per chunk -> LLM -> `clean_json_response` (create_viral_segments.py:36-169): strips `<think>...</think>` (DeepSeek-R1), unescapes `\\n`, finds each `segments` keyword and `raw_decode`s the enclosing object, then `ast.literal_eval` with brace balancing, then ```json fence, then **partial-item recovery from truncated JSON** (item-by-item `raw_decode`). Robust; worth porting the idea.
   - **Alignment** `process_segments` (create_viral_segments.py:333-500): parse `(Ns)` from `start_time_ref` -> find nearest transcript segment start (scan stops 10 s past, :377) -> back off 5 segments (:381) -> search next 50 segments for one whose punctuation-stripped lowercased text contains/is contained in `start_text` (substring either way, :392-405) -> else fallback to the reference segment's start; then search up to 200 segments after for `end_text` the same way (:416-423) -> end time = that segment's end; if not found `start + min_duration` (:430). Clamp to [min,max] (:433-443). Granularity is whole Whisper segments (not words), so start/end are only segment-accurate; the exact first/last words are not snapped.
   - Dedup by overlap >5 s; keep top N (create_viral_segments.py:462-492).
   - Saved to `viral_segments.txt` (JSON) (save_json.py:31).
4. **Cut** `cut_segments.cut()` — ffmpeg per segment -> `cuts/NNN_<title>_original_scale.mp4` and `subs/NNN_<title>_processed.json` (words re-based to 0) (cut_segments.py:98-154). Quirk: `duration >= 1000` is interpreted as milliseconds (cut_segments.py:51-55); int `start_time` is treated as ms (:76-78) — loose typing contract.
5. **Reframe** `edit_video.edit()` (edit_video.py:1087) — per clip, in order InsightFace -> MediaPipe -> Haar -> center (edit_video.py:1179-1229). Details §5.4. Writes `<clip>_timeline.json` (1-face/2-face segments), `<clip>_coords.json` (per-frame bboxes + relative face height), `face_modes.json` (edit_video.py:1030-1084, 1294-1301).
6. **Translate (optional)** `translate_json.translate_project_subs` (main_improved.py:633-639).
7. **Subtitles** `adjust_subtitles.adjust()` JSON->ASS (adjust_subtitles.py:221-264), then `burn_subtitles.burn()` writes `burned_sub/<name>_subtitled.mp4` (burn_subtitles.py:52-107).
8. `process_config.json` written with every parameter used (main_improved.py:676-711) — nice reproducibility pattern.

### 5.4 Reframing algorithm (edit_video.py:477-1084)
- Detection cadence: every `step` frames where step = `int(interval*fps)`; defaults `0.17 s` (1-face state) and `1.0 s` (2-face state) (main_improved.py:126, 380-393; edit_video.py:937-955). No detection while a transition is running (`len(transition_frames)==0` guard at :541).
- Filters: confidence > `--face-confidence-threshold` (default 0.30; UI default 0.40) (edit_video.py:559; webui/app.py:542); relative-area filter `area > filter_threshold * max_area` (default 0.35) removes background people (edit_video.py:573-581).
- Crowd mode: if raw face count >= 7 (before size filtering) => reset tracking and fall back to no-face mode (zoom/pad) (edit_video.py:613-625).
- 1-vs-2 face decision in `auto`: if >=2 valid faces and `second_area > two_face_threshold(0.60) * largest_area` => 2-face split, else 1-face (edit_video.py:760-772). With `--focus-active-speaker`: score diff > 1.5 => focus the winner; both scores > 4.0 => split (edit_video.py:735-755).
- Active speaker score: per-face state matched to previous frame by nearest center (<200 px); +1.5 per detection if MAR > 0.03 else `-decay (2.0)`, clamped to [0,20]; optional motion bonus up to 2.5 after subtracting global motion = min motion across faces (edit_video.py:653-699). MAR = |lip pt62-pt66| / |pt60-pt64| on `landmark_3d_68` (edit_video.py:194-223). The `activity` key used for `effective_area` is never set (only `activity_score` is), so the area weighting is a no-op (edit_video.py:569-570 vs 701) — dead logic.
- Lookahead: if fewer faces than target, read one frame ahead, re-detect with conf > 0.50, use it if better (edit_video.py:783-813).
- Identity continuity: no tracker/ReID. For 1 face, pick the face minimizing `dist_to_last_center - 0.0001*effective_area` (edit_video.py:828-839); for 2 faces keep left/right-top/bottom ordering by pairing with min squared distance (`sort_by_proximity`, edit_video.py:65-95).
- Smoothing/stability: **dead zone** — if every face center moved <= `dead_zone` px (default CLI 40, UI default 150 px, UI presets 100-200) keep previous bbox exactly (edit_video.py:877-898); otherwise linearly interpolate bbox over `transition_duration = 4` frames (edit_video.py:503, 913-923). The older MediaPipe/Haar paths use a 1-second linear transition (edit_video.py:255, 404). Between detections the last bbox is held up to 3 s then falls back to center zoom/pad (edit_video.py:253, 334-343, 960-975).
- Crop geometry: 1-face = full source-height 9:16 window horizontally centered on face center, **no zoom** — vertical center only matters when source is not 16:9 (face_detection_insightface.py:87-145). 2-face = each face gets a 2.2x-face-size window at 1080:960 AR, clamped, stacked vertically (two_face.py:5-103). Fallbacks: `padding` (black letterbox into 1080x1920, `resize_with_padding` one_face.py:37-59) or `zoom` (center crop, one_face.py:111-138). Note: "padding" pads the canvas with black and does **not** do the blurred-background look common in shorts.
- Subtitle placement coupling: the frame-level mode timeline is saved and reused so captions move to screen center (`\an5\pos(180,320)` at PlayRes 360x640) during 2-face split segments (adjust_subtitles.py:185-208). Good cross-component idea.
- Performance: per-frame Python loop with cv2 read/resize/write; GPU only for InsightFace ONNX. Detection on every ~5th frame, but crop+resize+encode every frame.

## 6. Prompts & heuristics (quoted)

**Main prompt** — repos/ViralCutter/prompt.txt:2-42 (verbatim essentials):
```
You are a World-Class Viral Video Editor and Storyteller. You are an expert at finding "gold nuggets" in long transcripts that can stand alone as viral TikToks/Reels/Shorts.
Your goal is to extract segments that have **Perfect Narrative Completeness** (Start, Middle, End) while respecting time constraints.

### INPUT FORMAT EXPLAINED
The transcript below is a continuous text stream with embedded **Time Tags** like `(12s)`.
- Example: `"Hello world (0s). Today we are going to (3s) fly to the moon."`
- These tags represent the approximate timestamp of the *preceding* text.
- Use them to calculate duration. Duration = (End Tag - Start Tag).

### STRICT VIRAL RULES (The "ViralCutter Standard"):
1.  **NO "GARBAGE" STARTS:**
    -   NEVER start with filler words ("Um", "Uh", "So...", "And then").
    -   NEVER start with low-energy intros ("Hi guys, welcome back").
    -   **ALWAYS** start with a Hook: A strong statement, a question, or an action verb.
    -   *Bad Start:* "(10s) So, I was thinking..."   *Good Start:* "(12s) I almost died yesterday."
2.  **THE "STANDALONE" TEST:**
    -   Can this segment be shown to a stranger without ANY context?
    -   If it contains "That's why *he* said that" (and "he" is unknown), it FAILS. Eliminate or expand context.
3.  **NARRATIVE ARC:**
    -   **Start:** Hook (0-3s).  **Middle:** Value/Story (Retain attention).
    -   **End:** Punchline/Conclusion (Satisfying ending). NEVER cut mid-sentence.
4.  **DURATION MATH:**
    -   Use the `(XXs)` tags to estimate duration.
    -   CONSTRAINT: Segment MUST be between {min_duration}s and {max_duration}s.

### YOUR TASK:
Analyze the transcript below. Find {amount} potential viral segments.
TRANSCRIPT:
{transcript_chunk}
### OUTPUT FORMAT (JSON ONLY):
{json_template}
Return ONLY VALID JSON. No markdown, no commentary.
```
Placeholders used: `{min_duration} {max_duration} {amount} {transcript_chunk} {json_template}`; `{context_instruction}` and `{virality_instruction}` are passed by code (create_viral_segments.py:633-641) but **absent from prompt.txt** (so themes + "Part i of n" are lost).

**JSON template** — create_viral_segments.py:578-591:
```
{ "segments" : [ {
  "start_text": "Exact first 5-10 words of the segment",
  "end_text": "Exact last 5-10 words of the segment",
  "start_time_ref": "Value of closest (XXs) tag",
  "title": "Viral Hook Title (Same Language as Transcript)",
  "reasoning": "Why this is viral? Hook? Value? (Same Language as Transcript)",
  "score": 95 } ] }
```
**Built-in fallback prompt** (when prompt.txt missing) — create_viral_segments.py:567-576: "You are a World-Class Viral Video Editor. {context_instruction} Analyze the transcript below with time tags (XXs). Find {amount} viral segments. Constraints: Each segment MUST be between {min_duration} seconds and {max_duration} seconds. IMPORTANT: Output "Title", "Hook", and "Reasoning" in the SAME LANGUAGE as the transcript ... OUTPUT JSON ONLY:" — note the fallback is the only one that carries the language instruction; prompt.txt only says it inside the JSON template.

**Theme-mode instruction (unused in practice)** — create_viral_segments.py:621-624: "analyze the segment for potential virality and identify {N} most viral segments from the transcript" / "...the best parts based on the list of themes {themes}."

**Local-LLM system message**: "You are a helpful assistant that outputs only JSON." (create_viral_segments.py:754). Gemini/g4f get only the single user prompt.

**Heuristic numbers (all in code, not tunable via config unless noted)**
| What | Value | Where |
|---|---|---|
| time-tag spacing | every >=4 s of transcript | create_viral_segments.py:190 |
| chunk overlap | max(1000 chars, 10%) | create_viral_segments.py:595 |
| default duration window | 15-90 s | main_improved.py:110-111 |
| overlap dedupe | >5 s overlap drops lower score | create_viral_segments.py:476 |
| anchor search | back 5 segs, forward 50 (start), 200 (end) | create_viral_segments.py:381,392,416 |
| face conf / area filter / two-face ratio | 0.30 / 0.35 / 0.60 | main_improved.py:127-129 |
| dead zone | 40 px CLI (150 UI) | main_improved.py:130; webui/app.py:543 |
| crowd threshold | >=7 faces | edit_video.py:613 |
| hold-last-face timeout | 3 s | edit_video.py:253 |
| MAR threshold / score diff / decay | 0.03 / 1.5 / 2.0 | main_improved.py:132-137 |
| Face presets (UI) | Default(0.35,0.60,0.40,150), Stable(0.60,0.80,0.60,200), Sensitive(0.10,0.40,0.30,100), High Precision(0.40,0.65,0.75,150) | webui/app.py:34-39 |

## 7. FFmpeg commands/filtergraphs actually used
1. Cut (cut_segments.py:110-135):
   `ffmpeg -y -loglevel error -hide_banner -ss <start> -i input.mp4 -t <dur> -c:v h264_nvenc -preset p1 -b:v 5M -c:a aac -b:a 128k out.mp4` (CPU: `-c:v libx264 -preset ultrafast -crf 23`). `-ss` before `-i` + re-encode => frame-accurate.
2. Encoder probe: `ffmpeg -hide_banner -encoders` string match for `h264_nvenc|h264_amf|h264_qsv|h264_videotoolbox` (edit_video.py:25-48; cut_segments.py:11).
3. Reframe pipe (fallback only) (edit_video.py:117-129):
   `ffmpeg -y -loglevel error -hide_banner -stats -f rawvideo -vcodec rawvideo -s 1080x1920 -pix_fmt bgr24 -r <fps> -i - -c:v <enc> -preset <p> -pix_fmt yuv420p out.mp4` (+`-b:v 5M` for nvenc/amf).
4. Audio extract + mux (edit_video.py:165, 171-179): `ffmpeg -y -i in -vn -acodec copy audio.aac`; `ffmpeg -y -i video -i audio -c:v <enc> -preset <p> -b:v 5M -c:a aac -b:a 192k -r <fps> final.mp4`. (No `-map`; relies on default stream selection. Same fps hard-trusted from OpenCV, UNVERIFIED for VFR sources.)
5. Burn (burn_subtitles.py:17-28): `ffmpeg -y -loglevel error -hide_banner -i video -vf "subtitles='C\:/path/x.ass'" -c:v h264_nvenc -preset p1 -b:v 5M -pix_fmt yuv420p -c:a copy out.mp4`; Windows path escaping `replace('\\','/').replace(':','\\:')` (burn_subtitles.py:14).
6. Subtitle Studio live preview (subtitle_studio.py:729-738): `ffmpeg -re -y -f lavfi -i color=c=0x1a1a1a:s=WxH:r=FPS:d=DUR -vf ass='<path>' -f image2pipe -pix_fmt bgr24 -vcodec rawvideo -`.
7. Alpha caption video (subtitle_studio.py:1156-1165): `-f lavfi -i nullsrc=s=WxH:d=D:r=FPS ... -filter_complex "[0:v]format=rgba[base];[base]subtitles=<ass>[out]" -map "[out]" -c:v utvideo -pix_fmt rgba out.avi` (stderr/stdout swallowed; `temp_base_video` referenced but undefined at :1180 -> caught by the broad except).
8. Premiere overlay per segment (export_xml_lib/rendering.py:36-41): `-loop 1 -i base_canvas.png -vf "format=rgba,setpts=PTS+{start}/TB,ass='<ass>',setpts=PTS-{start}/TB,format=rgba" -t <dur> -c:v qtrle -pix_fmt argb -an caption_i.mov`. The setpts shift trick (render ASS at absolute time then shift back) is neat for rendering sub-ranges of one ASS file.

**ASS generated** (adjust_subtitles.py:85-98): `PlayResX: 360 / PlayResY: 640`, one `Default` style `Style: Default,{font},{base_size},{base_color},&H00000000,{outline_color},{shadow_color},{bold},{italic},{underline},{strikeout},100,100,0,0,{border_style},{outline_thickness},{shadow_size},{alignment},-2,-2,{vertical_position},1`; events `Dialogue: 0,<s>,<e>,Default,,0,0,0,,<text>`. In `highlight` mode each word of a block of N words gets its own event: active word `{\fs35\c&H0000FF00&}WORD` others `{\fs30\c&H00FFFFFF&}word` (adjust_subtitles.py:165-171). Colors are BGR (main_improved.py:35-43). 2-face segments override with `{\an5\pos(180,320)}` (adjust_subtitles.py:197-205). The tiny PlayRes means font sizes are relative to 360x640 and libass upscales to the 1080x1920 video (inference from ScriptType/PlayRes; UNVERIFIED by render). The header is written from an indented f-string so every header line starts with 4 spaces (adjust_subtitles.py:85-98); libass tolerance of this is UNVERIFIED.

## 8. Dependencies & external services
- Local models: WhisperX (faster-whisper CTranslate2 + wav2vec2 aligner downloaded from HF per language), InsightFace `buffalo_l` (auto-downloads), MediaPipe, optional GGUF via llama-cpp-python. GPU strongly recommended (CUDA 12.4 targeted, install_dependencies.bat:20-26); README says VS C++ Build Tools needed to compile insightface (README_en.md:~58-62).
- Network/services: YouTube via yt-dlp (+browser cookies), Gemini API (free tier), g4f (scrapes free web endpoints of third-party providers), Google Translate scrape (deep_translator), Colab share (`demo.launch(share=True)`, webui/app.py:916).
- Windows-first: `.bat` launchers, `os.startfile`, CREATE_NO_WINDOW usage (subtitle_studio.py:739).

## 9. Performance notes
- Claims "transcribe once, cut N" via TSV/JSON slicing (cut_json.py) — true and good.
- Transcription skip when YouTube subs exist (alignment only) — real speedup, language bug above.
- Reframing is the bottleneck: full-resolution per-frame OpenCV loop + double/triple re-encode. Detection cadence reduction (0.17 s / 1 s) is the main optimization. InsightFace `det_size` 640 on 1080p frames (downscaled internally) can miss small faces — UNVERIFIED.
- NVENC preset `p1` (fastest/lowest quality) with fixed 5 Mb/s; CRF 23 ultrafast for CPU => larger/lower-quality files.
- No timing/benchmark data in repo.

## 10. Weaknesses / bugs / limitations (verified in code)
1. GPLv3 (not MIT as assumed) — copyleft; see §11.
2. Theme mode broken: prompt.txt lacks `{virality_instruction}` (prompt.txt vs create_viral_segments.py:621-641). "Part i of n" context also dropped.
3. YouTube-sub alignment path forces language `en` (transcribe_video.py:245-247).
4. Quadruple video encode with mp4v intermediate (edit_video.py:237, 491; :171-179; burn_subtitles.py:17-28). Quality and speed loss.
5. `generate_short_mediapipe` references undefined `coordinate_log` in the no-face branch (edit_video.py:341; defined only in the InsightFace function at :522) -> NameError -> caught and the clip falls to the next fallback (edit_video.py:368-370, 1213-1214). Also triple duplicate `finalize_video` calls at end of Haar fn (edit_video.py:471-475).
6. Dead/incorrect logic: `activity` key never set (edit_video.py:569), `zoom_ema_bbox` only reset (:624), "ACTIVITY / SPEAKER DETECTION ... disabled" comment (:550-553), `is_talking` computed twice. Active speaker is explicitly "Experimental".
7. No real tracker/ReID; identity only by nearest center. Two-face split orders faces by proximity to last frame, so a person walking across the other can swap panels (inferred from edit_video.py:65-95, 857-860).
8. Single-face crop has no zoom/headroom logic — talking-head close-ups from 16:9 sources keep the full height (face_detection_insightface.py:112-145).
9. Alignment granularity = Whisper segment, not word; fuzzy substring matching `s_text in target or target in s_text` can match tiny segments like "yeah" (create_viral_segments.py:402-405). Cut may start/end mid-word or mid-sentence. No post-verification that the final clip is within a sentence boundary.
10. LLM output parsed with permissive heuristics but never validated against a schema; score only used for sort; no retry-on-bad-output loop beyond 429s (create_viral_segments.py:199-230).
11. No OpenAI-compatible provider; Gemini key on CLI; secrets file `api_config.json` tracked in repo (empty here).
12. Security: Gradio `allowed_paths=[VIRALS_DIR, WORKING_DIR, os.getcwd(), "."]` with `share=True` in Colab mode (webui/app.py:903, 916, 931) exposes the working directory (which holds `api_config.json`) through Gradio's file route (reasoned inference; exploit **UNVERIFIED**); `/export_xml_api` joins unsanitized `project` into a path (webui/app.py:944-946 -> path traversal likely, **UNVERIFIED**); global `torch.load(weights_only=False)` patch (transcribe_video.py:29-37); debug logs `debug_colors.log` written on every color conversion (webui/app.py:65-66).
13. Colors/state passed as strings and renamed files; fragile folder/filename conventions with regexes like `output(\d+)` and `^(\d{3})_` (adjust_subtitles.py:42-48).
14. i18n wraps nearly every print; logs unparseable for agents. Mixed Portuguese/English code comments.
15. No tests, no CI (none found in file listing), duplicated `old_export_xml*` directories.
16. Font handling: no bundled fonts, relies on system-installed Montserrat; no `fontsdir`, so presets using Montserrat-ExtraBold silently fall back if missing (main_improved.py:57; webui/subtitle_handler.py:15).
17. `format_time_ass` truncates rather than rounds centiseconds (adjust_subtitles.py:9) — max 10 ms drift; negligible.
18. Highlight mode enlarges the active word via `\fs`, so text reflows each word; no karaoke `\k`/`\kf`, no box/pill behind active word, no pop animation (adjust_subtitles.py:165-171).

## 11. License & reuse notes
- **Repo license: GPL-3.0** (repos/ViralCutter/LICENSE:1-2; 35,823 bytes = standard text). The prompt said MIT — **that is incorrect**. No per-file headers or SPDX tags found (grep for copyright/SPDX/licen over py/md/json/js/bat found only the LICENSE and an author credit label "Desenvolvido por Rafael Godoy" in subtitle_studio.py:366).
- Implications: copying code (Python or by close translation to Rust) makes our derivative GPLv3. To keep our license free: reimplement from ideas/behaviors, not from code; do **not** vendor prompt.txt verbatim if the prompt text is considered expressive content (low risk, but safer to rewrite it); the numerical thresholds and the data-flow are not copyrightable but treat as inspiration. Running ViralCutter as an external sidecar CLI would not infect us (GPL applies to distribution of derived works), but also gives us little value given its quality.
- Dependencies (licenses NOT stated in repo; from general knowledge, **UNVERIFIED here**): WhisperX BSD-2; yt-dlp Unlicense; MediaPipe Apache-2.0; Gradio Apache-2.0; customtkinter MIT; g4f GPL-3.0; **InsightFace code MIT but the pretrained `buffalo_l` model pack is restricted to non-commercial research** — relevant if we ever mirror its detector; for our system prefer permissively-licensed detectors (e.g. YuNet/SCRFD-with-license-check/MediaPipe BlazeFace).
- Assets: no fonts, music, or model weights bundled (models/ contains only `.local-model-here`: repos/ViralCutter/models/.local-model-here). Montserrat is OFL if the user installs it (UNVERIFIED; not in repo). The `webui/advanced_editor` JS ("DUB Editor", `// static/script.js` header, script.js:1) may be vendored from another project — provenance UNVERIFIED.
- Demo videos linked in README are hosted on GitHub user-attachments; content rights unknown.

## 12. Steal list (ranked; method in brackets)
1. **Text-anchored LLM segment contract + deterministic aligner** [reimplement in Rust]. Prompt the LLM with a transcript that has sparse `(Ns)` time tags; ask for `start_text/end_text/start_time_ref/title/reasoning/score`; resolve to real word/segment timestamps in code with a bounded fuzzy search window (back 5/forward 50/200), then clamp to min/max duration. Improve: snap to **word** boundaries from WhisperX alignment and to sentence ends; reject/repair if anchors not found rather than `start+min_duration`; validate with a JSON Schema; expose as an MCP tool `clips.resolve_anchors` so an agent can supply anchors too (their "Raw Segment Repair" proves the pattern).
2. **Robust LLM JSON recovery** (strip `<think>`, find enclosing object around "segments", brace-balanced parse, fenced block, per-item recovery from truncated output) [reimplement, create_viral_segments.py:36-169]. Needed for local/small models behind OpenAI-compatible endpoints.
3. **Face-driven reframing policy** [reimplement with `ort` + a permissive detector]: detection cadence 0.17 s/1 s, confidence + relative-area filter (0.35), 1-vs-2-face ratio rule (0.60), crowd >=7 => fallback, last-known-face hold 3 s, dead-zone hold + short interpolation, nearest-center pairing for panel stability, split-screen 2x(1080x960) at 2.2x face-box zoom. Improve with proper One-Euro/Kalman smoothing, a real tracker (ByteTrack/IoU+Kalman), and emit a **keyframe track JSON** (their `_coords.json`/`_timeline.json` idea) that FFmpeg consumes via a `crop` + `sendcmd`/expression or `zoompan`, so reframing becomes data an agent can inspect and edit instead of baked pixels.
4. **Reframe plan as artifact + subtitle coupling**: persist per-clip `timeline.json` (1-face/2-face segments) and let caption layout read it to reposition captions in split mode (adjust_subtitles.py:185-208). In our system: one `ClipPlan` JSON (cuts, reframe keyframes, caption style, audio ops) + deterministic renderer; every pipeline stage idempotent via file/hash existence (their `process_config.json` reproducibility record is the right instinct, main_improved.py:676-711).
5. **Caption model + presets** [reimplement ASS emitter; data-only reuse of idea]: PlayRes-independent word-block layout with `words_per_block`, gap limit anti-flicker (stretch previous word end if gap < limit; subtitle_studio.py:966-973), punctuation/uppercase options, split block on punctuation/long gap/estimated width > 90% of screen (subtitle_studio.py:980-1029). Preset parameter set (font, sizes, colors, outline, shadow, box mode, block size, vertical pos) is a good minimal schema for our "caption template" JSON. Do better: `\k`-style or per-word `\t` animations, active-word pill, safe-area margins, emoji, real font measurement via the font file (their width estimator is a hard-coded Arial table, subtitle_studio.py:84-100).
6. **Pipeline-as-CLI with flags + JSON on disk** (`--workflow full|cut|subs`, `--project-path`, `--skip-prompts`) [reimplement as stable Rust CLI subcommands + MCP tools, but with structured JSON events/exit codes]. Their WebUI proves a GUI can be a thin shell over the CLI; our Tauri GUI and MCP server should both call the same Rust core instead of subprocess-scraping.
7. **Encoder capability probe and fallback chain** (NVENC > AMF > QSV > VideoToolbox > libx264 with per-encoder preset map) [reimplement; parse `ffmpeg -encoders` once, and actually test-encode 1 frame since string presence != working device].
8. **Alignment-only mode** from existing subtitles (YouTube SRT/VTT) to skip ASR [reimplement; with correct language detection] — plus VTT karaoke de-duplication logic (download_video.py:242-326) for yt-dlp auto-captions.
9. **Premiere/NLE export** (XML + alpha caption overlays via `ass` + `setpts` trick, rendering.py:36-41) as a later "handoff to human editor" feature [idea only]. The `setpts` shift trick for rendering ASS sub-ranges is directly reusable as a technique.
10. **Subtitle Studio live-preview technique**: render ASS over a synthetic `color`/`nullsrc` source in an FFmpeg subprocess piped as rawvideo into the UI [idea for the Tauri preview; consider piping JPEG/PNG or using libass via WASM instead].
11. Smaller: yt-dlp format ladder (`bestvideo[height<=N]+bestaudio/best[height<=N]`), cookies-from-browser retry loop (download_video.py:10-33), `Part i of N` chunk context + 10% overlap chunking (needs token-aware chunking in Rust), 429 "retry in Ns" parsing (create_viral_segments.py:216-223).

Do **not** adopt: god-script structure, monkeypatches, filename-convention state, Google Translate scraping, g4f, global CLI-key passing, mp4v intermediates, folder-rename bookkeeping.

## 13. Gaps (what we need that this repo lacks)
- Any agent/MCP interface, machine-readable progress/errors, JSON schemas, dry-run/plan-then-render split, and QA stage (no quality check at all: no caption overflow, face-lost, audio level, black-frame, duration/aspect validation).
- OpenAI-compatible LLM/STT/TTS endpoints; STT other than WhisperX; any TTS/voiceover/dubbing (editor labeled "DUB" but no TTS found).
- Audio: denoise, loudness normalization (EBU R128), silence/filler removal, music/SFX, ducking.
- Creative: animated captions, emoji, hooks/title overlays, b-roll, transitions, zooms/punch-ins, blurred-background fit, progress bar, templates beyond ASS style dict, bundled/managed fonts, multi-aspect outputs (1:1, 4:5, 16:9).
- Clip intelligence: speaker diarization, scene detection, audio-energy/laughter/emotion features, hook scoring separate from the LLM's self-score, non-contiguous (stitched) clips, per-segment re-edit after the fact.
- Reframing: object/body tracking when no face, screen-recording/slide content handling, speaker-driven switching using audio diarization instead of mouth-landmark heuristics, saved keyframes editable in GUI.
- Single-pass FFmpeg filtergraph (crop+scale+subtitles+loudnorm in one encode) and hardware-pipeline decode.
- Tests, benchmarks, reproducible environment (pins exist for torch/onnx only).
