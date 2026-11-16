---
id: templates-and-brand
title: "Templates: save a clip's look, apply it to many clips, carry it to another machine"
summary: "template create/update/show/list/diff/delete, template apply (JSON-Patch through timeline.apply, one undoable revision per clip, overrides, dry run), the built-in starters podcast-viral and streamer-split, clip create --template and the template.default setting, and export/import bundles with fonts, styles and assets."
order: 21
tags: [template, brand, look, style, starter, podcast-viral, streamer-split, export, import, batch]
---

# Templates and brand looks

A template is a clip's **look without its content**: caption style, position and words per line,
layout, hook title look (size, box, position, how long it stays), lower-third looks, brand images
(logo), progress bar, loudness / denoise / music, sound-effect and effect patterns (a punch-in at every
cut, every N seconds, at the start), transitions and canvas. Hook and label **text is not saved** unless
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
| `podcast-viral` | hormozi-pop captions (3 words, y 0.70), big boxed hook (font 0.10, top, 3 s, <= 7 words), punch-in every 5 s, yellow progress bar, -14 LUFS. Leaves framing alone (close-ups: `crop track-face`; wide shots: `clip set-layout --preset blur-fit`). |
| `streamer-split` | split-vertical (facecam top, gameplay below; segments that are already split keep their panes), hook on the seam, hormozi-pop, punch-in every 6 s, light denoise, progress bar, -14 LUFS. |

Read the `notes` in `template show`: they say what the template does not do for you.

## Save your own look

Style one clip by hand until it looks right (LOOK at `render preview-frame`), then:

```
rustler template create my-look --from-clip clp_x --description "Brand look for the X podcast"
```

The output lists `refs` (assets and files the look needs) and `notes` (what was not captured: word
overrides, masks, cutaways, effects at content times, text). Or write the JSON yourself
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
  applying again replaces them instead of stacking. Your own items are never removed.
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

A template refers to library assets by id (fonts, custom caption styles, music, sfx, images) or to plain
files. If one is missing, `template apply` fails with `not_found` naming it, and `template show` lists it
under `missing`. To move a look with everything it needs:

```
rustler template export my-look --out my-look.zip     # template + fonts + caption style + assets + files
rustler template import my-look.zip                   # on the other machine / project (--scope global)
```

Import adds the assets to the library (ids that already exist are kept), unpacks plain files next to the
template and saves it (a taken version with other content becomes the next free version).

## From a reference set

After `reference analyze` (see `learn-style-from-videos`), write the template JSON from the profile and
what you saw on the sheets, with `"based_on": {"reference_set": "<name>"}`; apply it to one clip, compare
the frames with the reference sheets, `template update`, then apply to the batch.
