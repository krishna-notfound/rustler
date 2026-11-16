# Muse live test (2026-11-15) — feature list and how v2 handles it

Source: Muse (cloud agent) ran Rustler on "Why Your Communication Makes People Ignore You" (Vinh Giang × Raj Shamani,
2h22m). Result: 3 clips, all QC-passed, 1080×1920. Measured on Muse's CPU-only VM: YouTube download ~1 MB/s (2.5 min
of video took ~5 min), render ~1 min/clip, URL → 3 clips ≈ 25 min, build from source 13 min, install ~5.5 min.
Muse's design rule: the separate commands stay (they give the agent full control); automation sits on top as an
optional wrapper.

| # | Muse item | Status in v1 (2026-11-16) | v2 design section |
|---|---|---|---|
| 1 | Active-speaker reframing default ON | `crop track-speaker` exists (opt-in, heavy) | 5.5(2) light active speaker default, 5.10 smooth camera |
| 2 | Smart default layout | manual | 5.5(1) smart defaults |
| 3 | Faster downloads: 720p + audio-first | captions-first exists; 1080p default | 5.7 720p default, `--audio-only` |
| 4 | `clips auto` one command | — | 5.7 optional wrapper |
| 5 | Burned-in subtitle detection | OCR opt-in only | 5.5(4) light text-band detection, auto position; masks (5.1) |
| 6 | Auto silence removal | `edit remove-pauses` manual | 5.5(1) pause trim default |
| 7 | Parallel batch rendering | `render batch` (parallel default 1) | 5.7 default 2 + ETA |
| 8 | Progress + ETA, clean stdout | progress events, stdout is one JSON | 5.7 `eta_s` in progress |
| 9 | More hook styles + auto-suggest + progress bar on | 1 hook style; progress bar opt-in | 5.5(3) hook presets; 5.9 hook lab (agent writes) |
| 10 | Music search + auto-ducking | ducking exists; bring-your-own music | 5.3 template music rules; asset tags |
| 11 | Caption auto-position | QC check only | 5.5(4) auto position |
| 12 | Stock B-roll + font import | `asset add` (file/URL) | agent fetches media itself; 5.2 assets; cutaways 5.1 |
| 13 | Setup self-fixes SSL/proxy + JS runtime | manual | 5.7 |
| 14 | Multi-source clips | exists (`clip add-segment --source-id`) | timeline v2 keeps it |
| 15 | Punch-in on emphasis words | time-based only | 5.5(5) `effect.add --words` |
| 16 | Auto filler removal | small list | 5.5(1) extended EN + Hinglish lists |
| 17 | Auto title + hashtags + description | template text only | 5.6 posting JSON, agent writes via skill |
| 18 | Emoji captions | — | 5.5(4) emoji-karaoke |
| 19 | Cover-frame picker | — | 5.5(6) |
| 20 | Auto B-roll cutaway | — | 5.5(5) weak-visual detector + cutaways |
| 21 | Highlight ranking without import | exists (`highlights candidates url_…`) | discovery fixed in guides; 5.9 director |
| 22 | Publish to Shorts/Reels/TikTok | out of scope (user: never auto-publish) | non-goal |
| 23 | Prebuilt binary default | v0.1.1 released | per-phase releases |
