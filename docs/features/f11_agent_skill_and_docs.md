# f11 — Agent Knowledge Book (guides, prompts, guide commands)

**Status:** Built  |  **Depends on:** f10  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

An agent that has only the `rustler` binary or the MCP server must learn how to edit video well without
reading this repo. The knowledge book is 13 workflow and craft guides and 8 MCP prompts, embedded in the
binary, and exposed through `rustler guide …`, `rustler prompt …` and the MCP prompt and resource endpoints.

## Scope

**In:**
- `knowledge/guides/*.md` (17 guides since the full command set landed; the original 13: start-here, workflows for local files and YouTube long-form, cut points, finding hooks, captions and style, layouts and framing, audio, Hinglish, platform specs, quality checklist, troubleshooting, CLI reference conventions; added later: streamer-clips, editing-toolkit, briefs-and-batches, assets).
- `knowledge/prompts/*.md` (10 prompts, incl. streamer_split_clip and polish_clip; the original 8: make viral clips, clip from stream, batch from brief, quick scan source, review clip, fix QC failures, restyle clip, Hinglish podcast clips).
- Loader crate `rustler-knowledge` (embeds `knowledge/` at build time).
- Commands: `guide.list`, `guide.show`, `guide.search`, `prompt.list`, `prompt.show`.

- Agent skill `skills/rustler/SKILL.md` and `skills/rustler/AGENTS.md` (commit 8e23db6), and `docs/agent-integration.md` (how to give an agent Rustler: CLI on PATH, MCP config, skill).

**Out (explicitly):**
- Provider-specific prompting guidance (no provider layer).
- Editing guides through the GUI.
- Any run showing an agent following the skill end to end (none recorded).

## User-facing Behavior

- `rustler guide list` lists guide ids; `rustler guide show <id>` prints one; `rustler guide search <text>` searches all guides.
- `rustler prompt list` and `rustler prompt show <id>` do the same for prompts.
- MCP clients read the same content as `rustler://guides/<id>` resources and as prompts.

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `knowledge/README.md`, `knowledge/guides/*.md`, `knowledge/prompts/*.md`
- `crates/rustler-knowledge/src/lib.rs`, `crates/rustler-knowledge/build.rs`
- `crates/rustler-commands/src/guide_cmds.rs`
- `skills/rustler/SKILL.md`, `skills/rustler/AGENTS.md`, `docs/agent-integration.md`

**Read-only dependencies:**
- `docs/research/` — source material for guide content (not embedded).
- `crates/rustler-mcp/src/knowledge.rs` — exposes the book over MCP (f10).

## Data and Interfaces

- `rustler-knowledge` exposes the embedded book (guides and prompts by id).
- Commands: `guide.list`, `guide.show`, `guide.search`, `prompt.list`, `prompt.show` (registered in `guide_cmds.rs`).
- Guide ids are assumed to be the file names without `.md` (for example `workflow-local-file`); confirm with `rustler guide list`.

## Rules and Edge Cases

1. Guide content is product content: edit `knowledge/`, never copy it into code.
2. Guide and prompt changes take effect after rebuild (content is embedded).
3. Guides describe only commands that exist in the registry; check `rustler commands list` before adding a command to a guide.
4. Spec §5.11 requires that transcripts and web text are treated as data, not instructions; the guide text should say so. Not yet checked line by line.

## Tasks

- [x] Write 13 guides and 8 prompts (`knowledge/`)
- [x] Loader and embedding (`rustler-knowledge`)
- [x] Guide and prompt commands (`guide_cmds.rs`)
- [x] `SKILL.md` for skills-aware agents (`skills/rustler/SKILL.md`, commit 8e23db6)
- [x] `AGENTS.md` for non-Claude agents (`skills/rustler/AGENTS.md`, commit 8e23db6)

## Acceptance Criteria

- [x] Knowledge loader tests (`crates/rustler-mcp/tests/knowledge.rs`; the progress log records 9 knowledge-crate tests and 5 guide-command tests)
- [x] MCP prompt and resource tests (`crates/rustler-mcp/tests/knowledge.rs`)
- [ ] Guide-command tests: the progress log records 5, but no test file under `crates/*/tests` names the guide commands. Unverified where they live.
- [ ] SKILL.md and AGENTS.md are exercised by an agent run (files exist; no run recorded)

## Verification

- `cargo test -p rustler-mcp --test knowledge` (PowerShell).
- `rustler guide list` returns the 13 ids; `rustler prompt show make_viral_clips` prints the prompt.

## Status Log

- 2026-11-15 — Knowledge book committed (13 guides, 8 prompts). Loader crate and guide and prompt commands merged; MCP prompts and resources render. Agent skill, skill AGENTS.md and integration guide committed (8e23db6). Guide-command test location unverified.
- 2026-11-15 — Docs pass for the full command set: `cli-reference-conventions` lists all 118 commands by area; 4 new guides (streamer-clips, editing-toolkit, briefs-and-batches, assets) and 2 new prompts (streamer_split_clip, polish_clip); every `rustler <group> <action>` in knowledge/, skills/ and docs/ checked against `commands list`; Windows PowerShell 5.1 JSON-quote note added.
