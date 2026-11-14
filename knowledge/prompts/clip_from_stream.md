---
name: clip_from_stream
title: "Clip a livestream VOD (webcam + gameplay)"
description: "Scan a long stream, find the reaction moments, and build split-vertical clips with the webcam on top and gameplay below."
arguments:
  - name: url
    description: "URL of the stream VOD (YouTube, Twitch VOD or other yt-dlp supported public page)."
    required: true
  - name: streamer_layout
    description: "Optional hints about the stream layout, e.g. 'webcam bottom-right, game is Valorant, chat on the left'."
    required: false
  - name: count
    description: "Maximum number of clips (default 5)."
    required: false
---

Make up to **{{count}}** (empty = 5) stream clips from **{{url}}**. Layout hints from the user (may be empty):
{{streamer_layout}}

Streams are long and mostly uneventful; the money is in short bursts: clutch plays, fails, rage,
laughter, chat interactions, wild statements. Read `workflow-youtube-longform`,
`layouts-and-framing` §4 and `finding-hooks` if you haven't this session. Never upload anything.

## 1. Scan without downloading
1. `rustler project create --name <slug> --dir ./<slug>`
2. `rustler source info "{{url}}"` → duration, chapters, **heatmap**, captions.
   - Heatmap spikes (value ≥ 0.6, ≥ 2× median, ignore the first 2 %) are your best leads; the spike
     is usually the payoff, so plan `[spike − 90 s, spike + 30 s]`.
3. `rustler source fetch-transcript "{{url}}"` if captions exist. Search for reaction language:
   "no way", "bro", "what", "let's go", "oh my god", "chat", "clip that", laughter, swearing bursts.
4. Pick 4-10 windows of 2-5 min. No captions and no heatmap → import 2-3 broad windows and use
   `analyze audio` loudness peaks (screams, laughs) as the lead.

## 2. Import only the windows
`rustler source import "{{url}}" --sections "H:MM:SS-H:MM:SS" --sections …` → one source per section
(times restart near 0). Then `rustler analyze quick <src>` for each.

## 3. Find the moments
- Read `transcript get` around the loudness `peaks`; look at `see tiles <src> --start <a> --end <b>`
  to see the gameplay event and the face reaction.
- A stream moment needs: a visible event (gameplay) + an audible/visible reaction + under ~5 s of
  setup. Pure talk with no event is fine only if the line itself is a strong hook.
- Length: reactions 8-25 s; stories/rants 30-60 s.

## 4. Find the layout once per section
1. `rustler analyze deep <src>` → faces: is there a small, steady face (webcam)? audio events: reactions.
2. `rustler layout detect-webcam <src> --clip-id <clip>` → webcam rect, gameplay rect, split and a ready
   `set-layout` line. It can include a sliver of gameplay: check the preview and nudge with
   `layout edit-region <clip> top --nudge …`. "No small, consistent face" → measure by hand: `see frame` at two
   times, webcam box px → normalised `{x,y,w,h}` inset 2-3 %, `split ≈ 0.5625 / (1.778 × w/h)` clamped
   0.30-0.50 (smaller if the webcam is < 400 px wide), gameplay rect full height excluding the webcam.
3. Save it: `layout save-preset <name> <clip>`, reuse with `layout apply-preset <clip> <name>`.
4. No webcam (just chatting full-cam) → `center-crop`; menus/text-heavy screens → `blur-fit`.
The full single-clip recipe is the `streamer_split_clip` prompt.

## 5. Build each clip
```
rustler clip create --name <slug> --source-id <src> --start-unit <a> --end-unit <b> \
  --expect-text "<opening words>" --layout split-vertical \
  --layout-params '{"top":{…},"bottom":{…},"split":<s>}'
rustler clip set-captions <clip> --style bold-karaoke --position-y <split + 0.06> --max-words-per-line 2
rustler overlay add-text <clip> "<payoff in <= 7 words>" --start 0 --end 2.5      # hook title
rustler effect add <clip> punch-in --start <reaction t> --end <t + 0.5>           # optional
rustler render preview-frame <clip> --out-t 1.0
```
Check the preview: face fills the top, no overlay border, no duplicate webcam, gameplay readable,
captions between the face and the action. Busy bright game → `--style boxed-highlight`. Two people on screen → `analyze speakers` +
`caption speaker-colors`. Polish pass: `polish_clip`.
For no-speech moments use `--start/--end` taken only from Rustler output (peaks, scenes, tile labels).

## 6. Render, watch, fix, export
`render run` → `qc run` → open the contact sheet → `fix suggest` / fix (`quality-checklist`) → `export clip`
(or `render batch --all true` + `export batch --all true` for all the clips).
Keep the reaction tail: end 0.5-1.5 s after the peak of the reaction.

## 7. Report
Per clip: file, section + units, duration, why it works (event + reaction), layout rects used, QC +
contact-sheet observations, title (≤ 60 chars), description, 3-5 hashtags (game + streamer niche),
and any risk (copyrighted stream music, slurs, platform-sensitive content).
