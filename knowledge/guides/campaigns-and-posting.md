---
id: campaigns-and-posting
title: "Campaigns and the posting JSON: write the title, description and hashtags yourself"
summary: "Create a campaign with machine and agent rules, write per-platform viral copy from clip.meta_context, mark agent-checked rules with a reason, and export the post JSON that QC and the campaign reviewer need."
order: 18
tags: [campaign, posting, title, description, hashtags, post-json, meta, viral, compliance, settings]
---

# Campaigns and the posting JSON

Paid clipping campaigns have two deliverables: the **video** and the **post** (title, description,
hashtags, mentions). Rustler never writes copy: **you** write it, from evidence Rustler gives you.
Every export writes `exports/<name>.post.json` next to the mp4, and QC will not call a clip done
without it once the project has a campaign.

## 1. The flow

```text
campaign create  ->  clip meta-context  ->  (you write the copy)  ->  clip meta-set
   -> campaign mark (agent rules)  ->  qc run  ->  export clip   (mp4 + report + post.json)
```

1. **Create the campaign once** from the creator's brief. Use `--input-json @campaign.json` on PowerShell:

```json
{"id": "spring-promo", "name": "Spring promo", "platform": "shorts",
 "payout": {"model": "per_view", "rate": 1.5},
 "duration": {"min": 20, "max": 60},
 "hashtags_required": ["ad"], "mentions_required": ["brand"],
 "rules": [
   {"id": "no-sponsor", "text": "Never say the word sponsor", "kind": "machine",
    "check": {"type": "banned_word", "value": "sponsor"}},
   {"id": "shop-link", "text": "Description links brand.shop", "kind": "machine",
    "check": {"type": "description_contains", "value": "brand.shop"}},
   {"id": "shows-product", "text": "The product is visible on screen", "kind": "agent"}
 ]}
```

`rustler campaign create spring-promo --input-json @campaign.json`, then `campaign show`, `campaign list`,
`campaign delete`. A project can hold many campaigns; the single `brief set` keeps working beside them.

2. **Rules.** `machine` rules carry a `check` Rustler tests every time: `duration_min`, `duration_max`
   (seconds), `hashtag`, `mention`, `banned_word` (title, description, hashtags **and spoken words**),
   `description_contains`, `title_max_chars`, `hashtags_max`. The campaign's own `duration`,
   `hashtags_required` and `mentions_required` are checked automatically (rule ids `duration`,
   `hashtags_required`, `mentions_required`). `agent` rules are judgement calls only you can make
   ("product on screen", "no competitor logo", "creator credited on screen").
3. **Gather context:** `rustler clip meta-context <clip_id>` returns the clip's spoken words, its hook (the
   hook-title overlay, else the first sentence), platform limits, the attached campaign with every
   requirement and a live `compliance_now`, the brief, and the copy already saved. Read all of it first.
4. **Write the copy** (section 2) and save it per platform:
   `rustler clip meta-set <clip_id> --input-json @meta.json`

```json
{"clip_id": "clp_X", "campaign_id": "spring-promo",
 "platforms": {"shorts": {"title": "He said it on live TV and nobody noticed",
                          "description": "Full story at brand.shop\n#ad @brand",
                          "hashtags": ["ad", "viral", "story"], "mentions": ["brand"]}}}
```

   Limits are enforced (Shorts title 100 chars / 15 hashtags, descriptions up to 2200 chars on
   Reels/TikTok). Limits change: if money depends on one, tell the user to verify.
   Saving returns `compliance_now`; fix what is failing before QC.
5. **Agent rules:** open the contact sheet or preview frames, then record each verdict **with a reason**:
   `rustler campaign mark <clip_id> shows-product --pass true --reason "product visible 0:05-0:09"`.
   A rule without a mark counts as not compliant; an empty reason is refused; marks are cleared if you
   re-attach the clip to a different campaign. Look first, then mark; never mark blind.
6. `rustler qc run <clip_id>` now includes two checks:
   `meta_present` (copy written, for the campaign's platform) and `campaign_compliant`. Both are listed with
   a `fix_hint`. Without any campaign in the project a missing copy is only a warning; with a campaign it fails.
7. `rustler export clip <clip_id>` writes `<name>.post.json`: `{clip, file, duration, platform, title,
   description, hashtags, mentions, platforms{shorts,reels,tiktok}, cover, campaign{id, requirements[],
   compliance{passed, rules[]}}, marks, score, created_at}`. The top-level `title`/`description` are the
   campaign platform's copy. A clip exported with no copy gets `meta_present: false` and a warning.

## 2. Writing copy that earns views

Base everything on what is **in the clip**: the transcript and hook from `meta-context`. Never promise
what the clip does not pay off; the viewer who feels tricked leaves and the algorithm remembers.

**Title (the hook in text form, one line, under 60 characters ideally)**

| Formula | Example |
|---|---|
| Result first | "He turned 500 rupees into a shop in a year" |
| Open loop | "The one thing she never told her team" |
| Contrast / stakes | "Everyone laughed. Then the scoreboard changed." |
| Direct question | "Would you take this deal?" |
| Number + payoff | "3 words that ended the argument" |
| POV | "POV: your boss hears this on the live call" |

Use the speaker's strongest line or the moment's tension, not a description of the video. Match the clip's
language and register (Hinglish clip -> Hinglish title, see `hinglish`). Capitalise like the platform's
native posts, no clickbait the clip cannot cash. Reels/TikTok have no separate title field: the title is the
first line / cover headline, so make the first 5 words count.

**Description**

- Line 1 repeats or extends the hook; add context the video does not give (who, where, why it matters).
- Then the campaign's required text and links exactly as the brief words them (`description_contains`).
- One light call to action at most ("Follow for part 2" only if part 2 exists).
- Required mentions and hashtags can sit in the description or the lists; the checker finds both.

**Hashtags**

- 3 to 5. One broad (`#shorts`-type reach tag only if the platform still uses it), two to three specific to the
  topic or niche, plus every campaign-required tag (`#ad` and similar) **always**.
- Never stuff: Shorts ignores all hashtags past 15. Never use banned or irrelevant tags.
- Required mentions go in `mentions` (and into the description when the brief says "in the caption").

**Per platform**

- Shorts: searchable title, informative description, keywords in the first line.
- Reels: conversational first line, the first line is what shows before "more".
- TikTok: shortest and punchiest; the on-screen hook and the caption should agree.
Write a separate version for each platform in the campaign; do not paste the same text everywhere.

## 3. Settings you can change once

`rustler config list` shows agent settings (project value wins over global, flags always win):
`download.quality`, `render.parallel`, `render.quality`, `captions.style`, `captions.words_per_line`,
`template.default`, `clip.smart_defaults`, `qc.faces`. Example: `rustler config set download.quality 1080 --scope global`.

## 4. Mistakes to avoid

- Writing the copy before reading `meta-context`, or from the video file name.
- Marking an agent rule `--pass true` without looking, or with a vague reason ("ok").
- Forgetting required hashtags/mentions; `compliance_now` tells you right after `meta-set`.
- Copy for a platform the campaign is not on (the campaign's `platform` decides which copy is checked).
- Posting or uploading: Rustler never does; the user publishes with the post JSON.
