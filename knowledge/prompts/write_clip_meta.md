---
name: write_clip_meta
title: "Write the title, description and hashtags for a clip"
description: "Read clip.meta_context, write viral platform-native copy that follows the campaign rules, save it with clip.meta_set, mark agent rules with reasons and get QC green."
arguments:
  - name: clip_id
    description: "The clip id (clp_...) to write posting copy for. It should be cut and rendered."
    required: true
  - name: platform
    description: "shorts, reels or tiktok: the platform to write for (default: the campaign's platform, else shorts)."
    required: false
---

Write the posting copy for clip **{{clip_id}}** (platform: {{platform}}; if that is empty use the campaign's
platform, else `shorts`). You write every word; Rustler only stores and checks it. Guide:
`campaigns-and-posting` (formulas, limits, mistakes). Read it if you have not this session.

## 1. Gather
`rustler clip meta-context {{clip_id}}`. Read: `transcript` and `hook` (what the clip actually says),
`campaign.requirements` and `rules` (the law), `platform_limits`, `brief`, `saved` (do not discard good copy
already there). No campaign attached but `campaigns_available` is not empty: pick the one this clip is for.

## 2. Decide the angle
In one sentence: what makes a stranger stop and finish this clip? (result, tension, surprise, a quote).
It must be true to the clip. Note every hard requirement from the rules: required hashtags, mentions,
links or text, banned words, title length.

## 3. Write
- **title**: one line, the angle as a hook (result first, open loop, contrast, question or number + payoff),
  ideally under 60 characters, in the clip's language and register.
- **description**: the hook extended with context the video does not give, then the campaign's required text
  and links exactly as worded, at most one light call to action.
- **hashtags**: 3 to 5, specific plus any campaign-required tags. **mentions**: the required ones.
Do not promise what the clip does not deliver. No banned word in the copy.

## 4. Save and check
Write `meta.json` and run `rustler clip meta-set {{clip_id}} --input-json @meta.json`:
`{"clip_id": "{{clip_id}}", "campaign_id": "<id or omit>", "platforms": {"<platform>": {"title": "...", "description": "...", "hashtags": ["..."], "mentions": ["..."]}}}`.
Read `compliance_now` in the answer; fix every failing machine rule by editing the copy (or the clip) and
saving again. For other platforms in the campaign, write a separate, platform-native version each.

## 5. Agent rules
For each `kind: agent` rule: look at the clip first (`rustler render preview-frame {{clip_id}} --out-t <t>` or
the QC contact sheet), then `rustler campaign mark {{clip_id}} <rule_id> --pass true|false --reason "<what you saw, with times>"`.
If a rule fails, fix the clip (cut, overlay, layout) rather than marking it passed.

## 6. Verify and export
`rustler qc run {{clip_id}}`: `meta_present` and `campaign_compliant` must pass (read each `fix_hint`).
Then `rustler export clip {{clip_id}}` and open `exports/<name>.post.json`: title, description, hashtags and
`campaign.compliance.passed` must be what you intend. Never post or upload: the user publishes.
