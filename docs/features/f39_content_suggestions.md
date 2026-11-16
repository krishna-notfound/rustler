# f39 — Content suggestions: bad takes, intro-trailer skip, hook and cold-open suggestions, caption fragments

**Status:** Built  |  **Depends on:** f12, f16, f20, f31, f33, f37  |  **Owner:** Opus agent (branch `worktree-agent-ae468d7e5aeb2d47b`)

## Purpose

Trial 3 (`docs/research/trial-3-2026-11-17.md`, improvements 1, 2, 4, 5, 6): `clips.auto` picked a moment from the
source's own intro trailer, every clip lacked a hook title, clips opening on the host's question scored 57-58, and
one caption chunk started with a fragment ("-UP, BUT MOST"). User rule: **Rustler suggests, the agent decides** —
nothing here applies a cut, a text or a re-order, except the intro-trailer skip in ranking (reported, with an
opt-out) and the caption fragment fix (a rendering correctness fix).

## Scope

**In:**
1. `edit.find_bad_takes <clip> {min_confidence?}` — read-only. From the clip's words (exact timings): `stutter`,
   `restart`, `false_start`, `repeated_sentence`; each with source and edit-timeline times, `cut_text`,
   `kept_text`, `word_indices`, `confidence`, `removes_seconds`, `why` and the exact
   `rustler edit remove-words <clip> --word-indices '[...]'` that removes the first attempt (keeps the later take).
   Approximate (captions-based) timings and sources without a transcript are skipped and named. The same
   findings appear in `clip.retention` as `take_fixes` (action `bad_take`) and in `clips.auto` `fixes_suggested`
   (never applied).
2. Intro-trailer detection (`intro_trailer.rs`): in `highlights.candidates` windows overlapping an opening montage
   are not ranked (`include_intro: true` keeps them); windows holding the original of a trailer line get +5 points
   (`trailer_line` part, reason names the trailer time); output `intro_trailer` (range, repeats with early/later
   times and text, scene cuts/min as evidence, skipped window count, note). `clips.auto` passes `include_intro`,
   copies the report to `ranking.intro_trailer`, and warns when an agent `range` sits in the trailer.
3. `clips.auto` hook + cold-open suggestions (`hook_suggest.rs`): per clip 2-3 `hook_title` lines (≤ 7 words, plain
   text; from the opening question and the strongest lines) with `rustler overlay add-text <clip> "<line>" --style
   hook-big`, and a `cold_open` (`rustler edit cold-open <clip> --start S --end E`, source seconds of the strongest
   answer line, 2-9 s) when the clip opens on a question. In real runs from the created clip's words; in a dry run
   from the ranking transcript with `<clip_id>` (URL: seconds relative to the section to download).
4. Caption fragments: `rustler_captions::join_fragments` joins a hyphen tail onto the word before it ("chop" +
   "-up," → "chop-up,"), drops one at the very start, strips the dash from a loose one; `build_caption_words`
   uses it and drops the clip's first word when the clip starts well inside it (> max(0.1 s, 30 % of the word)).
5. `edit.remove_words`: neighbouring removed words (transcript neighbours) are cut as one span including the gap
   between them, so a bad-take cut leaves no sliver of breath.

**Out:** applying any of the suggestions; burned-in source text/subtitles (improvement 3); QC silence alignment
(improvement 7); meaning-level judgement (the detectors read words, not meaning).

## User-facing Behavior

```
rustler edit find-bad-takes clp_X [--min-confidence 0.4]
rustler clip retention clp_X                      # take_fixes: bad_take with commands
rustler highlights candidates src_X [--include-intro true]
rustler clips auto --source-id src_X [--include-intro true] [--dry-run]
```

## Routes / Files

**Owned files (new):** `crates/rustler-commands/src/takes_cmds.rs`, `src/intro_trailer.rs`, `src/hook_suggest.rs`,
`crates/rustler-commands/tests/takes_hooks_intro.rs`, `docs/features/f39_content_suggestions.md`.

**Shared edits (minimal):** `src/lib.rs` (modules + register), `src/edit_cmds.rs` (remove_words span merge),
`src/director_cmds.rs` (`take_fixes` field), `src/highlights_cmds.rs` (intro skip, `include_intro`,
`intro_trailer`, `trailer_line`), `src/auto_cmds.rs` (`include_intro`, `ranking.intro_trailer`, suggestions),
`src/clip_cmds.rs` (`build_caption_words`), `crates/rustler-captions/src/chunk.rs` + `lib.rs`
(`is_continuation`, `join_fragments`), guides `cut-points`, `director-retention`, `clips-auto`,
`cli-reference-conventions`, `knowledge/README.md`, `docs/features/README.md`.

## Data and Interfaces

- `takes_cmds::{detect(&[ClipWord], min_conf) -> Vec<Hit>, findings, clip_words, for_clip, retention_fixes}`.
  Confidence: stutter 0.75 (0.3 for words doubled on purpose: very, really, no, yeah, had, that, ...); restart
  0.6 + 0.1 per matched word over 2 (max +0.2) + 0.1 after a pause/comma/dash; false start 0.45 + same bonuses,
  -0.25 when the phrase is only function words; repeated sentence 0.8 (0.65 when the second extends the first).
  A phrase in a stretch repeated ≥ 2.5 times (said three times) is emphasis: confidence 0.35 and a note.
  Windows: restart within 4 s and 8 words; repeated sentence within 12 s.
- `intro_trailer::detect(words, duration, scene_starts) -> Option<IntroTrailer>`: sources ≥ 360 s; 5-word phrases
  in the first 180 s found again ≥ 90 s later; a repeat = ≥ 3 consecutive phrases at the same offset (± 6 s;
  the later copy whose next words agree longest is preferred); first cluster (gaps ≤ 30 s) starting ≤ 60 s in;
  needs ≥ 15 s of repeated speech or ≥ 3 repeats; starts at 0 when the first repeat is ≤ 20 s in.
- `hook_suggest::{shorten, suggest}`; `auto_cmds::content_suggestions(store, clip_id)`.

## Acceptance Criteria

- [x] find_bad_takes lists restart/stutter with exact commands and never changes the clip; the command removes the
  first take as one span; approximate timings are skipped and named.
- [x] clip.retention lists `take_fixes`; clips.auto lists them in `fixes_suggested` and never applies them.
- [x] highlights.candidates skips the intro trailer, reports it, rewards originals; include_intro keeps it;
  clips.auto never picks inside it.
- [x] clips.auto suggests 2-3 hooks (≤ 7 words, hook-big command) and a cold open on question openers; nothing applied.
- [x] Caption fragment "-up," joins its word; a leading partial word is dropped.

## Verification

`cargo test -p rustler-commands --test takes_hooks_intro` (5), `--lib` (takes_cmds 4, intro_trailer 2,
hook_suggest 3), `cargo test -p rustler-captions` (chunk 2 new). Real check on a copy of the trial-3 project
(`D:\rustler-build\takes\project`, Sam Altman episode) — see Status Log.

## Status Log

- 2026-11-17 — Built. Real check on a copy of trial-3's project:
  - Intro trailer on `url_877241ee` (68 min, captions): 00:00-02:09.9 found (32 repeated lines, 108 s of speech);
    25 windows skipped; trial clip 1's moment (00:25, "Was there any point ever...") is no longer a candidate and
    its original at 27:27 is (rank 3, reason "the source's intro trailer features this line at 00:25").
    `clips auto --url ... --count 3 --dry-run` now picks 06:03, 27:27, 34:14 (trial run picked 00:25, 09:31, 56:48).
  - Bad takes on the three trial clips (whisper base section transcripts): clips 1 and 2 none; clip 3 only
    "Listen to less advice" said three times (deliberate emphasis: confidence 0.35, hidden at the default 0.4).
    Whisper's text drops many disfluencies, so few are visible; the captions transcript shows "I I like" but its
    timings are approximate (skipped).
  - Hooks (dry runs on the three sections): useful lines such as "Biggest misunderstanding about you in the
    world?", "Worst advice you've gotten as a founder?", "People didn't want to launch a chatbot"; weak ones
    too ("They tell you that you're digital sometimes?" from a whisper mishearing of "delusional"). Cold opens
    suggested on all three question openers; their quality was not judged by watching.
  - Caption fragment: the trial's "-up," (whisper split "chop-up") now joins "chop" (unit + integration tests);
    clips were not re-rendered.
