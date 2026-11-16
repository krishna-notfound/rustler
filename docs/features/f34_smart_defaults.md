# f34 — Smart defaults at clip.create + light active speaker + layout plan

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
  `clip set-layout --preset blur-fit|center-crop`). Its source rect is the shot's content box (engine `g.box`,
  10th/90th percentile over the shot, 3 % padding), used when it gains >= 1.25x over blur-fit (capped 2.5x).
- Cuts: unconfirmed scan peaks need diff >= 0.3; where the faces change between two samples the strongest
  peak between them counts from 0.15; `scenes.json` cuts snap to the strongest peak within 0.4 s.
- After cleanup, pieces < 0.6 s join the segment that continues them in the source (decision `fragment`),
  unless their layout carries on from the piece across the cut.
- Camera: the face box keeps >= 8 % of the crop width inside both crop edges (`CameraParams.margin_frac`),
  measured on the face's robust extent (box size and position jitter over ±0.5 s, one outlier in five dropped);
  new framings aim at 14 % (`comfort_frac`), and an old framing carries over a shot cut only when it frames the
  new shot with that 14 %; a move running at a shot cut stops there. Speaker shot-change events snap to the
  frame-accurate cut between the two samples.
- Pop-ups: `faces.active` with `popups: true` finds subscribe / channel cards burned into the source; only
  `subscribe` cards (red button) are masked, with a solid box in the colour that was under the card, on
  single-region `center-crop` segments (the rect covers the card wherever the crop path puts it, pause-cut
  pieces joined), decision `popup` with `mask remove` / `mask edit --kind blur`. A plain white card is listed,
  not masked. Masks live in extras: `history restore` does not remove them (the decision's commands do).
- `override_with` lists have no repeated commands.
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
- 2026-11-16: trial-2 tuning (copy of the trial-2 project, clips 02/04/05 re-created with `clip create`, before vs
  after binary, same engine home). Graphics: 04 and 02 infographics zoom-fit to the content box x 0.22-0.78/0.80,
  y 0.06-1.00 (1.7-1.8x larger than blur-fit, logo out) instead of blur-fit. Clip 04 split moved 20.80 -> 19.84 s
  (first close-up frame; the cut's frame difference is 0.23, under the 0.3 floor; confirmed by the faces).
  Duplicate `override_with` entries gone (04: 2 commands/1 unique -> 2/2; 02: 4/2 -> 2/2). Fragment merge: the
  02 0.275 s piece is kept on purpose (merging it into the centre crop showed the wide shot cropped for 7 frames;
  verified on a preview frame). Face margin: 05 samples with the framed face < 6.5 % from a crop edge 19 -> 15;
  a margin move crossing a shot cut made 02 worse until moves stop at cuts (fixed, 0 cut-off faces).
- 2026-11-17: framing, speed and pop-ups (copy of the trial-3 project, the five trial ranges re-created with
  `clip create`, before = v0.2.1 binary, same engine home). **Face at the edge** (5 fps YuNet samples, face box
  < 6.5 % of the crop width from a crop edge): 05 15 -> 2, 01 1 -> 0, 03 1 -> 0, 02/04 0 -> 0; at 10 fps on 05
  26 -> 4 (worst -17 % -> +1.4 %). Causes: the margin was met on the median box while YuNet's box width jitters
  10-25 % (the camera parked exactly at the limit: real boxes 2-4 % from the edge); framings carried over shot
  cuts at the limit; a speaker shot change cut the crop at the samples' midpoint, 2 frames late (half a face at
  100.48 s). The 2 left are the last 0.3 s before a source cut. Motion: 05 moves/min 2.9 -> 4.4 (crop-path runs)
  / QC 2.19, jitter 0 -> 0, `camera_motion` pass; 03 2.5 -> 0, 02 11.8 -> 9.4, 01 9.9 -> 9.9.
  **Speed**: `faces.active` on 05 (99 s, 960x540 proxy) 11.6 s -> 6.0 s with identical samples, mouths, tracks
  and peaks (YuNet in 3 threads, cut scan in its own capture/thread, histogram once per thumbnail). The
  decoder's raw luma plane was tried for the scan and dropped: OpenCV returns it without B-frame reordering
  (cuts moved by 2-3 frames). `clip create` 05 on an idle machine 16.0 s -> 7.0 s.
  **Pop-ups**: whole sources: 6 cards found (02 0-2.8, 03 33.8-38.2, 05 0-3.4 / 36.8-41.0 / 75.6-80.0), all
  real subscribe cards (checked on frames), 0 false positives (01-05, incl. the 02/04 infographics); cards
  already on screen at the range start are found when they leave. In the trial ranges: 03 one card and 05 two
  cards masked (renders checked: card hidden, solid box in the set's colour; no card frame left at the edges).
