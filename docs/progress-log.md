# Progress Log

## 2026-11-15
- Cloned 7 repos (shallow) into `repos/`: hypit (Apache-2.0, TS), openshorts (MIT, Py),
  supoclip (MIT, Py+TS), autoclip (MIT, Py), ViralCutter (MIT, Py), bridgeclip (MIT, Py+TS),
  opensource-clipping (MIT, Py).
- **License correction (verified by reading LICENSE files):** supoclip = **AGPL-3.0**, ViralCutter =
  **GPL-3.0**, hypit = **modified Apache-2.0** (no multi-tenant/SaaS or commercial redistribution
  without a commercial license), openshorts `cloud/` has its own separate LICENSE. My first quick scan
  wrongly said MIT (it matched "mit" inside "permitted"). Details: `findings/00-license-and-dependency-inventory.md`.
  Consequence: copy code only from MIT repos; from AGPL/GPL/modified-Apache repos, study ideas and
  reimplement independently (unless the user decides the whole product is GPL/AGPL and personal-use).
  ultralytics (YOLO) is AGPL-3.0 — avoid as a dependency, or isolate it.
- Wrote `docs/research-methodology.md` (shared evaluation template, categories A–I).
- Started 7 per-repo analysis agents + 1 license/dependency inventory agent → `findings/`.
- rea MCP fails: Node 24.21.0 too old (needs >= 24.22.1). See `tooling-notes.md`.

- All 7 per-repo reports + license inventory landed in `findings/`. Cross-repo comparison matrix
  being written by an Opus agent → `findings/00-comparison-matrix.md`.
- Comparison matrix done: `findings/00-comparison-matrix.md` (scores, best source per capability,
  key choices, gaps, asset shortlist, draft feature list f01–f20). Claude's note: pull frame
  viewing/contact sheets (draft f05) into the first slice — "agent sees the video" is a core ask.
- Brainstorming decisions recorded in `docs/00-vision-and-requirements.md` (confirmed section).

- **User chose approach B: Rust core + Python engine** (personal tool). GUI must be a Premiere-style
  editor with multi-region layouts (e.g. webcam top / gameplay bottom); everything the GUI can do,
  the agent can do ("agent and software are one").
- Design doc drafted: `docs/superpowers/specs/2026-11-15-rustler-design.md`; feature index f01–f29 in
  `features/README.md`. Builder check (Opus reviewer) running.
- Rust: rustup had no toolchain → installed stable (rustc 1.100.0). Linking failed: MSVC C++ build
  tools missing → installing VS 2022 Build Tools (VCTools workload) via winget.

## Open questions
1. Upgrade Node for rea? (system change — needs user OK; unanswered)
2. (resolved) Design approved 2026-11-15.

## Resolved
- "discord" = discover (typo). Content type: any — full visual+audio+speech understanding.
- Use: user clips to earn, tool runs on each agent's own machine; reimplement non-MIT ideas.

## 2026-11-15 (later)
- Design approved by user. Slice 1 implementation plan written:
  `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` (13 tasks in 4 waves; Opus for contracts,
  render compiler and e2e; Sonnet for feature crates; Haiku for docs/feature files).
- Added quick-scan design (user idea): fetch YouTube transcript/heatmap/chapters without downloading,
  then download only chosen sections (`source.info`, `source.fetch_transcript`, `source.import --sections`).
- VS 2022 Build Tools MSVC 14.44 present (linker for Rust).
- Waiting: user review of the plan before implementation.
