# Tooling Notes

## rea (plugin `rea@rea` v6.9.0)
- Installed as a Claude Code plugin (user scope).
- **Status 2026-11-15: MCP server fails to start.** Launcher error:
  `REA requires Node.js 22.x (>=22.23.4), 24.x (>=24.22.1), 26.12.1+; this process runs Node.js 24.21.0.`
  Fix: upgrade Node to >= 24.22.1, then restart Claude Code.
- **Scope:** rea's own skill says "Skip REA for ordinary source-repository architecture analysis."
  It is for shipped binaries, packaged apps, decompilation, runtime observation. In this project it fits:
  bundled binaries/models inside repos, closed-source tools we want to match (e.g. commercial clippers'
  desktop apps), and runtime observation of a repo's app while it runs.
  Source-code analysis uses normal repo reading.

## Superpowers skills
- `superpowers:brainstorming` → turns findings into feature design → `features/`.
- `superpowers:writing-plans` → implementation plans after specs approved.

## Model tiering (user rule)
| Work | Model |
|---|---|
| Deep analysis of large repos, architecture, synthesis | Opus |
| Medium repo analysis, spec drafting | Sonnet |
| Mechanical inventories, greps, formatting | Haiku |

Research run 2026-11-15: Opus → hypit, supoclip, openshorts; Sonnet → bridgeclip,
opensource-clipping, autoclip, ViralCutter; Haiku → license/dependency inventory.
