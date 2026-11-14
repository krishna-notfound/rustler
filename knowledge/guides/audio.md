---
id: audio
title: "Audio: loudness, music beds, ducking and silence"
summary: "Loudness targets (-14 LUFS, -2 dBTP), music bed at -18 dB with ducking, denoise, per-segment volume, sound effects on words, avoiding copyrighted music, and silence."
order: 8
tags: [audio, loudness, music, ducking, silence, copyright, denoise, sfx, volume]
---

# Audio

Bad audio makes viewers swipe faster than bad video. Short-form apps play clips back to back, so a
clip that is quieter than its neighbours feels weak and one that is louder feels harsh.

## 1. Loudness targets

Every render applies loudness normalisation by default (clip `audio` settings):

| Setting | Default | Notes |
|---|---|---|
| `loudnorm` | true | leave on |
| `target_lufs` (integrated) | **−14 LUFS** | the common short-form/streaming target |
| `true_peak` | **−2 dBTP** | −2 rather than −1 because AAC encoding adds inter-sample peaks |
| `lra` | 11 LU | loudness range |

QC measures the finished file with `ebur128`:

- `loudness` **warn** when integrated is outside −14 ± 1.5 LU, or true peak above −1.0 dBTP.
- A warn usually means a very dynamic clip (whisper then scream). Fixes: cut the extreme part, keep
  the clip shorter, or accept the warn if it sounds fine — and say so in the report.
- Never turn off `loudnorm` to "fix" a warning.

## 2. Music beds

Talking clips usually do **not** need music. Add a bed when the clip is slow, emotional, a montage,
or has silent visual stretches, or when the brief asks.

```
rustler audio add-music clp_X --path "D:/music/calm-lofi-01.mp3" --volume-db -18 --duck true
rustler audio remove-music clp_X
```

| Parameter | Start value | Adjust |
|---|---|---|
| `volume_db` | **−18 dB** | −22 for dense speech, −14 to −12 for montages with little speech |
| `duck` | true | lowers music while someone speaks (sidechain compression: threshold 0.08, ratio 5, attack 100 ms, release 1000 ms) |

Rules:

- Speech must stay clearly on top: music under speech should be about 15-20 dB below the voice.
  If you strain to hear a word, the bed is too loud.
- Match the mood: tense story → low drones; funny → light/quirky; motivation → building beat;
  emotional → soft piano. Music fighting the mood is worse than no music.
- Fades in/out are added automatically; the bed should not start on a loud downbeat over the hook.
- Never add a bed over a source that already has music (double music sounds amateur).

## 3. Copyright

- Use only music the user owns or that is licensed for this use (royalty-free libraries with a
  licence that covers social posting, the asset library's licensed tracks, or the user's own files).
- **Do not add popular commercial songs.** Platforms mute or block them and campaigns reject the clip.
- If the user wants trending sounds, they add them **in the app at upload time** from the platform's
  licensed library. Suggest a sound type in the report instead ("add a trending lofi sound in-app").
- Source clips with loud copyrighted music in the background (stream music, a song in a vlog) are
  risky: prefer other moments; if unavoidable, flag it in the report.

## 4. Silence and pauses

`analyze audio` gives `silences` (below −35 dB for ≥ 0.6 s) in the source. QC flags `long_silence`
(below −45 dB for ≥ 1.5 s) in the render.

- Dead air at the start is a hook killer: the first word should arrive within 0.15 s.
- Inside a clip, use `edit remove-pauses --min-gap 0.5` (if available) on talking heads; keep
  pauses after punchlines and before reveals (0.3-0.8 s). Details in `cut-points`.
- A QC `long_silence` warning with `fix_hint "consider cutting pause at <t>"` → map `<t>` (output
  time) to the segment with `clip show`, then trim/split there.
- Do not cut silence in screen-share/tutorial clips where viewers are reading.

## 5. Sources with problems

| Problem | What to do |
|---|---|
| No audio track | Render adds a silent track; add a music bed or skip the source for talking content |
| One channel only / mono | Fine: output is stereo 48 kHz AAC 160 kbps |
| Heavy noise (hiss, hum, fan) | `rustler audio denoise clp_X` (section 7); echo/reverb is not fixed by it, prefer cleaner moments |
| Clipping/distortion in the source | Can't be repaired by loudnorm; avoid or accept and flag |
| Game audio drowns the streamer | Prefer moments where the voice is clear; flag it |
| Big volume jumps between segments of a montage | Loudnorm evens the whole clip, not each segment: balance them with `rustler audio volume clp_X --segment-id seg_Y --db -4` (section 7); avoid joining a whisper to a shout |

## 6. Sound effects

Use them sparingly: 0-3 per clip, on real beats (a reveal, the cut to a reaction, the punchline
word). Overuse looks cheap. Only use files you are licensed to use (CC0 packs, the user's own).

```
rustler audio add-sfx clp_X C:/sfx/whoosh.wav --at 2.5                       # edit-timeline seconds
rustler audio add-sfx clp_X C:/sfx/impact.wav --at-word '{"source_id":"src_Y","word_index":42}' --volume-db -10
rustler audio list clp_X                                                       # resolved times, problems
rustler audio remove-sfx clp_X sfx_1
```

- `--at` is on the **edit timeline** (what `clip show` prints), the same timeline as overlays and
  effects. Default gain is −6 dB; the range is −60..12.
- `--at-word` anchors the effect to the start of a transcript word (`transcript get` shows word
  indices). It is re-resolved at every render, so after `edit trim`/`split`/`remove-words` it still
  lands on the word; if the word is cut out, the render skips it with a warning and `audio list`
  shows `problem`.
- Effects are mixed in after denoise, outside the music ducking sidechain (they never duck the
  music), and before loudnorm. Placement is sample-exact (`adelay` in samples).
- Whoosh on a transition: put it a little before the cut, e.g. `--at <cut time - 0.15>`.

## 7. Denoise and volume

```
rustler audio denoise clp_X                      # medium (default)
rustler audio denoise clp_X --strength light     # light | medium | strong | off
rustler audio volume clp_X --segment-id seg_Y --db -4   # one segment quieter (0 resets)
rustler audio volume clp_X --db 2                # whole dialogue (little effect while loudnorm is on)
```

- Denoise = 80 Hz high-pass + FFT denoise (`afftdn`). At render a fast audio-only pass measures
  the dialogue noise floor (10th percentile of 50 ms windows, never closer than 12 dB to the median
  window) and sets the denoise threshold from it, so it adapts to each clip and to re-cuts.
  `render run` reports `noise_floor_db` and the `denoise_filter` it used.
- Measured on this machine: light / medium / strong remove about 12 / 20 / 30 dB of white hiss
  and 8 / 12 / 19 dB of pink noise under real speech, with the speech level unchanged.
- Start with `light` or `medium`. `strong` can sound watery on voices — listen to a draft render.
- Denoise does not remove echo, music or crowd noise; prefer cleaner moments for those.
- Per-segment volume balances takes before loudnorm sets the overall level; use it when one
  segment is clearly quieter or louder than its neighbours.

## 8. Transitions change the audio timeline

`transition set` overlaps two segments (`acrossfade` on the audio, `xfade` on the picture), so the
rendered file is shorter than the edit timeline by the overlap (`transition list` shows both).
You keep giving times on the edit timeline; the render moves captions, overlays, sound effects and
effects with the picture. `audio list` shows `rendered_duration` and each effect's `rendered_at`.