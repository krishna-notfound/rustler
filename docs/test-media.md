# Test media (real footage used to test Rustler)

The video itself is never committed. Downloads live in `~/.rustler/test-media/` (not in the repo).
Small recorded results of the real runs (face samples, diarization turns, speaker embeddings) are committed
as JSON fixtures instead, see the last section.

## Two-person interviews (speaker diarization, speaker colours, `crop track-speaker`)

### 1. "Vice President Vance Holds a Fireside Chat" (White House, 2026-09-29)

- Page: https://commons.wikimedia.org/wiki/File:Vice_President_Vance_Holds_a_Fireside_Chat.webm
- Download (no login): https://upload.wikimedia.org/wikipedia/commons/0/0d/Vice_President_Vance_Holds_a_Fireside_Chat.webm
  (177,509,037 bytes, AV1 1920x1080 + Opus, 14 min 45 s, sha256 `a86b9a6be9909025f45d0575731d54ca7306af623b2186571094422bf0d25385`)
- Source on Commons: the White House YouTube channel, https://www.youtube.com/watch?v=wFXnUw93jsA
- License: **public domain**, Commons template `{{PD-USGov-POTUS}}` (work of the Executive Office of the President); read on the file page, 2026-11-15.
- Used excerpt: source seconds 30 to 125, scaled to 1280x720 H.264 (`vance_twoshot_95s.mp4`, 95 s):

  ```
  ffmpeg -ss 30 -t 95 -i vance_full.webm -vf scale=1280:-2 -c:v libx264 -crf 20 -preset veryfast -c:a aac -b:a 128k -movflags +faststart vance_twoshot_95s.mp4
  ```

  (sha256 of the excerpt on the dev machine `ca9121527c75e2d607a3b3fa43d895f49ce29726cc763db3cd87b543c78272b8`; an x264 build change changes it.)
- What is in it: host (left chair) and guest (right chair) in a **wide two-shot** for most of the excerpt (faces only 4-6 % of the frame
  width, so lip movement is invisible), plus **cuts to a close-up of the guest** (about 60-67 s and 90-95 s of the excerpt) and one stage-entrance
  shot at the start. Host speaks about 6-8 s and 17-40 s of the excerpt (two male voices, reverberant hall, applause at the start); the guest speaks the rest.
- Ground truth used for the accuracy numbers (written from the transcript and from looking at frames, not from the tools): host 5.9-7.6 s and
  17.1-39.8 s, guest 8.3-16.5 s and 41.0-95.0 s (excerpt time).

### 2. "Vice President JD Vance joins Breitbart's Matthew Boyle for a Fireside Chat" (White House, 2025-11-20)

- Page: https://commons.wikimedia.org/wiki/File:Vice_President_JD_Vance_joins_Breitbart%E2%80%99s_Matthew_Boyle_for_a_Fireside_Chat.webm
- Download (no login): https://upload.wikimedia.org/wikipedia/commons/6/67/Vice_President_JD_Vance_joins_Breitbart%E2%80%99s_Matthew_Boyle_for_a_Fireside_Chat.webm
  (281,078,195 bytes, AV1 1920x1080, 27 min 39 s, sha256 `8fd37624d0d8fc19a9f9c16f07f1f6ee6a991b0f8fba933b6b3fe4c766224edb`)
- Source on Commons: https://www.youtube.com/watch?v=t5v-NYVaHzs (White House channel)
- License: **public domain**, `{{PD-USGov-POTUS}}`; read on the file page, 2026-11-15.
- What is in it: medium two-shot (both chairs in frame, faces about 5-7 % of the width), very wide shots of the stage, close-ups of each speaker;
  a host who asks longer questions. See the progress log for the excerpt and the numbers.

### Looked at and not used

- "David Cox and Yoshua Bengio Fireside Chat" (IBM Research, Commons, CC BY 3.0, 730 MB): pure cut-between-speakers editing with
  close-ups only; not needed once the Vance footage turned out to contain cuts as well.

## Other test media on the dev machine

`jfk.wav`, the JFK demo mp4, `lincoln.jpg` (portrait, public domain) and `stream_demo.mp4` are listed in `docs/agent-brief.md` and the progress log.

## Fixtures committed from the real runs

| File | What it is |
|---|---|
| `crates/rustler-commands/tests/fixtures/speakers/vance_wide_active.json` | `speakers.active` output for the 95 s excerpt (5 fps; faces, person ids, mouth scores) |
| `crates/rustler-commands/tests/fixtures/speakers/vance_wide_turns.json` | `speakers.diarize` turns for the same excerpt (S1 guest, S2 host) |
| `engine/tests/fixtures/vance_window_embeddings.json` | 24-d random projection of the 1.5 s-window speaker embeddings of the excerpt, with host/guest labels from the ground truth |
