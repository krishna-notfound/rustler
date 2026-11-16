# Rustler v2 — Open Timeline, Agent Templates, OpusClip-Level Quality — Design

Status: **Approach A approved by user 2026-11-16 (proceeding per user: analyse → plan → build)** · Date: 2026-11-16
Inputs: Muse's live-test feature list (2026-11-15, 23 items, in the conversation), the user's new requirements
(2026-11-16), `docs/research/opusclip.md` (OpusClip analysis + 44-row gap table), the 5-clip trial report
(clips-trial-1, `docs/progress-log.md`), and v1 (`2026-11-15-rustler-design.md`, 119 commands).

## 1. Intent — the why

The user clips for money (campaigns + per-clip work) and now runs Rustler through **Muse** (cloud agent) while
travelling. The first real test produced QC-passing clips, but below **OpusClip quality**, slower than wanted, and
with the agent fighting the tool in places (layout by hand, framing fixes by hand, no way to hide source pop-ups,
hook text too small). The user wants:

1. **OpusClip-level clips** from any long video.
2. **The agent in full control**: it sees and edits *everything* on the timeline, makes many clips, and is never
   boxed in by fixed options.
3. **Agent-made assets and templates**: the agent designs its own caption styles, title cards, layouts and full
   looks ("templates"), saves them, and reuses/improves them on future jobs.
4. **Learning from example videos**: given a folder of videos, the agent studies them (it sees frames like a vision
   model; Rustler measures what eyes can't count) and turns the style into a template. A folder can also be raw
   footage to clip.
5. **A posting JSON per clip**: title, description, hashtags written by the agent, plus campaign requirements and
   whether the clip meets them.
6. Muse's practical fixes: faster downloads, works behind proxies, batch + ETA, prebuilt install.

Rustler still never calls AI APIs: anything needing judgement (moments, hooks, titles, style) is done by the agent,
guided by knowledge-book skills; Rustler supplies evidence, structure, rules, rendering and checks.

## 2. Goals, non-goals, anti-goals

**Goals**
- G1 A complete, documented **timeline document** per clip that the agent can read whole and edit by patch or
  replace; every existing editing command becomes an operation on it.
- G2 New layer types: **image** (logo, sticker, title card, any PNG/JPG/GIF), **video** (B-roll cutaway or
  picture-in-picture from any imported source/file), **mask** (blur/box to hide source pop-ups, logos, burned-in
  subtitles), and **per-word caption styling** (keyword colour, size bump, emoji).
- G3 **Templates** (brand kit + recipe) authored by the agent: save from a clip or write as JSON, apply to any clip,
  version, list, reuse across projects.
- G4 **Agent-generated graphics**: render title cards, lower thirds, badges, CTA cards from a spec (text, fonts,
  colours, shapes, gradient, rounded box, stroke, shadow) into PNG assets the agent then places.
- G5 **Style references**: analyse a folder of example clips into a measurable style profile + contact sheets.
- G6 **Posting JSON** per exported clip with agent-written copy and campaign compliance; QC requires it.
- G7 OpusClip parity on quality: smart defaults at clip creation, better framing (look-room, active speaker, per-
  scene layout), bigger animated hooks, keyword/emoji captions, captions that avoid faces and burned-in subtitles,
  pacing tools (flash/whip transitions, punch-in on words, B-roll cutaways), filler/pause/bad-take cleanup, cover
  frame, stored virality score.
- G8 Speed/ops: 720p default + `--quality`, audio-only import, proxy/SSL + JS-runtime self-fix, parallel batch with
  ETA, optional `clips auto` wrapper.
- G9 GUI shows and edits the new layers, templates, references and posting JSON (same commands).

**Non-goals**: publishing/scheduling/auto-posting (manual by the user — campaign rules, account safety);
calling stock-footage or AI APIs (the agent fetches media with its own tools and imports it); a bundled music
library (bring-your-own; opt-in CC0 pack stays); XML export to Premiere/DaVinci (backlog).

**Anti-goals**: automation that hides decisions from the agent (every automatic default is visible in the timeline
and overridable); breaking v1 commands or project files (v1 projects migrate automatically); heavy models turned on
by default (lite stays lite).

## 3. Constraints
- Rust core + Python engine + FFmpeg/yt-dlp, as v1. One encode per deliverable. Frozen v1 contracts may be
  **extended** (new optional fields) but not broken; the timeline v2 schema is a new versioned contract.
- Lite profile must stay small; anything new and heavy is opt-in (`--with`).
- Windows + Linux. PowerShell 5.1 JSON quoting caveat: every JSON-taking command accepts `--input-json @file`.
- Model tiering for the build (user rule): Opus for contracts/render/e2e, Sonnet for feature crates, Haiku for docs.

## 4. Approaches considered (user picks; A recommended)
| | Approach | Trade-off |
|---|---|---|
| **A** | **Foundation first** (open timeline + layers + templates + references + posting JSON), then OpusClip quality, then ops/automation; quick ops wins (720p, proxy fix) up front | Matches "agent has full control"; every quality feature becomes a template item. First visible quality gains after ~1–2 days of foundation work |
| B | Automation first (`clips auto` cloning OpusClip's pipeline) | Fast one-command output, but rule-based picks are weaker than the agent's judgement and it pulls against agent control |
| C | Quick quality fixes on v1 structure, foundation later | Earliest visible gains, but templates/open timeline get bolted on later → rework |

## 5. Design (approach A)

### 5.1 Timeline v2 — the single source of truth per clip
`clips/<id>/timeline.json` (schema `rustler.timeline@2`), replacing clip.json + extras.json (v1 files migrate on
first open; v1 commands keep working as operations on v2):
```
{ schema, id, name, revision, canvas{w,h,fps}, template_ref?{id,version},
  tracks: {
    video:     [segment{id, source_id, src_in, src_out, layout{regions[...], background}, crop_path?, speed?}],
    cutaways:  [item{id, out_start, out_end, media{source_id|asset|file, in}, mode: full|pip, rect?, fit, transition_in/out?}],
    images:    [item{id, out_start, out_end, asset, rect, scale, opacity, rotation, anim{in,out,kind}, z}],
    masks:     [item{id, out_start, out_end, rect, kind: blur|box|pixelate, strength, color?}],
    text:      [item{id, out_start, out_end, text, style_ref|style{…}, rect|position, anim, z}],
    captions:  { enabled, style_ref|style{…}, position_y|auto, words_per_line, script,
                 word_overrides: [{word_anchor{source_id,word_i}, color?, scale?, emoji?, hide?}] },
    audio:     { source_gain_db, segment_gain{seg_id:db}, denoise, music?{…, duck}, sfx:[{anchor|out_t, asset|file, gain}] },
    effects:   [ {id, kind: punch_in|zoom_slow|shake|flash|whip, out_start, out_end | word_anchor, amount} ],
    transitions: [ {after_segment, kind, duration} ],
    progress_bar?: {…}
  },
  meta: { score?{total, hook, flow, value, trend, by:"agent"}, post?{…see 5.6}, cover?{t, path} } }
```
Commands:
- `timeline.show {clip}` → the whole document (resolved times for word-anchored items, warnings).
- `timeline.schema {}` → JSON Schema (agents validate before applying).
- `timeline.apply {clip, base_revision, patch: RFC 6902 JSON-Patch | document}` → validated (rects 0–1, times in
  range, assets exist, anchors resolve), saved as a new revision with history/undo, returns the new document +
  diff summary. `--dry-run` validates only.
- Existing commands (edit.*, layout.*, overlay.*, audio.*, effect.*, transition.*, caption.*, crop.*) become typed
  helpers that build a patch and call the same apply path (one validation + history code path).
- Render compiles the v2 document (new layers composited in the same single graph; masks before captions; image/text
  layers by z; cutaways replace or PiP over the video track).

### 5.2 Agent-made assets and graphics
- `asset.add` (exists: file/URL/source, license optional) for media the agent fetched itself.
- **`asset.render_card {spec}`** → PNG asset from a JSON design spec: canvas size, background (solid/gradient/
  transparent), rounded rectangles, strokes, shadows, text blocks (font asset or installed family, size, weight,
  colour, outline, alignment, line wrap), emoji (Noto Color Emoji when available), images. Used for title cards,
  lower thirds, CTA cards, badges, logo plates. Preview returned as image for vision agents.
- **`caption.create_style`** (exists) gains keyword/emoji fields and animation kinds (pop, bounce, fade, typewriter).
- **`text.create_style`** for hook/text layers (box, two-line, big, animated in/out, sfx hook).
- All agent assets carry tags + optional license + `created_by: agent` and live in project or global scope.

### 5.3 Templates (brand kit + recipe)
`templates/<name>@<version>.json` (global `~/.rustler/templates/` or project):
```
{ name, version, description, created_by, based_on?{reference_set?, clip?},
  canvas, layout_rules:[{when: faces=1|faces>=2|no_face|screen|graphics, layout preset|regions, track: face|speaker|none, look_room}],
  captions: style_ref|style, words_per_line, position: auto|y, keyword_highlight{color, max_per_line}, emoji: on|off,
  hook: {text_style_ref, duration, sfx?, punch_in?}, cutaway_rules?{on_weak_visual_after_s, kind: punch_in|asset_tag},
  cleanup: {pauses_over_s, fillers: lang lists, bad_takes: on|off},
  effects: {punch_in_on_emphasis: on|off, flash_at_cuts: on|off}, transitions: default kind/duration,
  audio: {music: tag/mood|asset|none, music_db, duck, denoise}, progress_bar?, brand: {logo_asset?, rect, opacity, cta_card?, outro_card?},
  export: {quality, fps, max_len, min_len}, post_defaults: {hashtags_always?, mentions?, cta_line?} }
```
Commands: `template.create {name, from_clip? | json}`, `template.show/list/delete`, `template.update` (new version),
`template.apply {clip|clips|all, name[@ver], overrides?}` (writes the corresponding timeline items; everything stays
editable), `template.diff {a,b}`. `clip.create --template name` applies at creation.

### 5.4 Style references (learn from a folder of example videos)
- `reference.import {folder|files, name}` → a reference set (files referenced, proxies + audio extracted).
- `reference.analyze {set}` (light; no heavy models by default) → per video + aggregate **style profile**:
  duration, cuts/min and shot lengths (scene detect with adaptive threshold), caption band position/size and
  words-per-line estimate (text-band detection by edge density; OCR when the `ocr` extra is installed), dominant
  caption colours, hook text present in first 3 s, zoom/punch frequency (frame-scale change detection), face count/
  layout distribution (YuNet), loudness, music presence, average speech rate (transcript when available), plus
  labelled contact sheets and first-3-second filmstrips **for the agent to look at**.
- The agent combines the profile + what it sees into a template (`template.create … based_on {reference_set}`).
- A guide + prompt `learn_style_from_folder` teaches this.

### 5.5 Quality features (OpusClip parity), each a template-controllable default
1. **Smart defaults at `clip.create`** (visible + overridable; `--raw` disables): per-shot layout from face/shot
   analysis (faces≥1 → crop + track; graphics/wide-no-face → blur-fit or zoom-fit), pause trim > 1.0 s, filler
   removal (EN + Hinglish lists, extendable), bad-take/repeat detection (repeated n-grams/restarts).
2. **Framing**: look-room (shift crop toward gaze side using landmark/face-box asymmetry), lighter smoothing and
   faster lean response (tunable), **light active speaker** by default (mouth motion among visible faces; no
   diarization model), per-scene layout plan (`layout.plan {clip}` proposes, agent accepts), better scene-cut
   detection in dark footage (adaptive + histogram + luma-normalised).
3. **Hooks**: animated hook presets (big two-line, pop/zoom-in, with optional whoosh), 5+ text styles; agent writes
   text using the hooks guide.
4. **Captions**: keyword highlight (agent marks words or `caption.keywords auto` proposes from emphasis lists),
   emoji per word (agent picks; `emoji-karaoke` style), auto position avoiding faces and burned-in subtitle bands
   (text-band detection), larger default sizes for mobile (trial showed 45 px too small).
5. **Pacing**: flash-black and whip transitions, punch-in on chosen words (`effect.add --words`), B-roll cutaways
   from agent-supplied media, "weak visual" detector suggesting where to cut away.
6. **Cover frame**: `clip.cover {clip, auto|t}` → sharp, face visible, mouth open, not mid-blink.
7. **Score**: `clip.score set` stores the agent's 4-part score (hook/flow/value/trend); `highlights.candidates`
   evidence feeds it; export orders by score.

### 5.6 Posting JSON + campaigns
- `campaign.create/show/list {id, brief fields…}` (extends v1 brief: multiple campaigns per project; brief.* stays).
- `clip.meta_context {clip}` → transcript of the clip, hook, campaign rules, platform limits (title length, hashtag
  count, description length) — what the agent needs to write copy.
- `clip.meta_set {clip, platform, title, description, hashtags, mentions, campaign_id}`.
- `export.clip` writes `<name>.post.json`:
  `{clip, file, duration, platform, title, description, hashtags, mentions, cover, campaign{id, requirements[],
  compliance{passed, rules[]}}, score, created_at}`; QC rule `meta_present` + `campaign_compliant`.
- Guide + prompt `titles_hooks_hashtags` (formulas, limits per platform, Hinglish copy, avoid clickbait the clip
  doesn't pay off).

### 5.7 Speed / ops (Muse's list)
720p default + `--quality 480|720|1080|best`; `source.import --audio-only` (captions-less long videos: find moments
from audio + whisper, then fetch sections); setup detects proxy/self-signed CA (writes yt-dlp `--no-check-certificate`
only when the user opts in, or uses the system CA bundle) and ensures a JS runtime for YouTube; `render.batch`
default parallel 2 with ETA; progress events carry `eta_s`; `clips.auto {url|source, count, template}` optional
wrapper over existing commands (agent remains free to do any step itself).

### 5.8 GUI
Timeline shows the new tracks (cutaways, images, masks, text, word overrides); canvas editor drags image/mask/
text rects; template browser (apply, diff, versions); reference viewer (profile + sheets); post JSON editor per clip;
campaign page. All through the same commands.


### 5.9 Director layer — retention-first editing (user priority, 2026-11-16)
The user's words: *"it's the game of views — if we can't make someone hold and watch our reel, we won't go viral."*
The agent acts as a **director**: it reads the campaign, decides what will hold attention, and uses every tool to
keep viewers watching. Rustler supplies evidence, measurements and a craft playbook; the agent makes the calls.
- **Campaign analysis first**: `campaign.analyze {campaign}` returns the brief's hard rules (must/must-not, durations,
  credits, tags), the audience/platform, and the payout model (per view → optimise retention + volume; per clip →
  match the creator's style exactly). Prompt `direct_campaign` walks the agent from brief → moment strategy → template.
- **Retention map**: `clip.retention {clip}` (deterministic, light) returns a per-second timeline of what holds or
  loses attention: first-frame hook (text/face/motion in frame 0), speech start delay, seconds since last visual
  change (cut, zoom, layout change, B-roll, text pop — flag gaps > 2.5 s), caption density, silence, energy curve
  (loudness/speech rate), open-loop/payoff markers the agent set, and ending type (hard stop / loopable / CTA). Output
  includes a **retention score** and "dead zones" with suggested fixes (punch-in here, cut pause, cutaway, text pop).
- **Director's playbook** (knowledge guides + prompts, versioned): hook formulas (first 1 s: bold claim, question,
  pattern interrupt, "POV", number, controversy, result-first), the 3-second rule, open loops and payoff, pacing
  rhythm (visual change every 2–3 s), emotional arc, loop endings for replays, Hinglish hook writing, music-energy
  matching, what kills retention. Plus a **hook lab** prompt: write 5 hook variants (text overlay + first line + visual
  cold-open choice), pick the strongest by the rubric, optionally render variants for A/B.
- **Cold open**: allow a clip to start with its strongest line/moment (out of order) then cut back — `edit.cold_open
  {clip, from_unit, to_unit}` builds it on the timeline.
- **Score before export**: the agent records its 4-part score (hook/flow/value/trend) + the retention score; export
  orders clips by score; the posting JSON includes both.
- **Volume**: the director plans N clips per source (OpusClip-like counts by length), diversified by topic, and uses
  templates + `render.batch` to ship them all.

### 5.10 Smooth camera (user priority)
Camera motion must look like a camera operator, never jittery or robotic:
- Offline **trajectory planning** over the whole segment (we know the future): critically-damped / spline smoothing
  with ease-in/ease-out, max velocity and max acceleration limits, a dead zone so small head moves never move the
  frame, and **anticipation** (start moving slightly before a lean finishes).
- **Hold-or-move policy**: prefer locked shots; move only when the subject would leave the safe box; switch speakers
  with a smooth pan (≤ 0.6 s) or a cut on a sentence boundary (agent/template choice).
- **Look-room**: frame toward the side the speaker faces.
- **Intentional moves**: slow push-in (zoom 1.0→1.08 over a sentence) for emphasis, punch-in on keywords, all as
  timeline effects the agent can edit.
- Measured by a **motion QC check**: max jerk/velocity, number of moves per minute, jitter score; contact strip of the
  crop path for the agent to look at.

## 6. Build order (phases; detail in the plan)
- **Phase 0 (quick wins):** 720p default + `--quality`, proxy/SSL + JS-runtime check in setup, batch parallel 2 +
  ETA, bigger default caption size, `effect.add --words`.
- **Phase 1 (foundation):** timeline v2 + migration + `timeline.show/schema/apply`; image/video/mask/text layers in
  the compiler; per-word caption overrides; `asset.render_card`; templates; reference sets + analysis; posting JSON +
  campaigns.
- **Phase 2 (quality + director):** director layer (campaign.analyze, clip.retention + retention score, playbook
  guides/prompts, hook lab, cold open), smooth camera (trajectory planning, hold-or-move, motion QC), smart defaults,
  framing (look-room, light active speaker, per-scene plan, dark scene cuts),
  hook presets, keyword/emoji captions + auto position + burned-in band detection, pacing tools, cover frame, score.
- **Phase 3 (ops + GUI + skills):** `clips.auto`, audio-only import, GUI for all new parts, knowledge book +
  SKILL.md/AGENTS.md/MUSE.md updated, release v0.2.0, a second real trial (same podcast) compared side by side with
  clips-trial-1.

## 7. Decided vs left open
**From the user:** agent full control; agent-made assets/templates; learn from video folders; posting JSON with
campaign requirements; OpusClip quality target; no AI APIs; no auto-publish; lightweight default; Muse operates it.
**Claude's calls (overridable):** approach A; timeline v2 as one JSON doc with JSON-Patch; templates = brand kit +
recipe in one file; references analysed without heavy models by default; publishing/stock APIs/XML export out of
scope; smart defaults on by default with `--raw` to disable; 720p default.

**Builder-check answers (Claude's calls, 2026-11-16; overridable):**
- *v1 compatibility*: `timeline.json` is the source of truth; `clip.json` keeps being written as a compatibility
  mirror; `clip.show`/`clip.list` keep their current shape (plus a `timeline` field); v1 projects and packs migrate on
  open; undo/history covers every layer (snapshots of `timeline.json`). Older binaries may not open migrated projects.
- *Releases*: ship to Muse **per phase** (v0.1.2 after Phase 0, v0.2.0 after Phase 1, v0.3.0 after Phase 2, v0.4.0
  after Phase 3); every release keeps old projects working. No hard deadline given; Phase 0 first.
- *Campaign rules*: measurable rules are machine-checked; fuzzy rules ("creator visible in first 3 s") are checked by
  the agent and recorded as pass/fail **with a reason** (allowed for `campaign_compliant`). One `post.json` per clip
  with a `platforms{}` map for per-platform copy. Read by the user (and later any submit tool).
- *GUI*: for the user's laptop (local only); Muse stays headless (CLI/MCP). Remote GUI is out of scope.
- *Templates*: portable — `template.export/import` bundles the template with its fonts, styles and assets (zip).
  Rustler ships 2 starter templates (`podcast-viral`, `streamer-split`) built from the trial learnings.
- *Acceptance*: side-by-side against clips-trial-1 on the same podcast, judged by the user; Phase 2 priority:
  smooth framing → hooks → retention pacing → captions → B-roll/cleanup.
- *Canvas*: 9:16 (1080×1920) only in v2; other aspects backlog.
- *Muse's 23 items*: recorded in `docs/research/muse-live-test-2026-11-15.md` with their mapping to this design.

**Left to the builder:** internal module layout, exact schema field names (must be documented in `timeline.schema`),
detection thresholds (tuned on the trial footage), card renderer library (tiny-skia/ab_glyph vs ASS→PNG).
