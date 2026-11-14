# f23 - Asset library

**Status:** Built (merged, tests pass)  |  **Depends on:** f01, f16, f17  |  **Owner:** C2 agent

## Purpose

A reusable creative asset library agents can search, preview and apply: fonts, music, SFX, images, caption
styles, layouts, templates. Every file carries license metadata; unlicensed media is never added.

## Scope

**In:** global library `~/.rustler/assets/` + per-project `<project>/assets/` (same `manifest.json` schema, merged
at read time, project overrides global overrides built-in by id); `asset.list|search|show|add|remove`;
`caption.create_style`, `caption.styles` (custom styles saved as caption_style asset JSON and consumed by
rustler-captions); `asset:<id>` in place of a path (`audio.add_music`; `audio.add_sfx` via the same helper);
`rustler setup` fills the global library (built-in styles/layouts, installed OFL fonts, a curated subset of two
Kenney CC0 sound packs); font assets are staged for every render.
**Out:** bundled music (no verifiably CC0, login-free source: users add their own with `asset add --license`),
a full GUI asset browser (f25 has a music picker only), custom multi-region layouts as assets (layouts live in `<project>/layouts`, f15).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-commands/src/asset_cmds.rs` (asset.*, caption.create_style, caption.styles, `asset:<id>` resolver)
- `crates/rustler-tools/src/assets.rs` (manifest, built-ins, setup fetch)
- `crates/rustler-captions/src/custom.rs` (custom caption styles)
- Tests: `crates/rustler-commands/tests/asset.rs`

**Read-only dependencies:** `audio_cmds.rs` (calls the resolver for `audio.add_music` and `audio.add_sfx`), `setup_cmds.rs`.

## Data

`manifest.json`: `{"version":1,"assets":[{id, kind, name, path, tags, license, license_url?, source_url?, attribution?,
added_at, sha256}]}`. `kind`: font | music | sfx | image | caption_style | layout | template. `path` is relative to
the manifest folder (or absolute for fonts registered from `~/.rustler/fonts`); built-in entries use
`builtin:<kind>/<name>` and license `built-in`. Custom style file: `caption_styles/<name>.json` (`CustomStyle`,
colours `#RRGGBB[AA]`).

## Rules

1. `asset.add` requires `--license` (empty/placeholder values such as `unknown`, `n/a` are refused); extension is checked per kind
   (fonts ttf/otf, audio mp3/wav/ogg/m4a/aac/flac/opus, images png/jpg/webp/gif, styles/layouts json); duplicate sha256 refused.
2. Built-in assets cannot be removed; only files inside the library root are ever deleted.
3. `setup` registers a Kenney pack only after its `License.txt` states CC0; network failures are warnings.
4. Custom style names cannot reuse built-in names; project styles shadow global ones.

## Verification

`cargo test -p rustler-commands --test asset` (7 tests; ffmpeg ones skip without tools) and
`cargo test -p rustler-commands --test asset -- --ignored` (network: fetches the Kenney packs).

## Docs-pass check (2026-11-15)

`asset list` (4 caption styles, 3 layouts, 4 fonts, 26 SFX), `asset search`, `asset show style-neon-pop` (preview PNG), `asset add` (music, license `own work`, refused for a `.mp4`),
`caption create-style neon-pop --base hormozi-pop`, `caption set --style neon-pop`, `audio add-sfx asset:sfx-kenney-interface-click-1`, `audio add-music --path asset:music-... --dry-run`
all ran against a temp project. `--tags` takes a JSON array (`'["a","b"]'`); a plain comma string becomes one tag.

## Status Log

- 2026-11-15 - Built. Preview PNGs (font specimen, caption style on a gray 1080x1920 frame, render preview with a custom style) viewed.
