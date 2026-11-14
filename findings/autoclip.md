# Findings: autoclip

Repo: `repos/autoclip` (upstream `github.com/artbyjazi/autoclip`, author Jad Ghazi, v0.1.0.dev0, last commit 2026-08-01 `5d0eac3`). Analysed per `docs/research-methodology.md`. Paths below are relative to `repos/autoclip/`; the Python package root is `backend/autoclip/` (abbreviated `B/`). Everything cited was read in source. Nothing under `repos/` was modified.

## 1. Summary

AutoClip is a local-first Python/FastAPI + React tool. It turns long video (YouTube URL or file) into ranked, caption-burned 9:16 / 1:1 / 16:9 clips. The pipeline is faster-whisper word timestamps, then LLM highlight picking over overlapping transcript windows, then MediaPipe face tracking with a One-Euro-smoothed crop path, then ASS caption burn-in and a single-pass FFmpeg render with loudnorm (`B/pipeline/runner.py:171-192`).

Verdict: a small (~13k LOC incl. tests/UI), very clean codebase with several unusually good ideas (word-index prompting, silence-aligned cuts, shot-aware crop path rendered as one filtergraph, libass karaoke via per-word events). It is a good reference for the "find highlights, cut cleanly, reframe, burn captions" core. It is weak for our agent-driven goal: no JSON CLI, no MCP, no non-LLM scoring, no audio features beyond loudnorm, no TTS/music/SFX, no timeline, only 4 caption presets, and no STT provider abstraction. Several wired-but-dead options exist (see section 10).

Quality summary (1-5): A 3, B 3, C 2, D 4, E 4, F 2, G 1, H 3, I 3 (LLM only).

## 2. Stack & architecture

- Python 3.11-3.12 only (MediaPipe has no 3.13 wheels) (`pyproject.toml:28-29`). FastAPI + uvicorn + SSE, SQLite (WAL), Typer CLI, React 18 + Vite + Tailwind 4 frontend (`frontend/package.json:11-25`).
- One process, one FIFO job queue (one video at a time) (`docs/ARCHITECTURE.md:30-39`, `B/jobs/queue.py:1-10`). The pipeline runs blocking in a worker thread with its own event loop (`B/jobs/queue.py:159-165`).
- FFmpeg/ffprobe are subprocesses found via `shutil.which` (`B/pipeline/ffmpeg.py:81-98`). Stage artifacts are files in `work/{job_id}/`, so a retry resumes at the first missing artifact (`B/pipeline/runner.py:73-102`, `201-216`, `222-226`).

```
yt-dlp / upload -> prepare(16k wav, thumbs, silencedetect) -> faster-whisper(+whisperx diarize)
  -> highlights(8-min windows, LLM, word-index spans) -> boundaries.refine(sentence snap, duration clamp, silence align)
  -> reframe(per clip: scenedetect, MediaPipe landmarks, tracks, speaker map, 1-euro smooth) -> CropPath JSON
  -> export(ASS + one ffmpeg -filter_complex pass, loudnorm, h264_nvenc|libx264)
```

## 3. Repo map (important only)

| Path | Role |
|---|---|
| `B/pipeline/highlights.py` (285) | windowing, dedupe, ranking |
| `B/prompts/highlight_v1.txt` (61) | the one LLM prompt |
| `B/providers/base.py` (311) | provider ABC, JSON extraction, retry-with-feedback |
| `B/providers/{openai,anthropic,gemini,ollama}_provider.py` | 4 LLM adapters |
| `B/pipeline/boundaries.py` (261) | sentence snap, duration clamp, silence alignment |
| `B/pipeline/transcript.py` (214), `transcribe.py` (306) | word-level transcript model, faster-whisper, WhisperX diarize |
| `B/pipeline/captions.py` (399) | 4 presets, grouping, ASS generation |
| `B/pipeline/export.py` (326) | filtergraph build, encoder choice, render |
| `B/pipeline/reframe/*` (~1200) | faces, tracker, speaker, scenes, smoothing, croppath |
| `B/pipeline/prepare.py` | audio extract, thumbnails, silencedetect |
| `B/system.py`, `B/cuda.py` | capability probing (NVENC, CT2 compute types, libass) |
| `B/cli.py` (705) | Typer CLI |
| `B/api/*`, `B/db/*`, `B/jobs/*` | REST, SQLite, SSE |
| `frontend/src/components/{ClipPlayer,TrimBar,CaptionEditor}.tsx` | review UI |
| `tests/` (13 files, ~3.2k LOC) | unit, render (`-m slow`), e2e needing real media |

## 4. Capabilities by category

### A. AI clipping — quality 3
- Whole transcript is cut into 8-minute windows with 60 s overlap (`B/pipeline/highlights.py:27-28`, `43-82`). Each word is rendered `[index]word`, with `<SPEAKER_xx>` markers on speaker change (`highlights.py:98-114`).
- Per window, one LLM call returns JSON `{"clips":[{start_word_index,end_word_index,title,hook,score,reason}]}` (`B/prompts/highlight_v1.txt:46-57`, `B/providers/base.py:55-63`). Windows run concurrently: 3 for hosted providers, 1 for Ollama (`highlights.py:33-36`, `138`). A failed window is logged and skipped (`highlights.py:147-157`).
- Merge: dedupe candidates greedily by score using inclusive word-index IoU > 0.4 (`highlights.py:31`, `238-263`, `279-285`). Boundary-refine each. Dedupe again after refinement. Sort by score, truncate to `max_clips` (default 10), assign rank (`highlights.py:178-235`).
- The "score" is only the LLM's self-reported 0-100 integer. There is no feature-based scoring (no audio energy, laughter, pace, keyword or engagement signals, no embeddings). 0-1 floats are rescaled x100 (`base.py:71-83`).
- Duration defaults: min 20 s, max 90 s (`B/config.py:68-71`).
- The prompt asks for hook strength, self-containment, payoff and quotability, in that order. The `hook` field must quote the opening words verbatim "to verify the clip starts where you think it does" (`highlight_v1.txt:38`). But nothing in code verifies it (grep: `hook` is only stored, `highlights.py:220`).
- No cross-window global ranking pass, no "narrative / multi-segment stitched clip" support, and no re-prompt for hook rewriting. A clip is always one contiguous word span.

### B. Transcription & subtitles — quality 3
- faster-whisper `WhisperModel(model, device, compute_type)` with `word_timestamps=True, vad_filter=True, vad_parameters={"min_silence_duration_ms": 500}` (`B/pipeline/transcribe.py:123-135`). Default model `small` (`config.py:58`). Input is 16 kHz mono PCM WAV (`prepare.py:43-68`).
- Compute type is chosen by asking CTranslate2 what the device supports, not by inferring from GPU model (`B/system.py:307-320`, `transcribe.py:25-52`).
- Optional diarization via WhisperX/pyannote (needs HF token, gated models). Words get the speaker with maximum temporal overlap (`transcribe.py:192-242`, `285-306`). Returns the transcript unchanged on any failure.
- Transcript model: `Word(text,start,end,speaker)`, `Segment`, sentence/clause-end regexes, bisect lookups, `speaker_turns()` (`B/pipeline/transcript.py:19-47`, `101-184`).
- Captions: see C. The word text is user-editable in the UI (`CaptionEditor.tsx:1-107`), with timings locked. Edits are stored as `edited_words` in `clip_edits` (`B/db/schema.py:82-88`).
- Gaps:
  - STT is local faster-whisper only. There is no OpenAI-compatible `/audio/transcriptions` path.
  - `ingest.prefer_youtube_captions` downloads json3 captions (`B/pipeline/ingest.py:118-121`) but nothing parses them. It is dead config (grep shows no other reference).
  - No emoji, no profanity masking, no translation, no per-language font handling.

### C. Creative design — quality 2
- Four hardcoded presets in `PRESETS` (`B/pipeline/captions.py:102-167`): `bold_pop` (Anton, white, yellow `#FFE500` active word scaled 118%), `karaoke_fill` (`\kf` sweep, green `#31E981`), `clean_lower` (Inter, no animation, 8 words), `boxed` (opaque box BorderStyle 3, red `#FF4D4D` accent, 112%).
- Style model `CaptionStyle` (`captions.py:65-95`): font, size as fraction of height, colours, outline, shadow, bold, caps, bottom margin ratio, max words, animation in {none, scale, karaoke}, box colour/alpha. Authored against a 1920-px reference and scaled by output height (`captions.py:33-35`, `266`).
- No gradients, motion, transitions, per-word emoji, B-roll, title cards, progress bars, intros/outros, or user-defined templates (the preset dict is code, not data).
- Caption preview in the browser is only a CSS approximation of the ASS render (`ClipPlayer.tsx:453-510`; uses scale 1.08, not the 118% in ASS).

### D. Video processing & rendering — quality 4
- Single-pass render: `-ss` before `-i`, `-filter_complex`, `loudnorm`, NVENC or libx264, `+faststart` (`B/pipeline/export.py:221-326`). Full command in section 7.
- Capability probing: NVENC is verified by a real 1-frame encode (`B/system.py:223-254`). `ffmpeg -filters` is parsed for required filters. libass/fontconfig/libx264 are detected from the banner (`system.py:190-220`). A doctor command reports this (`cli.py:69`+).
- Progress via `-progress pipe:1` parsing of `out_time_us` (`B/pipeline/ffmpeg.py:336-358`). Cancellation by killing the process (`ffmpeg.py:344-346`, `361-369`).
- Windows path-escape problem is avoided by running ffmpeg with `cwd` = a render workspace and referencing `captions.ass` and `fonts` by bare name (`ffmpeg.py:234-265`, `export.py:139`, `256-258`). An unused `escape_filter_path` is also kept (`ffmpeg.py:109-133`).
- Thumbnails every 5 s at 160 px for UI scrubbing (`prepare.py:24-26`, `71-103`).
- No batch parallelism (one job at a time, clips rendered serially, `runner.py:365-406`). No proxy/preview render (the UI plays the original source with a CSS crop/caption overlay). No segment-level cuts inside a clip (no jump-cut/silence removal). No concat of multiple source ranges.

### E. Reframing — quality 4 (best part of the repo)
- Per clip (`B/pipeline/reframe/__init__.py:78-177`):
  1. Sample faces at 5 fps with MediaPipe FaceLandmarker (up to 4 faces, confidence 0.4) via OpenCV `VideoCapture` (`faces.py:21-28`, `115-137`, `140-176`).
  2. Detect shots with PySceneDetect `ContentDetector(27.0)`, merging shots under 0.8 s (`scenes.py:15-23`, `94-106`).
  3. Build tracks by greedy IoU (>=0.25) plus centre-distance fallback (<=1.2 face widths), with a max gap of 0.8 s and min track 0.5 s (`tracker.py:22-28`, `93-172`).
  4. Map diarized speakers to tracks once per clip by correlating mouth-aspect-ratio variance to speaker turns (`speaker.py:52-99`). Per shot, speaker turns drive the crop (`speaker.py:132-174`). Without diarization, mouth-activity variance picks the speaker, requiring a 1.6x margin over the runner-up. Otherwise the shot is "ambiguous" (`speaker.py:33-38`, `177-195`).
  5. Strategies (`croppath.py:35-47`): TRACK (follow, or lock if face > 0.55 of crop width or the movement spread is under the dead zone, `__init__.py:55-57`, `297-304`), WIDE (lock on the centroid of all faces, or `fit` = whole frame over a blurred copy if the spread exceeds 0.95 x crop width, `__init__.py:59-61`, `357-370`), GENERAL (centre crop).
  6. Eye line placed at 38% of crop height (`__init__.py:53`, `288-289`).
  7. Smoothing = One Euro filter (min_cutoff 0.6, beta 0.02), then a 12 px dead zone, then a 260 px/s velocity clamp (`smoothing.py:23-135`, `SmoothingConfig` at 87-94).
  8. Keyframes decimated to <= 48 per segment (epsilon 1 px) and emitted as a piecewise-linear ffmpeg expression (`croppath.py:26-32`, `208-246`, `254-282`).
- The crop path is serialised as JSON (`CropPath.to_dict/save`, `croppath.py:104-141`) and reused by the UI to preview the same crop (`api/clips.py:78-97`, `ClipPlayer.tsx:311-370`).
- Limits: face-only (no object/saliency/screen-content tracking), 5 fps sampling, no multi-speaker split-screen layout (only crop or blur-fit), mouth-motion speaker detection is admitted to be weaker than TalkNet/LR-ASD (`speaker.py:15-17`). `GENERAL_ZOOM=0.0` (`__init__.py:65`) and the zoompan branch is effectively disabled.

### F. Audio — quality 2
- Only: 16 kHz mono extract for STT, `silencedetect=noise=-32dB:d=0.20` for cut placement (`prepare.py:110-155`), and render-time `loudnorm=I=-14:TP=-1.5:LRA=11` single pass (`export.py:35-38`, `178-179`), AAC 192k @ 48 kHz (`export.py:40-41`).
- No denoise, no music/SFX, no ducking, no TTS, no two-pass loudnorm (single-pass dynamic mode is less accurate), no QC beyond the browser `measureOutputLevel` peak check (`ClipPlayer.tsx:245-309`, a UI debugging aid, not a pipeline QC).

### G. Agent integration — quality 1
- CLI is human-oriented (Rich tables/progress), not JSON-in/JSON-out. Commands: `version, doctor, config show/path/set-secret/delete-secret, init, clip, jobs, providers, fetch-models, update-ytdlp, serve, styles` (`B/cli.py:63-696`). `autoclip clip <url|file>` runs the whole pipeline in one shot (`cli.py:405-512`). There are no per-stage commands (the stages are only callable from Python).
- No MCP server, no skills, no tool schemas. The REST API (`B/api/*`) is the only machine interface: 20+ routes (ingest, jobs, SSE events, cancel/retry, clip PATCH trim/title/status, caption PATCH, export, download, caption-styles, providers/status, system). It has no authentication (loopback only, `docker/compose.yaml` comment).
- State/handoff is good: SQLite plus per-job artifact JSON (`transcript.json`, `silences.json`, `crops/{clip}.json`) with stable IDs. The core data model (word-indexed transcript, `CropPath`) is serialisable and would map cleanly to agent tool I/O.

### H. GUI — quality 3
- Pages: Ingest, JobProgress (SSE with snapshot-on-reconnect, `useJobStream.ts`), Review, Settings (`frontend/src/pages/*`).
- Review has a `ClipPlayer` (CSS crop window from the real crop path, caption overlay, volume/audio check), a `TrimBar` with handles that snap to word edges within 0.5 s (`TrimBar.tsx:44-58`), and a word-level `CaptionEditor` (click-to-correct text, timings locked). README states "No timeline editor" is a deliberate non-goal (`README.md:205-211`).
- Not present: timeline, multi-clip assembly, template editor, asset library, live render preview matching ASS.

### I. Provider abstraction — quality 3 (LLM only)
- `LLMProvider` ABC with `_complete(system,user,config)` and `health_check()`. The shared `detect_highlights` does prompt load, validate, one repair retry with the validation error appended (`B/providers/base.py:134-212`). Index clamping into the window and dropping end<=start spans (`base.py:201-212`).
- Robust JSON extraction: strip fences, `json.loads`, else brace-matching that respects strings, and accepts a bare array (`base.py:252-311`).
- Adapters: Anthropic (assistant prefill `{`, `max_tokens=4096`, `anthropic_provider.py:48-68`), OpenAI-compatible (any `base_url`; `response_format=json_object` then fallback without it on rejection, `openai_provider.py:69-98`; known endpoints OpenAI/OpenRouter/Groq/DeepSeek/Together/LM Studio, `26-33`), Gemini (`response_mime_type=application/json`), Ollama (`/api/generate`, `format:"json"`, 600 s timeout, `ollama_provider.py:56-90`).
- Registry dict `PROVIDERS` and `build_provider` (`providers/__init__.py:43-80`). Per-provider `model` and `base_url` in `config.json`. API keys via OS keyring with `AUTOCLIP_<NAME>_KEY` env override and a plaintext-with-0600 fallback flag (`config.py:195-215`, `218-236`).
- No STT or TTS provider abstraction at all. Whisper is hardwired. No embeddings. No streaming or tool-calling.

## 5. Pipeline walkthrough (with real references)

1. `cli.clip` or `POST /api/sources/youtube|upload` then `POST /api/jobs` creates a source and job (`cli.py:405+`, `api/jobs.py:49-70`). Ingest: yt-dlp format `bestvideo[height<=1080][ext=mp4]+bestaudio[ext=m4a]/best[height<=1080]`, cookies-from-browser for bot checks (`config.py:76`, `ingest.py:104-121`, `157-199`). Upload accepts mp4/mov/mkv/webm/avi/m4v and common audio, and rejects files with no audio (`ingest.py:28-30`, `282-289`).
2. `_stage_prepare`: extract wav, thumbnails (`runner.py:196-216`).
3. `_stage_transcribe`: faster-whisper, optional diarize, save `transcript.json`, upsert DB row (`runner.py:218-257`).
4. `_load_or_detect_silences`: `silencedetect` to `silences.json` (`runner.py:259-269`).
5. `_stage_highlights`: `highlights.detect` then `boundaries.refine` per candidate, `store.replace_clips` (`runner.py:271-299`).
6. `_stage_reframe`: `build_crop_path` per clip, cached to `crops/{clip_id}.json` (`runner.py:301-339`).
7. `_stage_captions`: only validates the style name, a no-op placeholder (`runner.py:341-348`).
8. `_stage_export`: for each clip, `transcript.slice(start_word,end_word)` becomes ASS, then `export_clip`, then an `exports` row and clip status `exported` (`runner.py:350-408`).
9. Review (web only): PATCH trim re-derives word range via `indices_in_range` (`api/clips.py:100-137`), PATCH captions saves edits, POST export re-renders (`api/clips.py:171-237`).

## 6. Prompts & heuristics

### The highlight system prompt (`B/prompts/highlight_v1.txt`, whole file is the system message)
Key quoted parts:

> "You are a short-form video editor. You find the moments in a long transcript that will hold a stranger's attention on TikTok, Instagram Reels, and YouTube Shorts." (`:1`)
>
> "Judge every candidate against four things, in this order of importance: 1. **Hook strength.** The first three seconds decide everything. A clip must open on a line that creates a question, states something contrarian, names a specific number, or drops the listener into the middle of tension. It must NOT open on 'so', 'yeah', 'um', 'and then', a topic change, or the tail of an answer to a question the viewer never heard. 2. **Self-containment.** ... No unexplained pronouns ... No 'like I mentioned' ... 3. **Payoff.** ... If you cannot name what the viewer walks away with, the score is below 50. 4. **Quotability.** ..." (`:9-15`)
>
> "Start on the first word of a complete thought. End on the last word of a complete thought — ideally the strongest line, not the trailing qualification after it. Never start or end mid-sentence. Do not extend a clip to reach a length target. A tight 25-second clip beats a padded 60-second one." (`:19-21`)
>
> Scoring: "Score 0-100. Be harsh and use the whole range ... 85-100 Would genuinely perform ... 70-84 Solid ... 50-69 Watchable but unremarkable ... Below 50 — Do not return it at all. ... Most sections of most videos contain nothing worth clipping; saying so is a correct answer." (`:23-32`)
>
> `title` under 60 chars, "how a human editor would label this clip"; `hook` = "quote the actual opening words of the clip, verbatim"; `reason` = one sentence naming hook and payoff (`:34-40`).
>
> Output: ONLY JSON `{"clips":[{"start_word_index":1042,"end_word_index":1385,"title":...,"hook":...,"score":87,"reason":...}]}`; indices "are the only timing signal you provide — do not estimate seconds"; empty -> `{"clips": []}` (`:44-61`).

### The per-window user message (`B/providers/base.py:231-249`)
```
Transcript section, words {first} to {last}.
Each word is tagged with its index as [index]word.
[This section has N distinct speakers (SPEAKER_00, ...); speaker labels are shown inline.]
Clip length must be between {min} and {max} seconds.
Return at most {max_clips} clips.
---
{window text}
---
Respond with ONLY a JSON object matching the schema. No prose, no markdown fences.
```
Window text looks like `<SPEAKER_00>\n[0]Hello [1]world ...` (`highlights.py:98-114`). Temperature is 0.3 (`base.py:118`).

### Repair prompt on schema failure (`base.py:175-181`)
`"{user}\n\nYour previous response did not match the required schema.\nValidation error:\n{err}\n\nRespond again with ONLY the corrected JSON object. No prose, no markdown fences."`

### Numeric heuristics (all code constants)
| Heuristic | Value | Cite |
|---|---|---|
| window / overlap | 480 s / 60 s | `highlights.py:27-28` |
| dedupe IoU | 0.4 | `highlights.py:31` |
| LLM concurrency | 3 hosted, 1 ollama | `highlights.py:35-36` |
| sentence-snap search | +-12 words; start prefers earlier, end prefers later | `boundaries.py:28`, `113-154` |
| silence-align search radius | 0.75 s | `boundaries.py:35` |
| lead / tail into silence | 0.12 s / 0.28 s; margin 0.04 s | `boundaries.py:39-45` |
| fallback pad (no silence) | 0.25 s lead / 0.35 s tail | `boundaries.py:31-32` |
| silencedetect | -32 dB, 0.20 s | `prepare.py:113-115` |
| caption grouping | max 4 words (preset-specific), break on sentence end, gap > 0.4 s, or count | `captions.py:27-31`, `201-234` |
| karaoke gap folding | add inter-word gap to preceding word if 0 < gap < 0.5 s | `captions.py:331-334` |
| face sampling | 5 fps, 4 faces | `faces.py:24`, `28` |
| smoothing | cutoff 0.6, beta 0.02, dead zone 12 px, 260 px/s | `smoothing.py:25`, `28`, `92-94` |
| mouth activity | pvariance of MAR; silent < 0.0004; dominance 1.6x | `speaker.py:33-38`, `tracker.py:67-77` |
| loudness | -14 LUFS, TP -1.5, LRA 11 | `export.py:35-38`, `config.py:89` |
| encode quality | CRF/CQ 18 | `config.py:92` |

### Boundary refinement logic (`boundaries.py:60-105`)
Sentence-snap both ends, then `_clamp_duration`: if too long, walk back to the last sentence end that fits, else the last word that fits; if too short, extend forward through later sentence ends up to max; drop the clip if still out of [min,max] (`boundaries.py:157-207`). Then `align_start`/`align_end` move the cut into the nearest measured silence (`215-237`). Re-cap at max duration (`99-101`).

## 7. FFmpeg commands/filtergraphs actually used

**Render (one pass per clip)**, `B/pipeline/export.py:268-297`, run with `cwd=<workspace>` and prefixed by `ffmpeg -hide_banner -nostdin -y -loglevel error [-progress pipe:1 -nostats]` (`ffmpeg.py:285-295`):
```
-ss {start:.4f} -t {dur:.4f} -i {source}
-filter_complex "<video graph>" -map [vout] -map 0:a?
-af loudnorm=I=-14.0:TP=-1.5:LRA=11.0
<encoder args> -pix_fmt yuv420p -c:a aac -b:a 192k -ar 48000 -movflags +faststart {out.mp4}
```
Encoder args (`export.py:182-213`): NVENC `-c:v h264_nvenc -preset p5 -rc vbr -cq {crf} -b:v 0 -profile:v high`, else `-c:v libx264 -preset medium -crf {crf} -profile:v high`. NVENC is used only if the 1-frame probe passed (`system.py:223-254`).

**Video graph** (`export.py:87-143`). Single segment: `[0:v]crop=...,scale=W:H:flags=lanczos,setsar=1,format=yuv420p[v0]`. Multi-segment, per shot:
```
[0:v]trim=start=S:end=E,setpts=PTS-STARTPTS[s0];
[s0]crop=w=607:h=1080:x='<expr>':y='<expr>',scale=1080:1920:flags=lanczos,setsar=1,format=yuv420p[v0];
...
[v0][v1]...concat=n=N:v=1:a=0[vcat];
[vcat]ass=filename=captions.ass:fontsdir=fonts[vout]      (or [vcat]null[vout] when no captions)
```
Crop expression is a nested piecewise-linear `if(lt(t,t1),(v0+slope*(t-t0)),<rest>)` with commas escaped as `\,` (`croppath.py:254-306`). Audio is NOT segmented; it is mapped from the single seeked input, so it stays continuous.

**Blur-fit segment** (when subjects are too spread to crop; `export.py:146-164`):
```
[src]split=2[fitbgN][fitfgN];
[fitbgN]scale=W:H:force_original_aspect_ratio=increase,crop=W:H,boxblur=24:2[bgN];
[fitfgN]scale=W:H:force_original_aspect_ratio=decrease:flags=lanczos[fgN];
[bgN][fgN]overlay=(W-w)/2:(H-h)/2,setsar=1,format=yuv420p[vN]
```
**Optional push-in** (disabled by default): `zoompan=z='min(zoom+{zoom/240},{1+zoom})':d=1:s=WxH:fps=30` (`export.py:167-175`).

**Other commands**
- Audio extract: `-i src -vn -ac 1 -ar 16000 -c:a pcm_s16le out.wav` (`prepare.py:52-67`).
- Thumbs: `-vf fps=1/5,scale=160:-2 -q:v 5 thumb_%05d.jpg` (`prepare.py:87-96`).
- Silence: `ffmpeg -hide_banner -nostdin -i audio -af silencedetect=noise=-32.0dB:d=0.2 -f null -`, parsed from stderr with regex (`prepare.py:125-153`).
- Probe: `ffprobe -v error -print_format json -show_format -show_streams` (`ffmpeg.py:157-166`). Cover art is treated as no video (`ffmpeg.py:186-187`).
- NVENC probe: `-f lavfi -i color=c=black:s=256x256:d=0.1 -c:v h264_nvenc -frames:v 1 -f null -` (`system.py:235-250`).

**ASS construction** (`captions.py:244-367`, via `pysubs2`): `PlayResX/Y` = output size, `ScaledBorderAndShadow=yes`, `WrapStyle=0`; one style `AutoClip`, bottom-centre, `marginv = h * margin_v_ratio`, `borderstyle 3` for boxed. Per animation mode:
- scale: one Dialogue per word, the whole group shown, active word wrapped as `{\c&HBBGGRR&\fscx118\fscy118}WORD{\r}`.
- karaoke: a single line of `{\kfNN}WORD` segments in centiseconds.
- none: a static line.

## 8. Dependencies & external services

- Runtime: `faster-whisper` (CTranslate2), `mediapipe` (+ `opencv-python` transitively), `scenedetect`, `pysubs2`, `yt-dlp`, `anthropic`, `openai`, `google-genai`, `httpx`, `fastapi`, `uvicorn`, `sse-starlette`, `keyring`, `typer`, `rich`, `numpy` (`pyproject.toml:33-65`). Optional: `whisperx` (pulls torch + pyannote, gated HF models), CUDA 12 cublas/cudnn wheels (`pyproject.toml:68-91`).
- External downloads: MediaPipe `face_landmarker.task` (~3.7 MB) from `storage.googleapis.com/mediapipe-models/...` on first use (`B/models.py:32-43`, `64-103`), no checksum verification (UNVERIFIED that any hash check exists elsewhere; none found in `models.py`). Whisper weights fetched by faster-whisper from HF at first run.
- FFmpeg must be a full build with libass and libx264. The README warns that plain Homebrew `ffmpeg` and `Gyan.FFmpeg.Essentials` lack it (`README.md:44-70`).
- Paid services: none required. Only transcript text goes to the LLM, never audio or video (`README.md:22-24`).
- GPU: optional; used for Whisper (CUDA via CT2) and NVENC. MediaPipe runs on CPU in this code (no delegate option set, `faces.py:115-121`).

## 9. Performance notes

- Face sampling at 5 fps and one-clip-at-a-time reframing keeps it cheap; the author calls sample fps the main cost lever (`faces.py:21-24`).
- `silencedetect` is run once over the whole job audio. Shot detection and face sampling run per clip, so overlapping clips decode the same footage repeatedly (inference from `runner.py:321-334`, not benchmarked).
- Transcript lookups recompute the `starts` list on every call (`transcript.py:96-110`), O(n) per `index_at_time`. Fine at this scale, but `build_windows` calls it per window.
- The render is one ffmpeg process per clip, serial. With NVENC `p5` + `-cq 18` it is fast. With many shots, the graph uses N trim branches decoding from one input (a single decoder feeding N filters), which is efficient.
- LLM cost: each word costs extra tokens for `[i]` tags (the author notes the verbosity, `highlights.py:98-104`). An 8-min window is about 1.2k words, which is roughly 5-6k tokens in. Ollama call sets no `num_ctx` (`ollama_provider.py:56-65`), so windows may be silently truncated by Ollama's default context (inference, UNVERIFIED at runtime).
- No benchmarks in repo. There are no measured numbers, so none are quoted here.

## 10. Weaknesses / bugs / limitations (verified in code unless marked)

1. **Job-level setting overrides are ignored in the web path.** `POST /api/jobs` merges overrides into a snapshot stored in the DB (`api/jobs.py:36-71`, `db/store.py:96`), but `JobQueue` builds `PipelineRunner(job, source, ...)` without settings (`jobs/queue.py:136-141`), and the runner falls back to `load_settings()` (`runner.py:118`). Nothing reads the job snapshot (grep for `job.settings` finds only the write). So per-job caption style, ratio, and max_clips from the UI are not applied. The CLI path passes settings correctly (`cli.py:469-474`).
2. **`--centre-crop` is a no-op.** The CLI sets `settings.export.__dict__["centre_crop"] = True` (`cli.py:443-444`), and no code reads it. `ReframeConfig.centre_only` is never set by the runner (`runner.py:314-319`).
3. **Stale crop path after trimming.** The crop path is cached per clip id (`runner.py:323-334`). `patch_clip` updates times but does not invalidate it (`api/clips.py:100-137`). `_crop_path_for` reuses the cache for 9:16 (`api/clips.py:315-317`). Keyframe times are relative to the old start, so a trimmed re-export has mismatched framing. Segment tiling also no longer equals the new duration (inferred; not run).
4. **Other ratios lose tracking on re-export.** 1:1 and 16:9 re-exports fall back to centre crop (`api/clips.py:319-326`).
5. **`_stage_captions` is a placeholder**, only validating the style key (`runner.py:341-348`). The `hook` is never verified against the transcript. YouTube captions are downloaded but unused. There are only 4 styles, defined in code.
6. Single-pass `loudnorm` (no measured two-pass) gives approximate LUFS (`export.py:178-179`). The README claims "AAC at −14 LUFS" from an e2e test, which is a real-media assertion (`README.md:195`), but tolerance is UNVERIFIED.
7. Variable font `Inter-Variable.ttf` is used with libass `bold=False`. libass variable-font instance selection is UNVERIFIED (no test inspected for rendered weight).
8. LLM quality is untested: README admits clip picks are unverified (`README.md:203`), and the reframe acceptance bar has not been run on a golden set (`README.md:201`, `CHANGELOG.md:7-9`). The `golden` pytest marker exists, but the golden media is not in the repo (grep: marker defined in `pyproject.toml`, no fixture files).
9. Highlight scoring is purely LLM-asserted with no calibration or cross-window normalisation. Window scores are from separate calls, so a "90" in one window is not comparable to a "90" in another.
10. Pace: one FIFO job queue, no concurrency, no authentication on the HTTP API (loopback-bound by default).
11. No agent/MCP/JSON surface (G). The CLI cannot run single stages or output machine-readable results (only `config show` prints JSON, `cli.py:343-347`).
12. TrimBar can only snap to words that are inside the existing clip, though the track window extends +-5 s (`TrimBar.tsx:37-41`), so widening cannot snap outside the original clip (inference from the `words` prop use).
13. Python-version pin <3.13 due to MediaPipe is a distribution weakness (`pyproject.toml:28-29`).
14. Windows-specific escape logic exists but is mostly bypassed. The bare-relative-name trick requires copying `captions.ass` and the fonts directory into a per-clip workspace for every render (`ffmpeg.py:249-263`).

## 11. License & reuse notes

- **Repo licence: MIT, verified.** `LICENSE:1-3` is "MIT License / Copyright (c) 2026 Jad Ghazi". `pyproject.toml:9` points to the file and the classifier says MIT. Code, prompt text and architecture are freely portable with attribution.
- **Bundled fonts (5 `.ttf`)**, identified from the OpenType `name` table (ID 0/13/14) by parsing the font files directly:

| File | Family | Copyright / licence per font metadata | Where used |
|---|---|---|---|
| `backend/autoclip/assets/fonts/Anton-Regular.ttf` (170,812 B) | Anton v2.116, Vernon Adams | "Copyright 2020 The Anton Project Authors"; SIL OFL 1.1 | burned-in captions (3 of 4 presets) |
| `backend/autoclip/assets/fonts/Inter-Variable.ttf` (876,576 B) | Inter 4.001, Rasmus Andersson (variable, has `fvar`) | "Copyright 2016 The Inter Project Authors"; SIL OFL 1.1 | `clean_lower` caption preset |
| `frontend/src/fonts/Archivo-Variable.ttf` (658,596 B) | Archivo 2.001 (variable; name table says "Archivo SemiBold"), Omnibus-Type | "Copyright 2020 The Archivo Project Authors"; SIL OFL 1.1 | UI font |
| `frontend/src/fonts/InstrumentSerif-Regular.ttf` (70,012 B) | Instrument Serif 1.000, Rodrigo Fuenzalida (fragTYPE) | "Copyright 2022 The Instrument Serif Project Authors"; SIL OFL 1.1 | UI font |
| `frontend/src/fonts/InstrumentSerif-Italic.ttf` (71,592 B) | Instrument Serif Italic | same | UI font |

  All five report OS/2 `fsType = 0` (installable embedding, no embedding restriction).
- **Font-licence caveat:** no OFL text or `OFL.txt` file is shipped beside the fonts, and no `NOTICE`/third-party file exists in the repo. OFL requires the licence and copyright notice to accompany redistribution. The README only mentions Anton and Inter (`README.md:159`, `224`), not Archivo or Instrument Serif. Reuse the fonts only with a proper OFL text from upstream: Anton (googlefonts/AntonFont), Inter (rsms/inter), Archivo (Omnibus-Type/Archivo), Instrument Serif (Instrument/instrument-serif). The font files themselves are fine to bundle under OFL with that text. The repo's licence-file gap is the only issue.
- **No bundled music, SFX, or ML weights** in the repo. The MediaPipe `.task` file is downloaded at runtime; its licence is UNVERIFIED here (MediaPipe code and models are generally Apache 2.0, but not checked in this repo).
- Dependency licences (not in repo; from general knowledge, UNVERIFIED against lockfiles): faster-whisper MIT, MediaPipe Apache-2.0, PySceneDetect BSD-3, pysubs2 MIT, yt-dlp Unlicense. WhisperX is BSD-2, but pyannote diarization models are gated on HuggingFace with their own terms (`README.md:114-116`). FFmpeg is LGPL/GPL depending on build (libx264 is GPL), used only as an external subprocess, which suits our plan.
- yt-dlp ingestion has ToS/legal exposure; the code explicitly declines DRM (`ingest.py:186-190`).

## 12. Steal list (ranked)

1. **Word-index prompting + `[i]word` windows + clamp/validate/repair loop** (`highlights.py:98-114`, `base.py:196-212`, `175-181`, `252-311`). Reimplement in Rust (serde structs, brace-matching JSON extractor, one retry with the error). Make it a core MCP tool contract: any agent LLM returns word-index spans, and the Rust core resolves times. Also use `ends_sentence` regexes (`transcript.py:19-25`).
2. **Boundary refiner**: sentence snap, then duration clamp that only stops on sentence ends, then silence-trough alignment from `silencedetect` (`boundaries.py:60-237`, constants in section 6). Port directly (about 250 lines of pure logic, easy to unit-test and expose as an agent tool `refine_cut(start_word,end_word)`).
3. **Shot-aware reframe architecture**: shots -> tracks -> speaker map once per clip -> per-shot strategy (track/wide/fit) -> One Euro + dead zone + velocity clamp -> piecewise-linear `crop` expression, rendered in one filtergraph with `trim`/`concat` (`reframe/*`, `croppath.py:254-306`, `export.py:87-143`). Port the algorithms (One Euro, tracker, MAR-variance speaker logic, decimation, expression builder) to Rust. Use a Rust ONNX face model or call a Python sidecar for landmarks. Persist the `CropPath` as editable JSON so agents can inspect and override a segment (this is the hook for "agent controls reframing").
4. **Caption-to-ASS techniques that work in libass**: per-word Dialogue events with an inline `\c` + `\fscx/\fscy` + `{\r}` for the active word, `\kf` karaoke with gap-folding, resolution-independent style ratios (reference height 1920), `BorderStyle 3` boxed style, grouping by punctuation/gap/word-cap (`captions.py:201-367`). Reimplement the generator in Rust (write ASS text directly, no pysubs2 needed). Turn the four hardcoded presets into data-driven JSON templates (this repo's `CaptionStyle` fields are a good starting schema) and add more.
5. **Render hygiene**: capability probing (functional NVENC test, parse `-filters`, libass check) (`system.py:190-254`), bare-relative-path + `cwd` workspace trick to dodge Windows filtergraph escaping (`ffmpeg.py:234-265`), `-progress pipe:1` parsing (`ffmpeg.py:336-358`), cover-art-not-video check (`ffmpeg.py:186-187`). Port all as a Rust `ffmpeg` module.
6. **Blur-fit fallback** for spread-out subjects (`export.py:146-164`) and the lock-instead-of-pan rules (`__init__.py:55-61`, `338-388`) as default framing policy.
7. **Artifact-per-stage resumability** with files as the source of truth and idempotent re-entry (`runner.py:201-226`, `259-269`, `277-282`, `323-325`). Adopt this for the agent workflow: every tool reads/writes JSON in a project dir, so any agent can resume or hand off.
8. **Provider design details**: `response_format` fallback when an endpoint rejects it, Anthropic `{` prefill, error translation with actionable hints, `health_check` per provider, local vs hosted concurrency (`openai_provider.py:79-98`, `anthropic_provider.py:58-68`, `highlights.py:33-36`). Use the OpenAI-compatible adapter as the only needed LLM path in Rust (plus a thin Anthropic adapter if wanted).
9. **Fonts**: Anton (impact captions) and Inter are safe OFL caption faces. Archivo and Instrument Serif are OFL too. Bundle with the upstream OFL text, not this repo's (missing) notices.
10. Small UX ideas for the Tauri GUI: trim handles that snap to word edges (0.5 s snap radius, `TrimBar.tsx:44-58`), click-to-fix-word caption editor with locked timings (`CaptionEditor.tsx:8-14`), an SSE snapshot-on-reconnect progress stream (`useJobStream.ts:15`), and showing the crop strategy per segment in the UI (`croppath.py:35-47`).

## 13. Gaps — what this repo does not do that we need

- Agent surface: no MCP, no JSON CLI, no per-stage tool commands, no tool/JSON schemas, no skill files, no structured "explain this decision" outputs (e.g., why a cut point or crop was chosen).
- Highlight selection beyond one LLM prompt: no multi-signal scoring (audio energy, laughter, speech rate, scene/visual change, keyword/topic), no hook rewriting or ordering, no multi-range "narrative" stitching, no regenerate/add-custom-clip API, no fact/quality verification (`hook` field is never checked).
- Edit operations: no jump-cut/silence/filler-word removal, no B-roll, zoom/punch-in effects (zoompan disabled), transitions, intro/outro, title cards, watermarks, emoji, speaker name tags, or multi-source timelines.
- Captions: only 4 presets in code, no template files, no per-word colour/emoji, no highlights by keyword, no non-Latin font handling or RTL, no translation.
- Audio: no denoise (e.g., `afftdn`/RNNoise), no music/SFX, no ducking, no TTS, no two-pass loudness, no true-peak limiter beyond loudnorm TP.
- STT: only faster-whisper, no OpenAI-compatible `/audio/transcriptions`, no Whisper-API chunking, no streaming, no word-timestamp refinement (forced alignment).
- QC: no automated post-render checks (duration match, loudness measured, caption-safe-area, black-frame, face-in-frame percentage). The "acceptance bar" (>=95% speaker on-screen) is documented but never run.
- Reframing: face-only; no object/screen/slide/saliency tracking, no split-screen two-person layout, no GPU/ONNX path, no user-editable crop keyframes.
- Packaging/ops: Python-only (<3.13), heavy deps (torch for diarization), single-user no-auth web server, no Rust/Tauri, no sandboxed tool contract for agents, no cost/token accounting, no retry/backoff on rate limits (a 429 is translated and aborts that window, `openai_provider.py:134-139`, then skipped by `highlights.py:147-157`).
