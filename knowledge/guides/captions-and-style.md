---
id: captions-and-style
title: "Captions and style"
summary: "The four caption presets and when to use each, position_y per layout, words per line, Hinglish script choice, emphasis and readability checks."
order: 7
tags: [captions, subtitles, style, karaoke, typography]
---

# Captions and style

Most short-form viewers watch with sound off for the first seconds. Burned-in word-by-word captions
are expected on almost every talking clip. Rustler renders them with libass from word-level timings,
so they stay in sync after every re-cut.

## 1. The settings

```
rustler caption list-styles                         # preset names; `caption styles` adds custom ones with font and kind
rustler clip set-captions clp_X --style bold-karaoke --position-y 0.70 --max-words-per-line 3 --script latin
rustler clip set-captions clp_X --enabled false     # turn off (e.g. music-only clip)
```

| Field | Default | Meaning |
|---|---|---|
| `enabled` | true (auto false when the transcript is empty) | burn captions or not |
| `preset` | `bold-karaoke` | visual style |
| `position_y` | 0.70 | **bottom edge** of the caption block, as a fraction of canvas height from the top (0.70 → 1344 px) |
| `max_words_per_line` | 3 | words shown at once; a new chunk also starts after a > 0.6 s gap or at sentence end |
| `script` | `latin` | `latin` (romanised), `devanagari`, `as-transcribed` |

Captions are one line per chunk; the active word is highlighted (colour/box only, no size change, so
the line does not jitter).

## 2. Presets and when to use them

| Preset | Look | Best for |
|---|---|---|
| `bold-karaoke` | Anton (tall condensed), uppercase, white with thick black outline, active word yellow; 78 px at 1080 wide | Default. Podcasts, commentary, storytelling, streams. High energy, reads on any background. |
| `hormozi-pop` | Big bold words that pop in, strong highlight colour on the active/key word | Hype, motivation, business advice, punchy hot takes; short chunks (1-2 words). |
| `boxed-highlight` | Words on a solid box/pill, active word boxed in an accent colour | Busy or bright backgrounds (gameplay, outdoor, white walls) where outline text gets lost. |
| `clean-white` | Lighter, sentence case, white with soft shadow, minimal animation | Calm, educational, emotional or premium content; brands that dislike "loud" captions. |

| `keyword-highlight` | Poppins ExtraBold, plain white lines shown whole (no word-by-word colour); only **keywords** get their own colour and 1.15x size | Talks where 2-5 words per clip carry the point (numbers, money, strong verbs). Pair with `caption keywords`. |
| `emoji-karaoke` | Poppins ExtraBold white, spoken word pink; emoji you set per word sit after their word | Lifestyle, comedy, food, fitness; sparing emoji (1-4 per clip). |

Exact fonts/colours come from the preset data; read `caption list-styles` output once per session.
If the brief names a caption style, match it as closely as these presets allow and say so in the
report if none fits.

### Custom styles (asset library)

`rustler caption styles` lists the four built-ins **plus** saved custom styles (scope `project` or
`global`). Make one when the brief wants a look no preset gives:

```text
rustler caption create-style neon-pop --base hormozi-pop --active-color #00FFFF --outline-width 8
rustler caption create-style yellow-box --base boxed-highlight --box-color #FFD400 --primary #000000 --outline #FFFFFF
rustler asset show style-neon-pop      # renders the style on a gray 1080x1920 frame; LOOK at the PNG
rustler clip set-captions <clip_id> --style neon-pop
```

Options: `--base` (any built-in or custom style), `--font-family` (must be an installed or asset font, otherwise
a warning says libass will substitute), `--size-factor` (0.02-0.15 of canvas width), `--primary`, `--outline`,
`--outline-width`, `--shadow`, `--uppercase`, `--active-kind recolor|box|pop`, `--active-color`, `--box-color`,
`--scope project|global`. Colours are `#RRGGBB`. A custom style is used by name everywhere a built-in is
(`clip.create --caption-style`, `clip.set-captions`, `variant create`, `render`); built-in names cannot be
overwritten. Fonts: add one with `rustler asset add Font.ttf --kind font --license OFL-1.1`
(only ttf/otf; the library's fonts are staged for every render). `rustler asset search <words>` finds assets.

Decision rules:

1. Brief specifies a style → follow it.
2. Bright/busy background behind the caption area (check a preview frame) → `boxed-highlight`.
3. Fast, loud, motivational → `hormozi-pop` with `--max-words-per-line 2`.
4. Slow, emotional, educational → `clean-white` with 4-5 words per line.
5. Otherwise → `bold-karaoke`, 3 words per line.

## 3. Where to put captions (`position_y`)

Rules in order: never over the face (especially the mouth/eyes), never in platform UI (bottom 22 %,
top 8 %, right 13 %), as close to the middle of attention as possible.

| Layout | Start with | Why |
|---|---|---|
| `center-crop` talking head | 0.70 (range 0.66-0.76) | below chin, above platform text |
| Face low in frame (chest-up, face at 50-60 %) | 0.78 or move above the head at 0.22 | keep clear of the mouth |
| `blur-fit` from 16:9 | 0.78 (just below the video band, which ends at 0.658) | captions on blur, picture untouched |
| `split-vertical` | `split + 0.06` (straddling the seam), e.g. 0.50 for split 0.44 | between face and gameplay centre; hides the hard seam |
| `split-vertical`, action in upper gameplay | 0.74-0.76 | lower gameplay is usually less important |
| Source has burned-in subtitles | `caption place` (moves ours off that band), or `blur-fit` and put ours below, or `mask add` over theirs | never stack two caption tracks |

Never set `position_y` above 0.78 (its bottom edge would sit in the platform's caption/username
area) or below ~0.18 (top UI). Two-line chunks rise about 0.05 above `position_y`.

Check with `rustler render preview-frame clp_X --out-t <t>` at a moment where the face is lowest or
largest, before a full render.

### Let Rustler measure it: `caption place`

```
rustler caption place clp_X                 # measure + suggest, changes nothing
rustler caption place clp_X --apply true    # set the suggestion (one revision; history undo reverts it)
rustler caption place clp_X --per-segment true --apply true   # keep the clip-wide spot, move only the shots it does not suit
```

Per segment it maps onto the 9:16 canvas (through the layout, fit and crop path): the source's own
**burned-in text** (an edge-density scan of the source range every 0.5 s, `subtitles` = text that keeps
changing, `static` = title/logo/watermark; plus the `analyze ocr` caption band when `ocr.json` exists) and the
**faces** (`analyze faces` data, else the engine's face detector on the segment). Then it scores every
`position_y` from 0.10 to 0.85 and suggests the closest one to the current spot that clears subtitles and
faces, weighted by segment length. Read `segments[].at_current` (what the current spot covers),
`at_suggested`, `safe_ranges` and `best_y`; `remaining_conflicts` lists what the suggestion still covers.
Then **look**: `render preview-frame` at the segment with source text. If the source subtitles sit where ours
must go, hide them instead with `mask add` (a blur box on the canvas band listed in `source_text`). The scan
reads no text; a busy shelf or table edge can show up as `static` text, so trust the frame over the number.

### A segment's own caption position

When shots need different spots (a wide stage shot with the source's subtitles under the middle, then
close-ups where 0.70 lands on the mouth), give a segment its own position. The clip-wide `position_y` stays
for every other segment.

```
rustler caption set clp_X --segment-id seg_S --position-y 0.19              # one undoable revision
rustler caption set clp_X --segment-id seg_S --clear-segment-position true  # back to the clip-wide spot
```

`caption place` suggests these for you:

- `suggested_segment_positions`: when no single position clears every segment, the segments the clip-wide
  suggestion still leaves on subtitles or a face, with their own best spot. `--apply true` sets the clip-wide
  suggestion and these together.
- `per_segment_positions` (`--per-segment true`): keep the current clip-wide spot and move only the segments
  it does not suit. Use it when one shot type is the exception; prefer one clip-wide spot when it clears
  everything, because captions that jump between shots are harder to follow.

Either `--apply` replaces earlier per-segment positions and is one revision (`history undo` reverts both the
clip-wide and the per-segment change). `segments[].current_y` shows the position each segment uses now.
Words take the position of the segment they are spoken in, and a caption line never spans two positions
(it is split at the cut). The positions are stored in `extras.json` `caption_positions`
(`{segment_id: position_y}`, shown in `timeline show` under `tracks.extra`). When `edit split`,
`remove-pauses` or `remove-words` re-cut a segment, the new pieces keep its position; when the segment is
deleted the position is dropped with a warning (`history undo` brings it back).

QC repeats the subtitle part as `caption_over_source_text` (warn when our caption band overlaps, or sits within 0.03 of, the
source's burned-in subtitles for 0.5 s or more; fix hint `caption place`). It, `caption_over_face` and
`caption_safe_zone` judge each segment at its own position; when the covered face is in a segment with its
own position, the hint names that segment (`caption set ... --segment-id`).

## 4. Words per line

| Pace / style | Words per line |
|---|---|
| Hype, hot takes, gaming reactions | 1-2 |
| Normal podcast / commentary (default) | 3 |
| Calm explainer, emotional story | 4-5 |

Keep each chunk under ~20 characters for uppercase presets so it fits one line at 1080 px. Long
Hinglish or German-style words: lower the word count. A chunk should be on screen ≥ 0.3 s; very fast
speakers need 2-3 words per line, not more.

## 5. Script for Hindi/Hinglish speech

- `latin` (default): romanised Hinglish ("bhai kya scene hai"). This is how most Indian short-form
  audiences type and read Hinglish. Use it for mixed Hindi-English content.
- `devanagari`: for Hindi-dominant audiences or when the brief asks. Needs the Devanagari font; check
  the preview for empty boxes (□), which mean a missing glyph.
- `as-transcribed`: keep whatever Whisper produced (mixed scripts). Rarely right for publishing.

See `hinglish` for transcription settings and spelling tips.

## 6. Emphasis

Good emphasis words: numbers and money ("₹40 lakh", "97%"), strong verbs ("fired", "quit"), names,
surprising nouns. 2-5 per clip, only words actually spoken. Never articles, pronouns or fillers.
The active-word highlight is automatic; per-word emphasis colours or emoji are available only if a
caption edit command exists (`rustler commands list` → `caption.*`). Don't add emoji unless the
brief or niche calls for it.

### Keywords (`keyword-highlight`) and emoji (`emoji-karaoke`)

```
rustler caption set clp_X --style keyword-highlight
rustler caption keywords clp_X --auto --dry-run          # what the rules would mark: numbers and number words (three, hundred, paanch), emphasis words (EN + Hinglish), names
rustler caption keywords clp_X --auto                    # mark them (about one word in six at most; `--max N` to change)
rustler caption keywords clp_X --words million,quit      # or name the words yourself (every occurrence, case/punctuation ignored)
rustler caption words-list clp_X                         # review; `caption word` changes one word, `caption words-clear` removes marks
rustler caption word clp_X --source-id src_X --word-i 42 --emoji "🔥"   # emoji after a word (any preset; emoji-karaoke is made for it)
```

- A keyword is a colour on that word. Default colour: green #2BFF5E, or yellow #FFD60A when the caption style's
  spoken-word colour is green (hormozi-pop), so a keyword never looks like the karaoke highlight; `keyword-highlight`
  (no spoken-word colour) keeps its own accent. `--color #RRGGBB` overrides. In `keyword-highlight` it also gets
  1.15x size unless you set a size with `caption word --scale`. Your own colours set earlier are never overwritten.
- `auto` names only Title Case words inside a sentence: the first word of a sentence or of a speaker turn (`>>`) is never a
  name, nor is a word the clip also writes in lower case ("Three." / "three"). Proper adjectives ("Western") still count.
- `auto` is a proposal, not a judgement: read the list, drop what is not the point, add what is. Marks follow re-cuts;
  a marked word that was cut out is skipped with a warning.
- Keep it to 2-5 keywords per clip; a highlight on every third word is no highlight.
- Emoji need an emoji font. On Windows Segoe UI Emoji is found automatically and draws the emoji as a one-colour
  outline in the caption colour (not full colour). With no emoji font installed the render **drops the emoji and warns**.
  Check a frame with `render preview-frame` before you rely on them.

### Speaker colours (interviews, podcasts, panels)

When two or more people talk, give each their own caption colour so viewers can follow who says what.

```
rustler analyze speakers src_X --num-speakers 2           # once per source; labels the transcript words
rustler caption speaker-colors clp_X --enabled true        # default palette: S1 yellow, S2 cyan, S3 green ...
rustler caption speaker-colors clp_X --enabled true --colors '{"S1":"#7CFF6B","S2":"#5EE0FF"}'
rustler caption speaker-colors clp_X --enabled false       # back to the preset colour (palette is kept)
rustler render preview-frame clp_X --out-t 4.0             # look at a frame of each speaker
```

- The colour is applied to the words of that speaker; the preset's own active-word highlight stays.
  `clean-white` and `bold-karaoke` highlight in yellow, so give S1 a colour other than yellow (as
  above) or the active word looks like a speaker change. Keep colours readable on the darkest and the
  brightest frame, and use two or three per clip.
- Speaker ids (`S1`, `S2`) come in order of first appearance. `rustler speaker rename src_X S1 Host`
  stores a display name for summaries; colours are keyed by the id.
- Short interjections get their own colour for a moment; if that flickers, pass `--num-speakers`
  explicitly or turn colours off for that clip.
- A clip made from a source that was never diarized renders with the plain preset colour and says so
  in the render warnings.

## 7. Readability checks (on the contact sheet and preview frames)

- Text fully inside the safe box; no letter cut at the edge.
- Not over the face in any tile; captions jump only between chunks, not mid-chunk.
- Contrast: readable on the brightest tile. If not → `boxed-highlight`.
- Spelling: names, brands, slang and numbers are where Whisper fails. Compare the caption text in
  the contact sheet with what is said. There is no caption word-edit command: pick another
  take or note the error in the report (a misspelt creator name can get a clip rejected).
- Profanity: if the brief or platform requires, mask or avoid clips where it can't be fixed.
- No double captions (source subtitles + ours).
- Captions present from the first word: the hook must be readable in the first second.
