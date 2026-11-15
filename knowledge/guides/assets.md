---
id: assets
title: "Assets: fonts, sound effects, music, caption styles, licenses"
summary: "Find and add library assets (asset list/search/show/add/remove), add anything yourself (file, --url, source:<id>), use them as asset:<id> in audio commands, optional license metadata, the opt-in Kenney CC0 SFX pack, and creating custom caption styles."
order: 17
tags: [assets, library, license, sfx, music, fonts, caption-style, custom-style]
---

# Assets

The library holds everything reusable that is not a source video: **fonts, sound effects, music,
images, caption styles and layouts**. Nothing is fixed: **you can add anything** (your own music, a
track from a link, the audio of any imported video). Every entry records a license field; if you do
not know it, it is stored as "unspecified (added by user/agent)" with a warning (Rustler never
refuses a file over the license text; the right to use it is yours). Two places, merged when you read them:

- global: `~/.rustler/assets/` (filled by `rustler setup run`; shared by all projects)
- project: `<project>/assets/` (added with `--scope project`, the default inside a project)

A project asset overrides a global one with the same id, which overrides a built-in.

## 1. What is there out of the box

After `setup run` (`rustler asset list` shows the real state; the SFX pack is **opt-in**, see below):

| Kind | Contents | License |
|---|---|---|
| `caption_style` | `style-bold-karaoke`, `style-clean-white`, `style-boxed-highlight`, `style-hormozi-pop` (the 4 presets) | built-in |
| `layout` | `layout-center-crop`, `layout-blur-fit`, `layout-split-vertical` | built-in |
| `font` | Anton, Poppins ExtraBold, Poppins Bold, Noto Sans Devanagari | OFL-1.1 |
| `sfx` | **only after `rustler setup run --with sfx-pack`** (or `--profile full`): about 26 curated sounds from the Kenney Interface Sounds and Impact Sounds packs: clicks, ticks, pops, swipes (`sfx-kenney-interface-swipe-up`), glitch, bong, punches and impacts (`sfx-kenney-impact-punch-heavy`, `sfx-kenney-impact-impact-metal`, `-glass`, `-bell`). Without the pack the library has no sfx: add your own | CC0 |
| `music` | **none by default, nothing fixed.** Add whatever you want (section 4). Optional CC0 starter pack (11 FreePD tracks, `rustler setup run --music-pack`): `asset list --kind music --tag upbeat`, then `audio add-music <clip> --path asset:<id>` | yours / CC0-1.0 (FreePD) |

Layout assets are only the three built-ins; custom multi-region layouts are saved with
`layout save-preset` (see `layouts-and-framing`).

## 2. Find, inspect, remove

```
rustler asset list                       # everything, global + project
rustler asset list --kind sfx            # kinds: font | music | sfx | image | caption_style | layout | template
rustler asset list --tag whoosh
rustler asset search swipe               # fuzzy over name, tags, id; --kind sfx narrows
rustler asset show sfx-kenney-impact-punch-heavy     # license, source, duration
rustler asset show style-hormozi-pop     # fonts and caption styles render a preview PNG (open it)
rustler asset remove music-my-bed --yes  # destructive; built-ins cannot be removed
```

`asset show` for a font or caption style returns `preview.image`: a sample burned on a gray 1080x1920
frame. Look at it before you choose.

## 3. Use an asset: `asset:<id>`

Wherever a command wants an audio file, pass `asset:<id>` (or a path, an `http(s)` link, or
`source:<source_id>[@start-end]`, see `audio` section 3):

```
rustler audio add-sfx clp_X asset:sfx-kenney-impact-punch-heavy --at 0.1 --volume-db -8
rustler audio add-sfx clp_X asset:sfx-kenney-interface-swipe-up --at-word '{"source_id":"src_X","word_index":212}'
rustler audio add-music clp_X --path asset:music-calm-bed --volume-db -18
```

Fonts are staged for the render automatically (a caption style naming a library font just works).
Sound effects: keep them subtle (-6 to -12 dB, the voice stays on top), one or two per clip, on a
cut, an emphasis word or the reveal; word-anchored (`--at-word`) effects follow the word when you
re-cut. See `audio` and `editing-toolkit`.

## 4. Add your own (anything; the license is optional)

```
rustler asset add D:/music/calm-bed.mp3 --kind music --name "Calm bed" --tags '["calm","bed"]'
rustler asset add --url https://example.org/lofi.mp3 --kind music --license CC-BY-4.0 --attribution "Artist - Track"
rustler asset add source:src_01k...@30-75 --kind music --name stream-bgm     # audio of an imported video
rustler asset add D:/fonts/Bangers-Regular.ttf --kind font --license OFL-1.1 --scope global
rustler asset add D:/sfx/whoosh.wav --kind sfx --license "own work" --scope project
```

- **Sources**: a local file, `--url <direct http(s) link>` (downloaded into the library; needs the
  project's network + downloads permission like `source import`; page URLs are not scraped, give the link
  to the file), or `source:<source_id>[@start-end]` (the audio of that range of an imported video, saved
  as m4a). A video file added as music/sfx is reduced to its audio track. Downloads and extractions are
  cached in `<project>/assets/_cache/`, so the same URL or range is fetched once.
- `--license` is **optional**. Missing (or a placeholder such as `unknown`) is stored as
  `unspecified (added by user/agent)` with a warning; any text you give is stored as-is
  (`CC0-1.0`, `CC-BY-4.0`, `OFL-1.1`, `own work`, `licensed: <where from>`...). Add `--attribution "<credit>"`
  when the license requires credit, `--license-url` / `--source-url` when you have them.
- Allowed extensions per kind: font `ttf/otf`; music/sfx `mp3 wav ogg m4a aac flac opus` (plus video
  containers, reduced to audio); image `png jpg webp gif`; caption style and layout `json`. A file already
  in the library (same sha256) is refused as a duplicate.
- Rights are the user's call, not Rustler's: platforms flag copyrighted tracks, so when in doubt about a
  commercial song, ask or leave music off (see `audio` section 3). If a track needs attribution, put it in
  the post description (see `platform-specs`).
- Note the id from the output (`music-calm-bed`): that is what `asset:<id>` needs.

## 5. Custom caption styles

The four presets cover most clips (`captions-and-style`). When a niche or brief needs a look of its
own, derive a style from a base instead of starting over:

```
rustler caption create-style neon-pop --base hormozi-pop --active-color "#00FFFF"
rustler caption create-style yellow-box --base boxed-highlight --box-color "#FFD400" --primary "#000000" --outline "#FFFFFF"
rustler caption create-style big-clean --base clean-white --size-factor 0.085 --uppercase true --scope global
rustler caption styles                   # presets + custom, with scope, font, active-word kind
rustler asset show style-neon-pop        # preview PNG: OPEN it
rustler caption set clp_X --style neon-pop
```

Fields: `--font-family` (a family name libass can see, e.g. "Anton", "Poppins ExtraBold", or a font you
added), `--primary`, `--outline`, `--outline-width`, `--shadow`, `--size-factor` (fraction of the canvas
width, 0.02-0.15), `--uppercase`, `--active-kind` (`recolor | box | pop`: how the spoken word shows),
`--active-color`, `--box-color`, `--scope` (`project` default, or `global`). Colours are `#RRGGBB`.
The name is lowercased, spaces become `-`, and it cannot reuse a built-in name. A custom style is a
caption-style asset (`style-<name>`), usable anywhere a preset name is (`caption set`, `clip create
--caption-style`, `variant create --caption-styles`, `brief set --caption-style`).

Readability rules still apply (`captions-and-style`): high contrast, bottom edge ≤ 0.78, not over the
face. A style with a font that lacks the glyphs (Devanagari!) shows □: use a Noto Sans Devanagari
based family for that script.

## 6. Licenses at a glance

- **CC0 / public domain**: free to use, no credit needed (the bundled Kenney sounds).
- **OFL-1.1** (fonts): free to use in videos; do not resell the font file.
- **CC-BY**: usable with credit in the description; record it with `--attribution`.
- **Own work / licensed / anything you write**: you state where the right comes from; Rustler records it and never verifies it.
- **unspecified (added by user/agent)**: what is stored when you gave no license. Fine for your own files and
  tests; for anything you publish, make sure the right to use it exists.
