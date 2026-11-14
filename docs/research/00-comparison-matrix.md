# Comparison matrix: cross-repo synthesis

Inputs: `docs/00-vision-and-requirements.md` (wins on conflict), `docs/research-methodology.md`, `docs/research/00-license-and-dependency-inventory.md`, and the 7 per-repo findings. The repo source was not re-read; all facts come from the findings files. Their citations use paths relative to `repos/<repo>/`. Anything a report marked UNVERIFIED stays UNVERIFIED here.

**Repo key / license / reuse mode.**

| Key | Repo | License | What we may take |
|---|---|---|---|
| HY | hypit | Modified Apache-2.0: no SaaS, no commercial redistribution | Ideas only |
| SC | supoclip | AGPL-3.0 | Ideas only. Prompts are expression, so rewrite them |
| OS | openshorts | MIT, except `cloud/`, which is commercial | Code and prompts (not `cloud/`) |
| BC | bridgeclip | MIT | Code and prompts. Not the OpusClip logo, Jev endpoint or brand assets |
| OC | opensource-clipping | MIT | Code and prompts. Not the bgm mp3s or YouTube overlay assets |
| AC | autoclip | MIT | Code and prompts |
| VC | ViralCutter | GPL-3.0 | Ideas only |

**Reuse modes.**
- **COPY**: port MIT code or prompt text to Rust and keep the MIT notice in `THIRD_PARTY_NOTICES`.
- **IDEA**: clean-room reimplementation from behaviour or algorithm only.
- **ASSET**: fetch from the original upstream with its license file.
- **AVOID**: do not use.

---

## 1. Scoring matrix (1 = absent/broken · 2 = naive · 3 = workable · 4 = solid · 5 = best-in-class)

Category rows in **bold** show the score each per-repo report gave itself. Sub-rows are synthesized from the evidence in those reports.

| ID | Capability | HY | SC | OS | BC | OC | AC | VC |
|---|---|---|---|---|---|---|---|---|
| **A** | **AI clipping (report score)** | 1 | 5 | 4 | 4 | 3 | 3 | 3 |
| A1 | Highlight prompt and rubric | 1 | 5 | 5 | 4 | 3 | 4 | 3 |
| A2 | Long-video windowing | 1 | 5 | 5 | 2 | 1 | 4 | 3 |
| A3 | LLM timing contract (ids, indices or anchors instead of seconds) | 1 | 5 | 3 | 4 | 2 | 5 | 3 |
| A4 | Cut-boundary snapping and repair | 1 | 5 | 4 | 4 | 1 | 5 | 2 |
| A5 | Global rerank, dedupe and clip-count policy | 1 | 5 | 5 | 4 | 1 | 3 | 2 |
| A6 | Non-LLM signals (audio peaks, triggers, priors, static drop) | 1 | 5 | 3 | 3 | 1 | 1 | 1 |
| A7 | Judge / repair QA loop on candidates | 2 | 4 | 2 | 5 | 1 | 2 | 1 |
| A8 | Visual path (silent footage, frame-based picking) | 5 | 3 | 4 | 4 | 1 | 1 | 1 |
| A9 | Multi-clip workflows (series, recap, trailer, stitched story) | 1 | 5 | 1 | 3 | 3 | 1 | 1 |
| **B** | **Transcription and subtitles (report score)** | 4 | 4 | 4.5/4 | 4 | 3 | 3 | 3 |
| B1 | Local STT engine and fallback rules | 3 | 4 | 5 | 2 | 3 | 3 | 3 |
| B2 | STT via HTTP endpoint (OpenAI-shaped) | 3 | 1 | 1 | 4 | 1 | 1 | 1 |
| B3 | Word timing: alignment, repair, no-fabrication | 5 | 4 | 4 | 4 | 2 | 3 | 4 |
| B4 | Diarization and speakers on words | 1 | 4 | 1 | 3 | 3 | 3 | 1 |
| B5 | Animated word captions (karaoke, active word) | 4 | 5 | 5 | 5 | 3 | 3 | 2 |
| B6 | Subtitle file export (SRT/VTT/ASS sidecar) | 1 | 2 | 4 | 4 | 1 | 2 | 4 |
| B7 | Emoji and emphasis words | 2 | 5 | 2 | 4 | 3 | 1 | 1 |
| **C** | **Creative design (report score)** | 4 | 4 | 3.5 | 5/2 | 3/2 | 2 | 3 |
| C1 | Caption template schema and presets as data | 5 | 4 | 3 | 5 | 2 | 2 | 3 |
| C2 | Typography and line fitting (real font metrics) | 4 | 5 | 3 | 4 | 3 | 2 | 2 |
| C3 | Caption and hook placement (face-aware, safe zone) | 3 | 4 | 4 | 5 | 1 | 2 | 3 |
| C4 | Hook title, title card and overlays | 4 | 4 | 4 | 3 | 3 | 1 | 1 |
| C5 | Transitions, motion and effects | 4 | 2 | 3 | 2 | 3 | 1 | 1 |
| C6 | Bundled fonts with license hygiene | 3 | 4 | 2 | 5 | 2 | 2 | 1 |
| **D** | **Video processing and rendering (report score)** | 4 (speed 2) | 5 | 4 | 5 | 2-3 | 4 | 2 |
| D1 | Single-pass filtergraph and A/V-sync discipline | 3 | 5 | 3 | 5 | 2 | 4 | 2 |
| D2 | HW encode and capability probe | 1 | 1 | 5 | 3 | 4 | 4 | 3 |
| D3 | Output validation (probe and timing; delete bad output) | 4 | 2 | 2 | 5 | 1 | 2 | 1 |
| D4 | Perception renders (frames, contact sheets, debug overlays) | 5 | 2 | 3 | 3 | 2 | 2 | 2 |
| D5 | Stage artifacts, resume, progress and cancel | 4 | 5 | 3 | 4 | 3 | 4 | 2 |
| **E** | **Reframing (report score)** | 2 | 5 | 3/4 | 4 | 3 | 4 | 3 |
| E1 | Face detection, license-safe and single-decode | 1 | 5 | 3 | 5 | 2 | 4 | 2 |
| E2 | Camera smoothing | 1 | 5 | 2 | 4 | 3 | 5 | 2 |
| E3 | Shot/scene detection | 2 | 4 | 5 | 4 | 1 | 4 | 1 |
| E4 | Layouts (split, screencast, inset, blur-fit) | 2 | 5 | 5 | 4 | 3 | 3 | 3 |
| E5 | Active speaker | 1 | 5 | 3 | 1 | 3 | 4 | 2 |
| E6 | Crop path as editable data, compact FFmpeg emission | 2 | 4 | 4 | 4 | 2 | 5 | 3 |
| E7 | "Don't crop" content awareness (text, screen, spread) | 1 | 5 | 4 | 4 | 1 | 3 | 1 |
| **F** | **Audio (report score)** | 3 | 3 | 2 | 2 | 2 | 2 | 1 |
| F1 | Loudness normalisation | 1 | 4 | 3 | 5 | 1 | 3 | 1 |
| F2 | Silence, filler and pause removal | 1 | 5 | 1 | 5 | 1 | 2 | 1 |
| F3 | Music bed and ducking | 3 | 1 | 1 | 1 | 4 | 1 | 1 |
| F4 | SFX placement | 2 | 1 | 1 | 1 | 1 | 1 | 1 |
| F5 | Sample-accurate audio graph | 5 | 4 | 2 | 5 | 2 | 3 | 1 |
| F6 | TTS / voice-over | 2 | 1 | 2 | 1 | 3 | 1 | 1 |
| F7 | Denoise / speech enhancement | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
| **G** | **Agent integration (report score)** | 4 | 3 | 5 | 2 | 1-2 | 1 | 2 |
| G1 | MCP server (typed tools, annotations, transport) | 1 | 3 | 4 | 3 | 1 | 1 | 1 |
| G2 | CLI contract (JSON out, error object, exit codes) | 4 | 1 | 3 | 3 | 2 | 2 | 2 |
| G3 | Granular typed edit operations | 4 | 4 | 3 | 2 | 3 | 2 | 2 |
| G4 | Project state, hand-off, plan replay | 5 | 4 | 2 | 4 | 3 | 4 | 3 |
| G5 | Agent skill and docs that teach judgement | 5 | 2 | 5 | 3 | 1 | 1 | 1 |
| G6 | Safety: approvals, injection posture, failure codes | 4 | 4 | 4 | 5 | 1 | 2 | 1 |
| **H** | **GUI (report score)** | 3.5 | 4 | 4 | 4 | 3 | 3 | 3 |
| H1 | Timeline and trim editor | 4 | 4 | 4 | 4 | 1 | 3 | 2 |
| H2 | Caption/subtitle editor | 4 | 4 | 3 | 4 | 1 | 3 | 4 |
| H3 | Preview/export parity | 5 | 2 | 2 | 2 | 1 | 2 | 4 |
| H4 | Agent activity, feedback and approvals in UI | 3 | 2 | 2 | 4 | 1 | 1 | 1 |
| **I** | **Provider abstraction (report score)** | 4 media / 1 LLM | 3 | 2.5 | 2 | 2 | 3 | 2 |
| I1 | OpenAI-compatible LLM (any `base_url`) | 1 | 4 | 3 | 2 | 2 | 4 | 1 |
| I2 | Structured output robustness (schema ladder, repair, JSON recovery) | 1 | 5 | 4 | 4 | 3 | 4 | 4 |
| I3 | STT provider abstraction | 4 | 2 | 2 | 3 | 1 | 1 | 1 |
| I4 | TTS provider abstraction | 3 | 1 | 2 | 1 | 2 | 1 | 1 |
| I5 | Async jobs, retries, cost accounting | 5 | 3 | 3 | 4 | 3 | 2 | 2 |
| I6 | Credentials (keychain, env references) | 4 | 3 | 2 | 4 | 2 | 4 | 1 |

**Read-across:**
- SC is the best reference for heuristics, but it is AGPL, so ideas only.
- OS, BC and AC are the best sources of MIT code.
- HY is the best reference for agent ergonomics and perception, ideas only.
- F (audio), F7 (denoise), I4 (TTS) and post-render QC are weak in every repo.

---

## 2. Best source per capability

| Cap | Winner(s) | File refs (from reports) | Mode | Notes for the Rust design |
|---|---|---|---|---|
| A1 prompt | OS, AC, BC (SC) | OS `gemini_worker.py:254-294,296-372`; AC `backend/autoclip/prompts/highlight_v1.txt:9-32,44-61`; BC `intelligence_planner.py:800-896`; SC `ai.py:283-428`, `workflows/prompts.py:20-42,425-450` | COPY (OS/AC/BC), IDEA (SC) | Write one English prompt from the MIT text. Core rules: 2-second test, standalone, payoff, diversity, "about this moment", harsh 0-100 scale. Add SC-style grounding rules in our own words. Store prompts as versioned data files. |
| A2 windowing | OS, SC | OS `clip_selection.py:71-82,114-130,228-283`, `main.py:2096-2286`; SC `ai.py:43-47,1184` | COPY (OS) | Flow: segment-aligned overlapping windows → score every window in batches → global shortlist → detail pass with the floor enforced in code. Pass `dedupe` before the floor; OS has a bug where dedupe runs after it (`main.py:2221-2258`). |
| A3 timing contract | SC workflows, AC | SC `workflows/units.py:56-189`, `workflows/prompts.py:26-36`, `workflows/boundaries.py:27-65`; AC `highlights.py:98-114`, `base.py:196-212` | IDEA (SC), COPY (AC) | See §3 C1. The LLM and the agent pick **unit ids**; code computes the times. |
| A4 boundaries | AC, SC, OS | AC `boundaries.py:60-237`; SC `timeline.py:45-156,343-530`; OS `clip_selection.py:328-390`; BC `transcription_service.py:156-428` | COPY (AC/OS), IDEA (SC) | Pure Rust functions: sentence snap → duration clamp on sentence ends → silence-trough alignment → weak-opener rewind. Weak-opener word lists from SC become per-language data files. |
| A5 rerank / count | SC, OS, BC | SC `ai.py:598-668,1283-1395`; OS `clip_selection.py:52-82,133-165,286-325`; BC `intelligence_planner.py:281-356`, `config.py:832-893` | IDEA (SC), COPY (OS/BC) | A comparative rerank call with monotone calibration (idea). For clip count, use BC's exp-saturation curve with a WPM factor. |
| A6 signals | SC | SC `timeline.py:750-872`, `visual_suitability.py:57-220`; OS `twitch_signals.py:85-141` | IDEA (SC), COPY (OS) | A cheap deterministic signal summary: trigger words, `?`/`!`, pauses, speaker turns, `astats` RMS peaks, static/slate drop. It works with no LLM and also feeds the agent. |
| A7 judge loop | BC | BC `coherence_review.py:22-66`, `sponsor_policy.py:3-26`; SC `workflows/llm.py:167-210`, `workflows/schemas.py:1-16` | COPY (BC rubric), IDEA (SC) | Yes/no questions with thresholds. A repair must cite the failed check, a real unit id and an exact quote. Use any LLM; Jev is AVOID. |
| A8 visual pick | HY, OS, BC | HY `packages/media-local/src/files.ts:808-819`, `skills/hypit/references/creation/reference-video.md:103-162`; OS `main.py:2295-2340`; BC `intelligence_planner.py:998,1076` | IDEA (HY), COPY (OS/BC) | The agent's own vision is used first, with a vision endpoint as fallback. OS 1-fps 360p proxy and BC sampled-frame planner for silent footage. |
| A9 workflows | SC (OC) | SC `workflows/partition.py:33-65,488-546`, `workflows/prompts.py:351-358,588-638`, `workflows/assembly.py:177-185`; OC `story_recipe.json:1-45` | IDEA (SC), COPY (OC) | Later feature. Series DP partition, recap knapsack and trailer "sell the question". OC recipe as the EDL shape. |
| B1 local STT | OS | OS `transcribe_backends.py:573-709`, `subtitles.py:30-35` | COPY (logic) | Engine is whisper.cpp; faster-whisper is Python, so excluded. Port the fallback rules: 0 words / unsupported language / <0.2 words/s, plus wrong-alphabet repair. |
| B2 STT endpoint | BC (HY) | BC `transcription_service.py:72,120-145,500-616`; HY `packages/provider-hypihub/src/provider.ts:613-619` | COPY (BC), IDEA (HY) | `/v1/audio/transcriptions` multipart with `verbose_json` and word granularity. Chunk 300 s with 1 s overlap and midpoint ownership. Fail rather than fabricate word timestamps. |
| B3 alignment | HY, VC | HY `packages/narrative-speech-alignment/src/align.ts:8-423`; SC `timeline.py:868-950`; OS `transcribe_backends.py:482-514` | IDEA | Script↔ASR DP so the agent can correct words and keep timing. Repair zero-length words. |
| B4 speakers | SC, AC | AC `transcribe.py:192-242,285-306`; SC `transcription.py:165-174` | COPY (AC), IDEA (SC) | Assign words by maximum overlap. The diarization source is a provider (endpoint); pyannote stays optional. |
| B5 karaoke ASS | OS, BC, SC | OS `subtitles.py:332-518`; BC `caption_generator.py:1-50,152-240,546-775`; AC `captions.py:201-367`; SC `captions.py:815-889` | COPY (OS/BC/AC), IDEA (SC) | Per-word events, full block repeated. Active word changes colour only (SC anti-jitter idea). BC 5-layer stack; `\kf` for sweep presets. |
| B6 sidecars | BC | BC `caption_generator.py:245-326` | COPY | SRT: 42 chars/line, 6 s maximum cue, 700 ms pause break. Also write VTT and ASS. |
| B7 emphasis / emoji | SC, BC | SC `emoji_captions.py:24-190`, `captions.py:147-201`; BC `intelligence_planner.py:170-176,904-907` | IDEA (SC), COPY (BC) | 2-5 planner emphasis words. Emoji map as a data file. Runtime libass emoji probe (render one frame, check saturation). |
| C1 template schema | HY, BC | HY `packages/caption-fine/src/style.ts:202-283`, `examples/ranking-football/recipes.svs:78-128`; BC `config.py:155-563`; AC `captions.py:65-95` | IDEA (HY), COPY (BC) | Style schema in TOML/JSON (schemars), using HY's vocabulary: karaoke current/trail × step/wipe, active-box, lead/tail frames. Start from BC's 13 presets. |
| C2 font metrics | SC | SC `font_registry.py:369-520`, `captions.py:48-102,266-305`; OS `subtitles.py:292-298` | IDEA | `ttf-parser`/`rustybuzz` advances; recursive balanced split; glyph-coverage fallback font. |
| C3 placement | BC, OS | BC `layout_renderer.py:722-945,851-860`; OS `hook_placement.py:33-180`, `burned_subtitles.py:34-93`; SC `source_captions.py:1-25,121-170` | COPY (BC/OS), IDEA (SC) | Choose the caption position by minimising face overlap, with hysteresis. Hook placement as a 1-D search. Move captions off any burned-in source caption band. |
| C4 hooks / titles | OS, BC | OS `hooks.py:43-91,207-224,656-668`, `hook_grounding.py:144-185`; BC `rendering_service.py:944-1020` | COPY | Render hook PNGs with the `image`/`tiny-skia` crates rather than PIL. Frame-grounded hook rewrite. |
| C5 effects | OS, HY | OS `edit_builder.py:15-23,94-155`, `editor.py:104-132,367-414`; OC `studio/v2_helpers.py:12-58`; HY `caption-fine/src/recipe.ts:3-7` | COPY (OS/OC), IDEA (HY) | Effects are a typed enum compiled to an allowlisted filter set; never free text. Fix the OS clip-relative time-base bug (`main.py:2845`). |
| C6 fonts | BC | BC `engine/assets/fonts/*-OFL.txt`, `engine/THIRD_PARTY_NOTICES.md:7-12`; SC `backend/fonts/SOURCES.md` | ASSET | Fetch from upstream Google Fonts with the OFL text (see §5). |
| D1 render graph | BC, SC | BC `layout_renderer.py:540-669`, `rendering_service.py:412-446,1100-1230`; SC `media/ffmpeg.py:388-713` | COPY (BC), IDEA (SC) | One `-filter_complex` per clip. The frame-grid allocation and silent-bed `amix` are hard-won A/V-sync fixes; so are `asetpts=N` after loudnorm, the NaN scrub and `-protocol_whitelist`. Add frame-exact xfade (SC idea) and sparse multi-input seek. |
| D2 HW encode | OS, AC | OS `ffmpeg_utils.py:26-52,157-236`, `bin/ffmpeg:23-106`; AC `system.py:190-254`; OS `osrender/src/main.rs:285-508` | COPY | Do a real 1-frame probe per encoder. NVENC `cq≈crf+7`. Encoder tiers as config. osrender (Rust, MIT) is the template for a later GPU path. |
| D3 output validation | BC, HY | BC `rendering_service.py:1146-1230`; HY `packages/provider-html-local/src/output.ts:39-73` | COPY (BC), IDEA (HY) | Bad output is deleted and the render drops down a fallback ladder: smart → letterbox with the same cuts → letterbox natural. |
| D4 perception renders | HY, OC | HY `files.ts:808-819`, `packages/studio/src/storyboard.ts:117-130`; OC `studio/render_hybrid.py:394-470` | IDEA (HY), COPY (OC) | `tiles --transcript --around`, filmstrip, and an OC-style "director's console" debug overlay so the agent can see crop decisions. |
| D5 resume / progress | AC, SC | AC `pipeline/runner.py:73-102,201-226`, `pipeline/ffmpeg.py:234-265,336-358`; SC `task_checkpoint_repository.py:13-52` | COPY (AC), IDEA (SC) | Files in the project dir are the source of truth, with input-hash invalidation. Parse `-progress pipe:1`. AC's Windows `cwd` + bare-name trick avoids filter-path escaping. |
| E1 face detection | BC, SC | BC `layout_analyzer.py:70,113-115,1739-1800`; SC `reframing.py:784-883`, `face_detection.py:17-27` | COPY (BC), ASSET (YuNet) | YuNet via the `ort` crate. A single FFmpeg decode feeds two branches: a scene-score branch and a raw analysis branch at 3-4 fps. |
| E2 smoothing | AC (SC) | AC `pipeline/reframe/smoothing.py:23-135`; OS `main.py:107,170-218`; SC `reframing.py:900-1042` | COPY (AC/OS), IDEA (SC) | See §3 C3. |
| E3 scenes | BC (OS) | BC `camera_scan.py:23`, `layout_analyzer.py:438,1771-1781`; OS `scene_detection.py:46,120,142-168` | COPY (BC) | FFmpeg `select='gte(scene,0)'` + `metadata=print` plus an HSV-histogram check. TransNetV2 needs torch, so it is a research item for ONNX. |
| E4 layouts | OS, SC | OS `split_layout.py:38-203`, `screencast_layout.py:45-156,434-749`, `camera_inset.py:25-455`, `gpu_render.py:232-326`; SC `reframing.py:1045-1122`; BC `layout_renderer.py:28-42,392-470` | COPY (OS/BC), IDEA (SC) | Layer model `(src_rect, dst_rect, blur)` → filtergraph, and later a GPU plan from the same layers. Port the OS thresholds. Use the quarter-resolution blur backdrop. |
| E5 active speaker | SC, AC, OS | SC `speaker_framing.py:381-436,780-900`; AC `pipeline/reframe/speaker.py:33-195`; OS `active_speaker.py:27-218` | IDEA (SC), COPY (AC/OS) | Correlate lower-face motion with diarized turns. Ambiguity margin, else fall back to the dominant face. Behind a flag. |
| E6 crop path data | AC, OS | AC `croppath.py:104-141,208-306`; OS `reframe_v2.py:81-92,917-921`; BC `manual_editor.py:311-370` | COPY | `CropPath` JSON is the source of truth. Emit it as `sendcmd` lines written only when the value changes (see §3 C4). |
| E7 don't-crop | SC, BC, OS | SC `scene_context.py:61-80,287-311`; BC `layout_analyzer.py:1277-1291`; OS `layout_picker.py:55-61`, `gemini_worker.py:142-171` | IDEA (SC), COPY (BC/OS) | Reasons that force a fit layout, plus a closed-choice vision layout prompt that answers "none" when unsure. |
| F1 loudness | BC | BC `rendering_service.py:900-930`, `layout_renderer.py:498-521`; OS `ffmpeg_utils.py:79-93` | COPY | See §3 C9. |
| F2 pacing | BC, SC | BC `clip_editor.py:23-56,85-101,306-363`; SC `timeline.py:1020-1213`, `gap_activity.py:107-200` | COPY (BC), IDEA (SC) | Per-layout pause caps, keep-ranges EDL, a TimeMap that remaps captions, protected reaction/action gaps. |
| F3 music ducking | OC | OC `studio/audio_bgm.py:36-67` | COPY | `sidechaincompress` threshold 0.08, ratio 5, attack 100 ms, release 1000 ms. Fix: `amix normalize=0`, add fades and a start offset, duck in every segment. |
| F5 audio graph | HY, BC | HY `packages/media-local/src/execute.ts:477-518,1236-1242`, `audio-level-automation.ts:4-31` | IDEA (HY), COPY (BC) | 48 kHz canonical clock; `adelay ..S:all=1`; `apad whole_len` + `atrim end_sample`; `aeval` gain envelopes. |
| F6 TTS | (none good) | OC `voiceover.py:74-130`; HY `provider-hypihub/src/provider.ts:249-299` | IDEA | Word-boundary grouping idea only. edge-tts is AVOID. See §4. |
| G1 MCP | OS, BC | OS `mcp_server.py:64-357,463-510,690-705`, `mcp_stdio.py:29-31,93-114`; BC `src/main/assistant/mcp-server.ts:11-17,36-200`, `tool-types.ts:11-31` | COPY | The `rmcp` crate. Annotations, `structuredContent`, stdio with stdout guarded. Loopback hardening: hashed bearer, Host/Origin checks, body cap. |
| G2 CLI | HY, BC | HY `packages/cli/src/output.ts:979-1038`, `main.ts:326-578`; BC `bridge/bridge_runner.py:38-183`; OS `cli/openshorts_cli.py:50-228` | IDEA (HY), COPY (BC/OS) | See §3 C11. |
| G3 edit ops | SC (OS) | SC `services/ai_edit_service.py:55-162`; OS `mcp_server.py:275-310,452-460` | IDEA (SC), COPY (OS) | Typed op set with clamping: keep/cut ranges, remove words/pauses, set, highlight/edit words, b-roll. Recut in source-time segments. "The model never writes the document." |
| G4 project | HY, BC | HY `skills/hypit/references/creation/project-files.md:89-114`, `SKILL.md:244-262`; BC `tests/fixtures/editor/project.json`, `manual_editor.py:311-370`; OC `runner.py:85-107` | IDEA (HY), COPY (BC/OC) | BRIEF / ANALYSIS / PROGRESS docs alongside a strict EDL JSON with revision-checked saves and a replayable plan. |
| G5 skill | OS, HY | OS `skills/openshorts/SKILL.md:19-113`; HY `skills/hypit/SKILL.md:218-323` | COPY (OS), IDEA (HY) | Rules to adopt: "choices the API cannot make for you", "Done means watched", budget consent, a routing table. Keep it short and procedural. |
| G6 safety | BC | BC `system-prompt.ts:27`, `tool-types.ts:22-27`, `bridgeclip-tools.ts:360-381`; SC `ai_edit_service.py:167-196` | COPY (BC), IDEA (SC) | Treat transcript, titles and web content as data, not instructions. Spend, publish and delete need confirmation. Never auto-publish. |
| I1 LLM | AC, OS | AC `providers/openai_provider.py:26-98`, `providers/base.py:134-311`; OS `llm_backend.py:33-151` | COPY | One OpenAI-compatible adapter. `response_format` ladder: json_schema → json_object → prompt-only. One repair retry carrying the validator error. |
| I2 JSON recovery | VC, AC, OC | VC `create_viral_segments.py:36-169`; AC `base.py:252-311`; OC `metadata.py:125-146`; SC `workflows/schemas.py:1-16` | IDEA (VC/SC), COPY (AC/OC) | Strip `<think>`; brace-matched extraction; per-item recovery from truncated output; key-alias healing. Small-model schema rules. |
| I3/I5 endpoints | HY | HY `packages/endpoint/src/index.ts:103-497`, `packages/generation/src/mapping.ts:22-78` | IDEA | `trait Endpoint { supports, start, poll, cancel, collect }` with checkpointed receipts, idempotency keys and data-only wire mappings. |
| I6 credentials | AC, HY | AC `config.py:195-236`; HY `packages/credential-store-local/src/file.ts:28-80` | COPY (AC), IDEA (HY) | `keyring` crate plus `RUSTLER_<NAME>_KEY` env override. Never pass keys in argv (a VC anti-pattern). |

---

## 3. Conflicts and choices

| # | Conflict (who does what) | Choice | Why (constraint) |
|---|---|---|---|
| C1 | **How the LLM references time.** Raw seconds (OS, OC; SC's highlights path uses MM:SS); word indices `[i]word` (AC); unit ids `#id` (SC workflows); segment ids + setup/payoff anchors (BC); quoted text anchors + `(Ns)` tags (VC) | **Unit ids** (sentence/clause units carrying a boundary strength) as the contract for both LLMs and agents. Word indices stay as a fine-grained refine option. A **verbatim quote of the opening words** is required and checked by code. | Ids remove timestamp hallucination and the 1 s precision problem (SC weakness #3). They cost fewer tokens than `[i]` per word (AC notes the verbosity). The quote check is the verification AC never runs. Text anchors (VC) are only segment-accurate and fuzzy, so they become a fallback for agent "raw repair". |
| C2 | **Selection shape.** Whole transcript in one call (BC, OC); per-window picks + IoU dedupe (AC); score all windows → shortlist → detail (OS); windows → dedupe → comparative rerank (SC) | OS two-pass (COPY), then an SC-style rerank (IDEA), then a BC judge (COPY). Every stage is its own tool, so the agent can replace any stage with its own judgement. | Scales to multi-hour sources. Per-call scores are not comparable across windows (AC weakness #9). Agent decides, tools execute. |
| C3 | **Camera smoothing.** One-Euro + 12 px dead zone + 260 px/s clamp (AC); SmoothDamp, 0.9 s, 5 % dead zone (SC); 25 % dead zone + constant 3/15 px/frame (OS); dead zone + proportional 0.30 (OC); dead zone 0.12 + speed cap 0.9 (BC); hold + 4-frame lerp (VC) | **Offline:** median filter → One-Euro (COPY AC) → dead zone → velocity clamp. All parameters are fractions of crop width per second, not pixels. Add OS 3-sample jump confirmation and snap on scene cut. Keyframes decimated to ≤48 per shot. | MIT code exists and the method is proven. OS's constant pixel speed depends on resolution (OS weakness #1). The whole track is known offline, so there is no causal-latency penalty. SmoothDamp stays a selectable alternative (idea only). |
| C4 | **Crop emission.** Nested piecewise `if(lt(t..))` expression (AC, SC); `sendcmd` file (OS); per-frame Python pipe (OC, VC) | **`sendcmd`** with lines written only on change, `trim`/`concat` per shot, and CropPath JSON as the source of truth. | FFmpeg's expression parser fails past ~100 terms (OS `gpu_decode.py:213-224`); SC suggests sendcmd too. Frame pipes need a Python host and re-encode. One caveat: on the FFmpeg path, changing crop **size** via sendcmd can truncate output (OS test comment, UNVERIFIED), so vary x/y only and keep w/h fixed per shot. |
| C5 | **Caption renderer.** ASS/libass via FFmpeg (SC, OS, BC, AC, OC, VC); headless-Chrome frame capture (HY); Remotion (OS) | **ASS/libass by default.** An optional later "HTML-overlay sidecar" renders an alpha track for motion graphics. Remotion is AVOID. | Single binary, no Chrome or Node, fast. Remotion carries a company-license risk. HY's Chrome path is slow (D speed 2) and cannot output alpha. |
| C6 | **Karaoke event model.** Per-word events repeating the block (OS, SC, AC); single-line `\kf` (AC, BC); per-word `\pos` events (OC); `\fs` growth (VC) | Per-word events where the active word changes **colour or box only, no scale** (SC anti-jitter idea). BC layered plate/glow/pill. `\kf` only for sweep presets. Pop is a one-shot line entrance. | Scaling one word reflows a centred line (SC `captions.py:815-850`; VC weakness #18). |
| C7 | **Preview vs export.** Canvas/CSS (SC), Remotion (OS), JS preview (BC), CSS (AC) all drift from libass. HY renders the same program; VC pipes FFmpeg + ASS output into the UI. | Previews use **the same libass path**: an FFmpeg one-frame render or a low-res proxy stream. | Parity was a weakness in 4 of 7 repos. |
| C8 | **Number of encodes.** 3-4 lossy passes (OS, OC, VC); 2 (SC: CRF-16 stitch + final); 1 (BC, AC) | **One encode per deliverable.** Captions, hooks, reframe, effects and audio all go in one filtergraph. An intermediate is allowed only for sparse multi-range sources, and only at near-lossless quality. | Generation loss (OS weakness #4, VC #4). |
| C9 | **Loudness.** Single-pass `I=-14:TP=-1.5:LRA=11` (SC, AC, BC short); `TP=-2` because AAC adds inter-sample peaks, and the limiter / linear two-pass "made it worse" (OS); two-pass linear (BC longform) | **Measure then apply** (two-pass) at I=-14 with **TP=-2** by default. Then **verify with `ebur128`** in QC and fall back to single-pass if the result is off target. | Clips are short, so the extra pass is cheap. The two repo findings disagree, and only measuring the output settles it. |
| C10 | **Face detector.** YuNet ONNX (SC, BC); MediaPipe BlazeFace / FaceLandmarker (OS, AC, OC); InsightFace buffalo_l (VC); YOLOv8 (OS, OC) | **YuNet via `ort`** (MIT). Mouth activity uses SC's **lower-face frame-difference** method, which needs no 68-point landmarks. | YuNet is MIT, small and needs no Python. YOLO is AGPL. buffalo_l is non-commercial. MediaPipe's model terms are unverified, and it needs Python 3.11-3.12 (AC). |
| C11 | **CLI contract.** Versioned JSON tags + `{ok:false,error:{code,message,help}}`, exit 0/1 (HY); `--json`, exit 1/2 (OS); stdin JSON / stdout JSONL with failure-code + stage table (BC) | HY envelope (`rustler.<cmd>@N`) + BC failure classes (code, stage, hint) + **granular exit codes** + NDJSON progress on stderr. MCP tools mirror CLI commands 1:1. | No single repo has all of it. OS progress is regex-parsed free text (OS weakness #9). |
| C12 | **Provider layer.** pydantic-ai multi-provider (SC); OpenAI-compatible + Anthropic/Gemini/Ollama adapters (AC); OpenRouter only (BC); Gemini at most call sites (OS) | **One OpenAI-compatible adapter** for LLM, vision, STT and TTS, with per-model capability flags. The HY async endpoint trait handles long jobs. Native adapters only if needed. | Requirement 8 (pluggable endpoints). BC's socket guard even blocks local endpoints (BC weakness #2). |
| C13 | **STT engine.** faster-whisper (OS, OC, AC); WhisperX + wav2vec2 alignment (HY, VC); openai-whisper / AssemblyAI (SC); OpenRouter (BC); Parakeet ONNX (OS) | **whisper.cpp** for local CPU/CUDA, plus any OpenAI-compatible STT endpoint. Port OS's fallback rules. Parakeet via `ort` is a research option. | No required Python (vision decision). |
| C14 | **Fit background.** Quarter-resolution gblur (OS, BC; ~45 % CPU saving); full-resolution boxblur (AC); black pad (VC) | Quarter-resolution blur, darkened (BC `lutyuv`). | Cheapest, and widely used in short-form output. |
| C15 | **Project and timeline model.** Compiled SVML language (HY); EDL JSON with scenes and crops (BC); flat plan replay (OC); keep-ranges + output offsets (SC) | **JSON project**: sources → keep-range EDL → per-shot layout/CropPath → caption track whose events are **word-anchored** (HY idea), so they move with their word on recut. Revision-checked saves (BC). Append-only history. | Editable plan files, agent hand-off (requirement 11). HY's compiler is too heavy for agents and humans (HY weakness #13). |
| C16 | **FFmpeg distribution.** LGPL build bundled and GPL filters avoided (BC); system FFmpeg via subprocess (all others) | `setup` **downloads** a static FFmpeg build into the tools dir; nothing is linked or bundled into our binary. Record which build (GPL/LGPL) was fetched; QC requires the `ass` filter and libass. | BC's lesson: decide the license early. A subprocess at arm's length keeps our binary clean. |

---

## 4. Gaps: needed by the vision, covered well by no repo

| Gap | Best partial | What to research (tools/libraries; licenses all **to verify**) |
|---|---|---|
| **Post-render quality loop** (hook, sync, captions, faces, crop, audio, integrity) | BC timing validation; HY "Done means watched" + ffprobe frame count; SC static/slate drop | FFmpeg `blackdetect`, `freezedetect`, `silencedetect`, `ebur128`, `astats`, `signalstats`, `idet`; SSIM/VMAF filters for output-vs-preview checks. Caption overflow/safe-zone check from font metrics before render plus a rendered-frame check. Face-visible ratio by re-running YuNet on the output. A vision-LLM review of output contact sheets using the BC yes/no rubric. |
| **Visual understanding index** (scenes, keyframes, OCR, audio events, faces) | HY transcript-labelled tiles; BC single-decode scene and face pass; SC text-row detection | OCR: Tesseract or an ONNX OCR such as PaddleOCR/RapidOCR-style models via `ort`. Audio events (laughter, applause, music): YAMNet / PANNs / CLAP-class ONNX models. Image embeddings (CLIP/SigLIP ONNX) for "find the shot where…". Keyframe selection by scene and sharpness. |
| **TTS abstraction with word timings** | HY model ports (HypiHub only, no timestamps); OC edge-tts (AVOID) | OpenAI-compatible `/v1/audio/speech`; local Piper / Kokoro-class ONNX voices; timings by forced alignment of TTS output (reuse B3). |
| **SFX library and placement** | HY moment-anchored SFX (ideas only) | CC0/royalty-free sources with per-file provenance: Freesound CC0 filter, Sonniss GDC bundles, Kenney audio. Placement rules: whoosh on transitions, impact on emphasis words. |
| **Music library, energy and beat matching** | OC ducking filter; OC bgm mp3s are AVOID | Royalty-free libraries with a license manifest; onset/beat detection (aubio-class library or a Rust DSP crate); trim to a high-energy section; fades. |
| **Denoise and speech enhancement** | none (F7 = 1 everywhere) | FFmpeg `afftdn`, `arnndn` (RNNoise model files), `anlmdn`, `highpass`/`lowpass`, `speechnorm`; DeepFilterNet-class ONNX. |
| **Brief compliance** (campaign brief, per-clip fee, platform rules) | BC sponsor policy + clip request rule; SC creator instructions; OS per-platform descriptions | A brief schema (duration, aspect ratio, required mentions or CTA, banned content, watermark/credit, payment model) and a checker over transcript, OCR and output metadata. Platform specs for TikTok, Reels and Shorts (durations, safe zones). |
| **Agent self-install in cloud sandboxes** | HY `programs prepare`; Docker files elsewhere | Static FFmpeg sources (BtbN, gyan, johnvansickle, evermeet; check GPL/LGPL variants); whisper.cpp release binaries and CUDA variants; onnxruntime via `ort` download strategies; SHA-256 manifests; offline cache; no-root install; GPU/NVENC detection. |
| **Diarization without gated models** | AssemblyAI (paid); pyannote (gated) | sherpa-onnx speaker diarization / speaker-embedding ONNX models; diarization via an STT endpoint (verbose_json with speakers). |
| **Permissioned FS / network / download / publish** | BC loopback MCP and network guard; OS SSRF check | Capability manifest per project; a download allowlist; yt-dlp kept optional and gated with no DRM bypass; publish always manual. |
| **Variant compare / A-B** | none | Render a preview matrix (style × layout) in one command; side-by-side contact sheet; agent picks the best. |
| **Language-agnostic heuristics** | SC multilingual conjunctions (partial); others English or Indonesian | Per-language data packs: fillers, weak openers, sentence regexes. Language ID (OS uses py3langid; we need a Rust alternative such as `whatlang`). |
| **Rust building blocks** (named in reports, unverified) | n/a | `rmcp` (MCP), `ort` (ONNX), `ttf-parser`/`rustybuzz`/`ab_glyph` (metrics), `image`/`tiny-skia` (PNG overlays), `whisper-rs` or the whisper.cpp CLI, `schemars` (JSON Schema), `keyring`. |

---

## 5. Asset shortlist

**Safe to bundle.** Always fetch from the original upstream with the license text; never copy from the repos.

| Asset | License | Seen in | Note |
|---|---|---|---|
| Anton | OFL-1.1 | AC, BC, OS, SC | Impact caption face. AC verified the OFL metadata in the font file. |
| Montserrat (ExtraBold, Black, variable) | OFL-1.1 | BC, OS (OFL text shipped), SC | "Hormozi"-style presets |
| Archivo Black, Archivo | OFL-1.1 | BC, SC, AC | |
| Poppins (Black, ExtraBold) | OFL-1.1 | BC, SC | |
| Plus Jakarta Sans | OFL-1.1 | BC | |
| Instrument Serif (Regular, Italic) | OFL-1.1 | BC, AC | Editorial presets |
| Inter (variable) | OFL-1.1 | AC, SC | Libass handling of variable-font instances is UNVERIFIED (AC #7). Prefer static weights. |
| Bebas Neue, Oswald, League Spartan, Barlow Condensed, Bangers, DM Sans, Nunito Sans, Raleway, Rubik, Sora, Urbanist, Work Sans | OFL-1.1 | SC (`backend/fonts/SOURCES.md` lists Google Fonts URLs) | |
| Roboto, Open Sans | Apache-2.0 | SC | |
| Noto Serif, Lora, Geist | OFL-1.1 | OS, OC, BC | Upstream only. OS ships them without OFL text. |
| Noto Color Emoji | OFL (per SC report) | SC (installed from OS packages) | Run the libass emoji probe before use. |
| TikTok Sans | OFL according to SC SOURCES.md; inventory says **check** | SC | Verify upstream before bundling. |
| YuNet `face_detection_yunet_2023mar.onnx` | MIT, (c) 2020 Shiqi Yu | SC, BC (license files shipped) | Fetch from opencv_zoo and pin SHA-256. |
| Tabler Icons | MIT (per HY report) | HY (via Iconify) | For UI and stickers. |

**Models fetched by `setup`, not bundled.**

| Model | License | Status |
|---|---|---|
| whisper.cpp ggml Whisper weights | MIT (per OC/SC reports) | **check** the specific ggml conversion source |
| Parakeet-tdt-0.6b-v3 ONNX | CC-BY-4.0, attribution required | OS report, external knowledge; check |
| Silero VAD | MIT | OS report, external knowledge; check |
| TransNetV2 | MIT per OS report; the inventory lists the pip package as unverified | check; ONNX export needed |
| MediaPipe face models | Apache code; model terms unverified | Not needed if we use YuNet |

**Avoid.**

| Item | Why |
|---|---|
| ultralytics / YOLOv8 code and `yolov8n.pt`; `Bingsu/adetailer` face_yolov8 weights | AGPL-3.0 (lineage) |
| InsightFace `buffalo_l` / SCRFD packs | Non-commercial model terms |
| pyannote gated models as a **required** dependency | Gated Hugging Face terms. Optional and user-supplied only. |
| OC `assets/bgm/*.mp3` (15 files) | No license; likely Pixabay or third-party songs |
| HY `examples/*/assets` wav/m4a and footballer photos | Provenance unknown |
| SC `backend/transitions/*.mp4`, `assets/blender`, `THEBOLDFONT.ttf` | No license stated |
| OS demo media (Churchill/Queen GIFs, demo mp4s), `assets/watermark.png`, everything under `cloud/` | Third-party footage; commercial license |
| BC `OpusClipLogo.tsx`, `opus-clip.ts` pricing constants, BridgeClip brand art | Trademark |
| OC and VC YouTube-downloaded glitch / film-burn overlays; OC logos | Unknown rights |
| edge-tts, g4f, Google Translate scraping (deep_translator) | ToS risk / unofficial endpoints |
| Remotion | Company license above free thresholds |
| Jev `typesafe/jev-1.13` endpoint | Proprietary alpha; reproduce the rubric with any LLM |
| Repo copies of fonts (OS, AC: no OFL text) | Re-fetch from upstream with OFL text |

---

## 6. Draft feature list (build order; f01-f04 = **Slice 1**, the smallest end-to-end vertical slice)

**Slice 1 goal.** An agent runs `setup`, then `transcribe`, reads `units`, chooses a unit range itself, runs `render` (9:16 blur-fit + one caption preset + loudnorm), and gets a `qc` JSON report. This needs no LLM, no MCP and no face model.

- **f01 core-cli-and-project.** Rust workspace (clap/tokio/serde/tracing). JSON project dir with history, the versioned JSON envelope, error classes and exit codes, NDJSON progress, and a `doctor` command.
  - Sources: HY CLI contract (IDEA), BC failure table (COPY), AC artifact-per-stage (COPY).
  - Depends on: none.
- **f02 setup-and-probe.** `setup` downloads static FFmpeg/ffprobe and whisper.cpp into a tools dir with SHA-256 checks. It probes encoders with a 1-frame test and checks filters (`ass`) and the GPU, with CPU fallback. Includes `probe` for source files.
  - Sources: AC `system.py` (COPY), OS probes (COPY), §4 self-install research.
  - Depends on: f01.
- **f03 transcribe-and-units.** whisper.cpp or an OpenAI-compatible STT endpoint produces `transcript.json` with word times; no fabricated times. Then sentence/clause **units** with boundary strength.
  - Sources: BC STT contract (COPY), OS fallback rules (COPY), SC units/boundaries (IDEA).
  - Depends on: f01, f02.
- **f04 render-and-qc-v1.** Input is a keep-range EDL (unit ids → times with snap and padding). Output is a single-pass 9:16 render: blur-fit or centre crop, one ASS karaoke preset with a bundled OFL font, loudnorm. Then `qc`: ffprobe integrity, duration vs target, `ebur128`, `blackdetect`/`freezedetect`/`silencedetect`.
  - Sources: BC graph builder (COPY), OS ASS (COPY), AC boundaries (COPY).
  - Depends on: f03.
- **f05 perception-pack.** `frames`, `tiles --transcript --around`, filmstrip, scene list (FFmpeg scene score + HSV), audio events (RMS peaks, silences, loudness curve), and short preview clips. Together these make the per-source **understanding index**.
  - Sources: HY tiles (IDEA), BC single-decode scan (COPY), SC `astats` peaks (IDEA).
  - Depends on: f02, f03.
- **f06 mcp-server-and-skill.** `rmcp` stdio and loopback HTTP that mirror the CLI 1:1, with annotations, `structuredContent` and the BC hardening. Ships an agent skill: "Done means watched", the choices only the agent can make, a routing table.
  - Sources: OS MCP and skill (COPY), BC hardening (COPY), HY skill rules (IDEA).
  - Depends on: f01, f04, f05.
- **f07 boundary-and-pacing-engine.** `refine_cut`: sentence snap, duration clamp, silence alignment, weak-opener rewind. Keep-ranges with filler and pause compression, a TimeMap for captions, per-language word lists.
  - Sources: AC `boundaries.py` (COPY), BC `clip_editor.py` (COPY), SC `timeline.py` (IDEA).
  - Depends on: f03, f04.
- **f08 provider-layer.** Config for OpenAI-compatible LLM, vision, STT and TTS endpoints; the `response_format` ladder; JSON recovery; keyring credentials; an async `Endpoint` trait with receipts, retries and cost.
  - Sources: AC/OS adapters (COPY), VC/OC recovery (IDEA/COPY), HY endpoint protocol (IDEA).
  - Depends on: f01.
- **f09 highlight-candidates.** Optional auto-pick: windows → score-all → shortlist → detail → snap → dedupe → rerank. Deterministic signal summary; visual-only path for silent footage. Each stage is a tool the agent can override.
  - Sources: OS two-pass and prompts (COPY), SC rerank and signals (IDEA), BC count curve (COPY).
  - Depends on: f05, f07, f08.
- **f10 face-analysis-and-reframe.** YuNet via `ort` in a single decode; tracks; One-Euro camera; CropPath JSON; `sendcmd` emission; layouts: track, fit, split, screencast, inset. Layer model and don't-crop reasons.
  - Sources: BC analyzer (COPY), AC smoothing/croppath (COPY), OS layouts (COPY), SC context (IDEA).
  - Depends on: f04, f05.
- **f11 caption-engine-v2.** Data-driven caption template schema; layered ASS; real font-metric line fitting; emphasis words and emoji; face-aware placement with hysteresis; SRT/VTT sidecars; libass-based preview frames.
  - Sources: BC captions and placer (COPY), HY style vocabulary (IDEA), SC metrics and emoji (IDEA).
  - Depends on: f04, f10.
- **f12 edit-ops-and-history.** Typed ops: keep/cut ranges, remove words/pauses, set style, set crop/layout per shot, edit caption words, move captions. Revision-checked saves, undo, word-anchored event projection on recut.
  - Sources: SC AI-edit ops (IDEA), BC EDL validator (COPY), HY word-anchored time (IDEA).
  - Depends on: f07, f10, f11.
- **f13 audio-post.** Two-pass loudnorm with ebur128 verification; sample-accurate mix graph; music bed with sidechain ducking and fades; denoise (`afftdn`/`arnndn`); SFX cue track anchored to words.
  - Sources: BC loudness (COPY), OC ducking (COPY), HY mix conventions (IDEA), §4 research.
  - Depends on: f04, f12.
- **f14 quality-loop-v2.** QC report covering caption safe-zone and overflow, face-visible ratio, hook in the first 3 s, A/V sync, and crop sanity via the debug overlay. Vision-LLM review of output contact sheets; BC-style yes/no judge with citation-required repairs feeding `fix` suggestions.
  - Sources: BC judge (COPY), OC dev overlay (COPY), HY "done means watched" (IDEA).
  - Depends on: f05, f08, f11.
- **f15 brief-and-variants.** Per-job brief schema (platform, duration, CTA and mentions, banned content, payment model) and a compliance checker. Variant matrix renders with a compare sheet.
  - Sources: §4 gap; BC sponsor policy (COPY).
  - Depends on: f12, f14.
- **f16 asset-library.** Font, caption-preset, SFX, music, layout and template packs with a metadata, tags and **license manifest per file**. `setup` fetches from upstream; no unlicensed media.
  - Sources: §5 shortlist, BC notices (ASSET).
  - Depends on: f02, f11, f13.
- **f17 hw-accel-and-throughput.** NVENC/QSV/VideoToolbox encoder tiers, NVENC slot limits, sparse analysis decode, parallel clip renders, proxies. A later CUDA single-pass path modelled on osrender.
  - Sources: OS `ffmpeg_utils` and osrender (COPY), SC sparse seek (IDEA).
  - Depends on: f04, f10.
- **f18 speakers-and-active-speaker.** Diarization via an endpoint or optional local model; lower-face-motion ↔ speaker-turn correlation with an ambiguity fallback; speaker-coloured captions and speaker cuts.
  - Sources: SC speaker framing (IDEA), AC `speaker.py` (COPY), OS `active_speaker.py` (COPY).
  - Depends on: f03, f10.
- **f19 creative-extras.** Hook title and title-card overlays (grounded hooks, 1-D placement); frame-exact xfade transitions; allowlisted effects enum (punch-in, colour-pop); multi-clip workflows (series, recap, trailer); TTS voice-over.
  - Sources: OS hooks and effects (COPY), SC xfade and workflows (IDEA), OC story recipe (COPY).
  - Depends on: f08, f11, f12, f13.
- **f20 tauri-gui.** Dashboard, source browser, libass-parity preview, timeline with word-snapped trim, caption editor, asset browser, render queue, agent activity and approvals. Same core, no separate engine.
  - Sources: AC TrimBar and CaptionEditor ideas (COPY), BC editor (COPY), HY Studio write-back (IDEA).
  - Depends on: f06, f12, f16.
