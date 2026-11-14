# License and dependency inventory: `repos/`

Scope: 7 repos under `C:\Users\krishna\Desktop\rustler\repos\`. Read-only inventory; nothing inside `repos/` was modified. `.git` and `node_modules` excluded. Items marked **check** are unverified guesses or need a human to confirm against current upstream terms. The full environment-variable name lists are in the appendix.

## 1. Repo license and copyright

| Repo | Root LICENSE | Copyright holder line | Notes |
|---|---|---|---|
| hypit | Modified Apache-2.0 with extra conditions (not OSI-approved) | (c) 2026 Hypit.AI | Restricts multi-tenant hosting and commercial redistribution without written authorization; requires keeping LOGO/copyright in CLI and run reports; output content owned by the user |
| openshorts | MIT | (c) 2024 OpenShorts | NOTICE says everything under `cloud/` is under the OpenShorts Commercial License instead |
| supoclip | GNU AGPL-3.0 | none in LICENSE (standard FSF text) | **check** holder; no notice found |
| autoclip | MIT | (c) 2026 Jad Ghazi | |
| ViralCutter | GNU GPL-3.0 | none in LICENSE (standard FSF text) | **check** holder in README |
| bridgeclip | MIT | (c) 2026 BridgeMind LLC | `engine/LICENSE` is MIT, (c) 2026 BridgeMind |
| opensource-clipping | MIT | (c) 2026 Muhammad Naufal Rizqullah | |

## 2. Other license, notice and copyright files (excluding .git, node_modules)

| Repo | Path | License type |
|---|---|---|
| hypit | (none beyond root LICENSE) | - |
| openshorts | `cloud/LICENSE` | OpenShorts Commercial License (c) 2024-present |
| openshorts | `NOTICE` | Notice file: MIT core, cloud/ carve-out |
| openshorts | `fonts/Montserrat-OFL.txt` | SIL OFL 1.1 (Montserrat Project Authors) |
| supoclip | `backend/src/media/models/LICENSE-yunet` | MIT, (c) 2020 Shiqi Yu (YuNet face model) |
| supoclip | `frontend/public/email-assets/GEIST-OFL.txt` | SIL OFL 1.1 (Geist Project Authors) |
| autoclip | (none) | - |
| ViralCutter | (none) | - |
| bridgeclip | `engine/LICENSE` | MIT, (c) 2026 BridgeMind |
| bridgeclip | `engine/THIRD_PARTY_NOTICES.md`, `THIRD_PARTY_NOTICES.md` | Third-party notice files (FFmpeg LGPL build, libass, Python PSF, yt-dlp Unlicense, Geist OFL, Electron and npm packages) |
| bridgeclip | `engine/assets/fonts/Anton-OFL.txt`, `ArchivoBlack-OFL.txt`, `InstrumentSerif-OFL.txt`, `Montserrat-OFL.txt`, `PlusJakartaSans-OFL.txt`, `Poppins-OFL.txt` | SIL OFL 1.1 |
| bridgeclip | `src/renderer/assets/fonts/Geist-OFL.txt` | SIL OFL 1.1 |
| opensource-clipping | (none) | - |

## 3. Dependency manifests and notable dependencies

Categories: STT = speech-to-text, LLM = language-model SDK, TTS = speech synthesis, Vision = CV/detection, Video = media processing, Audio = audio processing, Web = web/UI framework.

### hypit

| Manifest paths | Count / note |
|---|---|
| `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml` | Root pnpm monorepo |
| `packages/*/package.json` | About 100 workspace packages |
| `examples/*/package.json`, `examples/*/package-lock.json` | 15+ example projects |
| `packages/provider-whisperx-local/runtime/pyproject.toml`, `uv.lock` | Python runtime |
| `packages/provider-image-opencv-local/runtime/pyproject.toml`, `uv.lock` | Python runtime |
| `packages/yt-dlp/runtime/pyproject.toml`, `uv.lock` | Python runtime |

| Category | Dependency (version) | Where |
|---|---|---|
| STT | `whisperx==3.8.6` | provider-whisperx-local, @hypit/whisperx |
| Vision | `opencv-python-headless>=4.10,<5` | provider-image-opencv-local |
| Video | `yt-dlp[default,curl-cffi]==2026.8.19` | yt-dlp runtime |
| Image/Web | `puppeteer-core` 25.10.0, `sharp` 0.35.4 | browser-capture, provider-html-local, studio, media-local, caption-fine |
| Runtime | `koffi` 3.2.1 (FFI), `tsx`, `typescript` 5.9.3 | root |
| Fonts | `@fontsource-variable/inter`, `montserrat`, `league-spartan` 5.3.0 | fontsource packages |
| TTS / voice (provider packages) | `elevenlabs-speech`, `fishaudio-speech`, `mimo-speech` | No vendor SDK found in the package.json files searched (**check**) |
| Hosted media-AI providers | `gpt-image`, `nano-banana`, `seedance`, `seedream`, `wan`, `pixverse`, `grok-imagine`, `pollo`, `sume`, `tokendance`, `hiapi`, `beatapi`, `monid`, `hypihub`, `volcengine-matting`, `minimax-h3` | Provider packages; SDK deps not individually checked (**check**) |

| Flag | Detail |
|---|---|
| Restrictive repo license | Modified Apache-2.0 with commercial and multi-tenant limits, not OSI |
| whisperx (BSD-2-Clause code) | Diarization in WhisperX uses pyannote models, which are gated on Hugging Face (**check** whether the runtime pulls them) |
| Hosted providers | Each provider's API terms apply (**check**) |

Font files: none in repo (uses npm fontsource packages).
Audio: 33 `.wav` files under `examples/*/assets/` (46 KB to 435 KB each). Examples are voice and SFX samples; **check** source and license of the sample content.

### openshorts

| Manifest paths | Note |
|---|---|
| `requirements.txt`, `requirements-billing.txt` | Python |
| `cli/pyproject.toml` | Python CLI |
| `Dockerfile`, `docker-compose.yml`, `docker-compose.cloud.yml`, `docker-compose.worker.yml`, `docker-compose.e2e.local.yml` | Containers |
| `osrender/Cargo.toml` | Rust (`ffmpeg-sys-next` 7.1, `libloading`, `png`, `serde`) |
| `dashboard/package.json`, `dashboard/package-lock.json`, `dashboard/Dockerfile` | React/Vite |
| `remotion/package.json`, `remotion/package-lock.json` | Remotion |
| `render-service/package.json`, `render-service/package-lock.json`, `render-service/Dockerfile` | Express + Remotion renderer |

| Category | Dependency (version) |
|---|---|
| STT | `faster-whisper==1.2.1`; `whisper` referenced in Dockerfile |
| LLM | `google-genai==1.75.0` |
| TTS | none found |
| Vision | `ultralytics==8.4.46`, `mediapipe==0.10.14`, `torch==2.11.0`, `torchvision==0.26.0`, `transnetv2-pytorch==1.0.5`, `scenedetect==0.7`, `onnx` (Dockerfile) |
| Video | `yt-dlp` (unpinned), Remotion `^4.0.x` (`remotion`, `@remotion/*`), FFmpeg (Dockerfile, osrender), Chromium/Puppeteer (render-service Dockerfile) |
| Audio | none found |
| Web | `fastapi==0.136.1`, `uvicorn==0.46.0`, `React ^18.2`, `Vite`, `Tailwind`, `express ^4.21` |
| Other | `stripe==11.3.0`, `sqlalchemy`, `alembic`, `asyncpg`, `authlib`, `PyJWT`, `boto3==1.43.4`, `Pillow==12.2.0`, `py3langid==0.3.0`, `beautifulsoup4`, `zod`, `lucide-react`, `mediabunny` |

| Flag | Detail |
|---|---|
| ultralytics 8.4.46 | **AGPL-3.0** (certain). Network-use source-offer obligation applies if modified and offered as a service. Also the root LICENSE is MIT, so this dependency conflicts with the stated MIT license for distributions that include it. |
| Remotion | Source-available; a company license is required above the free-use thresholds (**check** current terms) |
| transnetv2-pytorch | License not verified (**check**) |
| mediapipe | Apache-2.0 code; bundled model files (**check**) |
| yt-dlp | Unlicense code; YouTube/platform terms apply (**check**) |
| cloud/ directory | Commercial license, not MIT |

Fonts (20 files): `dashboard/public/fonts/` Anton-Regular (Anton), Montserrat-ExtraBold (Montserrat), geist-400/500/600-latin and -latin-ext `.woff2` (Geist, 6 files), instrument-serif-400-latin(-ext) `.woff2` (Instrument Serif, 2 files), jetbrains-mono-400/500-latin(-ext) `.woff2` (JetBrains Mono, 4 files), noto-serif-700-latin(-ext) `.woff2` (Noto Serif, 2 files), `fonts/` Anton-Regular, Montserrat-ExtraBold, NotoSerif-Bold; `remotion/public/fonts/NotoSerif-Bold.ttf`. Licenses for these families are OFL-1.1 (known families); only the Montserrat OFL text is shipped (`fonts/Montserrat-OFL.txt`).
Audio: none found.

### supoclip

| Manifest paths | Note |
|---|---|
| `backend/pyproject.toml`, `backend/uv.lock`, `backend/Dockerfile` | Python (FastAPI backend) |
| `mcp/pyproject.toml`, `mcp/uv.lock`, `mcp/Dockerfile` | Python MCP server |
| `frontend/package.json`, `frontend/pnpm-lock.yaml`, `frontend/Dockerfile` | Next.js |
| `docker-compose.yml` | Containers |

| Category | Dependency (version) |
|---|---|
| STT | `openai-whisper>=20250625` (MIT); `assemblyai` (API, key `ASSEMBLY_AI_API_KEY`) |
| LLM | `pydantic-ai>=1.107.7,<2` (multi-provider; env names include `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`); `openai` |
| TTS | none found |
| Vision | `opencv`, `torch` |
| Video | `yt-dlp[default]>=2026.10.21`; `apify-client>=1.8.1` (Apify actors for YouTube/Kick/Twitch downloads) |
| Audio | none found |
| Web | `fastapi>=0.121.0`, `Next 15.5.28`, React, `Tailwind`, `Radix UI`, `better-auth` 1.7.17, `Prisma` 6, `Stripe`, `playwright` (frontend), `sharp`, `Vite`, `mcp>=1.30.0` |
| Other | `temporalio>=1.35.0`, `alembic`, `sqlalchemy`, `asyncpg`, `redis`, `boto3`, `posthog-js`, `next-intl`, `motion` |

| Flag | Detail |
|---|---|
| Project license | **AGPL-3.0** (certain). Network-use copyleft: source must be offered to users of a modified hosted version. |
| yt-dlp, Apify actors | Platform terms of YouTube, Kick, Twitch apply (**check**) |
| assemblyai | Commercial API terms (**check**) |
| sharp | Apache-2.0 wrapper; libvips is LGPL (prebuilt binary, **check**) |
| YuNet model | MIT (`LICENSE-yunet`) |

Fonts (24 files): `backend/fonts/` Anton-Regular (Anton), ArchivoBlack-Regular (Archivo Black), Bangers-Regular (Bangers), BarlowCondensed-Bold (Barlow Condensed), BebasNeue-Regular (Bebas Neue), DMSans (DM Sans), Inter (Inter), LeagueSpartan (League Spartan), Montserrat-Variable-wght (Montserrat), NunitoSans (Nunito Sans), OpenSans (Open Sans), Oswald-Variable-wght (Oswald), Poppins-ExtraBold (Poppins), Raleway-Variable-wght (Raleway), Roboto (Roboto), Rubik (Rubik), Sora (Sora), Urbanist (Urbanist), WorkSans (Work Sans), TikTokSans-Bold and TikTokSans-Regular (TikTok Sans), THEBOLDFONT (family unknown from filename, **check**). `frontend/public/email-assets/` geist-latin and geist-latin-ext `.woff2` (Geist).
Licenses: most families are OFL-1.1; Open Sans and Roboto are Apache-2.0 (known); TikTok Sans and THEBOLDFONT **check**.
Audio: none found.

### autoclip

| Manifest paths | Note |
|---|---|
| `pyproject.toml` | Python |
| `docker/Dockerfile` | Container |
| `frontend/package.json`, `frontend/package-lock.json` | React/Vite |

| Category | Dependency (version) |
|---|---|
| STT | `faster-whisper>=1.1`; speaker diarization uses pyannote (gated Hugging Face model, HF token via app settings; **check**) |
| LLM | `anthropic>=0.40`, `openai>=1.55`, `google-genai>=0.3` |
| TTS | none found |
| Vision | `mediapipe>=0.10.14`, `scenedetect>=0.6.4`, `numpy`, `torch` (via pyannote, large) |
| Video | `yt-dlp>=2026.1.1`; ffmpeg from Debian in Dockerfile |
| Audio | `pysubs2` (subtitles); none else |
| Web | `fastapi>=0.115`, `uvicorn`, `sse-starlette`, `pydantic`, `keyring`, `typer`; frontend `react ^18.3.1`, `react-router-dom ^6.28`, `tailwindcss ^4`, `vite ^6` |

| Flag | Detail |
|---|---|
| pyannote.audio | Code MIT; model weights gated (user must accept license on Hugging Face) (**check**) |
| mediapipe | Apache-2.0 code (**check** bundled model terms) |
| yt-dlp | Unlicense code; platform terms apply (**check**) |
| No AGPL/GPL dependency found in manifests | |

Fonts (5 files): `backend/autoclip/assets/fonts/` Anton-Regular (Anton), Inter-Variable (Inter); `frontend/src/fonts/` Archivo-Variable (Archivo), InstrumentSerif-Italic and InstrumentSerif-Regular (Instrument Serif). Licenses: OFL-1.1 (known families); no OFL text shipped (**check**).
Audio: none found.

### ViralCutter

| Manifest paths | Note |
|---|---|
| `requirements.txt` | Only manifest; no pinned versions (`numpy<2.1.0` is the only constraint) |
| `install_dependencies.bat`, `install_dependencies_advanced_LocalLLM.bat`, `ViralCutter.ipynb` | Install scripts (unpinned pip installs; not parsed further) |

| Category | Dependency |
|---|---|
| STT | `whisperx` (BSD-2-Clause code) |
| LLM | `google-genai`, `google-generativeai` (deprecated SDK), `g4f[all]` (**check**) |
| TTS | none found |
| Vision | `mediapipe`, `opencv-python`, `insightface` |
| Video | `yt-dlp`, `ffmpeg-python` |
| Audio | none found |
| Web | `gradio`, `fastapi`, `uvicorn` |
| Other | `deep-translator`, `psutil`, `tqdm` |

| Flag | Detail |
|---|---|
| Project license | **GPL-3.0** (certain). Derivatives distributed must be GPL-3.0. |
| insightface | Code MIT; pretrained model packs are non-commercial research use (**check** current model terms) |
| g4f (gpt4free) | Reverse-engineered access to third-party AI web services; license and ToS risk (**check**) |
| yt-dlp | Platform terms (**check**) |

Fonts: none. Audio: none.

### bridgeclip

| Manifest paths | Note |
|---|---|
| `package.json`, `package-lock.json`, `electron-builder.yml`, `electron.vite.config.ts` | Electron app |
| `engine/requirements.txt` | Python engine |
| `bridge/requirements.txt` | Empty file |

| Category | Dependency (version) |
|---|---|
| STT / LLM (via providers) | Hosted via OpenRouter (MAI Transcribe 2 transcription, model picking); other providers per `.env.example` (**check** full list) |
| TTS | none found |
| Vision | `opencv-python-headless>=4.10,<5`, `Pillow>=10.0` |
| Video | `yt-dlp[default]>=2026.8.19`; bundled FFmpeg 8.1.3 (LGPL build, see notices) |
| Audio | none found |
| Web/UI | `electron` (+ `electron-updater` 6.8.9, `@electron-toolkit/*`), React, `Tailwind`, `Vite`, `playwright-core`, `undici` |
| Python web | `fastapi>=0.109`, `uvicorn[standard]`, `pydantic`, `boto3>=1.34`, `httpx`, `PySocks` |

| Flag | Detail |
|---|---|
| FFmpeg | LGPL build shipped (dynamic). LGPL obligations apply to the bundled binary (**check** linking/notice text) |
| OpenH264 | Cisco binary license for CPU H.264 (**check**) |
| yt-dlp | Platform terms (**check**) |
| OpusClip wordmark | Third-party trademark in `src/renderer/components/brand/OpusClipLogo.tsx` (**check**) |
| docs/CLIPPING_REDESIGN_PLAN.md | Mentions InsightFace SCRFD/RetinaFace weights as non-commercial, as a rejected or considered option (a design doc, not a shipped dependency; **check** whether used) |

Fonts (10 files): `engine/assets/fonts/` Anton-Regular (Anton), ArchivoBlack-Regular (Archivo Black), InstrumentSerif-Italic (Instrument Serif), Montserrat-Black and Montserrat-ExtraBold (Montserrat), PlusJakartaSans (Plus Jakarta Sans), Poppins-Black and Poppins-ExtraBold (Poppins); `src/renderer/assets/fonts/` Geist-Variable.woff2 and GeistMono-Variable.woff2 (Geist, Geist Mono). OFL text files shipped for the families above.
Audio: none found.

### opensource-clipping

| Manifest paths | Note |
|---|---|
| `pyproject.toml`, `requirements.txt`, `requirements-kaggle.txt`, `requirements-test.txt`, `constraints-kaggle.txt` | Python |
| `Dockerfile`, `docker-compose.yml` | Containers |
| `web/dashboard/package.json`, `web/dashboard/Dockerfile` | React/Vite |

| Category | Dependency (version) |
|---|---|
| STT | `faster-whisper` |
| LLM | `google-genai`; `openai` (kaggle reqs); NVIDIA API key (hosted model) |
| TTS | `edge-tts` |
| Vision | `ultralytics`, `mediapipe`, `opencv-python`, `pyannote.audio`, `torch`, `torchaudio`, `speechbrain` (constraints), `dlib` (kaggle reqs) |
| Video | `yt-dlp[default]`; FFmpeg (Dockerfile); `pyngrok` (kaggle) |
| Audio | none beyond the bundled BGM tracks |
| Web | `fastapi`, `uvicorn`, `pydantic`, `google-api-python-client`, `google-auth*`; `Meta Graph` upload (`META_*`); frontend `react`, `vite` |
| Other | `gdown`, `requests`, `Pillow`, `numpy<2.0.0` |

| Flag | Detail |
|---|---|
| ultralytics | **AGPL-3.0** (certain). Same obligation as openshorts. Conflicts with the MIT root license for distributions that include it. |
| pyannote.audio | Code MIT; models gated on Hugging Face (`HF_TOKEN`) (**check**) |
| edge-tts | LGPL-3.0 (**check**); uses Microsoft Edge online TTS service, ToS applies (**check**) |
| dlib | Boost Software License (permissive; **check** version) |
| Meta Graph, YouTube Data API | Platform API terms (**check**) |
| BGM tracks | Filenames look like stock-site IDs (for example Pixabay); **check** license for each track |

Fonts: none.
Audio: 15 `.mp3` music tracks under `assets/bgm/{chill,epic,sad,suspense,upbeat}/`, about 1.0 MB to 9.3 MB each (**check** license for each).

## 4. Summary of license-relevant dependencies

| Item | Repo | Risk |
|---|---|---|
| ultralytics 8.4.46 (AGPL-3.0) | openshorts, opensource-clipping | Copyleft; conflicts with MIT repo license |
| AGPL-3.0 project license | supoclip | Network copyleft |
| GPL-3.0 project license | ViralCutter | Copyleft for derivatives |
| Restrictive modified Apache-2.0 | hypit | Non-OSI commercial and multi-tenant limits |
| Commercial license under cloud/ | openshorts | Mixed license in one repo |
| pyannote gated models | autoclip, opensource-clipping, possibly hypit (whisperx) | Requires HF account and model acceptance (**check**) |
| insightface model weights | ViralCutter | Non-commercial model terms (**check**) |
| Remotion | openshorts | Company license may be required (**check**) |
| g4f | ViralCutter | Unofficial access to third-party services (**check**) |
| FFmpeg LGPL build | bridgeclip | LGPL notice and dynamic linking obligations |
| Music / sample audio | opensource-clipping (15 mp3), hypit (33 wav) | Source and license of each file unverified (**check**) |

## Appendix: environment variable names

Names only, collected from code (`os.environ`, `os.getenv`, `process.env`, `import.meta.env`, `env::var`) and from `.env.example` / `.env.sample`. Values are not recorded. Source lists are at `C:\Users\krishna\AppData\Local\Temp\claude\C--Users-krishna-Desktop-rustler\9dc66510-e7db-4fd4-9296-20eb5dc3a507\scratchpad\env_code_*.txt` and `env_example_*.txt`.

Key and secret names (credential-type) per repo:

| Repo | Credential-type names |
|---|---|
| hypit | `DEEPSEEK_API_KEY`, `GH_TOKEN` (CI) |
| openshorts | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `GEMINI_API_KEY`, `MANAGED_GEMINI_API_KEY`, `LLM_API_KEY`, `JWT_SECRET`, `GOOGLE_CLIENT_SECRET`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `MANAGED_UPLOAD_POST_API_KEY`, `AGENTLEDGER_API_KEY`, `OPENPANEL_CLIENT_SECRET`, `DATABASE_URL` |
| supoclip | `ASSEMBLY_AI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, `APIFY_API_TOKEN`, `APIFY_STORAGE_*_SECRET_ACCESS_KEY`, `APP_SETTINGS_ENCRYPTION_KEY`, `BACKEND_AUTH_SECRET`, `BETTER_AUTH_SECRET`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `INSTAGRAM_APP_SECRET`, `TWITCH_CLIENT_SECRET`, `TELEGRAM_BOT_TOKEN`, `AXIOM_TOKEN`, `DATABASE_URL`, `MCP_OAUTH_SECRET` |
| autoclip | none in env; provider keys and HF token stored in app settings (`config.py`) |
| ViralCutter | `api_config.json` contains an `api_key` field; value not recorded (**check** if populated) |
| bridgeclip | none required; provider keys entered in-app and stored encrypted via OS keychain (`.env.example`) |
| opensource-clipping | `GOOGLE_API_KEY`, `HF_TOKEN`, `META_PAGE_ACCESS_TOKEN`, `NVIDIA_API_KEY`, `PEXELS_API_KEY`, `NGROK_AUTHTOKEN` |

Full name lists per repo follow.

### Full names: code-referenced (per repo)

**hypit** (39 names)

ANALYSIS_CONTEXT ANALYSIS_MAX_TOKENS ANALYSIS_MODEL BROWSER CANDIDATE_RUN CHROME_PATH DEEPSEEK_API_KEY GH_TOKEN GITHUB_EVENT_NAME GITHUB_EVENT_PATH GITHUB_OUTPUT GITHUB_REPOSITORY 
GITHUB_SHA GITHUB_STEP_SUMMARY HYPIT_ALPHA_PROOF_DIR HYPIT_BROWSER_TESTS HYPIT_CAPTURE_TEST_BROWSER HYPIT_MEDIA_ENV_PROBE HYPIT_OPENCV_PYTHON HYPIT_OPENCV_TESTS 
HYPIT_PRIVATE_ENV_PROBE HYPIT_RUNTIME_SCALE_TESTS HYPIT_TEST_CHROME_PATH INIT_CWD NLTK_DATA NODE_OPTIONS NO_COLOR PATH PATHEXT PUBLISH PUPPETEER_CACHE_DIR RELEASE_PACKAGES 
RELEASE_PRERELEASE RELEASE_TAG SELECTED_TOOL_ENV SYSTEMROOT TEMP TERM TMPDIR

**openshorts** (206 names)

ACTIVE_JOB_MAX_SECONDS ACTOR_URL_TTL_SECONDS ADMIN_EMAIL AGENTLEDGER_API_KEY AGENTLEDGER_API_URL AGENTLEDGER_TREASURY_ID ALLOWED_ORIGINS ASR_GPU_CONCURRENCY ASR_HOST_SLOTS 
ASR_LOCK_DIR AUDIO_NORMALIZE AUTOPILOT_DISABLED AUTOPILOT_TWITCH AUTO_CAPTIONS AUTO_HOOK AUTO_HOOK_SECONDS AUTO_HOOK_STYLE AUTO_LAYOUT AUTO_RETRY_DELAY_SECONDS AUTO_RETRY_LIMIT 
AWS_ACCESS_KEY_ID AWS_REGION AWS_S3_BUCKET AWS_S3_PUBLIC_BUCKET AWS_SECRET_ACCESS_KEY BGUTIL_BASE_URL BGUTIL_SCRIPT_PATH BILLING_ENABLED CLIP_RETRY_PAUSE_SECONDS CLIP_WORKERS 
CUT_NVENC_CQ CUT_NVENC_PRESET DATABASE_URL DEBUG_LOGS DETECT_STRIDE DIRECT_FIRST DISABLE_YOUTUBE_URL DOWNLOAD_SKIP_STATICS DRAIN_TIMEOUT_SECONDS EARLY_AUDIO EMAIL_FROM 
FFMPEG_ENCODER FFMPEG_GPU_CUT FFMPEG_THREADS FREE_SLOW_LANE_COUNTRIES FREE_TOOLS_GEMINI_MODEL FREE_TOOLS_TT_CONCURRENCY FREE_TOOLS_YT_CONCURRENCY FREE_TOOLS_YT_TIMEOUT 
FRONTEND_URL GEMINI_API_KEY GEMINI_IMAGE_MODEL GEMINI_MODEL GEMINI_MODEL_EDITOR GEMINI_MODEL_SAAS GEMINI_MODEL_THUMBNAIL GEMINI_THINKING_SCORE GENERAL_CONTENT_HEIGHT_RATIO 
GEOIP_COUNTRY_DB_PATH GEOIP_COUNTRY_DB_SHA256_URL GEOIP_COUNTRY_DB_URL GEOIP_TEST_DB GOOGLE_CLIENT_ID GOOGLE_CLIENT_SECRET GPU_DECODE GPU_DECODE_MIN_FREE_MB GPU_DECODE_RENDER 
GPU_MIN_FREE_MB GPU_RENDER HARD_EXIT_SECONDS HOOK_GROUNDING HOOK_GROUNDING_FRAMES HOOK_GROUNDING_MIN_SHARE HOOK_GROUNDING_WIDTH HOOK_LAZY INSET_PADDING INSTANCE_ID JOB_NICE 
JOB_NVENC_SLOTS JOB_RETENTION_SECONDS JUMP_CONFIRM_FRAMES JWT_SECRET LAYOUT_SAMPLE_FRAMES LAYOUT_SAMPLE_WIDTH LIFECYCLE_EMAILS_DISABLED LIVE_RESERVATION_MAX_HOURS LLM_API_KEY 
LLM_BASE_URL LLM_MODEL LLM_PROVIDER LLM_SCORE_BATCH LLM_TIMEOUT MANAGED_GEMINI_API_KEY MANAGED_UPLOAD_POST_API_KEY MAX_CONCURRENT_JOBS MAX_SOURCE_MINUTES MEDIA_LEGACY_CUTOFF 
MEDIA_LEGACY_UNTIL MEDIA_TOKEN_TTL_SECONDS MEDIA_URL_TTL_SECONDS MIN_SOURCE_SECONDS MIN_SPEECH_WORDS MIN_SPEECH_WORDS_PER_MIN NVENC_LOCK_DIR NVENC_SLOTS NVENC_SLOT_WAIT 
NVENC_WAIT_LOG OAUTH_REDIRECT_URI OPENAI_APPS_CHALLENGE OPENPANEL_API_URL OPENPANEL_CLIENT_ID OPENPANEL_CLIENT_SECRET OPENSHORTS_API_KEY OPENSHORTS_API_URL OPENSHORTS_LOGO_URL 
OSRENDER_BIN OSRENDER_TEST_VIDEO OUTPUT_DIR OUTPUT_MAX_GB PAID_PROXY_DAILY_MB PANEL_LAYOUT PANEL_TIGHTNESS PARAKEET_DEVICE PARAKEET_VAD_BATCH PATH PORT PROXY_DRAIN_SECONDS 
PROXY_URL PUBLIC_API_URL PUNCH_IN PUNCH_IN_ZOOM QUALITY_GATE_MIN_HEIGHT QUEUE_COUNTRY_HEADER QUEUE_EXPIRED_EMAIL_GAP R2_ACCESS_KEY_ID R2_BUCKET R2_ENDPOINT R2_PUBLIC_BASE 
R2_SECRET_ACCESS_KEY REFRAME_ENGINE REMOTE_WORKER_TOKEN REMOTION_BUNDLE_PATH RENDER_MAX_FRAMES RENDER_MAX_SIDE RENDER_SERVICE_URL RETENTION_COUPON_ID REVIEW_LOGIN_EMAIL 
REVIEW_LOGIN_PASSWORD SCENE_CUT_RESET SCENE_ENGINE SCENE_MIN_SEC SCREENCAST_LAYOUT SCREENCAST_MAX_SHOTS SLOW_LANE_BRAKE_CLIPS SMTP_HOST SMTP_PASSWORD SMTP_PORT SMTP_USER 
SOURCE_CAP_MINUTES SOURCE_RETENTION_SECONDS SOURCE_START_SECONDS SOURCE_URL_TTL_SECONDS SPARSE_TRAJECTORY SPEAKER_CUT SPEAKER_SIGNAL SPLIT_LAYOUT SPLIT_MIN_SHARE SPLIT_TIGHTNESS 
STATIC_PROXY_URLS STRIPE_SECRET_KEY STRIPE_WEBHOOK_SECRET SUBTITLE_BAND_CHECK TELEGRAM_BOT_TOKEN TELEGRAM_CHAT_ID TIKTOK_POST_MODE TRANSCRIBE_BACKEND TRANSNETV2_DEVICE 
TRANSNETV2_THRESHOLD TWITCH_CLIENT_ID TWITCH_CLIENT_SECRET UPLOADS_MAX_GB UPLOAD_POST_API_KEY UPLOAD_TTL_SECONDS VISION_MAX_SEGMENTS VISION_SEGMENT_SECONDS VITE_API_URL 
VITE_PROXY_TARGET VITE_RENDER_TARGET WATERMARK WHISPER_COMPUTE WHISPER_DEVICE WHISPER_MODEL WIDE_CONTENT_HEIGHT_RATIO WINBACK_PROMO_CODE WINBACK_PROMO_LABEL WORKER_ALLOW_HTTP 
WORKER_API_URL WORKER_MAX_SOURCE_GB WORKER_NAME WORKER_POLL_SECONDS WORKER_READY_TIMEOUT WORKER_SLOTS WORKER_TOKEN YOLO_MODEL_PATH YOUTUBE_COOKIES YT_ACCOUNT_STATE_DIR

**supoclip** (166 names)

AI_EDIT_MODEL APIFY_DOWNLOAD_MAX_CHARGE_USD APIFY_EXPENSIVE_FALLBACKS APIFY_FALLBACK_MAX_CHARGE_USD APIFY_KICK_HLS_ACTOR APIFY_KICK_MP4_ACTOR APIFY_LONG_VIDEO_SECONDS 
APIFY_RUN_TIMEOUT_SECONDS APIFY_STORAGE_ APIFY_TWITCH_ACTOR APIFY_YOUTUBE_DEFAULT_QUALITY APIFY_YOUTUBE_FALLBACK_ACTOR APIFY_YOUTUBE_FALLBACK_ACTORS APIFY_YOUTUBE_INPUT_ADAPTER 
APIFY_YOUTUBE_PROVIDERS APP_SETTINGS_ENCRYPTION_KEY ASSEMBLY_AI_HTTP_TIMEOUT_SECONDS ASSEMBLY_AI_REQUEST_TIMEOUT_SECONDS ASSEMBLY_AI_SPEECH_MODEL AUTH_SIGNATURE_TTL_SECONDS 
AWS_REGION BACKEND_AUTH_SECRET BACKEND_INTERNAL_URL BETTER_AUTH_SECRET BETTER_AUTH_TRUSTED_ORIGINS BETTER_AUTH_URL CAPTURE_LIBRARY_SCREENSHOTS CAPTURE_RECOVERY_SCREENSHOTS 
CAPTURE_STATUS_SCREENSHOTS CAPTURE_THEME_SCREENSHOTS CLIP_DURATION CYCLES_DEVICE DATABASE_URL DEFAULT_PROCESSING_MODE DISABLE_SIGN_UP EMBED_CRON_WORKER FAST_MODE_MAX_CLIPS 
KICK_DOWNLOAD_PROVIDERS LOCAL_TEST_ARTIFACTS LOCAL_TEST_BACKEND_URL LOCAL_TEST_USER_ID LOCAL_TEST_VIDEO_PATH LOCAL_TEST_YOUTUBE_URL LOG_LEVEL MAX_CLIPS MAX_VIDEO_DURATION 
MAX_VIDEO_UPLOAD_BYTES MCP_OAUTH_SECRET MEDIA_CACHE_TTL_SECONDS MEDIA_FALLBACK_REGION MEDIA_ORPHAN_GRACE_SECONDS MEDIA_PREFERRED_STORE MEDIA_S3_PREFIX MEDIA_S3_REGION 
MEDIA_STORAGE MEDIA_UPLOAD_CONCURRENCY MEDIA_UPLOAD_DAILY_LIMIT MEDIA_URL_TTL_SECONDS NEXT_BUILD_DIR NEXT_PUBLIC_API_URL NEXT_PUBLIC_APP_URL NEXT_PUBLIC_DATAFAST_ALLOW_LOCALHOST 
NEXT_PUBLIC_DATAFAST_DOMAIN NEXT_PUBLIC_DATAFAST_WEBSITE_ID NEXT_PUBLIC_LANDING_ONLY_MODE NEXT_PUBLIC_MAX_VIDEO_UPLOAD_BYTES NEXT_PUBLIC_MCP_URL NEXT_PUBLIC_POSTHOG_HOST 
NEXT_PUBLIC_POSTHOG_KEY NEXT_PUBLIC_PRO_PLAN_MINUTE_LIMIT NEXT_PUBLIC_PRO_PRICE_ANNUAL NEXT_PUBLIC_PRO_PRICE_MONTHLY NEXT_PUBLIC_PRO_YOUTUBE_MAX_MINUTES 
NEXT_PUBLIC_SCALE_PLAN_MINUTE_LIMIT NEXT_PUBLIC_SCALE_PRICE_ANNUAL NEXT_PUBLIC_SCALE_PRICE_MONTHLY NEXT_PUBLIC_SCALE_YOUTUBE_MAX_MINUTES NEXT_PUBLIC_SELF_HOST 
NEXT_PUBLIC_SOCIAL_PUBLISHING NEXT_PUBLIC_TILLY_WEBSITE_ID NEXT_RUNTIME NODE_ENV OUTPUT_DIR PLAYWRIGHT_BACKEND_PORT PLAYWRIGHT_BASE_URL PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH 
PLAYWRIGHT_FRONTEND_PORT POSTHOG_HOST POSTHOG_PERSONAL_API_KEY POSTHOG_PROJECT_ID POSTHOG_PROJECT_TOKEN POSTHOG_RELEASE_VERSION PROCESSING_TASK_TIMEOUT_SECONDS 
PRO_PLAN_MINUTE_LIMIT PRO_YOUTUBE_MAX_VIDEO_DURATION QUEUED_TASK_TIMEOUT_SECONDS RAPIDAPI_YOUTUBE_HOST RAPIDAPI_YOUTUBE_RELAY_HOST RAPIDAPI_YOUTUBE_TIMEOUT_SECONDS REDIS_HOST 
REDIS_PORT RETRY_LIVE_ARTIFACTS RETRY_LIVE_LLM RETRY_LIVE_SOURCE REVENUECAT_PRO_PRODUCT_IDS REVENUECAT_SCALE_PRODUCT_IDS REVENUECAT_WEBHOOK_AUTH_HEADER SCALE_PLAN_MINUTE_LIMIT 
SCALE_YOUTUBE_MAX_VIDEO_DURATION SOCIAL_METRICS_MAX_POST_AGE_DAYS SOCIAL_METRICS_REFRESH_HOURS SOCIAL_PERFORMANCE_MIN_POSTS SOCIAL_PUBLISHING_ACCESS 
SOCIAL_PUBLISHING_ALLOWED_EMAILS SOCIAL_PUBLISH_MAX_ATTEMPTS STRIPE_ACCOUNT_MIGRATION_ENABLED STRIPE_CHECKOUT_ACCOUNT_ID STRIPE_DESTINATION_ACCOUNT_ID 
STRIPE_DESTINATION_PROMOTION_CODE_MAP STRIPE_DESTINATION_PRO_PRICE_ID STRIPE_DESTINATION_REQUIRED_TAX_REGISTRATION_IDS STRIPE_DESTINATION_SCALE_PRICE_ID 
STRIPE_DESTINATION_SECRET_KEY STRIPE_DESTINATION_TOPUP_TAX_CODE STRIPE_DESTINATION_WEBHOOK_SECRET STRIPE_PRO_PRICE_ID STRIPE_SCALE_ANNUAL_PRICE_ID STRIPE_SCALE_PRICE_ID 
STRIPE_SECRET_KEY STRIPE_SOURCE_ACCOUNT_ID STRIPE_WEBHOOK_SECRET SUPOCLIP_API_KEY SUPOCLIP_API_URL SUPOCLIP_APP_URL SUPOCLIP_AUTH_SECRET SUPOCLIP_DOWNLOAD_DIR SUPOCLIP_E2E_API_URL 
SUPOCLIP_E2E_BACKEND_AUTH_SECRET SUPOCLIP_E2E_MCP_URL SUPOCLIP_E2E_USER_ID SUPOCLIP_MCP_HOST SUPOCLIP_MCP_MOUNT_PATH SUPOCLIP_MCP_PORT SUPOCLIP_MCP_PUBLIC_URL 
SUPOCLIP_MCP_REQUIRE_BEARER_AUTH SUPOCLIP_MCP_TRANSPORT SUPOCLIP_SERVICE SUPOCLIP_TIMEOUT SUPOCLIP_USER_ID TEMP_DIR TEST_DATABASE_URL TEST_MIGRATION_DATABASE_URL 
TRANSCRIPTION_LANGUAGE TRANSCRIPTION_PROVIDER TWITCH_DOWNLOAD_PROVIDERS VITEST WHISPER_MODEL WHISPER_MODEL_SIZE WORKFLOWS_ACCESS WORKFLOWS_ALLOWED_EMAILS WORKFLOWS_PUBLIC 
YOUTUBE_DOWNLOAD_PROVIDER YOUTUBE_METADATA_PROVIDER YOUTUBE_POT_PROVIDER_URL YOUTUBE_PROXY_COUNTRY YOUTUBE_PROXY_URL

**autoclip** (1 names)

PATH

**ViralCutter** (2 names)

ORT_LOGGING_LEVEL TF_CPP_MIN_LOG_LEVEL

**bridgeclip** (50 names)

ADVANCED_TRANSCRIPTION_MODEL APPDATA BRIDGECLIP_BATCH_SCREENSHOT BRIDGECLIP_CONTENT_BANK_SCREENSHOT BRIDGECLIP_DISABLE_AUTO_UPDATE BRIDGECLIP_E2E BRIDGECLIP_E2E_APP_DIR 
BRIDGECLIP_E2E_BROWSER_URL BRIDGECLIP_E2E_OPENROUTER_URL BRIDGECLIP_E2E_SHOTS BRIDGECLIP_E2E_SKIP_BUILD BRIDGECLIP_E2E_TOOLS BRIDGECLIP_E2E_TRANSCRIPTION_URL 
BRIDGECLIP_E2E_UNLOAD_CHOICE BRIDGECLIP_ENHANCEMENT_SCREENSHOT BRIDGECLIP_FFMPEG_ARCHIVE BRIDGECLIP_FFMPEG_MIRROR BRIDGECLIP_GUIDANCE_SCREENSHOT 
BRIDGECLIP_LIBRARY_FILTER_SCREENSHOT BRIDGECLIP_UPLOAD_RECOVERY_SCREENSHOT BRIDGECLIP_USER_DATA_DIR BRIDGECLIP_WORK_ROOT BRIDGECLIP_ZERNIO_API_URL CLIPPING_MODE 
EDITORIAL_REPAIR_MODEL ELECTRON_RENDERER_URL GITHUB_ACTIONS HOME LAYOUT_VISION_ENABLED LOCALAPPDATA LOCAL_MODE LOCAL_OUTPUT_DIR NODE_ENV PATH PLANNER_FALLBACK_MODELS 
PLANNER_MAX_OUTPUT_TOKENS PLANNER_MODEL PLANNER_SUPPORTS_IMAGES SHELL SIGNING_ACCOUNT SIGNING_ENDPOINT SIGNING_PROFILE TEST_FFMPEG TEST_FFMPEG_DIR TEST_FFPROBE TEST_PYTHON USER 
YTDLP_NO_PLUGINS YTDLP_PROXIES YTDLP_PROXY

**opensource-clipping** (16 names)

APP_TIMEZONE GOOGLE_API_KEY HF_TOKEN MAX_CONCURRENT_JOBS META_GRAPH_VERSION META_PAGE_ACCESS_TOKEN META_PAGE_ID MPLBACKEND NGROK_AUTHTOKEN NVIDIA_API_KEY OSC_JOB_STORE_PATH 
OSC_LOG_DIR OSC_LOG_RUN_ID OSC_VIDEO_SCALE_ALGO PEXELS_API_KEY PYTHON_DOTENV_DISABLED

### Full names: from .env.example / .env.sample (per repo)

**openshorts** (45 names)

ACTOR_UPLOAD_RETENTION_SECONDS ASR_GPU_CONCURRENCY AWS_ACCESS_KEY_ID AWS_REGION AWS_S3_BUCKET AWS_S3_PUBLIC_BUCKET AWS_SECRET_ACCESS_KEY BACKUP_AGE_RECIPIENT BACKUP_DEST 
BACKUP_S3_ENDPOINT DATABASE_URL DEBUG_LOGS FFMPEG_ENCODER FORWARDED_ALLOW_IPS FRONTEND_URL GEMINI_IMAGE_MODEL GEMINI_MODEL_THUMBNAIL HSTS_HEADER LIFECYCLE_EMAILS_DISABLED 
LLM_API_KEY LLM_BASE_URL LLM_MODEL LLM_SCORE_BATCH LLM_TIMEOUT MEDIA_TOKEN_TTL_SECONDS MEDIA_URL_TTL_SECONDS OAUTH_REDIRECT_URI PAID_PROXY_DAILY_MB PUBLIC_API_URL 
QUALITY_GATE_MIN_HEIGHT RATE_LIMIT_AUTH RATE_LIMIT_ENABLED RATE_LIMIT_MCP RATE_LIMIT_PROCESS RETENTION_DAYS THUMBNAIL_RETENTION_SECONDS TRANSCRIBE_BACKEND VITE_OPENPANEL_API_URL 
VITE_OPENPANEL_CLIENT_ID WHISPER_COMPUTE WHISPER_DEVICE WHISPER_MODEL WINBACK_PROMO_CODE WINBACK_PROMO_LABEL YOUTUBE_COOKIES

**supoclip** (204 names)

AI_EDIT_DAILY_LIMIT AI_EDIT_MODEL ALLOW_UNSIGNED_BACKEND_AUTH ANTHROPIC_API_KEY APIFY_API_TOKEN APIFY_DOWNLOAD_MAX_CHARGE_USD APIFY_EXPENSIVE_FALLBACKS 
APIFY_FALLBACK_MAX_CHARGE_USD APIFY_KICK_HLS_ACTOR APIFY_KICK_MAX_VIDEO_SIZE_MB APIFY_KICK_MP4_ACTOR APIFY_RUN_TIMEOUT_SECONDS APIFY_STORAGE_BUCKET APIFY_STORAGE_KEY_TEMPLATE 
APIFY_STORAGE_READER_ACCESS_KEY_ID APIFY_STORAGE_READER_SECRET_ACCESS_KEY APIFY_STORAGE_REGION APIFY_STORAGE_WRITER_ACCESS_KEY_ID APIFY_STORAGE_WRITER_SECRET_ACCESS_KEY 
APIFY_STREAM_PROVIDERS APIFY_STREAM_TIMEOUT_SECONDS APIFY_TWITCH_ACTOR APIFY_YOUTUBE_DEFAULT_QUALITY APIFY_YOUTUBE_DOWNLOADER_ACTOR APIFY_YOUTUBE_FALLBACK_ACTOR 
APIFY_YOUTUBE_FALLBACK_ACTORS APIFY_YOUTUBE_INPUT_ADAPTER APIFY_YOUTUBE_PROVIDERS APP_SETTINGS_ENCRYPTION_KEY ASSEMBLY_AI_API_KEY ASSEMBLY_AI_HTTP_TIMEOUT_SECONDS 
ASSEMBLY_AI_REQUEST_TIMEOUT_SECONDS ASSEMBLY_AI_SPEECH_MODEL AUTH_SIGNATURE_TTL_SECONDS AWS_ACCESS_KEY_ID AWS_REGION AWS_SECRET_ACCESS_KEY AXIOM_DATASET AXIOM_TOKEN 
BACKEND_AUTH_SECRET BACKEND_CPU_ONLY BACKEND_PLATFORM BETTER_AUTH_SECRET BETTER_AUTH_TRUSTED_ORIGINS BETTER_AUTH_URL COMPOSE_BAKE COMPOSE_DOCKER_CLI_BUILD COMPOSE_PROFILES 
CORS_ORIGINS DEFAULT_PROCESSING_MODE DISABLE_SIGN_UP DISCORD_FEEDBACK_WEBHOOK_URL DISCORD_SALES_WEBHOOK_URL DOCKER_BUILDKIT EMBED_CRON_WORKER FAST_MODE_MAX_CLIPS 
FAST_MODE_TRANSCRIPT_MODEL FRONTEND_PORT GOOGLE_API_KEY HOOK_FIRST_TEASER INSTAGRAM_APP_ID INSTAGRAM_APP_SECRET KICK_DOWNLOAD_PROVIDERS LLM MAX_CLIPS MAX_VIDEO_DURATION 
MAX_VIDEO_UPLOAD_BYTES MCP_OAUTH_SECRET MEDIA_CACHE_TTL_SECONDS MEDIA_FAILOVER_ENABLED MEDIA_FALLBACK_ACCESS_KEY_ID MEDIA_FALLBACK_BUCKET MEDIA_FALLBACK_ENDPOINT 
MEDIA_FALLBACK_REGION MEDIA_FALLBACK_SECRET_ACCESS_KEY MEDIA_ORPHAN_GRACE_SECONDS MEDIA_PREFERRED_STORE MEDIA_S3_ACCESS_KEY_ID MEDIA_S3_BUCKET MEDIA_S3_ENDPOINT MEDIA_S3_PREFIX 
MEDIA_S3_REGION MEDIA_S3_SECRET_ACCESS_KEY MEDIA_S3_SESSION_TOKEN MEDIA_STORAGE MEDIA_UPLOAD_CONCURRENCY MEDIA_UPLOAD_DAILY_LIMIT MEDIA_URL_TTL_SECONDS NEXT_PUBLIC_APP_URL 
NEXT_PUBLIC_DATAFAST_ALLOW_LOCALHOST NEXT_PUBLIC_DATAFAST_DOMAIN NEXT_PUBLIC_DATAFAST_WEBSITE_ID NEXT_PUBLIC_LANDING_ONLY_MODE NEXT_PUBLIC_POSTHOG_HOST NEXT_PUBLIC_POSTHOG_KEY 
NEXT_PUBLIC_PRO_PLAN_MINUTE_LIMIT NEXT_PUBLIC_PRO_PRICE_ANNUAL NEXT_PUBLIC_PRO_PRICE_MONTHLY NEXT_PUBLIC_SCALE_PLAN_MINUTE_LIMIT NEXT_PUBLIC_SCALE_PRICE_ANNUAL 
NEXT_PUBLIC_SCALE_PRICE_MONTHLY NEXT_PUBLIC_SOCIAL_PUBLISHING NEXT_PUBLIC_TILLY_WEBSITE_ID OLLAMA_API_KEY OLLAMA_BASE_URL OPENAI_API_KEY OPENROUTER_API_KEY 
OPENROUTER_FALLBACK_MODEL PEXELS_API_KEY POSTGRES_DB POSTGRES_PASSWORD POSTGRES_USER POSTHOG_HOST POSTHOG_PERSONAL_API_KEY POSTHOG_PROJECT_ID POSTHOG_PROJECT_TOKEN 
PRO_PLAN_MINUTE_LIMIT PRO_YOUTUBE_MAX_VIDEO_DURATION QUEUED_TASK_TIMEOUT_SECONDS QUEUE_DELAY_ALERT_INTERVAL_SECONDS QUEUE_DELAY_ALERT_SECONDS RAPIDAPI_YOUTUBE_API_KEY 
RAPIDAPI_YOUTUBE_HOST RAPIDAPI_YOUTUBE_RELAY_HOST RAPIDAPI_YOUTUBE_TIMEOUT_SECONDS REDIS_PASSWORD RERANK_LLM REVENUECAT_PRO_PRODUCT_IDS REVENUECAT_SCALE_PRODUCT_IDS 
REVENUECAT_WEBHOOK_AUTH_HEADER SCALE_PLAN_MINUTE_LIMIT SCALE_YOUTUBE_MAX_VIDEO_DURATION SELF_HOST SES_FROM_EMAIL SOCIAL_METRICS_MAX_POST_AGE_DAYS SOCIAL_METRICS_REFRESH_HOURS 
SOCIAL_OAUTH_REDIRECT_BASE_URL SOCIAL_PERFORMANCE_MIN_POSTS SOCIAL_PUBLIC_MEDIA_BASE_URL SOCIAL_PUBLISHING_ACCESS SOCIAL_PUBLISHING_ALLOWED_EMAILS SOCIAL_PUBLISHING_PROVIDERS 
SPEAKER_FRAMING STRIPE_CHECKOUT_URL STRIPE_CREATOR_COUPON_ID STRIPE_CUSTOMER_PORTAL_URL STRIPE_PRICE_ID STRIPE_PRO_ANNUAL_PRICE_ID STRIPE_PRO_LEGACY_PRICE_IDS STRIPE_PRO_PRICE_ID 
STRIPE_SCALE_ANNUAL_PRICE_ID STRIPE_SCALE_LEGACY_PRICE_IDS STRIPE_SCALE_PRICE_ID STRIPE_SECRET_KEY STRIPE_WEBHOOK_SECRET SUPOCLIP_API_KEY SUPOCLIP_API_URL SUPOCLIP_APP_URL 
SUPOCLIP_AUTH_SECRET SUPOCLIP_DOWNLOAD_DIR SUPOCLIP_MCP_HOST SUPOCLIP_MCP_PORT SUPOCLIP_MCP_PUBLIC_URL SUPOCLIP_MCP_REQUIRE_BEARER_AUTH SUPOCLIP_MCP_TRANSPORT SUPOCLIP_TIMEOUT 
SUPOCLIP_USER_ID TIKTOK_APP_AUDITED TIKTOK_CLIENT_KEY TIKTOK_CLIENT_SECRET TRANSCRIPTION_LANGUAGE TRANSCRIPTION_PROVIDER TWITCH_DOWNLOAD_PROVIDERS UPLOAD_COPY_WAIT_SECONDS 
VIDEO_JOB_DEFER_SECONDS VIDEO_JOB_USER_CONCURRENCY WHISPER_MODEL_SIZE WORKER_DATABASE_URL WORKER_ENV_FILE WORKER_IMAGE WORKER_REDIS_HOST WORKER_REDIS_PORT WORKER_SCRATCH_PATH 
WORKFLOWS_ACCESS WORKFLOWS_ALLOWED_EMAILS WORKFLOWS_PUBLIC WORKFLOW_FULL_CONTEXT_MAX_WORDS WORKFLOW_PLAN_TIMEOUT_SECONDS WORKFLOW_TOPIC_MAX_CLIPS YOUTUBE_DATA_API_KEY 
YOUTUBE_DOWNLOAD_MAX_BYTES YOUTUBE_DOWNLOAD_PROVIDER YOUTUBE_MAX_HEIGHT YOUTUBE_METADATA_PROVIDER YOUTUBE_OAUTH_CLIENT_ID YOUTUBE_OAUTH_CLIENT_SECRET YOUTUBE_POT_PROVIDER_URL 
YOUTUBE_PREFLIGHT_TIMEOUT_SECONDS YOUTUBE_PROXY_COUNTRY YOUTUBE_PROXY_TIMEOUT_SECONDS YOUTUBE_PROXY_URL YOUTUBE_SECTION_PADDING_SECONDS YOUTUBE_SOURCE_CACHE 
YOUTUBE_SOURCE_CACHE_TTL_SECONDS

**opensource-clipping** (8 names)

APP_TIMEZONE GOOGLE_API_KEY HF_TOKEN META_GRAPH_VERSION META_PAGE_ACCESS_TOKEN META_PAGE_ID NVIDIA_API_KEY PEXELS_API_KEY

bridgeclip: .env.example declares no variables (provider keys are entered in-app).
