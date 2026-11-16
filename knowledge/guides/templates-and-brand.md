---
id: templates-and-brand
title: "Templates: save a clip's look, apply it to many clips, carry it to another machine"
summary: "template create/update/show/list/diff/delete, template apply (JSON-Patch through timeline.apply, one undoable revision per clip, overrides, dry run), per-shot layout rules, keyword/emoji captions, cutaway rules, music by tag, brand kit (logo, CTA, outro), export and posting defaults, the built-in starters podcast-viral and streamer-split (no automatic zooms), clip create --template and the template.default setting, and export/import bundles with fonts, styles and assets."
order: 21
tags: [template, brand, look, style, starter, podcast-viral, streamer-split, export, import, batch, layout-rules, keywords, emoji, cutaway, music, logo, cta, outro, hashtags]
---

# Templates and brand looks

A template is a clip's **look without its content**: caption style, position and words per line,
keyword highlight and emoji, layout and per-shot layout rules, hook title look (size, box, position, how
long it stays), lower-third looks, brand kit (logo, CTA card, outro card) and other brand images, progress
bar, loudness / denoise / music (an asset, or picked by tag / mood), sound-effect and effect patterns,
cutaway rules for weak visual stretches, transitions, canvas, export defaults (quality, fps, length window)
and posting defaults (always-hashtags, mentions, CTA line). Hook and label **text is not saved** unless
you ask (`--include-text`): hooks are written per clip.

A section that is missing from a template leaves that part of the clip alone. Templates live in
`templates/<name>@<version>.json` in the project, or `<home>/templates/` with `--scope global`.

## Start from a starter

```
rustler template list                          # project, global and built-in templates
rustler template show podcast-viral            # the JSON, what it needs, its notes
rustler template apply podcast-viral --clip-ids clp_x --dry-run
rustler template apply podcast-viral --clip-ids clp_x
```

| Starter | Look |
|---|---|
| `podcast-viral` | per-shot framing by `layout_rules` (one face: center-crop + face track; two or more: center-crop + active speaker; screen: zoom-fit; no face / graphics: blur-fit), hormozi-pop captions (3 words, y 0.70) with one keyword a line highlighted, big boxed hook (font 0.10, top, 3 s, <= 7 words), yellow progress bar, -14 LUFS, export quality final at 30 fps. **No automatic punch-ins.** |
| `streamer-split` | split-vertical (facecam top, gameplay below; segments that are already split keep their panes), hook on the seam, hormozi-pop, light denoise, progress bar, -14 LUFS, export quality final at 30 fps. **No automatic punch-ins.** |

The starters reference no assets, so they apply on a fresh install.

**Zooms are your decision.** No starter adds punch-ins (and no starter sets a punch-in `cutaway_rules`). Add a
punch-in yourself where a moment earns it, subtle and held for a phrase:
`rustler effect add <clip> punch-in --start 4.2 --end 9.0 --amount 1.08`. Your own template may still carry a
punch-in pattern (captured `--from-clip` from a clip you edited, or written by hand); it then goes through the
punch-in planner (the same one `clip retention`'s `punch_in` suggestion uses):
`{"kind": "punch-in", "at": "emphasis", "duration": 5.0, "amount": 1.06, "amount_max": 1.12, "per_10s": 1.5}`
places each zoom just before a stressed word, holds it for a phrase, keeps the density under `per_10s` changes
(cuts, text pops, zoom in and out) in any 10 s, stays 0.5 s away from cuts, layout changes and transitions,
skips graphic / blur-fit segments and cutaways, does not stack on your own punch-ins, varies the amount between
`amount` and `amount_max`, and centres the zoom on the face when the source has `faces.json`
(`rustler analyze faces <source_id>`). Other effect kinds and `start` / `cuts` / `time` are placed exactly as
written.

## The other sections

All optional; a missing section leaves that part of the clip alone. Items they add are tagged `by_template`
(re-apply replaces them). `template create --from-clip` captures each one when the clip shows it.

| Section | What apply does | Captured from a clip |
|---|---|---|
| `layout_rules: [{when, layout?, params?, regions?, background?, track?, look_room?}]` | Per segment, the first rule whose `when` matches the shot sets the layout. `when`: `faces=1`, `faces>=2`, `no_face` (graphics too), `screen` (no face but a screen / window box, or graphics), `graphics`. Shot data comes from the smart defaults' shot plan (`rustler layout plan <clip> --apply`, or `clip create` smart defaults) or `analysis/<source>/faces.json` (`rustler analyze faces <source_id>`); a segment with neither is left unchanged with a warning. A segment already on the rule's preset keeps its framing and crop path. A new center-crop is centred on the face. `track: face / active / speaker` needs a crop path the template cannot compute (no face engine at apply time): the warnings give the exact `rustler crop track-face` / `track-active` / `track-speaker` command; `track: none` drops the crop path. `zoom-fit` keeps an existing zoom-fit, else blur-fit. Segments no rule matches get `layout`. | One rule per shot class with that segment's preset (custom regions for custom layouts), `track` from whether it has a crop path. |
| `captions.keyword_highlight {color?, max_per_line?}` | Marks keywords with the `caption keywords --auto` rules (numbers, emphasis words, names), at most `max_per_line` (default 1) per caption line, as coloured caption word overrides. Words you styled yourself keep their look. Colour default as `caption keywords`. | Coloured word overrides: the main colour and the most per line. |
| `captions.emoji: on / off` | `off` removes the emoji from caption words; `on` keeps them (you pick emoji per word: `rustler caption word <clip> --source-id <src> --word-i <i> --emoji "..."`; a warning when there are none). | `on` when the clip has emoji words or uses `emoji-karaoke`. |
| `cutaway_rules {on_weak_visual_after_s, kind, tag?, duration?, mode?, rect?}` | Finds edit-timeline stretches longer than `on_weak_visual_after_s` with no visual change (cuts, text, images, cutaways, freezes; zooms count as motion). `kind: asset_tag` puts a cutaway of an image asset with library tag `tag` in the middle of each (`duration`, default 1.5 s; `mode` full or pip); no asset with the tag is a warning. `kind: punch_in` fills them with the punch-in planner at its own (subtle) defaults: only use it when you decided the look should zoom. | Cutaways of tagged library images give an `asset_tag` rule (threshold 3 s by default). |
| `audio.music {tag? / mood?, volume_db, duck, fades}` | Picks a music asset whose library tags contain `tag` (or `mood` / `mood:<mood>`); the same clip gets the same pick on re-apply. None in the library is a warning (music left as it was). `asset` / `file` / `none` work as before. | The music asset id. |
| `brand {logo_asset?, rect?, opacity?, cta_card?, outro_card?}` | Logo = image layer over the whole clip (default top right, opacity 0.9). Cards are image assets (design them with `rustler asset render-card`): `cta_card {asset, at?, duration?, rect?}` at `at` s (default 40 % in, 3 s), `outro_card {asset, duration?, rect?}` until the end (default 2.5 s, whole canvas). A missing asset is `not_found` naming it. | Library images tagged / named logo, cta, outro, or placed like one (small over the whole clip = logo; to the end from past the middle = outro). |
| `export {quality?, fps?, max_len?, min_len?}` | Stored in the clip (extras `export`): `render run` without `--quality` uses `quality` (else the `render.quality` setting); `fps` sets the canvas fps; `qc run` checks the length window (instead of the brief's); a clip outside it is a warning at apply. | The clip's stored export defaults, else fps + the last render's quality. |
| `post_defaults {hashtags_always?, mentions?, cta_line?}` | Added to every platform's posting copy (`clip meta-set`) without replacing what you wrote: missing hashtags / mentions appended, the CTA line appended to a description that lacks it. Stored in the clip too, so `export clip`'s post JSON gets them for copy written later. Rustler never writes titles or descriptions. | The clip's stored defaults, else hashtags / mentions every platform shares. |

Older template files (without these sections) load and apply unchanged.

Read the `notes` in `template show`: they say what the template does not do for you.

## Save your own look

Style one clip by hand until it looks right (LOOK at `render preview-frame`), then:

```
rustler template create my-look --from-clip clp_x --description "Brand look for the X podcast"
```

The output lists `refs` (assets and files the look needs) and `notes` (what was not captured: word
overrides that are not keywords, masks, untagged cutaways, effects at content times, text). Or write the JSON yourself
(`--input-json @template.json` with `{"name": "my-look", "template": {...}}`); unknown fields are
rejected, so `template show podcast-viral` is the best example of the shape.

Change it later as a new version (old versions stay; `name@N` picks one):

```
rustler template update my-look --patch '{"captions":{"position_y":0.65}}'   # merge patch on the latest
rustler template update my-look --from-clip clp_y                            # re-capture
rustler template diff my-look@1 my-look@2
rustler template delete my-look@1
```

Updating a built-in starter saves your own version in the project that shadows it.

## Apply

```
rustler template apply my-look --clip-ids clp_a --clip-ids clp_b
rustler template apply my-look@2 --all --dry-run       # every clip in the project; shows the patch
rustler template apply podcast-viral --clip-ids clp_x --overrides '{"hook":{"text":"He quit his job"}}'
```

- Each clip gets one JSON-Patch through `timeline.apply`: fully validated, one revision,
  `rustler history undo <clip>` reverts it. Every clip is checked before any clip is saved.
- Items the template adds (effects, sound effects, images, texts) carry `by_template: "<name>@<ver>"`;
  applying again replaces them instead of stacking (also after `clip create --template` / `clips auto` and
  later `effect add` calls). Your own items are never removed.
- Edits that shorten a clip (`edit delete-segment`, `trim`, `set-range`, `remove-pauses`, ...) move effects,
  texts, images, masks, cutaways, sound effects and freezes with their content, drop the ones whose content was
  cut out and clamp the ones that run past the new end; each is named in the edit's `warnings`. Items an older
  build left past the end are dropped / clamped (with a warning) when a template is applied.
- The hook look restyles existing hook-top titles; with no hook on the clip you get a warning (or the
  hook is added when the template or `--overrides` has `hook.text`). Hooks longer than `max_words` warn.
- A layout already in use on a segment is kept (its crop path or detected webcam panes stay); set
  `"force": true` in the template's `layout` to re-lay it anyway.
- `--overrides` is a merge patch for this run only (`{"effects": []}` turns the effect pattern off).
- `timeline show <clip>` prints `template_ref` (name and version) afterwards.

New clips can be styled at creation: `rustler clip create ... --template my-look`, or set a default for
every new clip: `rustler config set template.default my-look` (a default that cannot be applied is a
warning, an explicit `--template` that cannot be applied is an error and no clip is made).

## Brand assets and other machines

A template refers to library assets by id (fonts, custom caption styles, music, sfx, images, brand logo and cards) or to plain
files. If one is missing, `template apply` fails with `not_found` naming it, and `template show` lists it
under `missing`. To move a look with everything it needs:

```
rustler template export my-look --out my-look.zip     # template + fonts + caption style + assets + files
rustler template import my-look.zip                   # on the other machine / project (--scope global)
```

Import adds the assets to the library (ids that already exist are kept), unpacks plain files next to the
template and saves it (a taken version with other content becomes the next free version). Music and cutaway images picked by tag are chosen at apply time from the library there, so they are not bundled.

## From a reference set

After `reference analyze` (see `learn-style-from-videos`):

```
rustler template create my-learned --from-reference my-style
```

maps the learned profile onto a template: caption `position_y` (bottom of the measured caption band, kept in
0.10-0.85), `max_words_per_line` (median words per line), a built-in caption style close to the measured
accent colour (`clean-white` when the captions are plain white), the hook look (position and how long it
stays, only when most references open with on-screen text), and the loudness target. Pacing (cuts per
minute), zoom rate, music yes/no and the common shot type go to `notes` with the command to act on them:
the template never adds zooms (Rustler adds no automatic zooms; place punch-ins yourself) and picks no music
track. Fonts, caption animation, transitions and brand images are not derived. Apply it to one clip with
`--dry-run`, compare the frames with the reference sheets, `template update`, then apply to the batch.
