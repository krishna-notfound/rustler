# Agent Context (non-Claude agents)

Rustler is driven by an agent through the `rustler` CLI (`rustler commands list` for discovery) or the MCP
server (`rustler mcp serve`). This file is the read order for any agent working on the Rustler repo itself.
Claude Code reads `CLAUDE.md` at the repo root instead.

## Application Building Context

Read the following files in order before implementing or making any architectural decision:

1. `docs/README.md` — documentation index and repository layout
2. `docs/progress-log.md` — what is built and tested (the current truth)
3. `docs/00-vision-and-requirements.md` — the user's vision and confirmed decisions
4. `docs/superpowers/specs/2026-11-15-rustler-design.md` — approved design
5. `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` — plan; read the "5-HOUR SCOPE" section last
6. `docs/features/README.md`, then only the `docs/features/fNN_*.md` file(s) for the current task
7. `docs/context/project-overview.md` — product definition, goals, scope
8. `docs/context/architecture-context.md` — crates, data model, invariants
9. `docs/context/ui-context.md` — the simple web GUI
10. `docs/context/code-standards.md` — implementation rules and definition of done
11. `docs/context/ai-workflow-rules.md` — workflow, scoping, parallel work
12. `docs/context/progress-tracker.md` — pointer to the progress log, open questions, deferred work

Update `docs/progress-log.md` (dated entry) and the feature file's *Status Log* after each meaningful change.

If implementation changes the architecture, scope or standards documented in `docs/context/`, update the relevant
file before continuing.

## Using Rustler as an editing agent (v2)

If your task is to *make clips* (not to change the repo), read `skills/rustler/SKILL.md` and `rustler guide show start-here`.
v2 capabilities: smart defaults at `clip create` (layout and framing per shot, filler/pause trim; `--raw` off), templates
(`template apply`, starters `podcast-viral` and `streamer-split`, `clip create --template`), the whole-clip timeline document
(`timeline show/schema/apply`, cold open, insert, speed, freeze, merge), retention/director tools (`clip retention`, campaigns),
hook styles, keyword captions, flash/whip transitions, `clip cover`, and per-platform post JSON (`clip meta-set`).
178 commands; `rustler commands list` is the authority.

## Rules that apply to every agent

- No AI API calls. Rustler runs local tools only.
- Never invent command names, flags or file paths; check `rustler commands list` and the crates.
- Never publish or upload output.
- Build with `cargo build --workspace` from PowerShell on Windows; test with `cargo test --workspace`.
