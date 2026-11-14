# Giving an agent Rustler

Rustler is built for agents: it calls no AI APIs. The agent reads transcripts, looks at images,
chooses moments and judges the result; Rustler downloads, transcribes, renders and runs QC. Any agent
that can run shell commands or speak MCP can drive it (Claude Code, Muse, opencode, Antigravity, Grok).

Files that ship for this:

| File | For |
|---|---|
| `skills/rustler/SKILL.md` | Claude Code / Agent Skills: loaded automatically when the user asks for clips |
| `skills/rustler/AGENTS.md` | Any other agent: the same essentials in plain markdown, plus MCP config |
| `knowledge/` | The knowledge book (17 guides, 10 prompts), served by `rustler guide` / `rustler prompt` and by MCP |

## 1. Put the CLI on PATH

Build or install the `rustler` binary and make sure `rustler --version` works in the shell the agent uses.
For a source checkout: `cargo build --release`, then add `target\release` (Windows) or `target/release`
to PATH, or copy `rustler.exe` somewhere already on PATH. If PATH cannot be changed, give the agent
the absolute path instead (in MCP configs use it as `command`).

Then bootstrap once per machine (idempotent, no admin rights, resumable):

```
rustler doctor run     # ready=true?
rustler setup run      # installs ffmpeg, yt-dlp, uv, the Python engine, whisper models, fonts into ~/.rustler
rustler project create "my-clips" --dir ./my-clips
```

Point the agent at the project with `RUSTLER_PROJECT=<dir>` (env), `--project <dir>` (every CLI call),
or by running it from inside the project folder.

## 2. Connect an agent

Two routes, same commands. MCP gives typed tools, prompts and inline images. The plain CLI works
anywhere a shell does.

### MCP: generic stdio

```json
{
  "mcpServers": {
    "rustler": {
      "command": "rustler",
      "args": ["mcp", "serve"],
      "env": { "RUSTLER_PROJECT": "<dir>" }
    }
  }
}
```

Replace `<dir>` with the project folder. The top-level key and file location vary by host; the three
values under `rustler` do not. Every tool also accepts an optional `project` argument that overrides the
server default for one call, so one server can serve several projects.

| Agent | How |
|---|---|
| Claude Code | `claude mcp add rustler -- rustler mcp serve` (add `--env RUSTLER_PROJECT=<dir>` before the name; `--scope user` to make it global). Also install the skill, below. |
| opencode | Add the stdio entry to the `mcp` section of `opencode.json` (local server, command `["rustler","mcp","serve"]`, environment `RUSTLER_PROJECT`). Drop `skills/rustler/AGENTS.md` into the project as `AGENTS.md` (or append it to yours). |
| Antigravity | Add the generic stdio entry in the MCP servers settings (raw config). Put the contents of `AGENTS.md` in the workspace rules or project instructions. |
| Grok / Muse | Use the host's MCP server settings with the generic stdio entry. If the host has no MCP support, use the CLI route and paste `AGENTS.md` into the agent's system or project prompt. |

Exact config file names change between host versions; check the host's MCP documentation and map the
generic entry onto it.

### CLI only

Tell the agent to run `rustler ...` in a shell and parse the single JSON envelope on stdout (progress
is NDJSON on stderr). Image commands return a file path; the agent opens the file with its own image
viewer. Give it `skills/rustler/AGENTS.md` as instructions. Cold start for an agent that knows nothing:
`rustler help`, then `rustler guide show start-here`.

### Claude Code skill

Copy the skill folder so Claude Code discovers it:

```
# per user (all projects)
mkdir -p ~/.claude/skills/rustler && cp skills/rustler/SKILL.md ~/.claude/skills/rustler/
# Windows PowerShell
New-Item -ItemType Directory -Force $HOME\.claude\skills\rustler
Copy-Item skills\rustler\SKILL.md $HOME\.claude\skills\rustler\
# or per project: <project>\.claude\skills\rustler\SKILL.md
```

The skill's `description` makes Claude load it when the user talks about clips, shorts, reels, TikTok,
highlights, captions or reframing. It is short on purpose and points to guides for depth.

## 3. What the agent sees

- **Tools.** Every registry command becomes a tool named `group_action`: `source_info`,
  `source_import`, `analyze_quick`, `transcript_get`, `see_tiles`, `clip_create`, `render_run`,
  `qc_run`, `export_clip`, and so on: 118 tools (`rustler commands list` shows all; the guide
  `cli-reference-conventions` groups them by area: project, source, analyze, highlights, clip, edit,
  layout, crop, caption, overlay, transition, effect, audio, asset, render, qc, fix, export, brief,
  variant, speaker). Annotations mark read-only, destructive, network and long-running tools.
- **Prompts.** Ready workflows: `make_viral_clips`, `clip_from_stream`, `streamer_split_clip`,
  `quick_scan_source`, `hinglish_podcast_clips`, `batch_from_brief`, `polish_clip`, `review_clip`,
  `fix_qc_failures`, `restyle_clip`.
  On the CLI: `rustler prompt list` / `rustler prompt show <name> --args '{...}'`.
- **Resources.** `rustler://guides/<id>` (the 17 guides), `rustler://commands`,
  `rustler://project/state`.
- **Images.** `see_frame`, `see_tiles`, `render_preview_frame` and the QC contact sheet come back as
  MCP image content, so a vision-capable agent looks at them directly. Over the CLI the output holds
  a path to the jpg (also for `variant compare`, `qc compare`, `asset show` previews).
- **Guardrails built in.** Clips are cut from transcript unit ids with `--expect-text` so the agent
  cannot silently pick the wrong words; `export clip` refuses a failed QC; every edit is a revision
  with `history undo`; Rustler never uploads anything.

## 4. Sample session (illustrative)

The user asked an agent: "Make two shorts from this podcast: https://youtu.be/EXAMPLE". Outputs are
shortened and invented for teaching; IDs and numbers will differ.

```
agent> rustler doctor run
{"ok":true,"data":{"ready":true,"libass":true,...}}

agent> rustler guide show start-here          # first time only
agent> rustler project create "ep212" --dir ./ep212
agent> export RUSTLER_PROJECT=./ep212

agent> rustler source info https://youtu.be/EXAMPLE
{"ok":true,"data":{"title":"Ep 212: ...","duration":5400,"heatmap_peaks":[{"start":2820,"end":2880},
 {"start":4360,"end":4420}],"chapters":[...],"caption_langs":["en"]}}

agent> rustler source fetch-transcript https://youtu.be/EXAMPLE
agent> rustler transcript get src_01J... --start 2780 --end 2900
 212  00:47:02.1  Nobody tells you this when you start a company.
 213  00:47:05.0  The first hire is not about skill, it is about trust.
 ...
(agent shortlists two windows from the transcript and the heatmap)

agent> rustler source import https://youtu.be/EXAMPLE --sections 46:50-48:10 --sections 72:30-73:50
agent> rustler analyze quick src_01J...
agent> rustler transcript search src_01J... "nobody tells you this"
 hit unit 212 (neighbours 210-215)
agent> rustler see tiles src_01J... --around "nobody tells you this"
{"ok":true,"data":{"image":"analysis/tiles_212.jpg",...}}
(agent opens the image: speaker is left of center, no webcam overlay)

agent> rustler clip create --name nobody-tells-you --source-id src_01J... --start-unit 212 \
        --end-unit 219 --expect-text "nobody tells you" --layout center-crop --caption-style bold-karaoke
{"ok":true,"data":{"id":"clp_01J...","duration":41.3,...},"warnings":[]}

agent> rustler crop track-face clp_01J...
agent> rustler render preview-frame clp_01J... --out-t 1.0
(agent opens the jpg: caption sits inside the safe zone, face centered)

agent> rustler render run clp_01J...
agent> rustler qc run clp_01J...
{"ok":false,"error":{"code":"qc_failed","message":"QC failed: loudness","hint":"... contact sheet (open it): clips/clp_01J.../qc/contact.jpg ..."}}
(exit 7; agent opens the contact sheet, frames look right; reads the hint, trims dead air at the end)

agent> rustler edit remove-pauses clp_01J... --keep 0.25
agent> rustler render run clp_01J...
agent> rustler qc run clp_01J...
{"ok":true,"data":{"passed":true,...}}
(agent opens the new contact sheet: hook visible in first frame, captions legible, clean ending)

agent> rustler export clip clp_01J... --name nobody-tells-you
{"ok":true,"data":{"mp4":"exports/nobody-tells-you.mp4","report":"exports/nobody-tells-you.report.json"}}

agent> Done. Two clips are in ./ep212/exports (one skipped: the second window had no standalone hook).
       I did not upload anything; the files and reports are ready for you to review and post.
```

## 5. Troubleshooting for integrators

- `rustler: command not found` in the agent but fine in your terminal: the agent's shell has a
  different PATH. Use an absolute path.
- MCP server starts but tools report "no project": set `RUSTLER_PROJECT` in the server `env`, or pass
  `project` on each call.
- Exit 4 (dependency missing): run `rustler setup run`, then `rustler doctor run`.
- Network commands refused: the project needs permission (`rustler project set-permissions`).
- Windows PowerShell 5.1 strips double quotes inside JSON flag values (`--layout-params '{"top":...}'` fails
  validation): put the JSON in a file and pass `--input-json @file.json`, or escape quotes as `\"`.
  bash, zsh and cmd are unaffected.
- More: `rustler guide show troubleshooting`.
