# Rustler

Agent-native toolkit for making short-form vertical clips (TikTok / Reels / Shorts) from long videos.
Any AI agent — Claude Code, Muse, opencode, Antigravity, Grok — installs it and works as the editor:
it scans a source, reads the transcript, **looks at frames**, picks hooks, cuts, lays out, captions,
mixes, renders, checks quality and exports. Rustler calls **no AI APIs**; the agent does the thinking,
Rustler provides the tools (FFmpeg, yt-dlp, local faster-whisper, OpenCV face tracking).

- **CLI** `rustler` — the complete surface, JSON in/out, discoverable (`rustler help`, `rustler commands list`).
- **MCP server** `rustler mcp serve` — every command as a tool, knowledge guides as resources, workflows as prompts,
  frames returned as images to vision agents.
- **GUI** `rustler gui` — local web editor (regions, monitor, timeline, inspector) over the same commands.
- **Knowledge book** `rustler guide list` / `rustler prompt list` — 22 guides and 15 workflows: how to edit like a pro.

## Install
Prebuilt binaries (Windows x64 zip with CLI + desktop window, Linux x86_64 tar.gz) are GitHub release assets of the
private repo. One-file installers verify SHA256, install into `~/.rustler/app` without admin/root, add `rustler` to the
user PATH, give Claude Code the skill and run `rustler setup run`:
```powershell
gh api repos/krishna-notfound/rustler/contents/scripts/install.ps1 -H "Accept: application/vnd.github.raw" > $env:TEMP\rustler-install.ps1; powershell -ExecutionPolicy Bypass -File $env:TEMP\rustler-install.ps1
```
```sh
gh api repos/krishna-notfound/rustler/contents/scripts/install.sh -H "Accept: application/vnd.github.raw" | bash
```
Needs `gh auth login` or `RUSTLER_TOKEN`/`GITHUB_TOKEN` (private repo). Details, flags, manual download: [`docs/install.md`](docs/install.md).
Music is bring-your-own (`rustler asset add`); an optional CC0 starter pack: `rustler setup run --music-pack`.

## Quick start
```bash
cargo build --release -p rustler-cli        # from source (or install a release, above); Windows: run from PowerShell
rustler setup run                           # LITE: FFmpeg, yt-dlp, uv + base Python engine, whisper base, fonts, built-in styles -> ~/.rustler
rustler setup run --with ocr,speakers       # opt in to heavy analyzers later (also: audio-events, cuda, sfx-pack, or --profile full)
rustler doctor run                          # what works (GPU, encoders, engine); ready=true means you can render
rustler project create ep12 --dir ./ep12
cd ep12
rustler source info <youtube-url>           # title, chapters, most-replayed peaks - no download
rustler source fetch-transcript <url>       # existing captions, no download
rustler source import <url> --sections 47:00-49:30      # or a local file: rustler source import "D:/videos/ep12.mp4"
rustler analyze quick <source-id>           # audio + scenes + transcript + units
rustler analyze deep <source-id>            # on-screen text, laughter/applause, faces
rustler highlights candidates <source-id>   # ranked windows with unit ids and a ready clip create line
rustler transcript search <source-id> "phrase"
rustler see tiles <source-id> --around "phrase"
rustler clip create --name hook1 --source-id <id> --start-unit 12 --end-unit 14 --expect-text "first words" --layout blur-fit
rustler edit refine <clip-id>               # skip weak openers, end on a sentence end
rustler overlay add-text <clip-id> "Hook in seven words" --start 0 --end 2.5
rustler render preview-frame <clip-id> --out-t 1.0      # look at it
rustler render run <clip-id>
rustler qc run <clip-id>                    # then LOOK at the contact sheet
rustler fix suggest <clip-id>               # QC report -> exact fix commands
rustler export clip <clip-id>
rustler project status                      # what is done, stale or unchecked
```
Streamer clips: `rustler layout detect-webcam <source-id> --clip-id <clip-id>`. Briefs, variants and batches:
`rustler brief set ...`, `rustler variant create <clip-id> ...`, `rustler render batch --all true`, `rustler export batch --all true`.
v2: `clip create` picks layout and framing per shot by itself, `rustler template apply podcast-viral` gives clips one look,
`rustler clip retention` finds dead zones, `rustler overlay hook-styles` lists viral hook titles, `rustler clip cover` picks the
cover frame and `rustler clip meta-set` stores the per-platform post copy that export writes as `<name>.post.json`.
Every command (179): `rustler commands list`. On Windows PowerShell 5.1, pass JSON flag values via `--input-json @file.json`.

Give an agent the skill in `skills/rustler/` and see `docs/agent-integration.md`.

## Docs
Everything is in [`docs/`](docs/README.md): vision & decisions, design, plan, progress log, research.
