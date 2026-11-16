# f33 — Smart defaults at clip.create + light active speaker + layout plan

**Status:** Built  |  **Depends on:** f14 (camera planner), f19 (cuts), f29 (speaker crop helpers), config settings  |  **Owner:** smart-defaults agent (wave D)

## Purpose

Muse items 1, 2, 5 (partly), 6, 16 and v2 design §5.5(1)(2): a clip comes out right with **no manual layout or
framing calls**, while the agent can override every automatic decision. Lightweight: lite install only (OpenCV +
YuNet), no diarization or recognition model.

## Scope

**In:**
- Engine `faces.active` (`engine/rustler_engine/active.py`): one sequential pass; faces + landmarks at 5 fps, mouth
  motion from the mouth-corner landmarks (head motion subtracted, contrast-normalised), positional tracks,
  optional luma-normalised cut scan (frame-accurate peaks; dark footage) and graphics stats.
- `crop.track_active {clip_id, segment_id?, style?, switch?, look_room?}`: light active speaker. Mouth energy
  compared between visible faces during speech (transcript words → audio silences → none), hysteresis (≥ 1 s of
  the new face's own lead; ambiguous keeps current; silent mouth movement down-weighted), switches snap to word
  starts, then the smooth camera planner (`camera::plan_detailed`) with speaker-switch and shot-cut events.
- `layout.plan {clip_id, apply?}`: per-shot plan (one face → center-crop + smooth track; ≥ 2 → center-crop +
  active speaker; no face / graphics → blur-fit, zoom-fit when the content is centred); splits segments only where
  the layout changes (changes < 1.2 s absorbed). Proposes by default; `--apply` saves a revision + crop paths.
- `clip.create` smart defaults unless `--raw` or setting `clip.smart_defaults=false`: layout plan (skipped when
  `--layout`/`--layout-params` given), filler removal (`data/fillers_{en,hi}.txt`), pause trim > 1.0 s to 0.15 s.
  Each step is a revision; `smart.decisions[]` lists every decision with `override_with` commands; `smart.undo_all`.
- Guides: `layouts-and-framing` (smart defaults, layout plan, track-active), `start-here` (loop).

**Out:** bad-take/repeat detection (cleanup task), caption auto-position (hooks/captions task), templates.

## Routes / Files

**Owned files:** `crates/rustler-commands/src/smart_cmds.rs` (new) + registration in `lib.rs`; hook in
`clip_cmds.rs` (`ClipCreateIn.raw`, `ClipView.smart`, call after save); `crates/rustler-commands/data/fillers_{en,hi}.txt`;
`crates/rustler-commands/tests/smart.rs`; `engine/rustler_engine/active.py` + `faces.active` in `rpc.py` +
`engine/PROTOCOL.md` section; `engine/tests/test_active.py`. One-line fixture change in `tests/fix.rs` (`raw: true`).

## Rules and Edge Cases

- No engine / missing media / vertical source → layout skipped with a `skipped` decision; cleanup still runs.
- Approximate word timings (caption-based transcripts) → no filler/pause trim (decision says why).
- `zoom-fit` is stored as a region layout with `preset: "zoom-fit"` (not a `layout::PRESETS` entry; override with
  `clip set-layout --preset blur-fit|center-crop`).
- The report is stored in extras `smart_defaults` only when something was applied.

## Acceptance Criteria

- Synthetic: one moving face → center-crop + smooth path following it; two faces with alternating mouths → 3 turns
  L/R/L with switches within 0.7 s; face then text card → 2 segments (center-crop, zoom-fit/blur-fit) split at the cut.
- `--raw` and `clip.smart_defaults=false` keep the raw clip; filler files load; undo commands restore.

## Status Log

- 2026-11-17: built. Real check (temp copy of `rustler-real1` + the hand-labelled Vance/Boyle excerpts), % of 1-s
  samples in two-shots with the speaker's face inside the crop: trial two-shots (Vinh speaking) track-active
  14/14, 8/8, 6/6 vs track-speaker 0/14, 8/8, 6/6; Vance excerpt 53/70 (76 %) vs 70/70; Boyle 28/43 (65 %) vs
  43/43. Light mouth motion is reliable on medium faces, weak on 4-6 % wide-shot faces (switches ~2 s late, no
  identity across cuts) — there `crop track-speaker` (diarization + SFace) stays better.
