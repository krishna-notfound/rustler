---
id: audio
title: "Audio: loudness, music beds, ducking and silence"
summary: "Loudness targets (-14 LUFS, -2 dBTP), adding a music bed at -18 dB with ducking, avoiding copyrighted music, and handling silence and pauses."
order: 8
tags: [audio, loudness, music, ducking, silence, copyright]
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
rustler audio add-music clp_X --path "D:/music/calm-lofi-01.mp3" --volume-db -18 --duck true   # (if available)
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
| Heavy noise / echo | Prefer cleaner moments; a denoise command may exist later (`commands list` → `audio.*`) |
| Clipping/distortion in the source | Can't be repaired by loudnorm; avoid or accept and flag |
| Game audio drowns the streamer | Prefer moments where the voice is clear; flag it |
| Big volume jumps between segments of a montage | Loudnorm evens the whole clip, not each segment; compare segment loudness in `analyze audio` `curve` and avoid joining a whisper to a shout |

## 6. Sound effects

SFX (whoosh, ding, bass hit) can be added only if an SFX command exists (`audio.sfx`, check
`commands list`). Use them sparingly: 0-3 per clip, on real beats (reveal, cut to reaction).
Overuse looks cheap.
