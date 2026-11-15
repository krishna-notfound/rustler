# Docs Index

**All project documentation lives in `docs/` and is kept updated after every merge** (user rule).
Read this file first when resuming work, then `progress-log.md`.

| File | Purpose |
|---|---|
| `00-vision-and-requirements.md` | User's vision + every confirmed decision (source of truth for intent) |
| `progress-log.md` | Dated log: what was built/merged, test results, problems, decisions, open questions |
| `superpowers/specs/2026-11-15-rustler-design.md` | **System design** (architecture, contracts, feature slices) — approved |
| `superpowers/plans/2026-11-15-slice1-first-real-clip.md` | **Implementation plan** (Slice 1 + 5-hour scope) |
| `agent-brief.md` | Rules every implementation agent reads first (toolchain, ownership, commits) |
| `MUSE.md` | **Give Rustler to Muse / any cloud agent: token, install, copy-paste prompt** |
| `agent-integration.md` | How to give an agent Rustler: CLI on PATH, MCP config per agent, Claude Code skill, sample session |
| `linux.md` | Running Rustler on Linux x86_64 without a GPU (cloud agents): install, build (incl. no-root zig route), setup, doctor, measured timings, known limits |
| `install.md` | Installing prebuilt release binaries (Windows zip, Linux tar.gz) with `scripts/install.ps1` / `scripts/install.sh`: private-repo token/`gh` auth, flags, manual download, how a release is cut |
| `desktop.md` | Desktop app (`rustler desktop` / `rustler-desktop`): native window + menu hosting the same GUI (wry/tao, WebView2), why not Tauri, Linux feature gate, how it was verified |
| `features/README.md` | Feature index f01–f29 with slices, dependencies, status |
| `research/00-comparison-matrix.md` | Research synthesis: best source per capability, choices, gaps, assets |
| `research/00-license-and-dependency-inventory.md` | Licenses of the 7 studied repos and their deps/assets |
| `research/<repo>.md` | One deep research report per studied repo |
| `research-methodology.md` | How repos were evaluated; scoring scale |
| `tooling-notes.md` | Plugins/tools/models used to build this (rea, superpowers, model tiering) |
| `context/` | Project context template files (CLAUDE.md read order) |
| `../knowledge/` | Agent knowledge book (13 guides + 8 MCP prompts) — product content embedded in the binary |

## Repository layout
```
docs/        all project documentation (this folder)
  research/    per-repo research reports, comparison matrix, license inventory
  features/    feature index + per-feature specs (fNN_*.md)
  context/     project context files
  superpowers/ design spec + implementation plans
crates/      Rust workspace: core, registry, tools, engine-bridge, media, captions, commands,
             cli (`rustler` binary), mcp, gui, knowledge
engine/      Python engine (faster-whisper, scenes, faces, Hinglish translit), driven by the Rust core
knowledge/   agent guides + MCP prompts embedded in the binary (`rustler guide …`, MCP prompts/resources)
repos/       cloned upstream repos studied in research (gitignored, read-only)
```

## Phase status
| Phase | Status |
|---|---|
| 1 Research | done for 7 repos — `research/00-comparison-matrix.md` |
| 2 Architecture | approved — `superpowers/specs/2026-11-15-rustler-design.md` |
| 3 Specs | design approved; f01–f09 detailed in the Slice 1 plan |
| 4 Implementation plan | approved; executing (Slice 1 + 5-hour scope) |
| 5+ Implementation | COMPLETE — all slices built, CLI (118 cmds) + full GUI; see `progress-log.md` |
