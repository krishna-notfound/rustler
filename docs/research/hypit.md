# hypit — Findings

Repo: `repos/hypit` (hypit-ai/hypit v0.3.2, TypeScript pnpm monorepo, ~88 packages under `packages/`).
Method: `docs/research-methodology.md`. Every claim cites `repos/hypit/<path>:<line>`. Things not checked are marked **UNVERIFIED**.
Analysis date: 2026-11-15. Source was read directly by the author, plus four parallel read-only sub-audits (rendering, captions, providers, agent/CLI). Load-bearing citations were spot-checked.

---

## 1. Summary

Hypit is **not an AI clipper**. It is an **agent-native video *authoring* system**. A coding agent (Claude Code or Codex) uses a Skill to write a declarative XML-like language called **SVML**, plus CSS-like **.svs Recipes** and **.svrun** run files.

- A typed compiler, kernel and executor turns those sources into a build graph.
- The graph calls generation providers (image, video, TTS, WhisperX).
- An **HTML program** is rendered **frame by frame in headless Chrome via Puppeteer + CDP screenshots**, and the PNGs are piped into FFmpeg/libx264.
- The selling point is "cloning" a reference video's structure, with every event anchored to **script words rather than seconds**.

There is **no automatic highlight, virality or hook scoring**. Reference analysis is done by the coding agent itself, which looks at transcript-labelled frame grids that the CLI produces.

**Verdict.** Low value for "find highlights" (category A). **High value as a reference design** for these areas:
- an agent-drivable CLI with versioned JSON;
- a word-anchored timing model;
- a script↔ASR alignment DP;
- a declarative caption-style vocabulary;
- a deterministic frame-exact HTML renderer;
- an async provider/endpoint protocol.

The license is **not plain Apache-2.0**: it is "modified Apache" with anti-SaaS and anti-commercial-redistribution clauses (see §11). **Do not copy code; reimplement ideas.**

## 2. Stack & architecture

- **Runtime.** Node ≥22.15, TS 5.9, run directly through `tsx` with no build step (`repos/hypit/bin/hypit.mjs:3,25`). pnpm workspace (`repos/hypit/pnpm-workspace.yaml`). The single native helper is `koffi` FFI to `kernel32.dll`, used only for atomic file replace on Windows (`repos/hypit/packages/atomic-file/src/replace-file-windows.ts:2-4`).
- **Rendering.** Puppeteer driving a pinned Chrome Headless Shell 152 (`repos/hypit/packages/provider-html-local/package.json:16-17`). `.puppeteerrc.cjs` sets `skipDownload: true` (`repos/hypit/.puppeteerrc.cjs:3`).
- **Python sidecars**, managed with `uv`:
  - WhisperX 3.8.6 (`repos/hypit/packages/provider-whisperx-local/runtime/pyproject.toml:10`)
  - OpenCV still-image ops (`repos/hypit/packages/provider-image-opencv-local/runtime/raster_execute.py`)
- **GUI ("Studio").** A Vite 8 dev server with a plain-TS DOM UI, no framework (`repos/hypit/packages/studio/package.json:27`, `repos/hypit/packages/studio/index.html:9`).
- **State.** SQLite through `node:sqlite` `DatabaseSync` (`repos/hypit/packages/runtime-local/src/sqlite-state.ts:961-964`). Results are kept on the filesystem under `.hypit/results` (`repos/hypit/packages/runtime-local/src/config.ts:317`).

Data flow (from `repos/hypit/packages/compiler/README.md:8-13,49-54`, `repos/hypit/packages/protocol/src/build.ts:140-254`):

```
.svml/.svs/.svrun  --(markup: header-selected parser, package "Surfaces")-->  hypit.author-module@1
   --(author linker: refs/types/cycles)-->  hypit.graph@1  +  hypit.run-graph@1
   --(kernel)-->  hypit.plan@1  -->  hypit.build-definition@1  -->  append-only hypit.build-fact@1 log
   --(executor)-->  Producers (pure compute)  +  Endpoints (immediate | async start/poll/collect)
   --(html-video fragment)--> compile-program -> rasterize (Chrome) -> audio program (FFmpeg) -> mux -> MP4
```

Packages are plugins. Each one exports `hypitPackage = {format:"hypit.package@1", facets:[...]}` with producer, admission, markup, endpoint and Studio facets (`repos/hypit/packages/provider-tokendance/src/activation.ts:13-60`, `repos/hypit/packages/caption-fine/src/activation.ts:16-38`).

## 3. Repo map (important parts only)

| Path | Role |
|---|---|
| `skills/hypit/SKILL.md` (328 lines) + `references/**` (~60 md) | The agent "brain": a director persona, rules, and a routing table to sub-docs |
| `packages/cli` | Generic CLI: `check/plan/pricing/build/status/logs/inspect/get/auth/doctor` |
| `packages/media-local` | `hypit media probe/cut/frames/tile/tiles/boundaries`, all FFmpeg normalize/mix/mux |
| `packages/provider-html-local` | Headless-Chrome frame capture → libx264 |
| `packages/html-program` | Compiles a composition into a deterministic seekable HTML page |
| `packages/caption`, `caption-fine`, `narrative-caption` | Caption data model, styled karaoke renderer, word→caption timing |
| `packages/script`, `narrative*`, `temporal`, `timeline-author` | Script language, anchors, frame-exact rational time |
| `packages/narrative-speech-alignment` | Script-text ↔ WhisperX word alignment DP |
| `packages/whisperx`, `provider-whisperx-local` | STT capability plus a local Python HTTP sidecar |
| `packages/endpoint`, `generation`, `provider-*` | Provider abstraction and 8 hosted vendors (HypiHub, TokenDance, BeatAPI, HiAPI, Pollo, Sume, Monid) |
| `packages/ranking`, `comment-sticker`, `interview-emoji-reveal`, `depth-stack`, `visual-track`, `text-fine` | Motion-graphics components |
| `packages/*-kits` (`seedance-kits`, `gpt-image-kits`) | Axis-based prompt templates in `.svs` |
| `packages/studio`, `studio-companion` | GUI and the per-component GUI plugin interface |
| `examples/{ranking-football,podcast,interview,complex-explainer,...}` | Full productions (`.svml`/`.svs`/`.svrun`) plus assets |

Binary assets (noted, not analysed):
- Example `.wav`/`.m4a` voice and soundtrack files, plus real footballer `.jpg` photos, under `repos/hypit/examples/*/assets/`.
- Tabler-icon PNGs (`repos/hypit/examples/interview/assets/emoji-reveal/rendered/`).
- `preview/` PNGs per package, and logos in `docs/public`.
- **No font files are bundled.**

## 4. Capabilities by category

### A. AI clipping — 1/5 (absent by design)
- There is no highlight, virality or hook detection and no scoring. "viral" appears only in README marketing (`repos/hypit/README.md:8,174`). `packages/ranking` is a tier-list *graphic* (`repos/hypit/packages/ranking/README.md:15-19`). "highlight" in code means a user-set `highlightedOutputs` field (`repos/hypit/packages/result/src/types.ts:197-198`).
- The agent picks moments by reading the material. It is told to "Watch the whole reference from opening to close. Follow its hook, argument or story" (`repos/hypit/skills/hypit/references/creation/reference-video.md:12`) and to "Discover why it holds attention" (`repos/hypit/skills/hypit/SKILL.md:19-20`).
- The only mechanical aid is `hypit media boundaries`: a naive cut detector, 32×32 RGB at 12 fps, mean absolute difference, threshold 0.1 (`repos/hypit/packages/media-local/src/files.ts:467-491`). It is explicitly labelled "scores are not shot labels" (`repos/hypit/packages/media-local/src/files.ts:799`).
- **Perception tools for the agent (the useful part).** `tile`/`tiles` build contact sheets labelled with time and words. Options: `--transcript`, `--around "<phrase>"` with `--occurrence` and `--padding`, `--every-frame`, `--ranges <json>` (`repos/hypit/packages/media-local/src/files.ts:808-819`; `repos/hypit/skills/hypit/references/creation/reference-video.md:103-162`). This is a strong pattern for letting a vision-capable LLM "watch" video.

### B. Transcription & subtitles — 4/5
- **WhisperX.** Hosted through HypiHub `POST /audio/transcriptions` with `model:"victor-upmeet/whisperx", response_format:"verbose_json", timestamp_granularities:["segment","word"]` (`repos/hypit/packages/provider-hypihub/src/provider.ts:595,613-619`).
  - Locally it runs as a loopback Python `ThreadingHTTPServer` (`GET /health`, `POST /transcribe`) (`repos/hypit/packages/provider-whisperx-local/runtime/src/hypit_whisperx_service/server.py:43-53,89`).
  - Local defaults: model `small`, `cpu`, `int8`, batch 8 (`repos/hypit/packages/provider-whisperx-local/src/provider.ts:22-29,110`).
  - It handles one request at a time, raising `InferenceBusyError` otherwise (`.../runtime/src/hypit_whisperx_service/engine.py:170-171`).
  - Missing word times stay untimed: "Never manufacture a tick" (`.../engine.py:70`).
- **Script↔ASR alignment** is the standout piece (§6). Caption *text* comes from the authored Script, not from the ASR output (`repos/hypit/packages/whisperx/README.md:54-55`).
- **Speakers** are Script Roles (`<HOST>`, `<WIFE>`) carried on `CaptionCue.role` (`repos/hypit/packages/caption/src/types.ts:27-31`). There is no diarization.
- **Cue segmentation is authored**, using `||` in the Script. There is no auto-chunking: "A fixed word or character count cannot make that choice" (`repos/hypit/skills/hypit/references/playbooks/craft/captions.md:74-75`). A segment close is a hard cue boundary (`repos/hypit/packages/script/src/narrative.ts:101-104`). CJK is split one character per word (`repos/hypit/packages/script/src/lexical.ts:6`).
- **No SRT/VTT/ASS export or import.** These appear only in prose (`repos/hypit/packages/caption/README.md:16`).
- **Emoji** captions are UNVERIFIED (fonts-and-text docs mention Emoji, `repos/hypit/skills/hypit/SKILL.md:308`). The emoji-reveal component uses rasterized Tabler icons.

### C. Creative design (captions, fonts, motion) — 4/5
- **Caption engine** is `caption-fine`. It produces declarative keyframes, then CSS `@keyframes` with paused state, then a sample per frame through a throwaway `new Animation(new KeyframeEffect(...))` with `currentTime = localTime` (`repos/hypit/packages/html-program/src/document.ts:711-717`). It uses no GSAP and no canvas.
- **Karaoke.**
  - `karaoke: off|current|trail`, `karaoke-transition: step|wipe`. Wipe is a `clip-path: inset(0 X% 0 0)` (RTL aware) (`repos/hypit/packages/caption-fine/src/render.ts:458-461`).
  - Active box modes `off|current|trail`, with continuity `isolated|joined`. Joined boxes are measured in the DOM and merged into one SVG outline (`repos/hypit/packages/caption-fine/src/joined-box.ts:1-60`).
  - Default active fill is `#FFD54A` (`repos/hypit/packages/caption-fine/src/style.ts:106`).
  - 24 one-shot motions (fade/pop/spring/wipe-*) (`repos/hypit/packages/caption-fine/src/recipe.ts:3-7`).
  - Loops: shake, wobble, glow-pulse, breathe, float, pulse, flicker (`repos/hypit/packages/caption-fine/src/style.ts:280-283`).
  - Easing: easeOutBack, Bounce and Elastic (`repos/hypit/packages/caption-fine/src/render.ts:265-290`).
- **Glyph paint.** Stroke is drawn outside the glyph as ordered paint layers rather than `-webkit-text-stroke` (`repos/hypit/packages/caption-fine/src/render.ts:75-85,140-164`). Long shadow uses up to 32 steps (`:104-117`).
- **Line breaking.** `max-words-per-line` is greedy (`repos/hypit/packages/caption-fine/src/render.ts:628-639`). `max-lines` is a hard assert, not a fitter (`:719-723`). Otherwise CSS width wrapping applies (`:709-716`).
- **Text fitting (text-fine).** `overflow: shrink` runs a 20-step binary search on CSS `scale()` using DOM `getBoundingClientRect` (`repos/hypit/packages/html-program/src/text.ts:1069-1141`).
- **Fonts.**
  - Loaded from installed `@fontsource[-variable]/*` npm packages by reading `metadata.json` and CSS without running their code; only woff2 plus unicode-range is used (`repos/hypit/packages/fontsource/src/surface.ts:34,55-71,119-133`). Local `.ttf/.otf/.woff/.woff2` files are also accepted (`repos/hypit/packages/media/README.md:35-46`).
  - `@font-face` uses `font-display:block`, a hashed family id, and `font-synthesis:none` (`repos/hypit/packages/html-program/src/document.ts:624-634`; `repos/hypit/packages/html-program/src/text.ts:70`). Rendering waits on `document.fonts.ready`.
  - Dev fonts: Inter, League Spartan, Montserrat and Noto Sans SC variable, plus Archivo Black, all at 5.3.0 (`repos/hypit/package.json:308-312`).
- **Templates are `.svs` Recipes per project.** There is no built-in preset library. A real "color-box karaoke" recipe (`repos/hypit/examples/ranking-football/recipes.svs:78-128`):
  ```
  caption.primary { x: 0.5; y: 0.55; width: 0.88; height: 0.18; max-lines: 2; max-words-per-line: 4;
    line-height: 1.28; size: 44; letter-spacing: -0.8; fill: #FFFFFF; stroke-color: #090A0F; stroke-width: 2;
    shadow-opacity: 0.72; shadow-y: 3; shadow-blur: 7; karaoke: current; karaoke-transition: step;
    active-box: current; active-box-continuity: isolated; active-box-background: #008FC7; active-box-padding: 4 9;
    active-box-radius: 5; active-box-transition-frames: 2; active-response: pop; active-response-frames: 3;
    active-scale: 1.04; lead-frames: 2; tail-frames: 2; handoff: cut; cue-enter: fade; cue-enter-frames: 2; cue-exit: fade; cue-exit-frames: 2; }
  ```
  Speaker-coloured styles are one Style per Role, e.g. wife `#FFABD4`, wojak `#42D16B` (`repos/hypit/examples/interview/recipes.svs:78,172`), selected with `<caption-fine:Use role="WOJAK" .../>` (`repos/hypit/examples/interview/swap-host.svml:239`).
- **Components.**
  - Ranking: TierBoard/Column/TopThree with synced appear/move SFX (`repos/hypit/packages/ranking/src/schedule.ts:826-887`).
  - Comment stickers (`repos/hypit/packages/comment-sticker/src/types.ts:14-73`).
  - Emoji reveal strip (`repos/hypit/packages/interview-emoji-reveal/src/style.ts:35-43`).
  - Depth-stack card deck (`repos/hypit/packages/depth-stack/src/types.ts:14-69`).
  - Path/point/flow text (`repos/hypit/packages/text-fine/README.md:6-13`).
- **Schemas** are hand-written decoders that reject unknown keys, plus in-house `ValueSchema`. No zod (`repos/hypit/packages/caption-fine/src/recipe.ts:99-120`).

### D. Video processing & rendering — 4/5 (correctness), 2/5 (speed)
- **Deterministic HTML frame program.**
  - The page exposes `window.__hypitFrameProgram.prepare()/applyFrame(frame)` (`repos/hypit/packages/html-program/src/document.ts:735-762`).
  - Each frame: apply the frame, swap `<video>` for pre-decoded `<img>` frames, wait for image decode and `document.fonts.ready`, then CDP `Page.captureScreenshot {format:"png", optimizeForSpeed:true, fromSurface:true}` (`repos/hypit/packages/provider-html-local/src/opaque-capture.ts:228-283`).
  - The background is forced to opaque black (`:20`), so the **final render can never carry alpha**.
- **Parallelism.**
  - Workers are separate Chrome processes. The auto ceiling is `min(cpu-2, floor(mem/2/1.5GiB))` (`repos/hypit/packages/provider-html-local/src/concurrency.ts:9-14`), capped at **one browser per second of video** (`:17-23`).
  - An adaptive controller adds or removes browsers based on measured throughput (`:39-87`).
  - Batches are about one second of frames (`repos/hypit/packages/provider-html-local/src/capture.ts:46-47`).
  - `OrderedFrameSink` reorders PNGs within a 256 MiB budget and streams them to **one** FFmpeg stdin, so no chunks are concatenated (`repos/hypit/packages/provider-html-local/src/ordered-frame-sink.ts:12-139`).
  - The README's "64 headless Chromium processes" (`repos/hypit/README.md:95`) is not a code constant. Which configuration produced it is UNVERIFIED.
- **Output verification.** An ffprobe check confirms exactly one h264 stream with the expected size, fps and counted frame count (`repos/hypit/packages/provider-html-local/src/output.ts:39-73`). Surfaces are validated as SDR sRGB/BT.709, square pixels, no rotation (`repos/hypit/packages/provider-html-local/src/surface-validation.ts:91-103,180-197`).
- **Media normalization** to a CFR timeline. Remuxing is skipped when the source is already compliant H.264 (`repos/hypit/packages/media-local/src/execute.ts:616-645`). Alpha is kept as lossless VP9 `yuva420p` (`:707-708`).
- **No HW encode** (no NVENC/QSV/VAAPI). The only HW use is macOS VideoToolbox decode for HDR sources (`repos/hypit/packages/provider-html-local/src/source-frame-decoder.ts:115`).
- **Downloads.** yt-dlp `--format "bv*+ba/b" --format-sort res:1080,vcodec:h264 --no-playlist` (`repos/hypit/packages/yt-dlp/src/download.ts:69-78`).

### E. Reframing — 2/5
- Purely geometric fitting: contain, cover, fit-width/height, native, scale-down, stretch, with anchors and offset (`repos/hypit/packages/spatial/src/geometry.ts:189-219`).
- **No face or saliency detection in the repo.** `region-evidence` takes precomputed per-frame boxes `[x,y,w,h]|null` and "performs no detection, identity inference, interpolation" (`repos/hypit/packages/region-evidence/README.md:3-5`).
- Head-tracked captions place the cue at the box's top centre each frame and hide it on `null` (`repos/hypit/packages/caption-fine/src/render.ts:600-626`).
- Face boxes came from Google Video Intelligence `FACE_DETECTION` run outside the repo (`repos/hypit/skills/hypit/references/playbooks/craft/caption-tracking.md:34-71`). The YOLOv8 AnimeFace step claimed in the README (`repos/hypit/README.md:139`) has no code (UNVERIFIED).
- Matting is remote: a Volcengine model through HypiHub `POST /v1/videos` (`repos/hypit/packages/volcengine-matting/src/index.ts:8-16`).
- Split-screen interview layouts are authored as Frames and Visual Clips, not automatic.

### F. Audio — 3/5
- **Sample-accurate FFmpeg mix on a 48 kHz clock.** Per clip: `atrim`, optional `aloop`, `atempo` chain (each step 0.5–2), `apad`, `volume`, then an `aeval` gain envelope and `adelay` (`repos/hypit/packages/media-local/src/execute.ts:477-518`). Then `amix ... normalize=0` (`:1236-1242`).
- **Gain automation** is one per-sample `aeval` expression with linear ramps (`repos/hypit/packages/media-local/src/audio-level-automation.ts:4-31`). Envelope building block: `repos/hypit/packages/composition/src/audio-level-automation.ts:4-19`.
- **No loudnorm, no limiter, no automatic ducking, no silence removal, no denoise of audio, no beat detection.**
  - "Ducking, crossfades … are ordinary components" (`repos/hypit/packages/audio-track/README.md:49-52`).
  - The `loudness: "preserve"` profile is the only one (`repos/hypit/packages/media-local/src/execute.ts:397`).
  - `@hypit/bach` is a video model, not Bach music or beat analysis (`repos/hypit/packages/bach/README.md:3-4`).
- SFX are tied to Script Moments, e.g. ranking appear/move sounds.

### G. Agent integration — 4/5 (no MCP)
- **CLI** with versioned JSON outputs: `hypit.cli-check@1`, `hypit.cli-plan@1`, `hypit.cli-build@1`, and `hypit.cli-error@1 {ok:false,error:{code,message,help?,trace?}}` (`repos/hypit/packages/cli/src/main.ts:207,360,548`; `repos/hypit/packages/cli/src/output.ts:1029-1038`). Progress goes to stderr, the result to stdout (`repos/hypit/packages/cli/README.md:60-62`). Exit codes are only 0/1.
- **Agent loop:** `check → plan (preflight diagnostics, no spend) → pricing → build --follow → status --watch / logs → inspect / get` (`repos/hypit/skills/hypit/references/production/builds.md:20-25`). Human output prints the next commands to run (`repos/hypit/packages/cli/src/main.ts:435-484`).
- **No MCP server.** Grepping for `mcp|modelcontextprotocol` matches only README partner blurbs. Integration is CLI plus a Skill (`skills/hypit`, symlinked into `.claude/skills` and `.codex/skills`; Codex metadata in `repos/hypit/skills/hypit/agents/openai.yaml:1-4`).
- **Skill design.**
  - Persona: "You are the director and producer" (`repos/hypit/skills/hypit/SKILL.md:8`).
  - Standing rules: Money/budget consent (`:220-225`), Execution (`:234-237`), Secrets (`:238-240`), Evidence (`:241-243`), **"Files are the memory"** (`:244-250`), **"Done means watched"** (`:256-262`).
  - A 57-row routing table points to the reference doc to read per question (`:264-323`).
  - Project memory layout: `references/<ref>/{ANALYSIS.md,TIMELINE.md,transcript.json,evidence/}` and `productions/<target>/{BRIEF.md,TREATMENT.md,PROGRESS.md,authors/,recipes/,runs/}` (`repos/hypit/skills/hypit/references/creation/project-files.md:89-114`).
- **Paid-work safety.** Async submissions keep receipts so paid jobs are not resubmitted. HypiHub sends an `idempotency-key` (`repos/hypit/packages/provider-hypihub/src/provider.ts:494`).
- **Doc/CLI drift.** Docs list `hypit media fetch` but it does not exist (`repos/hypit/skills/hypit/references/creation/reference-video.md:101` vs `repos/hypit/packages/media-local/src/files.ts:19`). The snapshot doc uses a positional HTML path that the CLI rejects (`repos/hypit/packages/studio/src/snapshot-cli.ts:56`).

### H. GUI — 3.5/5
- Studio is Vite on port 5179 with:
  - a frame-snapped player;
  - a timeline with Script projection rows;
  - drag/trim edits **written back to the SVML source** in one transaction, rejected if the recompile lands elsewhere (`repos/hypit/packages/studio/README.md:214-230,299-301`);
  - a source editor and an Inspector;
  - timestamped Comments saved to `FEEDBACK.json` (`:100-122`). Agent notification is "planned" only (`:141-142`).
- **Preview** loads the same compiled HtmlProgram in a sandboxed iframe, seeking via `__hypitSeekFrame` (`repos/hypit/packages/studio/src/ui/stage.ts:179`). Live reload goes over the Vite websocket (`repos/hypit/packages/studio/src/server.ts:156,166`).
- **Component GUI plugin.** `studio-companion` declares lanes, items, Where/When/How inspector fields and edit handles as data; "cannot ship arbitrary DOM or CSS" (`repos/hypit/packages/studio-companion/src/index.ts:174,263-302`; `repos/hypit/packages/studio/README.md:273-274`).
- "Opening Studio never spends and never creates a Build" (`repos/hypit/packages/studio/README.md:303-310`).

### I. Provider abstraction — 4/5 (media), 1/5 (LLM)
- **Endpoint protocol** (`repos/hypit/packages/endpoint/src/index.ts`):
  - Immediate handlers, or async `{start, poll, cancel?, collect?}` (`:180-185`).
  - Outcomes: `pending{handle,wakeAt}` / `completed` / `failed` / `ready` (`:144-155`).
  - Providers say when to poll again with `wakeAfter` (`:478-497`); the runtime owns no polling loop.
  - `checkpoint` persists the remote receipt (`:119-123`).
  - Cancel results: `confirmed|accepted|unsupported|too-late` (`:174-178`).
  - Capacity claims `pool:`/`capacity:`/`rate:` (`:125-142,443-452`).
  - `supports(request)` (`:209-211`).
- **Model ports vs Provider wire mapping**, kept separate and data-only:
  - A model declares a `GenerationPortTable` (`repos/hypit/packages/generation/src/ports.ts:78-84`).
  - A provider maps it with a `GenerationWireMapping {capability, result, routes, fields, constants}` (`repos/hypit/packages/generation/src/mapping.ts:49-60`). "First matching route wins".
- **Retries are per provider.** 429/≥500 during poll leads to pending with `Retry-After` (`repos/hypit/packages/provider-tokendance/src/provider.ts:67-74`). A submit transport error is terminal `*_SUBMISSION_UNKNOWN` (`:75-80`). The executor never retries (`repos/hypit/packages/executor/src/planning.ts:20`).
- **Pricing** is a page link, or a passthrough of the vendor's `/v1/pricing` response. No cost totals (`repos/hypit/packages/endpoint/README.md:110-111`).
- **TTS** (ElevenLabs voice-design, FishAudio design/clone, MiMo design/clone) is declared as model ports, but **only routed through HypiHub `POST /v1/audio/speech`**. No direct vendor clients and **no TTS timestamps**, so timing always comes from a later WhisperX pass (`repos/hypit/packages/provider-hypihub/src/provider.ts:249-299`).
- **LLM: none.** Hypit makes no chat/completions calls. The only one is CI issue triage to DeepSeek (`repos/hypit/.github/automation/analysis.mjs:69`). The coding agent does all reasoning. The docs warn that "'OpenAI-compatible' branding do[es] not establish matching … protocols" (`repos/hypit/skills/hypit/references/environment/model-and-provider.md:175`).
- **Credentials.**
  - `{store,key}` references.
  - Env store reads only the exact variable named (`repos/hypit/packages/credential-store-env/src/index.ts:8-22`).
  - Local store: macOS Keychain, Windows Credential Locker (512-char limit), or files `key-<hex>.json` with mode 0600 (`repos/hypit/packages/credential-store-local/src/file.ts:28-34,70-80`).
  - State root: `$HYPIT_STATE_HOME`, else OS default (`repos/hypit/packages/cli/src/paths.ts:14-29`).

## 5. Pipeline walkthrough (reference clone → MP4)

1. **Fetch.** `hypit download <url> --to ref.mp4` (`repos/hypit/packages/yt-dlp/src/cli.ts:44`).
2. **Transcribe.** `hypit transcribe ref.mp4 --language en --to transcript.json` writes `hypit.transcript@1` (`repos/hypit/packages/whisperx/src/cli.ts:86-90,129-131`). Audio is extracted as 16 kHz mono s16 (`repos/hypit/packages/media-local/src/speech.ts:55-57`).
3. **Agent reads the reference** with `media boundaries`, then `tile`/`tiles --transcript --around "<phrase>"`, and writes ANALYSIS.md and TIMELINE.md (`repos/hypit/skills/hypit/references/creation/reference-video.md:94-198`).
4. **Agent authors** the SVML Script with `||` cue breaks, `@{moment!}` Moments, `@{x}…@{/x}` Selections and `<display|spoken>` Dual Text (`repos/hypit/packages/script/README.md:75-87`). It also writes generation requests (`gpt:Image`, `seedance:ReferenceVideo`, TTS), Recipes and the Film tracks (`repos/hypit/examples/ranking-football/reference.svml:1-356`).
5. **Check and plan.** `hypit check` validates; `hypit plan` runs preflight with no spend (`repos/hypit/packages/cli/src/main.ts:326-333,567-578`).
6. **Build.** Async provider jobs are submitted, polled and collected. Generated takes are normalized (`mediaop:Normalize`). `whisperx:Alignment` gives NarrativeAlignment, which drives caption timing and every semantic Window/Instant (`repos/hypit/examples/ranking-football/reference.svml:220-231,337-344`).
7. **Render** (`repos/hypit/packages/html-video/src/fragment.ts:20-60`): composition to HtmlProgram, Chrome rasterize to H.264, audio program to 48 kHz WAV, then mux with `-c:v copy -c:a aac`.
8. **Results.** Written to `.hypit/results/<bucket>/<buildId>/result.json` plus `execution.jsonl` (`repos/hypit/packages/result/src/node/store.ts:49-50,204-212`).
9. **Review.** The agent reviews via `hypit snapshot`/Studio and watches the output ("Done means watched").
10. **Reuse.** Earlier outputs are reused **only explicitly** through `.svrun` `<build-record>` + `<satisfy>` lines (`repos/hypit/examples/complex-explainer/productions/explainer/runs/export-parts.svrun:9-10`).

## 6. Prompts & heuristics

**Hypit has no internal LLM prompts.** Its "prompts" are (a) the agent Skill and (b) generation Prompt Kits.

**Prompt Kits** are axis-based templates in `.svs`: fixed blocks plus `axis` blocks with enumerated choices (`repos/hypit/packages/seedance-kits/kits/broll-v1.svs:3-40`):
```
text-template.broll-v1 { default-material-mode: practical-real; default-story-shape: single-moment; default-edit-language: continuous-shot; ... }
block.contract { kind: fixed; order: 20; text: "B-ROLL CONTRACT: create visual support footage. Focus on visible action, object behavior, camera movement, and editing rhythm. ..." }
block.material { kind: axis; order: 40; parameter: material-mode; }
choice.material.practical-real { text: "MATERIAL MODE: practical real-world UGC footage with believable physical causality and a lived-in phone-camera feeling." }
choice.story-shape.reaction-beat { text: "STORY SHAPE: stimulus, realization, then a clearly visible reaction." }
choice.edit.jump-cut-beats { text: "EDIT LANGUAGE: crisp jump-cut beats, with each meaningful action stage landing cleanly." }
```
Other kits: speaker, podcast, street-interview, call, camera-reference, motion-reference (`repos/hypit/packages/seedance-kits/kits/`) and phone-ugc for GPT-Image (`repos/hypit/packages/gpt-image-kits/kits/phone-ugc-v1.svs`).

**Script-to-ASR alignment DP** (`repos/hypit/packages/narrative-speech-alignment/src/align.ts`):
- Normalize: NFKC, lowercase, keep only letters, marks and numbers (`.../normalize.ts:1-6`).
- Fast path when tokens are identical (`:129-139`).
- Otherwise a monotonic M:N DP with groups up to `DEFAULT_MAX_GROUP = 4` (`:8`). Omission costs 0.5 (`:237`), insertion 0.35 (`:254`).
- Pair cost (`:72-77`):
  `editDistance(chars)/width * reliability + 0.055*(s+e-2) + 0.06*unmatchedWordBoundaries`
- `reliability = 0.5 + 0.5*avgScore`, defaulting to 0.8 when there are no scores (`:25-32`).
- Tie-break order: more exact tokens, fewer omissions, fewer insertions, lower complexity (`:91-99`).
- Within a group, a char-level Levenshtein backtrace assigns time spans (`.../locate.ts:298-339`).
- Untimed tokens are interpolated between neighbours weighted by character count (`:358-390`).
- Quantized to frames with floor for start, ceil for end, minimum 1 frame (`:409-423`).

**Speech duration estimate without TTS:**
- `seconds = syllables / rate + padding`.
- en rates 4.2 / 4.6 / 5.6 syllables per second for slow / normal / fast; zh 5.25 normal; ja 7.5 normal (`repos/hypit/packages/speech-estimate/src/program.ts:17-20`).
- Syllables from CMUdict with a vowel-group fallback (`:53-80`).

**Other thresholds:**
- Boundaries: rate 12, threshold 0.1 (`repos/hypit/packages/media-local/src/files.ts:467-468`).
- Render workers: about 1.5 GiB per Chrome (`repos/hypit/packages/provider-html-local/src/concurrency.ts:12`).
- Caption `wordGapPx = 0.25*fontSize` (`repos/hypit/packages/caption-fine/src/style.ts:208`).
- Rational time rounds to the nearest frame, half to the later frame (`repos/hypit/packages/temporal/src/rational.ts:74-80`).

**Key Skill directives** (quoted):
- "For spoken references, use transcript-linked grids so the words and picture changes can be read together" (`repos/hypit/skills/hypit/SKILL.md:161-163`).
- "**Done means watched.** Watch and listen to the actual deliverable and judge it against the Brief, Treatment…" (`:256-257`).
- "Unsupported claims about observed media remain unknown" (`:242-243`).
- "Before paid work, establish the billing accounts, covered work and expected cost or budget the user accepts" (`:220-221`).

## 7. FFmpeg commands actually used (verbatim)

**Final encode**, PNG frames from Chrome (`repos/hypit/packages/provider-html-local/src/capture.ts:143-150`). CRF is draft 28 / standard 23 / high 18.
```
-v error -y -f image2pipe -framerate {num}/{den} -vcodec png -i pipe:0 -frames:v N -an -c:v libx264 -crf {28|23|18}
-preset {veryfast|medium} -vf scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv
-movflags +faststart visual.mp4
```

**Source-frame decode for the renderer** (`repos/hypit/packages/provider-html-local/src/source-frame-decoder.ts:114-125`). It forces libvpx/prores decoders for alpha and uses `-hwaccel videotoolbox` only for macOS HDR.
```
[-c:v libvpx-vp9|libvpx|prores] -ss t -i path -t dur [-vf fps=num/den | -fps_mode cfr -r R] -frames:v count -q:v 0 -compression_level 1 -f image2pipe -vcodec png pipe:1
```

**Normalize video** (`repos/hypit/packages/media-local/src/execute.ts:687-722`):
```
-vf setpts=PTS-STARTPTS,fps=fps={fps}:round=near:start_time=0:eof_action=round,trim=start_frame=0:end_frame={N},setpts=N*{den}/({num}*TB),
    scale=trunc(iw*max(sar\,1)/2)*2:trunc(ih*max(1/sar\,1)/2)*2,setsar=1  -frames:v N -r fps -fps_mode cfr
  opaque: -c:v libx264 -preset veryfast -pix_fmt yuv420p -movflags +faststart
  alpha : -c:v libvpx-vp9 -pix_fmt yuva420p -lossless 1 -auto-alt-ref 0 -row-mt 1 -deadline realtime -cpu-used 8
```

**Remux fast path** (`:644-645`):
```
-y -copyts -i in -map 0:{idx} -an -sn -dn -map_metadata -1 -c:v copy -avoid_negative_ts disabled -movflags +faststart
```

**Still images to video** (`:832-873`):
```
select=eq(n\,0),loop=loop=-1:size=1:start=0,trim=start_frame=0:end_frame=F,settb=expr=den/num,setpts=N*den/(num*TB)
scale=W:H:force_original_aspect_ratio=decrease,pad=W:H:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1 ... concat=n=K:v=1:a=0
```

**Normalize audio** (`:749-763`):
```
asetpts=PTS-STARTPTS,aresample=48000:async=0:first_pts=0,aformat=sample_rates=48000:channel_layouts=stereo,atrim=...,asetpts=N/SR/TB,adelay={head}S:all=1,apad=whole_len={S},atrim=start_sample=0:end_sample={S},asetpts=N/SR/TB
-c:a pcm_s16le -ar 48000 -ac 2
```

**Timeline mix**: per-clip chain at `:499-518`, mix at `:1236-1242`, gain at `repos/hypit/packages/media-local/src/audio-level-automation.ts:31`.
```
[i]atrim=start_sample=..:end_sample=..,asetpts=PTS-STARTPTS,[aloop=loop=-1:size=L:start=0,]atempo=..,atrim,apad=whole_len=L,atrim,volume=g,aeval=exprs='val(0)*(G)|val(1)*(G)',atrim,asetpts=N/SR/TB,adelay=xS:all=1[a_i];
[a_0]..[a_n]amix=inputs=n:duration=longest:dropout_transition=0:normalize=0,apad=whole_len=W,atrim=start_sample=0:end_sample=W,asetpts=N/SR/TB[out]
-c:a pcm_s16le -ar 48000 -ac 2      (silence: -f lavfi -i anullsrc=r=48000:cl=stereo)
```

**Final mux** (`repos/hypit/packages/media-local/src/execute.ts:1316-1319`):
```
-y -i visual -i audio -map 0:{v} -map 1:0 -c:v copy -c:a aac -ar 48000 -ac 2 -movflags +faststart out.mp4
```

**CLI evidence tools** (`repos/hypit/packages/media-local/src/files.ts`):
- Cut: `-c:v libx264 -preset veryfast -crf 23 -c:a aac -b:a 96k -movflags +faststart` (`:289-292`).
- Time-label overlay: `[0:v][1:v]overlay=8:8:eof_action=repeat[v]` fed by an rgb24 rawvideo pipe (`:307-310`).
- Multi-span cut: `split/asplit → trim/atrim → concat`, `-crf 17 -preset medium -fps_mode vfr`, audio `-b:a 192k` (`:335-353`).
- Accurate seek: input `-ss (t-2)`, then output `-ss 2` (`:199-203`).
- Boundaries: `-vf fps=12,scale=32:32,format=rgb24 -f rawvideo -` (`:475-476`).

**Studio filmstrip** (`repos/hypit/packages/studio/src/storyboard.ts:117-130`):
```
fps=..,scale=w:80:force_original_aspect_ratio=decrease,pad=..:black,tile=layout=CxR:nb_frames=N:padding=0:margin=0
```

**Probing** (`repos/hypit/packages/media-local/src/execute.ts:271`):
```
ffprobe -v error -print_format json -show_format -show_streams -show_frames in
```
`-show_frames` runs on every inspection and is expensive.

## 8. Dependencies & external services
- **Required locally:** Node 22.15+, pnpm, FFmpeg/ffprobe on PATH or configured, and Chrome Headless Shell 152 installed via `hypit programs prepare` (`repos/hypit/packages/provider-html-local/README.md:39-41`).
- **Optional local:** Python + uv for WhisperX (CPU works; GPU optional, UNVERIFIED for CUDA paths) and for OpenCV.
- **Hosted, all paid:**
  - HypiHub (`https://hypit.ai/v1`, OAuth PKCE, `repos/hypit/packages/provider-hypihub/src/provider.ts:573,639-646`) is the primary route, and the **only** route for TTS, matting and hosted WhisperX.
  - Generation gateways: TokenDance, BeatAPI, HiAPI, Pollo, Sume, Monid.
  - Models: Seedance 2, GPT-Image 2, Seedream, Wan, PixVerse, Grok Imagine, MiniMax, Nano Banana, Bach, ElevenLabs/FishAudio/MiMo TTS.
- **Off-repo:** Google Video Intelligence for face boxes (manual).

## 9. Performance notes
- Rendering screenshots every frame as PNG over CDP, then PNG-decodes in FFmpeg. CPU-heavy, but it scales linearly with Chrome workers, and an adaptive controller tunes the worker count (`repos/hypit/packages/provider-html-local/src/concurrency.ts:39-87`).
- Auto mode is capped at one browser per second of video, so short clips get little parallelism (`:22`).
- Source video inside the HTML is decoded by a single FFmpeg pump in windows of at most 16 frames, PNG-encoded, written to disk and served over HTTP to `<img>`, under a 1 GiB LRU (`repos/hypit/packages/provider-html-local/src/source-frame-store.ts:7,123-130`; `repos/hypit/packages/provider-html-local/src/render.ts:66`). This is heavy I/O.
- Single-frame decode uses output-side `-ss` (`repos/hypit/packages/provider-html-local/src/source-frame-decoder.ts:117`), which is slow on long sources.
- No content-addressed caching: blobs get random `res_<uuid>` ids (`repos/hypit/packages/runtime-local/src/resource-store.ts:90`). Cross-build reuse is manual.
- Within one build, the kernel only re-wakes steps whose inputs changed (`repos/hypit/packages/kernel/README.md:13-16`).
- Timeouts: render 30 min (`repos/hypit/packages/provider-html-local/src/render.ts:62`), media ops 10 min, hosted operations 20–30 min.

## 10. Weaknesses / bugs / limitations
1. No automatic clip or highlight selection, no scoring, no hook detection. All of category A is left to the agent's judgement.
2. No face tracking, auto-reframe or diarization in the repo. Head-tracked captions need external detector output, and there is no smoothing inside Hypit (`repos/hypit/packages/caption-fine/README.md:37-39`).
3. Audio: no loudnorm or limiter, and `amix normalize=0` summed into s16 can clip (`repos/hypit/packages/media-local/src/execute.ts:1236`). No auto-ducking, silence removal or beat detection. The AAC bitrate is left at FFmpeg's default (`:1318`).
4. No subtitle-file export (SRT/VTT/ASS). Captions exist only as burned-in pixels.
5. No HW encoding. The final render is opaque yuv420p only, so no alpha or ProRes output (`repos/hypit/packages/provider-html-local/src/opaque-capture.ts:20`).
6. Mixed CRF/preset across intermediate steps (veryfast at default CRF), so generations of quality loss accumulate (`repos/hypit/packages/media-local/src/execute.ts:710,872,993`).
7. `hypit media` CLI hardcodes `"ffmpeg"` and ignores a configured path (`repos/hypit/packages/media-local/src/files.ts:291,475`).
8. Chrome always runs with `--no-sandbox` (`repos/hypit/packages/provider-html-local/src/capture.ts:187-191`). `browserGpu` `auto` and `hardware` give the same arguments.
9. TTS returns no timestamps and is routed only through HypiHub, which creates vendor lock-in for speech, matting and hosted STT.
10. No LLM abstraction. Outside a coding agent there is no autonomy at all.
11. Exit codes are only 0/1. There is no MCP server. Docs drift from the CLI (`media fetch`, snapshot positional path).
12. The Skill prose is long and abstract (directing philosophy rather than procedures), which is expensive for agent context.
13. The engine is very heavyweight: ~88 packages and a custom type system/compiler for what is essentially a timeline. The learning curve is steep for agents and humans.

## 11. License & reuse notes
- **Repo license** (`repos/hypit/LICENSE:3-29`): "a modified version of the Apache License 2.0" with these conditions:
  - (a) **No multi-tenant / SaaS** use without written authorization.
  - (b) **No commercial redistribution.** You may not "sell, license for a fee, or otherwise supply for commercial gain Hypit or any derivative work of it, whether standalone or as a component of … any product". Forks must stay under the same license and be non-commercial.
  - (c) The Hypit name, logo and copyright must not be removed from the CLI, reports and manifests.
  - (2a) The producer may change the license terms.
  - (3) Outputs belong to the user.
- **Consequence:** this is **not OSI open source**. Copying or porting code into our Rust system would make it a derivative work, which blocks commercial distribution and hosted offerings. **Treat as read-only design reference; reimplement clean-room from the descriptions above.** Package manifests say `"SEE LICENSE IN LICENSE"` (`repos/hypit/package.json:10`).
- **Fonts:** none bundled. Fontsource packages are fetched from npm (families are typically OFL-1.1; licenses in `node_modules` are UNVERIFIED because it is not installed).
- **Icons:** Tabler Icons (MIT) via Iconify (`repos/hypit/packages/interview-emoji-reveal/README.md:40-43`).
- **Example assets:** real footballer photos and `.wav`/`.m4a` files have **no license notes** (UNVERIFIED provenance). Do not reuse.
- **Third-party runtime:** WhisperX (BSD-2, UNVERIFIED here), Puppeteer (Apache-2.0), Chrome Headless Shell, FFmpeg (as an external subprocess).

## 12. Steal list (ranked, clean-room reimplementation only)
1. **Agent perception CLI: transcript-labelled frame grids.** Reimplement in Rust as `clip media tiles <file> --transcript t.json --around "<phrase>" [--occurrence n] [--padding s] --every <s>|--every-frame --columns --rows --cell` and `frames`, `cut --label-time`, `boundaries --json`. Expose them as MCP tools that return image paths plus JSON. This is the best pattern here for letting any vision LLM "watch" footage and choose cut points (`repos/hypit/packages/media-local/src/files.ts:808-819`). *Reimplement* with FFmpeg plus the `image` crate.
2. **Script↔ASR alignment DP**, so captions use the authored or corrected text while keeping ASR timing. Port the algorithm, not the code: M:N groups ≤4, cost `edit/width*rel + 0.055*(s+e-2) + 0.06*unmatched`, omission 0.5, insertion 0.35, char-level backtrace, neighbour-weighted fill, frame quantization (`repos/hypit/packages/narrative-speech-alignment/src/align.ts:8-423`). This lets an agent fix transcript typos without breaking timing. *Reimplement in Rust core.*
3. **Declarative caption style vocabulary** (karaoke current/trail × step/wipe, active-box isolated/joined, active-response pop/scale, lead/tail frames, handoff cut, cue-enter/exit, stroke/shadow/glow, max-lines and max-words-per-line, per-Role styles) as a JSON/TOML template schema that agents can edit. Our renderer can compile it to **ASS** (libass via FFmpeg) for speed, and to a WGPU/skia path for effects ASS cannot do (`repos/hypit/examples/ranking-football/recipes.svs:78-128`; `repos/hypit/packages/caption-fine/src/style.ts:202-283`). *Reimplement schema.*
4. **Word-anchored time model.** Events reference `moment`/`selection`/`token` ids resolved through the alignment to frames, with exact rational frame arithmetic and offsets like `claim+2s` and `end-12f`. When a clip is re-cut or re-voiced, every caption, SFX and overlay moves with its word (`repos/hypit/packages/narrative-temporal/src/projection.ts:14-123`; `repos/hypit/packages/temporal/src/rational.ts:74-80`; `repos/hypit/packages/timeline-author/README.md:61-73`). *Reimplement.*
5. **Agent-safe CLI/JSON contract.**
   - Versioned output tags (`x.cli-plan@1`) and a structured error object `{ok:false,error:{code,message,help}}`.
   - stderr progress / stdout result.
   - `check → plan (dry-run with diagnostics and cost) → build → status/logs → inspect/get`.
   - Printed next-step hints; receipts and idempotency keys for paid calls.

   Mirror the same commands 1:1 as MCP tools (`repos/hypit/packages/cli/src/output.ts:979-1038`; `repos/hypit/packages/cli/src/main.ts:326-578`).
6. **Async endpoint protocol** for STT/TTS/LLM/generation providers: `start/poll/collect/cancel`, provider-chosen `wakeAt`, checkpointed receipts, `supports()` gating, capacity and rate claims. As a Rust trait: `trait Endpoint { fn supports(&self,&Req)->Support; async fn start(..)->Outcome; async fn poll(handle)->Outcome; ... }` (`repos/hypit/packages/endpoint/src/index.ts:103-497`). Pair it with data-only "wire mappings" so a new OpenAI-compatible vendor is config, not code (`repos/hypit/packages/generation/src/mapping.ts:22-78`).
7. **Deterministic frame-exact rendering principles**, even if we do not use Chrome:
   - pose = f(absolute frame) only;
   - pre-decode source frames instead of seeking `<video>`;
   - ordered frame sink into a single encoder stdin;
   - ffprobe post-check of codec, size, fps and frame count;
   - an adaptive worker controller (`repos/hypit/packages/provider-html-local/src/{ordered-frame-sink,concurrency,output}.ts`).

   An optional **HTML-overlay sidecar** (headless Chrome rendering only a transparent overlay track) is worth considering for rich motion-graphics templates. Composite in FFmpeg; Hypit can't do alpha output, but we could.
8. **Skill / project-memory conventions**: ANALYSIS.md, TIMELINE.md, BRIEF.md, TREATMENT.md, PROGRESS.md; rules for budget consent and "done means watched"; a routing table of "when the question is X, read Y" (`repos/hypit/skills/hypit/SKILL.md:218-323`; `repos/hypit/skills/hypit/references/creation/project-files.md:89-114`). *Adopt the concept* in our agent skill, written more procedurally and briefly.
9. **Speech-duration estimator** (syllables/rate table) for planning clip length before TTS (`repos/hypit/packages/speech-estimate/src/program.ts:17-80`). *Reimplement* as a small Rust module.
10. **Axis-based prompt kits** (fixed contract blocks plus enumerated axis choices) for any generative B-roll or image prompts (`repos/hypit/packages/seedance-kits/kits/broll-v1.svs`). *Reimplement format.*
11. **Sample-accurate audio graph conventions**: a 48 kHz canonical clock, `adelay=..S:all=1`, `apad whole_len` + `atrim end_sample` to pin exact lengths, chained `atempo`, and an `aeval` gain envelope (`repos/hypit/packages/media-local/src/execute.ts:477-518,1236-1242`). Add `loudnorm`, `sidechaincompress` ducking and a limiter, which they lack.
12. **Studio ideas**: timeline edits written back to source and re-validated by recompiling; per-component GUI descriptors as data (lanes, inspector fields, handles); timestamped comments file for agent feedback (`repos/hypit/packages/studio/README.md:100-310`; `repos/hypit/packages/studio-companion/src/index.ts:174-302`). *Concept only*, for the Tauri GUI.

## 13. Gaps (what hypit doesn't do that we need)
- Long-form → short-clip highlight discovery: LLM transcript scoring, hook detection, virality heuristics, duration constraints.
- Face/speaker tracking, auto 9:16 reframing, multi-speaker layout switching, diarization.
- Silence/filler removal, denoise, loudness normalization (EBU R128), auto-ducking, beat detection.
- SRT/VTT/ASS export; emoji/keyword auto-highlighting in captions.
- An LLM provider abstraction (OpenAI-compatible chat), local TTS with timestamps, local STT beyond the WhisperX sidecar.
- An MCP server; granular exit codes.
- HW encoding (NVENC/QSV/VideoToolbox), alpha/ProRes output, content-hash caching of intermediate artifacts.
- Automated media QC (black/freeze/silence detect, loudness, duration-vs-target, caption-safe-zone checks). Hypit's QC is static graph checks plus the agent watching the output (`repos/hypit/skills/hypit/references/production/review.md:50-53`).

### Scores for the comparison matrix
A 1 · B 4 · C 4 · D 4 (speed 2) · E 2 · F 3 · G 4 · H 3.5 · I 4 (media) / 1 (LLM)
