# Rustler

Agent-native toolkit for making short-form vertical clips (TikTok / Reels / Shorts) from long videos.
Any AI agent — Claude Code, Muse, opencode, Antigravity, Grok — installs it and works as the editor:
it scans a source, reads the transcript, **looks at frames**, picks hooks, cuts, lays out, captions,
mixes, renders, checks quality and exports. Rustler calls **no AI APIs**; the agent does the thinking,
Rustler provides the tools (FFmpeg, yt-dlp, local faster-whisper, OpenCV face tracking).

- **CLI** `rustler` — the complete surface, JSON in/out, discoverable (`rustler help`, `rustler commands list`).
- **MCP server** `rustler mcp serve` — every command as a tool, knowledge guides as resources, workflows as prompts,
  frames returned as images to vision agents.
- **GUI** `rustler gui` — simple local web UI over the same commands.
- **Knowledge book** `rustler guide list` / `rustler prompt list` — how to edit like a pro.

## Quick start
```bash
cargo build --release -p rustler-cli        # Windows: run from PowerShell
rustler setup                               # FFmpeg, yt-dlp, uv/Python engine, whisper model, fonts → ~/.rustler
rustler doctor                              # what works (GPU, encoders, engine)
rustler project create --name ep12 --dir ./ep12
cd ep12
rustler source info <youtube-url>           # title, chapters, most-replayed peaks — no download
rustler source fetch-transcript <url>       # existing captions, no download
rustler source import <url> --sections "47:00-49:30"
rustler analyze quick <source-id>
rustler transcript search <source-id> --query "..."
rustler see tiles <source-id> --around "..."
rustler clip create --name hook1 --source-id <id> --start-unit 12 --end-unit 14 --expect-text "first words" --layout blur-fit
rustler render run <clip-id>
rustler qc run <clip-id>                    # then LOOK at the contact sheet
rustler export clip <clip-id>
```
Give an agent the skill in `skills/rustler/` and see `docs/agent-integration.md`.

## Docs
Everything is in [`docs/`](docs/README.md): vision & decisions, design, plan, progress log, research.
