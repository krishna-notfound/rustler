# Docs Index

Read this first when resuming work. Keep it current.

| File | Purpose |
|---|---|
| `00-vision-and-requirements.md` | User's vision + hard requirements (source of truth for intent) |
| `research-methodology.md` | How every repo is evaluated; findings file structure; scoring scale |
| `tooling-notes.md` | Tools/plugins/models used in this project and their status (rea, superpowers, model tiering) |
| `progress-log.md` | Dated log of what was done, decisions, open questions |
| `superpowers/specs/2026-11-15-rustler-design.md` | **System design** (architecture, contracts, feature slices) |
| `../features/README.md` | Feature index f01–f29 with slices, dependencies, status |
| `../findings/00-comparison-matrix.md` | Research synthesis: best source per capability, choices, gaps, assets |

## Workspace layout
```
repos/      cloned upstream repos (read-only reference, never edited)
findings/   one research report per repo + comparison matrix + license inventory
features/   feature specs (fNN_*.md) produced from brainstorming
docs/       vision, methodology, decisions, progress (this folder)
context/    project context template files (to be filled once architecture is decided)
```

## Phase status
| Phase | Status |
|---|---|
| 1 Research | done for 7 repos — see `findings/00-comparison-matrix.md`; gap research pending |
| 2 Architecture | approved — `superpowers/specs/2026-11-15-rustler-design.md` |
| 3 Specs (features/) | design approved; f01–f09 detailed in Slice 1 plan |
| 4 Implementation plan | Slice 1 plan written, awaiting review |
| 5+ Implementation | not started |
