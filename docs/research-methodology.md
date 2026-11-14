# Research Methodology — Repository Evaluation

Every repo in `repos/` is evaluated with the same method so findings are comparable.
One findings file per repo: `findings/<repo>.md`. Cross-repo synthesis: `findings/00-comparison-matrix.md`.

## Rules
- Read source code, not just READMEs. Every claim cites `repos/<repo>/<path>:<line>`.
- Mark anything not verified as **UNVERIFIED**. Never invent features.
- Judge implementation quality, not stars or marketing.
- Record exact license per file/asset where it differs from the repo license (fonts, music, models, vendored code).

## Capability categories (from the master prompt)
A. AI clipping (highlight/viral detection, hooks, scoring, duration, narrative)
B. Transcription & subtitles (word timestamps, STT engines, animated/karaoke captions, emoji, speakers)
C. Creative design (caption templates, fonts, typography, animations, gradients, motion, transitions)
D. Video processing & rendering (FFmpeg/FFprobe usage, composition, encoding, HW accel, batching, previews)
E. Reframing (face/speaker/object tracking, dynamic crop, multi-speaker layouts, 9:16)
F. Audio (denoise, silence removal, loudness norm, music, SFX, ducking)
G. Agent integration (CLI, MCP, skills, JSON I/O, tool schemas, state, handoff)
H. GUI (timeline, preview, subtitle editor, templates, assets, render UI)
I. Provider abstraction (LLM / STT / TTS endpoints — OpenAI-compatible, local models, pluggability)

## Per-repo findings file structure
1. Summary (3–5 lines: what it is, verdict)
2. Stack & architecture (languages, frameworks, services, how modules talk; small diagram if useful)
3. Repo map (important dirs/files only)
4. Capabilities by category A–I: what exists, how it works (algorithm), key files/functions, quality 1–5
5. Pipeline walkthrough (input → output, with real file:function references)
6. Prompts & heuristics (LLM prompts, scoring formulas, thresholds — quote the important ones)
7. FFmpeg commands/filtergraphs actually used (quote them)
8. Dependencies & external services (APIs, models, paid services, GPU needs)
9. Performance notes
10. Weaknesses / bugs / limitations
11. License & reuse notes (repo license, asset licenses, copyleft deps)
12. **Steal list** — ranked: exact things worth adopting into our Rust system, and how (port / reimplement / call as sidecar / use asset)
13. Gaps — what this repo doesn't do that we need

## Scoring scale (used in the comparison matrix)
1 = absent/broken · 2 = naive · 3 = workable · 4 = solid · 5 = best-in-class reference
