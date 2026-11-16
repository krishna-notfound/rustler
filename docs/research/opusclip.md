# OpusClip (opus.pro): how it makes clips, and what Rustler needs to match it

Researched 2026-11-16 from public pages only. No account was used and no clip was generated, so everything below is
documentation, marketing, changelog or third-party review, never first-hand output. Page text was read through a
summarising fetch tool, so quotes are paraphrases; numbers were copied as reported. Anything I could not confirm is
marked **UNVERIFIED**. Where two sources disagree, both are given.

Source key (used in citations below):

- HC = OpusClip help center, `https://help.opus.pro/docs/article/<slug>` (every article has a `.md` form). Index: https://help.opus.pro/llms.txt
- API = `https://help.opus.pro/api-reference/...`
- PRICE = https://www.opus.pro/pricing
- CHG = https://opusclip.canny.io/changelog
- Third-party reviews are named inline and treated as lower trust (several are competitors' blogs).

---

## 1. Pipeline: input to clip

1. **Input.** Link or upload. Sources: YouTube on Free; adds Google Drive, Vimeo, Zoom, Rumble, StreamYard on Starter; adds public MP4 URL, Dropbox, Twitch, Facebook, Instagram, TikTok, LinkedIn, X, Loom, Riverside, Frame.io, Medal.tv on Pro. Local upload limit 10 GB (Free/Starter) or 30 GB (Pro/Business). Max video length 10 hours; a time window can be chosen for longer ones ([PRICE](https://www.opus.pro/pricing), [HC video-length](https://help.opus.pro/docs/article/video-length.md)).
2. **Submit options** (the "Copilot" panel): clip model (ClipBasic or ClipAnything), optional genre, optional natural-language prompt (ClipAnything) or topic keywords (ClipBasic), clip length (default "Auto (0m-3m)"), optional time range, caption preset or none, Recipe + Brand Kit ([HC how-to-use-clipanything](https://help.opus.pro/docs/article/how-to-use-clipanything.md), [HC select-clip-length](https://help.opus.pro/docs/article/select-clip-length.md), [API create-project](https://help.opus.pro/api-reference/endpoints/create-project)).
3. **Understand.** Transcription (ASR; the 2024 "core AI engine upgrade" says Whisper large-v3 was added, errors cut 13.7%, a multimodal model for speaker tracking, and GPT-4 Turbo's 128k context for a "new curation prompt architecture", [CHG core-ai-engine-upgrade](https://opusclip.canny.io/changelog/core-ai-engine-upgrade)). ClipAnything also reads frames, audio events and sentiment (see section 2).
4. **Curate.** The model proposes moments, scores each (virality score), and applies a narrative structure (ClipAnything). Results are listed sorted by score, highest first ([HC virality-score](https://help.opus.pro/docs/article/virality-score.md)).
5. **Clip.** Each moment becomes a clip with auto reframe/layout, captions, optional auto headline or Viral Hook, optional B-roll, speech cleanup, transitions (all per Recipe/Brand Kit).
6. **Review/edit** in a web editor (transcript-based trim, "Add a section", timeline, text overlays, manual reframe), **export/post** ([HC trim-extend](https://help.opus.pro/docs/article/trim-extend-the-video.md)).
7. **Count of clips** depends on source length and model, e.g. 3-10 min gives 3-14 clips, 30-60 min gives 23-32, over 120 min gives 42-55; ClipAnything yields slightly more ([HC how-many-clips](https://help.opus.pro/docs/article/how-many-clips.md)).

OpusClip 4.0 (blog dated 2026-11-02 per search results) widened the product from "clip highlights" to "auto-edit raw talking footage": speech cleanup, captions, B-roll, Viral Hook, Motion Graphics (animated icons, numbers, charts from the spoken content), saved Recipes, and an "AI Producer" plugin usable from ChatGPT or Claude ([blog opusclip-4-0](https://www.opus.pro/blog/opusclip-4-0)). It is also an official Claude connector (2026-09-11 changelog entry) and has an MCP server ([CHG](https://opusclip.canny.io/changelog), [claude-connector](https://help.opus.pro/api-reference/claude-connector.md)). So OpusClip itself is moving to "agent-driven", which is the same direction as Rustler.

## 2. ClipAnything, curation and the virality score

**ClipBasic vs ClipAnything** ([HC clipanything-qa-2](https://help.opus.pro/docs/article/clipanything-qa-2.md), [HC clip-anything](https://help.opus.pro/docs/article/9947095-clip-anything.md)):

| | ClipBasic | ClipAnything |
|---|---|---|
| Content | talking heads | any genre incl. vlog, sport, news, music, low-dialogue |
| Prompt | keywords that occur in the video | free-form English sentence (subject, visual, action, sentiment, sound) |
| Vision | speaker positions only | objects, colours, actions, on-screen text (scoreboards, name cards, logos) |
| Audio | transcript only | who speaks, laughter, cheering, shouting, arguing, environment sounds, SFX, music |
| Sentiment | none | range of emotions |
| Narrative | none | "storyline structures from a library built with award-winning producers, matched to genre" |
| Reasoning | none | multimodal, across scenes |

- **Prompt manual** ([HC clip-anything-prompt-manual](https://help.opus.pro/docs/article/clip-anything-prompt-manual.md)): prompts find a moment, action, emotion, character, or a compilation ("Compile all Man City's scoring"). They must NOT try to edit visuals/audio, control length, or add cinematic effects. English only. Say what you want, not what to exclude.
- **Narrative templates** are selectable, "all suitable ones selected by default"; names not published in text I could read ([HC clipanything-qa-7](https://help.opus.pro/docs/article/clipanything-qa-7.md)). Template names: **UNVERIFIED**.
- **Virality score**: integer 0-99, "estimates potential for engagement and shares", not a guarantee. Four factors ([HC virality-score](https://help.opus.pro/docs/article/virality-score.md)):
  - **Hook**: does the opening grab attention and relate to the main topic.
  - **Flow**: logical progression and a satisfying end.
  - **Value**: offers value, resonates emotionally, connects.
  - **Trend**: aligns with current trends/audience interests.
  - With ClipAnything it also checks match to your prompt. The 2024 engine note says the score also considers visual cut quality ([CHG](https://opusclip.canny.io/changelog/core-ai-engine-upgrade)). Weights and the model are not published (**UNVERIFIED**). Score is hidden on the Free plan ([PRICE](https://www.opus.pro/pricing)).
- Real-time trend analysis is Business-only ([PRICE](https://www.opus.pro/pricing)); the "Trend" factor therefore likely uses a static or periodically updated notion of trends (**UNVERIFIED**).
- **Clip length**: default Auto 0-3 min; the dropdown's other buckets are not published in the help text. The API accepts `clipDurations` as ranges in seconds, examples `[[0,180]]` (ClipBasic) and `[[0,90]]` (ClipAnything) ([API create-project](https://help.opus.pro/api-reference/endpoints/create-project)). A tutorial mentions a "30s-59s" option ([Ben's Bites](https://bensbites-site.pages.dev/tutorial/turn-videos-into-social-clips)); full bucket list **UNVERIFIED**.
- Other curation controls: `topicKeywords` (Basic), `customPrompt` (Anything), `range`, `genre`, `skipCurate` (process the whole video as one project), `enableAutoHook`, `forceStrategy` ([API create-project](https://help.opus.pro/api-reference/endpoints/create-project)). Reprompt clipping on paid plans; "AI clipping by visuals, sound, emotion, genre" is Pro+ only ([PRICE](https://www.opus.pro/pricing)).
- To combine several moments into one clip, the editor's "Add a section" is used; prompts return one clip per match ([HC prompt manual](https://help.opus.pro/docs/article/clip-anything-prompt-manual.md)).

## 3. AI reframe and layouts

- **Auto Reframe** "automatically tracks moving and active speakers"; face tracking cannot be toggled manually, the AI decides; users choose which layouts the AI may use in the Brand Kit, then fine-tune ([HC how-to-enable-face-tracking](https://help.opus.pro/docs/article/how-to-enable-face-tracking.md)).
- **ReframeAnything** (alpha at launch) detects video type (podcast, demo, sport), tracks speakers/players/products, identifies number of speakers and who is talking/moving to pick a layout; manual "pick the object" was announced as coming soon ([Product Hunt](https://www.producthunt.com/p/opus-clip/opusclip-reframeanything-2), [HC clip-anything](https://help.opus.pro/docs/article/9947095-clip-anything.md)).
- **Layouts for 9:16** (seven on the help page; the search summary said six, the page itself lists seven) ([HC layout-and-reframing](https://help.opus.pro/docs/article/layout-and-reframing.md)):
  - **Fill**: crop on the speaker to fill 9:16.
  - **Fit**: original cropped to 4:3 (or 1:1 option) with the rest of the canvas a blurred background (Brand Kit crop setting; [HC how-to-use-brand-kit](https://help.opus.pro/docs/article/how-to-use-brand-kit.md)). The layout article calls the padding "opaque"; the Brand Kit article says blurred. Conflict, minor.
  - **Split**: two speakers stacked; only if both appear together in the source frame.
  - **Three** / **Four**: three/four speakers when they appear together (panels).
  - **Screenshare**: screen on top half, speaker on bottom half; falls back to another layout if no screen is detected.
  - **Gameplay**: speaker top 30%, gameplay bottom 70%.
  - "Pin first speaker" keeps speaker 1 in the top slot for Split/Three/Four.
  - 16:9, 1:1, 4:5 only get Fill and Fit ([HC how-to-use-brand-kit](https://help.opus.pro/docs/article/how-to-use-brand-kit.md)). Free/Starter output 9:16 only; Pro adds 1:1 and 16:9 ([PRICE](https://www.opus.pro/pricing)).
- **Per-scene layout**: AI picks the best allowed layout for each scene; the user can override per segment or for the whole clip; Manual Reframe crop window opens with the Crop button, double-click or X; multi-select with Shift ([HC layout-and-reframing](https://help.opus.pro/docs/article/layout-and-reframing.md), [HC how-to-manually-reframe](https://help.opus.pro/docs/article/how-to-manually-reframe-content.md)).
- B-roll segments get Fill / Picture-in-Picture / Split framing ([HC apply-the-layouts](https://help.opus.pro/docs/article/apply-the-layouts.md)).
- 4K: only when the source is genuinely 3840x2160 (no upscaling), Pro+ for download, Business for 4K posting ([HC download-and-post-in-4k](https://help.opus.pro/docs/article/download-and-post-in-4k.md)). A separate "AI video upscaling" feature exists on Starter+ ([PRICE](https://www.opus.pro/pricing)).
- **Weakness (reviews)**: reframe drifts or locks onto the wrong person on multi-person and screen-share video; needs review ([ScaleReach / aggregated via search, see section 12](https://www.scalereach.ai/blog/opus-clip-review), fetch of this page failed on a TLS error so the claim comes from the search summary only; **UNVERIFIED** at the primary-page level).

## 4. Captions

What is documented ([HC change-captions](https://help.opus.pro/docs/article/change-captions.md), [HC ai-emojis-keywords](https://help.opus.pro/docs/article/ai-emojis-keywords.md), [HC how-to-use-brand-kit](https://help.opus.pro/docs/article/how-to-use-brand-kit.md)):

- Word-by-word animated captions, editable per word in a Captions Editor (correct text, remove caption only, or delete word from video).
- Controls: font, size, colour, stroke, shadow, upper/lower case, drag to position anywhere; Brand Kit also sets weight, alignment and highlight colour; one caption style per kit; position/size per aspect ratio.
- **AI keyword highlight** (emphasises key words in a different colour) and **AI Emoji** (toggle "AI Emoji" under "AI Enhance"; per-word emoji editable) ([HC ai-emojis-keywords](https://help.opus.pro/docs/article/ai-emojis-keywords.md), [tutorial summary](https://videomaker.me/blog-design-options-for-your-short-form-videos-opus-clip-tutorial-52532)). Beta languages may get "inaccurate emojis or highlighted keywords" ([HC video-languages-supported](https://help.opus.pro/docs/article/video-languages-supported.md)).
- Caption position is also shown with "safe zones" in the Brand Kit preview (toggle only affects preview) ([HC how-to-use-brand-kit](https://help.opus.pro/docs/article/how-to-use-brand-kit.md)).
- Brand Vocabulary (added 2026-09-07) fixes names/terms across clips, and editor corrections can be saved for future clips ([CHG](https://opusclip.canny.io/changelog)).
- Auto-censoring of curse words: Pro default list, Business custom list ([PRICE](https://www.opus.pro/pricing)).
- **Preset names** exposed in the API: `preset-fancy-Karaoke`, `Gameplay`, `Beasty`, `Deep_Diver`, `Youshaei`, `Pod_P`, `Mozi`, `Popline`, `Simple`, `Think_Media` ([API brand-template](https://help.opus.pro/api-reference/brand-template.md)). A tutorial also lists "Alex Hormozi style" ([tutorial](https://videomaker.me/blog-design-options-for-your-short-form-videos-opus-clip-tutorial-52532)).
- **Typical look**: I found NO official font names, hex colours or pixel sizes for the presets. Descriptions are vague ("Youshaei: clean modern", "Mozi: bold, readable"). Treat fonts/colours of the presets as **UNVERIFIED**; the best-effort picture from tutorials and reviews is: large sans-serif (Karaoke/Hormozi style = heavy condensed), uppercase, one to three lines, active word recoloured (yellow/green are the common choices in this genre; not confirmed for OpusClip), optional emoji, placed in the middle-lower area while avoiding the speaker's head and platform UI ([dev.to experiment](https://dev.to/ireadcustomer/opusclip-vs-premiere-pro-what-a-20-video-experiment-reveals-about-cost-and-speed-in-2026-39oa), [tutorial](https://videomaker.me/blog-design-options-for-your-short-form-videos-opus-clip-tutorial-52532)). Recommendation: capture screenshots of each preset from the product (free plan) to measure it, rather than trust this section.
- OpusClip's own SEO blog gives generic rules (Montserrat Bold/Bebas Neue/Impact 60-80pt for bold statement; 40-50pt Helvetica/Lato for minimal; one emoji per line max; captions lead the audio by 100-200 ms; avoid bottom 20%; word-by-word base white, active word yellow or brand accent) ([blog best-caption-presets-styles-boost-retention](https://www.opus.pro/blog/best-caption-presets-styles-boost-retention)). These are marketing content that contradicts itself elsewhere (another post says place in the bottom 25% and min 18-24pt, [blog video-caption-design-placement](https://www.opus.pro/blog/video-caption-design-placement)); useful as hints, not as spec.
- Reported caption accuracy: the homepage claims 99%+ (97% in FAQ data) ([opus.pro](https://www.opus.pro/)); an independent test says 95%+ with spelling fixes needed, and a review cites skipped sentences ([dev.to](https://dev.to/ireadcustomer/opusclip-vs-premiere-pro-what-a-20-video-experiment-reveals-about-cost-and-speed-in-2026-39oa)).

## 5. AI hook / headline at the start

Two distinct things ([HC auto-headline](https://help.opus.pro/docs/article/auto-headline.md), [HC add-a-viral-hook](https://help.opus.pro/docs/article/add-a-viral-hook.md)):

- **Auto headline** (older names: Auto hook, Auto Text Overlay Hook): a short one-line text overlay at the start of each of the top 10 clips; AI generates the text; on by default for manual uploads; styled in the Brand Kit (font, size, weight, colour, word background colour, corner radius, box width); its position is chosen by AI from the layout and faces, not fixed. New styles added 2026-10-21 ([CHG](https://opusclip.canny.io/changelog)). Overlay panel: up to 5 text overlays, box radius 0-80 px, width 1-100% ([HC add-text-overlays](https://help.opus.pro/docs/article/add-text-overlays.md)).
- **Viral Hook** (4.0): template library (styles such as "Clean & corporate", "High-energy"); fills the template with the clip's own opening words; adds animated text, zoom effects and matching sound effects over the first seconds; does not add footage or lengthen the clip; options to hide captions while it plays and mute its SFX; costs 20 credits per up to 10 clips, 2 credits per editor use (Trial/Pro/Max/Business). Duration of the hook: not stated (**UNVERIFIED**).

## 6. B-roll

([HC ai-broll](https://help.opus.pro/docs/article/ai-broll.md), [PRICE](https://www.opus.pro/pricing))

- Three types, mixable in one clip: AI image, AI video, stock video. "Find spots" reads the transcript and suggests insertion points (about 30 s); the user deselects spots, chooses fill type, adds. AI images are planned jointly from the whole video for topical and stylistic consistency; AI video style defaults to matching the footage's colour/light/camera feel.
- Auto B-roll at project creation works up to 120 min of source. Stock is free; AI types cost credits (failed generations refunded).
- Plan limits: Starter 3 AI-image and 3 stock clips per month; Pro 50 AI-image clips/day, unlimited stock, AI video included.
- A 2024 note says GPT-4 Turbo is used to pick relevant B-roll ([CHG](https://opusclip.canny.io/changelog/core-ai-engine-upgrade)).
- **Weakness**: in one test 4 of 11 B-roll inserts were contextually wrong ([search summary of reviews](https://www.ssemble.com/blog/opus-clip-review-2026); numbers **UNVERIFIED** at primary level).
- Motion Studio (2026-09-18) generates B-roll animations from images/video/audio references ([CHG](https://opusclip.canny.io/changelog)).

## 7. Transitions, zooms, text, audio

- **Transitions** ([HC add-transition-effects](https://help.opus.pro/docs/article/add-transition-effects.md)): Auto Transition inserts a **0.5 s Flash black** at each cut when both clips are at least 1 s and at least one is at least 5 s. Manual styles: Slide left/right/up/down, Flash black, Flash white, Blur, Push in, Light, Wink, Stretch, Flip, Film burn, Glitch; duration 0.2-5 s (default 0.5). Older cross fade/zoom/fade now act on a single clip. Transitions are not carried into XML export, API or MCP.
- **Zooms**: Viral Hook adds "zoom effects"; general punch-in auto-zoom is not documented (**UNVERIFIED**).
- **Speech cleanup** ([HC speech-cleanup](https://help.opus.pro/docs/article/speech-cleanup.md)): fillers (um, uh, "you know", stutter repeats; claims 90%+ detection) with a checklist; pauses (default threshold 0.5 s, slider); **bad takes** (retakes, restarts, self-corrections, repeated phrases) flagged in orange for keep/remove; nothing is cut without approval. Starter has fillers/pauses; bad takes are Pro.
- **Audio**: Speech Enhancement (noise reduction and voice sliders; Pro 10/day), AI sound effects (Pro), music from library/upload, AI voice-over, video dubbing, eye-contact correction (Pro) ([PRICE](https://www.opus.pro/pricing), [CHG](https://opusclip.canny.io/changelog)).
- **Motion graphics** (icons/numbers/charts) in 4.0 ([blog](https://www.opus.pro/blog/opusclip-4-0)); lower-third name tag / location presets (12 animated) ([CHG](https://opusclip.canny.io/changelog)).

## 8. Brand Kit, Recipes and "templates"

- **Brand Template** was renamed **Brand Kit** in 4.0: "Brand Kit controls style, Recipe controls features" ([HC how-is-brand-kit-different](https://help.opus.pro/docs/article/how-is-brand-kit-different-from-brand-template.md)).
- **A Brand Kit contains** ([HC how-to-use-brand-kit](https://help.opus.pro/docs/article/how-to-use-brand-kit.md), [HC how-do-recipes-and-brand-kit-work-together](https://help.opus.pro/docs/article/how-do-recipes-and-brand-kit-work-together.md)):
  - one logo (JPG/PNG/SVG/GIF/WebP, draggable, per-ratio position/size);
  - caption style (preset + size, weight, alignment, highlight colour...);
  - auto-headline style;
  - allowed layouts per aspect ratio (9:16: Fill, Fit, Split, Three, Four, ScreenShare, Gameplay; others Fill, Fit), crop mode for Fit, "pin first speaker";
  - intro and outro per aspect ratio (Pro+);
  - background music;
  - emoji/caption/logo placement per 9:16, 16:9, 1:1, 4:5.
- **A Recipe contains**: which edits are on (clipping, captions, auto hook/headline, B-roll type, speech cleanup, etc.), output ratio, clip length, plus which Brand Kit to apply. Built-in recipes: Clip long video, Clean up recording, Auto-edit talking videos, Captions, Reframe, Social edits, AI B-roll, Translate & dub ([HC how-to-use-recipes](https://help.opus.pro/docs/article/how-to-use-recipes.md)).
- Plan caps: 1 kit on Free/Starter, up to 4 on Pro; custom fonts up to 2 on Pro; custom media library; brand vocabulary ([PRICE](https://www.opus.pro/pricing)).

## 9. Export, posting, titles/descriptions/hashtags

- **Export**: MP4 up to 1080p (Free) or highest available; watermark-free from Starter; bulk export and Premiere/DaVinci XML on Pro ([PRICE](https://www.opus.pro/pricing)).
- **XML to Premiere/DaVinci** ([HC import-to-adobe-premiere](https://help.opus.pro/docs/article/import-to-adobe-premiere.md)): per-clip download with the raw source video, an `.xml`, and an `.srt`. XML carries auto captions (as non-editable overlays), auto reframe, B-roll (only if auto B-roll was on), music, voice-over, logo, emoji. Segments extendable up to 30 s before/after. Limits: 1.5 h source, one short at a time, transitions not included, other editors may differ.
- **Social posting** (Starter: YouTube Shorts, TikTok, Instagram; Pro adds Facebook, LinkedIn, X, Pinterest, Threads, Bluesky) and a calendar **scheduler** (Pro) with **Bulk Scheduler** (order, start time, fixed interval; no "best time" logic) ([PRICE](https://www.opus.pro/pricing), [HC bulk-scheduler](https://help.opus.pro/docs/article/bulk-scheduler.md), [HC schedule-clips](https://help.opus.pro/docs/article/schedule-clips.md)).
- **Titles, descriptions, hashtags**: "the AI generates descriptions and hashtags" per clip when scheduling; titles/descriptions editable in the iOS app ([HC schedule-clips](https://help.opus.pro/docs/article/schedule-clips.md), [HC customize-my-posts](https://help.opus.pro/docs/article/9747769-customize-my-posts.md)). Format rules per platform are not documented (**UNVERIFIED**).
- **Subtitle export**: `.srt` download of the original transcript ([HC download-transcripts-subtitles](https://help.opus.pro/docs/article/download-transcripts-subtitles.md), listed in the help index).
- **API/MCP**: REST `clip-projects` with `brandTemplateId`, webhooks, social posting, and an MCP server (about 27-37 tools depending on the page; count conflicts) ([API create-project](https://help.opus.pro/api-reference/endpoints/create-project), [claude-connector](https://help.opus.pro/api-reference/claude-connector.md)).

## 10. Languages and Hinglish

- Documented spoken languages: "over 25"; English, German, Spanish, French, Portuguese standard; Italian, Dutch, Russian, Polish, Indonesian, Ukrainian, Swedish, Turkish, Norwegian, Croatian, Romanian, Slovak, Greek, Danish, Finnish, Hungarian, Czech, Japanese, Korean, Vietnamese in beta ([HC video-languages-supported](https://help.opus.pro/docs/article/video-languages-supported.md)).
- **Hindi is NOT supported** in the product per the help center ([HC does-opusclip-support-hindi](https://help.opus.pro/docs/article/does-opusclip-support-hindi.md)); an OpusClip API blog lists Hindi as planned for the API ([blog multi-language-captions-api](https://www.opus.pro/blog/multi-language-captions-api)). No source confirms Hinglish (Roman-script Hindi). This is a real Rustler advantage (the `hinglish` guide, `--script latin`).
- Subtitle translation and dubbing exist ([HC select-subtitle-language](https://help.opus.pro/docs/article/select-subtitle-language.md), [HC video-dubbing](https://help.opus.pro/docs/article/video-dubbing.md); titles seen in the help index, contents not read). ClipAnything prompts must be English.

## 11. Speed and pricing

- **Speed**: no official figure ("depends on length and demand"). Reports: about 5 min AI processing plus 15 min human review for a one-hour episode ([dev.to experiment](https://dev.to/ireadcustomer/opusclip-vs-premiere-pro-what-a-20-video-experiment-reveals-about-cost-and-speed-in-2026-39oa)); "2-5 minutes per video" ([Ssemble](https://www.ssemble.com/blog/opus-clip-review-2026), a competitor); a competitor's rule of thumb is 3-4 min processing per input minute (**UNVERIFIED**). Processing is queued by plan tier ("faster than Free/Starter", Business "dedicated queue", [PRICE](https://www.opus.pro/pricing)). Reliability complaints: videos hanging or failing.
- **Pricing** ([PRICE](https://www.opus.pro/pricing)): Free $0; Starter $15/mo; Pro $29/mo; Business custom. Credit amounts are not published on the pricing page; third parties report 60 / 150 / 300 minutes per month for Free / Starter / Pro and credits charged per source minute, not per clip ([Ssemble](https://www.ssemble.com/blog/opus-clip-review-2026), **UNVERIFIED**). The homepage mentions a 7-day Pro trial and 60 free credits/month ([opus.pro](https://www.opus.pro/)).
- **What the tiers reveal about the value stack**: Free = clips by spoken words, watermark, 9:16 only. Starter adds virality score, text overlay, timeline editing, speech cleanup (fillers/pauses), social posting. Pro adds visual/emotion clipping, prompts, bad-take cleanup, B-roll (AI video), transitions, SFX, intro/outro, custom fonts, scheduler, XML export, 1:1/16:9. Business adds 4K posting, trend analysis, CMS, SSO.

## 12. Known weaknesses and complaints

All from reviews, several by competitors (SSemble, ScaleReach, NextClip, FastLane) so directional only:

- Inconsistent clip quality, lacking narrative coherence; needs review before posting; wants finer control over selection ([CheckThat summary: G2 4.6/5 from 115, Trustpilot 4.4/5 from 500+](https://checkthat.ai/brands/opusclip/reviews)). A 30-day test reported 47 of 76 clips publishable, 19 needing light edits, 10 unusable (secondhand via search summary of [ScaleReach](https://www.scalereach.ai/blog/opus-clip-review); **UNVERIFIED**, direct fetch failed).
- Favours loud/high-energy moments, "cannot handle the power of silence", unsuitable for cinematic/hero content; caption spelling errors need fixing ([dev.to](https://dev.to/ireadcustomer/opusclip-vs-premiere-pro-what-a-20-video-experiment-reveals-about-cost-and-speed-in-2026-39oa)).
- Weak on gaming, music video, physical comedy, visual-only moments (same secondhand 30-day test, **UNVERIFIED**).
- Reframe tracking wrong subject on multi-person and screen-share; B-roll mismatches (section 3 and 6).
- Virality score "can be inconsistent"; no independent accuracy study found ([Filmora, via search summary](https://filmora.wondershare.com/video-editor-review/opusclip-ai.html)).
- Credits and billing: credits per source minute, expiry, hard cancellation, projects vanishing after subscription lapse ([Ssemble](https://www.ssemble.com/blog/opus-clip-review-2026)).
- No Hindi; ClipAnything prompts English only (section 10).
- Notably the clip count is model-driven, not user-driven ("working on improving our algorithm so that we will generate more clips", [HC how-many-clips](https://help.opus.pro/docs/article/how-many-clips.md)).

---

## 13. Gap table: OpusClip feature vs Rustler today

Rustler facts come from `knowledge/guides/cli-reference-conventions.md` (command map) and the other guides in `knowledge/guides/`, and from a grep of `crates/rustler-captions/src` (no per-word emphasis or emoji support found there). I did not run Rustler, so "exists" means "the command is documented", not "output verified". Constraint: Rustler never calls AI APIs; the driving agent (with vision) does the judgement, so "need" below is split into **tool** (deterministic, in Rust/engine) and **skill** (agent guide/prompt content in `knowledge/`).

Legend: E = exists, P = partial, M = missing, N = deliberately out of scope.

| # | OpusClip feature | Rustler today | Status | What Rustler needs |
|---|---|---|---|---|
| 1 | Moment selection by transcript (ClipBasic) | `highlights.signals`, `highlights.candidates` (ranked sentence-aligned windows, reasons, breakdown, `create_cmd`), `transcript.search/read_window`, guide `finding-hooks` | E | Skill: rubric tuned to Opus factors (see row 3). Tool: none critical |
| 2 | Prompted selection (ClipAnything: visual, sound, emotion) | `see.tiles/frame` (agent vision), `analyze.events` (opt-in audio events), `analyze.ocr`, `analyze.scenes` | P | Tool: `see.scan` that returns a cheap grid of tiles over the whole source at N-second spacing with captions of transcript so the agent can "prompt" the video in a few image calls; `highlights.signals` extended with laughter/applause flags when events extra is on. Skill: a `clipanything_prompt` MCP prompt (agent turns a user prompt into a search plan: transcript query, then tiles to confirm) |
| 3 | Virality score 0-99 (hook / flow / value / trend) | `finding-hooks` guide has a 5-part rubric (hook, standalone, payoff, emotion, share; total 84 example), heuristic `breakdown` in candidates | P | Tool: store the score on the clip (`clip.set_score {hook,flow,value,trend,total,reasons}`, shown in `clip.list`, sortable, in exports). Skill: align the rubric to Opus's four factors plus "prompt match"; require agent to justify each part with a transcript quote. "Trend" cannot be local: leave as agent judgement, flag as low confidence |
| 4 | Narrative templates (story structures per genre) | none as data | M | Skill only: add `knowledge/guides/narrative-structures.md` (hook-context-payoff, myth-bust, listicle, problem-solution, before-after, cliffhanger-loop) with how each maps to segments (`clip.add_segment`) |
| 5 | Clip length choice (Auto 0-3 min; buckets) and clip-count expectations | `highlights.candidates --min-len --max-len --platform`, `brief.set min/max-duration` | E | Skill: copy Opus's clip-count calibration table (section 1 item 7) as a target for "how many clips from an N-minute source" |
| 6 | Sentence-accurate boundaries, skip weak openers | `edit.refine`, `edit.trim --snap`, unit ids, `cut-points` guide | E | None |
| 7 | Filler and pause removal | `edit.remove_words` (um, uh, hmm by default), `edit.remove_pauses --min-gap --keep` | E | Tool: `edit.detect_fillers` preview mode (list, don't cut) so the agent can approve like Opus's checklist (check if `--dry-run` already gives this; **UNVERIFIED**). Add "you know/I mean/like" phrase fillers and stutter-repeat detection |
| 8 | Bad-take removal (restarts, self-corrections, repeated phrases) | none | M | Tool: `transcript.find_repeats` (adjacent near-duplicate n-grams, restarts "I mean... I", with unit ids). Skill: how to decide and cut with `edit.set_range`/`delete_segment` |
| 9 | AI reframe: Fill with face/active-speaker follow | `clip.set_layout center-crop`, `crop.track_face`, `crop.track_speaker` (needs `speakers` extra) | E | Tool: stability tuning (smoothing, dead zone, hand-off between speakers) verified against Opus output; see risk section |
| 10 | Fit (blurred background) | preset `blur-fit` | E | Option for 4:3 / 1:1 inner crop (Opus "Crop" setting): `layout-params` check |
| 11 | Split (2 speakers), Three, Four | `split-vertical` is webcam/gameplay; two-person split via `layout.from_frame_boxes`/`add_region` by hand | P | Tool: presets `split-2`, `split-3`, `split-4` that take face boxes from `analyze.faces` automatically (`layout.auto_split <clip>`), with "pin first speaker" |
| 12 | Screenshare (screen top, speaker bottom) | doable by regions (`layout.add_region`) | P | Tool: preset `screenshare` and a detector (large static high-contrast region + face, using scene/face data) or agent picks boxes from `see.frame` |
| 13 | Gameplay (speaker top 30%, game bottom 70%) | `layout.detect_webcam` + `split-vertical`, `streamer-clips` guide | E | Match the 30/70 ratio as a named preset (check current default ratios) |
| 14 | Per-scene automatic layout switching | segments each carry a layout (`clip.add_segment --layout`, `clip.set_layout --segment-id`) | P | Tool: `layout.plan <clip>` returning per-segment face count / screen presence so the agent assigns layouts; or `clip.auto_layout` that applies deterministic rules. Agent does the creative choice |
| 15 | Manual reframe crop window | `layout.edit_region`, GUI source panel | E | None |
| 16 | Aspect ratios 9:16, 1:1, 4:5, 16:9 | `render.run` produces 1080x1920; `brief.set aspect` exists (render support for other ratios not verified, **UNVERIFIED**) | P/M | Tool: clip `canvas` of 1080x1920 / 1080x1350 / 1080x1080 / 1920x1080 and layouts reflowing. Medium priority |
| 17 | 4K / upscaling | none documented (output 1080x1920) | M | Low priority; only if source is 4K (`render.run --quality final --scale 2`). Skip upscaling |
| 18 | Animated word-by-word captions | `caption.*`, presets `bold-karaoke`, `hormozi-pop`, `boxed-highlight`, `clean-white`, custom styles (`caption.create-style`), libass | E | Skill/tool: add presets that mimic the Opus set (Karaoke, Beasty, Deep Diver, Pod P, Mozi, Popline, Simple, Think Media, Hormozi) after sampling real outputs (colours/fonts are UNVERIFIED, section 4) |
| 19 | AI keyword highlight (key words in another colour/size) | active word only; per-speaker colours (`caption.speaker_colors`); `captions-and-style` states per-word emphasis unavailable | M | Tool: `caption.emphasize <clip> --words idx... --color #hex [--kind recolor\|scale\|box]` stored per unit id and re-applied after re-cuts. Skill: agent picks keywords (numbers, names, claims, strong verbs; max 1-2 per line) |
| 20 | AI emoji in captions | none | M | Tool: `caption.add_emoji <clip> --at-word i --emoji ...` rendered as image overlay (libass cannot render colour emoji reliably; needs an emoji PNG pack asset, e.g. Twemoji/Noto under a license recorded by `asset add`). Skill: one emoji per line maximum, only where it adds meaning |
| 21 | Caption position avoids faces/platform UI | `position_y`, QC `caption_over_face` + `fix.apply` | E | None (Rustler is arguably ahead via QC) |
| 22 | Brand vocabulary / caption correction | no transcript edit command listed (CLI map has `transcript.get/search/read_window` only) | M | Tool: `transcript.correct <src> --unit n --text ...` and a project/global glossary applied to romanisation and captions. Opus's reviews show spelling errors are a main complaint, so this matters for both |
| 23 | Auto headline / hook title (one line, top 10 clips) | `overlay.add_text --style hook-top`, `variant create --hooks`, `finding-hooks` and `editing-toolkit` guides | P | Tool: overlay entrance animation (pop/slide/scale) and a style that sets its background box with radius (`overlay.add_text` has `box?`; confirm radius and animation options). Skill: policy "every exported clip gets a headline <= 7 words from the clip's own claim" |
| 24 | Viral Hook (templated animated opening: text + zoom + SFX) | pieces exist: `overlay.add_text`, `effect.add punch-in/flash`, `audio.add-sfx` | P | Tool: `hook.apply <clip> --template <name> --text` macro that expands to overlay + effect + sfx at t=0..3 as ONE reversible revision. Skill: 4-6 hook templates (high-energy, clean) |
| 25 | Text overlays, lower thirds / name tags | `overlay.add_text` (`lower-third`, `plain`), `overlay.edit`; speaker names via `speaker.rename` | E/P | Auto name tag from `speaker.rename` names (`overlay.name_tag <clip> <speaker>`); animated presets |
| 26 | Transitions (auto + 14 styles) | `transition.set` (cut, fade, dissolve, wipeleft, wiperight, slideup, zoomin) | P | Tool: add flash-black, flash-white, blur, push, glitch; `transition.auto <clip>` implementing Opus's rule (0.5 s flash at cuts where both sides >= 1 s and one >= 5 s). Skill: restraint rules exist already |
| 27 | Punch-in / zoom emphasis | `effect.add punch-in|zoom-slow|shake|flash` | E | Tool: `effect.auto_punch <clip>` placing punch-ins at emphasis words/segment starts (agent can already do it from word timings, so skill-only is acceptable) |
| 28 | AI B-roll (image, stock, video) with "find spots" | none; overlay/regions only use the source video; "sticker/image layers" is listed as deferred in CLAUDE.md | M | Tool: **image/video overlay layer** `broll.add <clip> --asset asset:<id> --start --end --fit fill|pip|split --motion ken-burns`, plus `broll.suggest_spots <clip>` (transcript sentences with concrete nouns/numbers + gaps, ranked) and stock fetch via `asset add --url` (Pexels/Pixabay direct links; no API key flow, agent searches the web). Skill: B-roll policy (cover cuts, never cover the speaker's key reaction, max ~35% of runtime, match the sentence). No AI image generation (no AI APIs); the agent may supply images produced elsewhere |
| 29 | Motion graphics (icons, numbers, charts) | none | M | Out of reach short-term; a stat-callout overlay style (big number + label) via `overlay.add_text --style stat` covers 80% |
| 30 | Music, ducking, SFX | `audio.add_music/add_sfx` (`duck`, fades, anchors to words), `asset` library, opt-in `sfx-pack`/music-pack | E | Skill: default recipes (low music bed -18 dB, whoosh on transitions) |
| 31 | Speech enhancement (noise/voice) | `audio.denoise`, `audio.volume`, two-pass loudnorm | E | None; eye-contact correction and AI voice-over are N |
| 32 | Brand Kit: logo | `brief.set watermark-text/credit-text` (text only) | M | Tool: image logo overlay (same layer as row 28) with per-canvas position and size, `brand.set --logo asset:<id>` |
| 33 | Brand Kit: intro/outro, music | none as a feature; `clip.add_segment` could append an imported intro/outro source | P | Tool: `brand.set --intro <media_ref> --outro <media_ref>` rendered as first/last segments with audio, applied at `render.run` |
| 34 | Brand Kit / Recipe as reusable bundle | `layout.save_preset`, custom caption styles, `brief.set` (platform, durations, caption style, layout, tags) | P | Tool: `kit.save <name> --from-clip/--from-brief` and `kit.apply <clip>` bundling caption style, layout, hook template, logo, intro/outro, music, transition, effect density, canvas; `render.batch` applies a kit to all clips. This is Opus's biggest consistency lever |
| 35 | Ranked results list with scores, thumbs up/down feedback | `clip.list`, `project.status`, `variant.compare` | P | Tool: sort by score; `clip.rate`. Low priority |
| 36 | Multiple clips from one source in one go ("Clip long video" recipe) | `prompt make_viral_clips`, `batch_from_brief`, `render.batch`, `export.batch` | E | Skill: a "recipe" prompt that bundles candidates, kit apply, QC and export with the Opus clip-count targets |
| 37 | XML export to Premiere / DaVinci (cuts, reframe, captions as overlays, B-roll) | `export.clip` (mp4 + report + srt); `caption.export_srt` | P | Tool: `export.xml <clip> --format fcpxml\|xmeml` writing the cut list (segments in source time), crop/scale keyframes from layout and face path, transitions, audio levels, and the SRT. Medium effort, high value for editors; deliverable validated by importing in DaVinci Resolve (free) |
| 38 | Bulk download, 4K | `export.batch` | P/E | None beyond row 17 |
| 39 | Title, description, hashtag generation | `brief.description` (hook line, credit, mentions, required hashtags), `platform-specs` guide (title limits) | P | Skill: a `post_copy` prompt: per platform (YT Shorts title <= 100 chars, IG/TikTok caption + 3-5 hashtags) in the clip's language; tool: write the result to `exports/<clip>/post.json` so a human or scheduler can paste. No API calls needed |
| 40 | Social posting and calendar scheduling | none, by design ("publishing is manual", CLAUDE.md) | N | If the user later wants it: `export.schedule_sheet` CSV (clip, title, description, hashtags, suggested time) importable into a scheduler. Not a core gap |
| 41 | Multi-language captions, translation, dubbing | Hinglish/Devanagari/Latin script (`caption.set --script`), whisper `--language` | P (and ahead on Hindi) | Tool: `transcript.import_translation <src> <srt>` or `caption.set --lang` so the agent can translate and burn translated captions with unit timings preserved. Dubbing: N |
| 42 | Speed (about 5 min per hour of source, cloud queue) | local faster-whisper (`base` by default in lite), scene detect, proxies; no measurement recorded | UNVERIFIED | Measure `analyze quick` on a 60-min podcast on the target machine; target under 10 min CPU. Cache everything per source (already resumable) |
| 43 | QC and fix loop | `qc.run`, `fix.suggest/apply`, `qc.compare`, contact sheet | E (ahead) | Opus has no equivalent; keep as differentiator |
| 44 | Progress bar, speaker-colour captions, variants A/B | `overlay.progress_bar`, `caption.speaker_colors` (Opus: "coming soon" Business), `variant.*` | E (ahead) | None |

### Where Rustler is already ahead or equal

- Cut precision and revisions: unit-id based cuts with `--expect-text`, undo/restore, `edit.refine`.
- Closed-loop QC with a vision contact sheet and automated fixes.
- Hinglish/Hindi handling (OpusClip: not supported).
- Speaker-coloured captions and a progress bar.
- Local, no credits, no clip-count caps.

### Biggest gaps (ranked by impact on "looks like Opus")

1. **B-roll / image layer + logo** (rows 28, 32): the largest visible difference. Needs a new render layer, so it also unlocks logo, intro/outro stills, and stat callouts.
2. **Caption emphasis: keyword colour and emoji** (rows 19, 20): the most visible caption difference. Needs per-word styling that survives re-cuts.
3. **Kit/Recipe bundle** (rows 33, 34, 24): the thing that makes 20 clips look consistent in one command.
4. **Auto layout planning with Split/Three/Four/Screenshare** (rows 11, 12, 14): Opus's multi-person quality comes from picking layouts per scene.
5. **Score persisted on the clip + Opus-style rubric and narrative skill** (rows 3, 4): mostly agent skill content, cheap and high leverage.
6. **Bad-take / repeat detection, fuller filler list, transcript correction** (rows 7, 8, 22).
7. **Hook macro and a bigger transition set with an auto rule** (rows 23, 24, 26).
8. **XML export** (row 37) and **post copy** (row 39): packaging.

### Suggested agent skill (knowledge book additions)

New files under `knowledge/guides/` and one MCP prompt (the existing 10 prompts are in `knowledge/prompts/`):

- `opus-parity-checklist.md`: per clip, in order: (1) pick by rubric hook/flow/value/trend/prompt-match, record score; (2) clean cut: refine, remove fillers/pauses/bad takes; (3) layout per scene; (4) hook headline <= 7 words + hook macro; (5) captions with keyword emphasis (and emoji only if a pack exists); (6) one B-roll or punch-in per 6-10 s of static talking head; (7) transitions only at topic changes, 0.5 s; (8) music bed + SFX on transitions; (9) kit apply; (10) QC and look at contact sheet; (11) post copy; (12) export.
- `narrative-structures.md` (row 4) and `clip-count-and-length.md` (row 5 numbers).
- `broll-policy.md` once the image layer exists.
- Prompt `opus_style_batch`: source in, N ranked clips with kit applied, score table, post copy, in one flow.

### Risks and open questions

- **Colour emoji in libass**: unreliable; plan on PNG overlays (so rows 20 and 28 share the image layer).
- **Reframe quality is the top reviewer complaint for OpusClip itself**, so an equal bar is "no wrong-subject lock", which `qc.run --faces` and `crop.track_speaker` can verify; test on 3-person and screen-share inputs.
- **Caption look**: exact OpusClip preset fonts/colours are UNVERIFIED. Recommended next step: run the free plan on one 5-minute video, screenshot each preset, then derive Rustler styles from measurements.
- **Speed number for OpusClip** is only reported, not measured; same for Rustler (measure).
- The fetch tool summarises pages with a small model; fields such as clip-length buckets, narrative template names and Brand Template JSON schema returned empty from the schema pages (`render-preferences`, `curation-preferences`, `brand-template-representation`). Reading those pages in a browser would complete rows 5, 16 and 34.
- CLAUDE.md says Rustler has "118 commands"; the task brief said 119 (the progress log says "119 MCP tools"). The command map above is the 118-command table; `rustler commands list` is the authority.
