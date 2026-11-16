# f31 — Director layer: retention map, agent score, campaign analysis

**Status:** In progress  |  **Depends on:** f18, f20, f21, f22  |  **Owner:** director agent (wave C)

## Purpose

The user's priority (v2 design §5.9): *"if we can't make someone hold and watch our reel, we won't go viral."*
The agent acts as a director. Rustler measures and advises, deterministically and locally (no AI APIs); the agent
judges. After this feature the agent can measure where a clip loses attention (`clip.retention`), record its own
4-part score (`clip.score_set`), and turn a brief/campaign into a strategy and checklist (`campaign.analyze`).

## Scope

**In:**
- `clip.retention {clip_id, render_id?, timeline_only?}` — per-second retention map of the OUTPUT clip, derived
  metrics (first-frame hook, speech start delay, dead zones, ending type, pacing, caption density), a 0–100
  retention score with a per-factor breakdown, and exact fix commands per dead zone. Light: no engine; one ffmpeg
  frame-difference pass + one audio pass when a current render exists.
- `clip.score_set {clip_id, hook, flow, value, trend, notes?}` — agent's score (0–25 each) in extras `meta.score`.
- `campaign.analyze {campaign_id?, brief?, source_minutes?}` — hard rules, duration window, platform, payout →
  strategy, recommended clip count, required mentions/hashtags/credit, checklist.
- `export.clip` report gets a `score` field (agent score + last retention score).
- Knowledge: guide `director-retention`, prompts `direct_campaign`, `hook_lab`.

**Out (explicitly):** `edit.cold_open` (timeline_edit_cmds, other agent), export ordering by score, GUI, campaign CRUD
(task 1.7, other agent; read leniently here).

## Routes / Files

**Owned files:**
- `crates/rustler-commands/src/director_cmds.rs` (new) + registration line in `crates/rustler-commands/src/lib.rs`
- `crates/rustler-commands/tests/director.rs` (new)
- `crates/rustler-media/src/retention.rs` (new) + `pub mod retention;` in `crates/rustler-media/src/lib.rs`
- one line in `crates/rustler-commands/src/qc_cmds.rs` (export report `score`)
- `knowledge/guides/director-retention.md`, `knowledge/prompts/direct_campaign.md`, `knowledge/prompts/hook_lab.md`

**Read-only dependencies:** `fx_cmds` (TransitionMap, effects), `overlay_cmds`, `clip_cmds` (caption words),
`brief_cmds` (Brief), `analysis/<source>/{scenes,audio_events}.json`, `rustler_media::analyze_audio`.

## Data and Interfaces

- Time base: everything in `clip.retention` is on the **rendered** timeline (after transitions). Fix commands are
  converted back to the edit timeline (what `effect.add` / `overlay.add_text` take).
- extras `meta` (object, other sub-keys preserved): `meta.score = {hook, flow, value, trend, total, notes?, by, at}`,
  `meta.retention = {score, render_id?, measured_on, at}` (written by `clip.retention`, not in dry runs).

## Rules and Edge Cases

1. Visual change = segment cut (different source / gap > 0.25 s / layout change), transition, scene cut inside a
   segment (source `scenes.json`), effect start (punch-in also its end; zoom-slow/shake keep motion over their
   range), text overlay / image / cutaway appearing, frame-difference spike on the render (`scene > 0.25`).
2. Dead zone: > 2.5 s without a visual change (static) or ≥ 0.8 s of silence.
3. The latest render is used only when no clip edit was logged after it (else timeline-only + warning).
4. No transcript → speech metrics null and neutral points, with a warning; no audio data → neutral energy.
5. `score_set`: each part finite and 0..=25, else `validation`.

## Acceptance Criteria

- [ ] Synthetic clip: static dead zone detected; after the suggested `effect.add` the zone is gone and the score rises.
- [ ] First-frame hook detection (overlay at 0, fast speech, none); ending type (loopable / hard stop / cta).
- [ ] `campaign.analyze` with and without a brief; with a campaign file.
- [ ] `score_set` validation 0–25; score in `clip.show` extras and export report.
- [ ] Real run on a copy of rustler-real1: 5 clips scored; dead zones checked against contact sheets.
- [ ] `cargo test -p rustler-commands --test director`, `cargo test -p rustler-media retention`, `cargo test -p rustler-knowledge` pass.

## Status Log

- 2026-11-16 — feature file written; build started.
