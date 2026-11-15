---
id: hinglish
title: "Hinglish: transcription, romanisation and captions"
summary: "Language settings for Hindi-English mixed speech, how Rustler romanises Devanagari, searching romanised transcripts, and caption and hook tips for Hinglish audiences."
order: 10
tags: [hinglish, hindi, language, transcription, captions, india]
---

# Hinglish

Hinglish = Hindi and English mixed in one sentence ("bhai, ye business model hi broken hai"). It is
the default language of Indian podcasts, streams and creator content. Whisper handles it, but you
must steer it.

## 1. Transcription settings

```
rustler analyze transcribe src_A --language hi --model small      # Hindi-dominant
rustler analyze transcribe src_A --language en --model small      # English-dominant with some Hindi
rustler analyze transcribe src_A --language auto                  # unknown; check the result
```

| Speech | `--language` | Why |
|---|---|---|
| Mostly Hindi with English words | `hi` | Hindi words recognised correctly; English words usually come out in Devanagari or Latin, both fine after romanisation |
| Mostly English with Hindi phrases | `en` | English stays accurate; short Hindi phrases come out romanised or approximated |
| 50/50 or unknown | `auto`, then check | auto picks from the first ~30 s; if the intro is English and the rest Hindi, it guesses wrong |

Model size: `small` is the default and is usable. For important Hinglish jobs use `--model medium`
(slower, noticeably better on code-switching) when time allows. `base`/`tiny` are only for scanning.

Check the result: `transcript get src_A --start 300 --end 420`. Red flags:

- English translation instead of transcription (the speaker said Hindi, the text is fluent English)
  → `--language` was `en` on Hindi speech; rerun with `hi`.
- Garbled near-English words for Hindi speech ("tom at lub key") → rerun with `hi`.
- Very few words for dense talk → bigger model.

A new transcript means new units: re-read unit ids before creating clips. Transcription resumes
from saved chunk parts, so a rerun with a different `--language` or `--model` may reuse the old
parts: if it finishes instantly with the same text, check `rustler commands describe
analyze.transcribe` for a force/overwrite option, and report it if there is none. Decide the
language **before** the long run (from the title, the scan transcript or the brief) to avoid this.

## 2. Romanisation (Devanagari → Latin)

With the project caption script `latin` (default), Rustler transliterates Devanagari words to Latin
Hinglish spelling after transcription. The original is kept in `transcript.raw.json`.

| Devanagari | Romanised |
|---|---|
| क्या | kya |
| है | hai |
| नहीं | nahi |
| भाई | bhai |
| मैं | main |
| ज़िंदगी | zindagi |
| "bro क्या scene है" | "bro kya scene hai" |

Behaviour: lowercase, final silent "a" dropped (`kya`, not `kyaa`), long vowels simplified (`ii` →
`i`), nasal marks → `n`. English words in Latin pass through unchanged.

Known limits: spellings follow rules, not habits; some words come out differently from how people
type them ("accha" vs "achha", "hain" vs "hai"). Names and brands can be wrong in either script.

## 3. Searching

`transcript search` is case-, accent- and romanisation-insensitive, then fuzzy (≥ 0.8). Search in
Latin: `rustler transcript search src_A --query "kya scene hai"`. If a phrase isn't found, try a
shorter distinctive part ("scene hai") or common spelling variants.

For `--expect-text` use the words exactly as `transcript get` prints them (romanised), 2-4 words.

## 4. Picking Hinglish moments

Everything in `finding-hooks` applies. Extra signals that work for Indian audiences:

- Relatable middle-class pain: salary, EMI, rent, parents, arranged marriage, exams, jobs, startups.
- Desi humour: roasting, family-WhatsApp energy, regional references.
- Money numbers in lakh/crore ("20 lakh ka loss") are strong number hooks.
- Strong Hinglish openers (illustrative): "Sach bataun toh…" is weak; "Maine 20 lakh gawa diye ek
  galti se" is strong — a concrete claim in the first sentence.

Weak Hinglish openers to cut: toh, matlab, haan, na, accha, dekho, yaar, bas, aur, phir, woh, arre,
"toh basically", "matlab ki". Filler-ish tails: "na?", "samjhe?", "you know na".

## 5. Captions for Hinglish audiences

- **Default `--script latin`.** Most Hinglish social audiences read and type Roman Hinglish.
- Use `devanagari` when the brief says so or the content is pure Hindi for a Hindi-belt audience.
  Check a preview frame for □ boxes (missing glyphs) — the Devanagari font must be installed by
  `setup` (`doctor run` → `fonts`).
- Romanised Hindi words are longer than English ones; use `--max-words-per-line 2-3`.
- Uppercase presets (`bold-karaoke`) work for Hinglish; for long emotional passages `clean-white`
  in sentence case reads more naturally.
- Fix glaring spellings that change meaning or look wrong to natives (if a caption word-edit command
  exists); otherwise mention them in the report.
- Mixed scripts in one caption (some words Devanagari, some Latin) look broken. Pick one script.

## 6. Titles, descriptions and hashtags

- Write titles in the clip's language mix: Hinglish clip → Hinglish title in Latin script
  ("Isne 20 lakh gawa diye ek galti se 😳"), unless the brief wants English.
- Hashtags: mix English niche tags with Hinglish/Indian ones (#hinglish only if it fits;
  #indianpodcast, #startupindia, #desi, niche-specific). 3-5 total.
- Keep cultural context accurate; don't translate jokes into English in the title if the humour
  depends on Hindi.

## 7. Reading Hindi on-screen text (OCR)

`rustler analyze ocr src_X` reads Latin text by default and cannot read Devanagari. For Hindi titles,
subtitles or overlays pass the script: `--lang devanagari` (Devanagari recognition model), or
`--lang auto` (Latin first; on frames where it finds nothing or only weak text the Devanagari model also
runs and each line keeps the more confident reading, tagged `lang: "latin" | "devanagari"`). The model
(about 8 MB) downloads on first use and is pre-fetched by `rustler setup`. `analyze deep` accepts the same
`--lang`. Raw OCR text stays in Devanagari.
