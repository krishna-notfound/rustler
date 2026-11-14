---
id: platform-specs
title: "Platform specs: TikTok, Reels, Shorts"
summary: "Output format, durations, safe areas, clip lengths that perform, and how to write titles, descriptions and hashtags for the export report."
order: 11
tags: [platform, tiktok, reels, shorts, export, metadata]
---

# Platform specs

Platform rules change often. The brief and the platform's current help pages win over this table;
when a number matters for money (e.g. a minimum length for a monetisation programme), tell the user
to verify it.

## 1. Output format (all three)

Rustler's default render fits all three platforms:

| Property | Value |
|---|---|
| Resolution | 1080×1920 (9:16 vertical) |
| Frame rate | 30 fps |
| Video | H.264, yuv420p, `+faststart` |
| Audio | AAC 160 kbps, 48 kHz stereo, −14 LUFS, −2 dBTP |
| Container | .mp4 |

## 2. Durations (as of late 2025 — verify for money-critical rules)

| Platform | Max length (upload) | Notes |
|---|---|---|
| YouTube Shorts | up to 3 min | clips over 60 s may be treated differently for some features and claims; ≤ 60 s is safest |
| Instagram Reels | up to 3 min (longer uploads exist) | reach usually best under 90 s |
| TikTok | up to 10 min in-app (longer via upload) | creator monetisation programmes have required **> 60 s** videos — check the brief |

## 3. Lengths that tend to perform

| Content | Shorts | Reels | TikTok |
|---|---|---|---|
| Reaction / fail / clutch | 8-20 s | 8-20 s | 10-25 s |
| Hot take / quote | 15-30 s | 15-30 s | 20-40 s |
| Story | 30-55 s | 30-60 s | 40-75 s |
| Value / how-to | 25-50 s | 25-60 s | 30-70 s |

Completion rate drives distribution: a viewer finishing a 20 s clip beats a viewer leaving a 60 s
clip at 15 s. Let the moment decide the length; the table is a sanity check.

Campaign briefs often fix a range (e.g. "30-60 s", "min 15 s"). Respect it exactly; QC duration is
precise, the reviewer's is too.

## 4. Safe areas

Keep captions, faces and key action inside **x 0.05-0.87, y 0.08-0.78** of the frame (details in
`layouts-and-framing`). Bottom ~22 % = username/description/sound; right ~13 % = buttons; top ~8 % =
tabs/status bar. Shorts' subscribe button and Reels' audio label sit low; TikTok's description can
expand to several lines.

## 5. Platform habits

| | TikTok | Reels | Shorts |
|---|---|---|---|
| Hook style | raw, fast, personality | polished, aesthetic, relatable | clear value, curiosity, loops |
| Captions | almost always | almost always | almost always |
| Loops | strong: end that flows into the start helps rewatches | moderate | strong: Shorts replays loop automatically |
| Text in description | hashtags + short line | caption text + hashtags | title matters for search |

Loop trick (only when natural): end on a line that makes the opening line make sense again, so
rewatches feel seamless. Never cut a payoff short to force a loop.

## 6. Titles, descriptions, hashtags (for the export report)

You write these; Rustler stores them in your report/notes. The user posts them.

**Hook text / title (≤ 10 words, ≤ 60 characters):**

- Name the concrete thing in this clip, not the episode ("He quit Google for chai", not "Ep 12
  highlights").
- Patterns: open question ("Why do 90% of startups die here?"), hot take ("Degrees are useless now"),
  number shock ("₹40 lakh lost in one call"), story loop ("She said one sentence and he left"),
  POV ("POV: your manager reads your resignation").
- Match the speaker's energy; no clickbait the clip doesn't deliver; no invented scandal, quotes or
  claims (the title must be supported by what is said).
- Unique across clips from the same source.

**Description (1-2 sentences):** what happens + a reason to comment ("Would you have taken the deal?").
Credit the creator/source if the brief requires (many campaigns do: "@creator on YouTube").

**Hashtags (3-5):** 1-2 broad niche tags + 2-3 specific ones. No banned or misleading tags; no
#fyp-spam unless the brief asks. Required campaign tags go first, exactly as written in the brief.

**YouTube Shorts title** ≤ 100 characters; front-load the hook words. Instagram/TikTok: the first
line of the description acts as the title.

## 7. Report block per clip (template)

```
clip: cofounder-emptied-account.mp4  (exports/)
source: <url or file> · units #89-#92 · 0:16.4
score: 84 (hook 18, standalone 17, payoff 17, emotion 17, share 15)
layout: center-crop + track-face · captions: bold-karaoke 0.70, latin
qc: passed (loudness −14.2 LUFS) · contact sheet checked: face visible all tiles, captions clear
brief: 15-60 s ✓ · creator credit ✓ · required tag #brandX ✓
title: My co-founder emptied our account before launch
description: The night before launch… then this text arrived. What would you do?
hashtags: #startup #founder #entrepreneur #startupindia
notes: laugh at 0:13 kept; music: none
```
