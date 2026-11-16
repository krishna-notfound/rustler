//! Commands: clip.meta_context, clip.meta_set (v2 Task 1.7); the posting JSON written by `export.clip`
//! and the QC checks `meta_present` / `campaign_compliant`.
//!
//! Every exported clip gets `<name>.post.json` with the title, description, hashtags and mentions **the
//! agent wrote** (Rustler never generates copy). `clip.meta_context` gives the agent what it needs to write
//! it: the clip's spoken words, its hook, the platform limits and the campaign's rules.
//! `clip.meta_set` stores the copy per platform in `clips/<id>/extras.json` under `post`:
//!
//! ```json
//! {"campaign_id": "spring", "platforms": {"shorts": {"title": "..", "description": "..", "hashtags": ["#a"], "mentions": []}},
//!  "marks": {"shows-product": {"pass": true, "reason": "..", "at": ".."}}}
//! ```

use crate::brief_cmds::{Platform, load_brief};
use crate::campaign_cmds::{Campaign, Compliance, Subject, evaluate, list_campaigns, load_campaign};
use crate::extras;
use chrono::{DateTime, Utc};
use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, QcCheck, QcStatus, RustlerError};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

type Result<T> = rustler_core::Result<T>;

pub const SCHEMA: &str = "rustler.post@1";
const KEY: &str = "post";
/// Longest transcript (characters) returned by `clip.meta_context`.
const TRANSCRIPT_CHARS: usize = 6000;

fn validation(msg: impl Into<String>, hint: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, "post", msg).hint(hint)
}

// ---------------------------------------------------------------------------------------------
// Stored data
// ---------------------------------------------------------------------------------------------

/// The agent's copy for one platform.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct PlatformMeta {
    /// The headline. Shorts: the video title. Reels/TikTok have no separate title: it is the first line / cover text.
    pub title: String,
    pub description: String,
    /// With or without the leading `#`.
    #[serde(default)]
    pub hashtags: Vec<String>,
    /// With or without the leading `@`.
    #[serde(default)]
    pub mentions: Vec<String>,
}

/// Copy per platform; only the platforms you post to need to be present.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Default, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Platforms {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shorts: Option<PlatformMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reels: Option<PlatformMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiktok: Option<PlatformMeta>,
}

impl Platforms {
    /// The platforms that have copy, in a fixed order.
    pub fn written(&self) -> Vec<(&'static str, &PlatformMeta)> {
        [("shorts", &self.shorts), ("reels", &self.reels), ("tiktok", &self.tiktok)]
            .into_iter()
            .filter_map(|(k, m)| m.as_ref().map(|m| (k, m)))
            .collect()
    }

    fn slot(&mut self, key: &str) -> Option<&mut Option<PlatformMeta>> {
        match key {
            "shorts" => Some(&mut self.shorts),
            "reels" => Some(&mut self.reels),
            "tiktok" => Some(&mut self.tiktok),
            _ => None,
        }
    }
}

/// An agent's verdict on an agent-checked campaign rule.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Mark {
    pub pass: bool,
    pub reason: String,
    pub at: DateTime<Utc>,
}

/// `extras.json` key `post`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Default, PartialEq, Debug)]
pub struct PostData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(default)]
    pub platforms: Platforms,
    /// Agent rule id -> verdict.
    #[serde(default)]
    pub marks: BTreeMap<String, Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

pub fn load_post(store: &ProjectStore, clip: &ClipId) -> Result<PostData> {
    Ok(extras::get(store, clip, KEY)?.unwrap_or_default())
}

/// Read-modify-write of the clip's `post` key (other extras keys untouched).
pub fn update_post<T>(store: &ProjectStore, clip: &ClipId, f: impl FnOnce(&mut PostData) -> Result<T>) -> Result<T> {
    let mut out = None;
    extras::update(store, clip, KEY, |cur| {
        let mut data: PostData = match cur {
            None | Some(Value::Null) => PostData::default(),
            Some(v) => serde_json::from_value(v)
                .map_err(|e| RustlerError::new(ErrorCode::Internal, "post", format!("extras.json `post` is invalid: {e}")))?,
        };
        out = Some(f(&mut data)?);
        data.updated_at = Some(Utc::now());
        serde_json::to_value(&data)
            .map(Some)
            .map_err(|e| RustlerError::new(ErrorCode::Internal, "post", e.to_string()))
    })?;
    out.ok_or_else(|| RustlerError::new(ErrorCode::Internal, "post", "post update did not run"))
}

/// `shorts|reels|tiktok` for a specific platform; `None` for `all`.
pub fn platform_key(p: Platform) -> Option<&'static str> {
    match p {
        Platform::Shorts => Some("shorts"),
        Platform::Reels => Some("reels"),
        Platform::Tiktok => Some("tiktok"),
        Platform::All => None,
    }
}

/// Hard limits `clip.meta_set` enforces. Platform rules change: verify money-critical ones.
fn limits(key: &str) -> Value {
    let (title, description, hashtags, note) = match key {
        "shorts" => (100, 5000, 15, "YouTube ignores ALL hashtags when a video has more than 15; the first 3 show above the title"),
        "reels" => (100, 2200, 30, "Reels has no title field: `title` is the first line / cover headline; keep 3-5 hashtags"),
        _ => (100, 2200, 30, "TikTok has no title field: `title` is the first line / on-screen headline; keep 3-5 hashtags"),
    };
    json!({
        "title_max_chars": title, "description_max_chars": description, "hashtags_max": hashtags,
        "hashtags_recommended": "3-5", "note": note,
    })
}

fn tidy_tags(items: &[String], sigil: char, what: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for s in items {
        let s = s.trim().trim_start_matches(sigil);
        if s.is_empty() {
            continue;
        }
        if s.chars().any(char::is_whitespace) {
            return Err(validation(format!("'{sigil}{s}' is not a valid {what} (no spaces)"), format!("write it as {sigil}name")));
        }
        let t = format!("{sigil}{s}");
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    Ok(out)
}

fn check_meta(key: &str, m: &PlatformMeta) -> Result<PlatformMeta> {
    let title = m.title.trim().to_string();
    let description = m.description.trim().to_string();
    if title.is_empty() || description.is_empty() {
        return Err(validation(
            format!("{key}: title and description must not be empty"),
            "write both; `rustler clip meta-context <clip_id>` shows what to base them on",
        ));
    }
    let hashtags = tidy_tags(&m.hashtags, '#', "hashtag")?;
    let mentions = tidy_tags(&m.mentions, '@', "mention")?;
    let lim = limits(key);
    let max = |k: &str| lim[k].as_u64().unwrap_or(u64::MAX) as usize;
    if title.chars().count() > max("title_max_chars") {
        return Err(validation(
            format!("{key}: title has {} characters, max {}", title.chars().count(), max("title_max_chars")),
            "shorten the title",
        ));
    }
    if description.chars().count() > max("description_max_chars") {
        return Err(validation(
            format!("{key}: description has {} characters, max {}", description.chars().count(), max("description_max_chars")),
            "shorten the description",
        ));
    }
    if hashtags.len() > max("hashtags_max") {
        return Err(validation(format!("{key}: {} hashtags, max {}", hashtags.len(), max("hashtags_max")), "use 3-5 focused hashtags"));
    }
    Ok(PlatformMeta { title, description, hashtags, mentions })
}

// ---------------------------------------------------------------------------------------------
// Shared helpers (export + QC)
// ---------------------------------------------------------------------------------------------

fn clip_duration(store: &ProjectStore, clip: &Clip) -> f64 {
    crate::fx_cmds::effective_duration_of(store, clip).unwrap_or_else(|_| clip.segments.iter().map(|s| s.end - s.start).sum())
}

/// The clip's spoken words on the output timeline, joined.
fn spoken_text(store: &ProjectStore, clip: &Clip) -> (String, Vec<String>) {
    match crate::clip_cmds::caption_words_for_clip(store, clip) {
        Ok((words, warnings)) => (words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "), warnings),
        Err(e) => (String::new(), vec![e.message]),
    }
}

/// The campaign attached to the clip and the compliance of the clip against it.
fn compliance_of(store: &ProjectStore, clip: &Clip, post: &PostData) -> Result<Option<(Campaign, Compliance)>> {
    let Some(cid) = &post.campaign_id else { return Ok(None) };
    let campaign = load_campaign(store, cid)?;
    let (spoken, _) = spoken_text(store, clip);
    let spoken = crate::transcript_cmds::normalize(&spoken);
    let subject = Subject { duration: clip_duration(store, clip), post, spoken: &spoken };
    let compliance = evaluate(&campaign, &subject);
    Ok(Some((campaign, compliance)))
}

/// The platform whose copy fills the top-level post fields: the campaign's platform, else the first written.
fn primary_platform<'a>(post: &'a PostData, campaign: Option<&Campaign>) -> Option<(&'static str, &'a PlatformMeta)> {
    let written = post.platforms.written();
    if let Some(key) = campaign.and_then(|c| platform_key(c.platform)) {
        if let Some(hit) = written.iter().find(|(k, _)| *k == key) {
            return Some(*hit);
        }
    }
    written.into_iter().next()
}

/// The `<name>.post.json` body for an export, plus warnings (missing copy, failing compliance).
pub(crate) fn post_json(store: &ProjectStore, clip: &Clip, file: &Path) -> Result<(Value, Vec<String>)> {
    let post = load_post(store, &clip.id)?;
    let mut warnings = Vec::new();
    let compliance = match compliance_of(store, clip, &post) {
        Ok(c) => c,
        Err(e) => {
            warnings.push(format!("campaign `{}` could not be loaded: {}", post.campaign_id.clone().unwrap_or_default(), e.message));
            None
        }
    };
    let campaign = compliance.as_ref().map(|(c, _)| c);
    let primary = primary_platform(&post, campaign);
    if primary.is_none() {
        warnings.push(format!(
            "no posting copy for clip {}: the post JSON has empty title/description. Write it: rustler clip meta-context {} then rustler clip meta-set",
            clip.id, clip.id
        ));
    }
    if let Some((_, c)) = &compliance {
        if !c.passed {
            warnings.push(format!("campaign compliance failed: {}", c.problems().join("; ")));
        }
    }
    let extra = |k: &str| extras::get::<Value>(store, &clip.id, k).ok().flatten();
    let campaign_json = match (&compliance, &post.campaign_id) {
        (Some((c, comp)), _) => json!({
            "id": c.id, "name": c.name, "platform": c.platform, "requirements": c.requirements(),
            "payout": c.payout, "compliance": comp,
        }),
        (None, Some(id)) => json!({"id": id, "missing": true}),
        (None, None) => Value::Null,
    };
    let body = json!({
        "schema": SCHEMA,
        "clip": clip.id,
        "name": clip.name,
        "file": file.file_name().map(|n| n.to_string_lossy().into_owned()),
        "duration": clip_duration(store, clip),
        "platform": primary.map(|(k, _)| k),
        "title": primary.map(|(_, m)| m.title.clone()),
        "description": primary.map(|(_, m)| m.description.clone()),
        "hashtags": primary.map(|(_, m)| m.hashtags.clone()).unwrap_or_default(),
        "mentions": primary.map(|(_, m)| m.mentions.clone()).unwrap_or_default(),
        "platforms": post.platforms,
        "meta_present": primary.is_some(),
        "cover": extra("cover"),
        "campaign": campaign_json,
        "marks": post.marks,
        "score": crate::director_cmds::report_score(store, &clip.id),
        "created_at": Utc::now(),
    });
    Ok((body, warnings))
}

/// The QC checks `meta_present` and `campaign_compliant` for a clip (appended by `qc.run`).
pub(crate) fn qc_checks(store: &ProjectStore, clip: &Clip) -> Vec<QcCheck> {
    let post = load_post(store, &clip.id).unwrap_or_default();
    let campaigns = list_campaigns(store).unwrap_or_default();
    let mut out = Vec::new();

    // meta_present: the agent wrote the copy. A hard requirement once the project uses campaigns.
    let wanted = post.campaign_id.as_ref().and_then(|id| campaigns.iter().find(|c| &c.id == id)).and_then(|c| platform_key(c.platform));
    let have: Vec<&str> = post.platforms.written().iter().map(|(k, _)| *k).collect();
    let present = match wanted {
        Some(k) => have.contains(&k),
        None => !have.is_empty(),
    };
    let required = post.campaign_id.is_some() || !campaigns.is_empty();
    out.push(QcCheck {
        name: "meta_present".to_string(),
        status: if present { QcStatus::Pass } else if required { QcStatus::Fail } else { QcStatus::Warn },
        value: json!({"platforms": have, "needs_platform": wanted}),
        expected: match wanted {
            Some(k) => format!("title, description and hashtags written for {k} (clip.meta_set)"),
            None => "title, description and hashtags written for at least one platform (clip.meta_set)".to_string(),
        },
        fix_hint: (!present).then(|| {
            format!(
                "write the posting copy yourself: `rustler clip meta-context {id}` shows the transcript, limits and campaign rules, then `rustler clip meta-set {id} --input-json @meta.json`",
                id = clip.id
            )
        }),
    });

    // campaign_compliant: only when the clip is attached to a campaign.
    out.push(match compliance_of(store, clip, &post) {
        Ok(None) => QcCheck {
            name: "campaign_compliant".to_string(),
            status: if campaigns.is_empty() { QcStatus::Pass } else { QcStatus::Warn },
            value: json!({"campaign": null, "campaigns_in_project": campaigns.len()}),
            expected: "the clip follows its campaign's rules".to_string(),
            fix_hint: (!campaigns.is_empty()).then(|| {
                format!("this project has campaigns but the clip is not attached to one: `rustler clip meta-set {} --input-json @meta.json` with campaign_id", clip.id)
            }),
        },
        Ok(Some((c, comp))) => QcCheck {
            name: "campaign_compliant".to_string(),
            status: if comp.passed { QcStatus::Pass } else { QcStatus::Fail },
            value: json!({"campaign": c.id, "rules": comp.rules}),
            expected: format!("every rule of campaign `{}` passes", c.id),
            fix_hint: (!comp.passed).then(|| {
                format!(
                    "fix: {}. Machine rules: edit the clip or `clip meta-set`; agent rules: check the clip then `rustler campaign mark {} <rule_id> --pass true --reason \"...\"`",
                    comp.problems().join("; "),
                    clip.id
                )
            }),
        },
        Err(e) => QcCheck {
            name: "campaign_compliant".to_string(),
            status: QcStatus::Fail,
            value: json!({"campaign": post.campaign_id, "error": e.message}),
            expected: "the clip's campaign exists and its rules pass".to_string(),
            fix_hint: Some(format!("{}; re-attach the clip with `rustler clip meta-set {}` (campaign_id)", e.message, clip.id)),
        },
    });
    out
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "clip.meta_context",
        "Everything you need to WRITE the posting copy for a clip (Rustler never writes it): the clip's spoken words and hook, per-platform limits, the attached campaign's rules with a live compliance preview, the project's brief, the copy already saved. Read it, write a viral title/description/hashtags yourself (see guide `campaigns-and-posting`), then clip.meta_set.",
        &["clip_id"],
        Annotations { read_only: true, ..Annotations::default() },
        vec![Example { description: "gather context before writing the copy", cli: "rustler clip meta-context clp_X" }],
        meta_context,
    ));
    reg.register(typed(
        "clip.meta_set",
        "Save the posting copy you wrote: per platform (shorts, reels, tiktok) a title, description, hashtags and mentions; optionally attach the clip to a campaign. Validates the platform limits; saved platforms are replaced one by one, others kept. export.clip writes it to <name>.post.json and QC checks `meta_present` and `campaign_compliant`.",
        &["clip_id"],
        Annotations::default(),
        vec![Example {
            description: "copy for Shorts and TikTok, attached to a campaign (use --input-json @meta.json on PowerShell)",
            cli: "rustler clip meta-set clp_X --input-json @meta.json",
        }],
        meta_set,
    ));
}

#[derive(Deserialize, JsonSchema)]
pub struct ContextIn {
    pub clip_id: ClipId,
}

async fn meta_context(ctx: Ctx, i: ContextIn) -> Result<Value> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let post = load_post(&store, &clip.id)?;
    let (spoken, warnings) = spoken_text(&store, &clip);
    let spoken_short: String = spoken.chars().take(TRANSCRIPT_CHARS).collect();
    let overlays = crate::overlay_cmds::load_overlays(&store, &clip.id).unwrap_or_default();
    let mut texts: Vec<_> = overlays.iter().collect();
    texts.sort_by(|a, b| a.start.total_cmp(&b.start));
    let hook = texts
        .iter()
        .find(|o| serde_json::to_value(&o.style).is_ok_and(|s| s == "hook-top"))
        .or(texts.first())
        .map(|o| o.text.clone())
        .or_else(|| spoken.split_inclusive(['.', '?', '!']).next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));

    let campaigns = list_campaigns(&store)?;
    let attached = post.campaign_id.as_ref().and_then(|id| campaigns.iter().find(|c| &c.id == id));
    let campaign = match compliance_of(&store, &clip, &post) {
        Ok(Some((c, comp))) => json!({
            "id": c.id, "name": c.name, "platform": c.platform, "payout": c.payout, "notes": c.notes,
            "requirements": c.requirements(), "rules": c.rules, "compliance_now": comp,
        }),
        Ok(None) => Value::Null,
        Err(e) => json!({"id": post.campaign_id, "error": e.message}),
    };
    let wanted = attached.and_then(|c| platform_key(c.platform));
    let limits_by_platform: Value = ["shorts", "reels", "tiktok"].iter().map(|k| (k.to_string(), limits(k))).collect::<serde_json::Map<_, _>>().into();
    let mut next = vec![
        "write the title, description and hashtags yourself from `transcript` and `hook` (guide: rustler guide show campaigns-and-posting; prompt: write_clip_meta)".to_string(),
        format!("save it: rustler clip meta-set {} --input-json @meta.json", clip.id),
    ];
    if post.campaign_id.is_none() && !campaigns.is_empty() {
        next.insert(0, format!("pick the campaign this clip is for (campaigns_available) and pass its id as campaign_id in clip.meta_set; for agent rules then `rustler campaign mark {} <rule_id> --pass true --reason \"...\"`", clip.id));
    }
    Ok(json!({
        "clip": {"id": clip.id, "name": clip.name, "duration": clip_duration(&store, &clip), "segments": clip.segments.len()},
        "hook": hook,
        "overlay_texts": texts.iter().map(|o| json!({"start": o.start, "text": o.text})).collect::<Vec<_>>(),
        "transcript": spoken_short,
        "transcript_truncated": spoken.chars().count() > TRANSCRIPT_CHARS,
        "platform_limits": limits_by_platform,
        "needs_platform": wanted,
        "campaign": campaign,
        "campaigns_available": campaigns.iter().map(|c| json!({"id": c.id, "name": c.name, "platform": c.platform})).collect::<Vec<_>>(),
        "brief": load_brief(&store).ok().flatten(),
        "saved": post,
        "warnings": warnings,
        "next": next,
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct SetIn {
    pub clip_id: ClipId,
    /// Attach the clip to this campaign (must exist). An empty string detaches it. Changing the campaign clears the agent-rule marks.
    pub campaign_id: Option<String>,
    /// Copy per platform: `{"shorts": {"title", "description", "hashtags", "mentions"}, "reels": .., "tiktok": ..}`.
    #[serde(default)]
    pub platforms: Platforms,
}

async fn meta_set(ctx: Ctx, i: SetIn) -> Result<Value> {
    let store = ctx.store()?;
    store.load_clip(&i.clip_id)?;
    let mut checked: Vec<(&'static str, PlatformMeta)> = Vec::new();
    for (key, m) in i.platforms.written() {
        checked.push((key, check_meta(key, m)?));
    }
    let campaign_change: Option<Option<String>> = match i.campaign_id.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(id) => Some(Some(load_campaign(&store, id)?.id)),
    };
    if checked.is_empty() && campaign_change.is_none() {
        return Err(validation(
            "nothing to save",
            "pass platforms {\"shorts\": {title, description, hashtags, mentions}} and/or campaign_id",
        ));
    }
    let mut saved = PostData::default();
    if ctx.dry_run {
        saved = load_post(&store, &i.clip_id)?;
    } else {
        update_post(&store, &i.clip_id, |p| {
            for (key, meta) in &checked {
                if let Some(slot) = p.platforms.slot(key) {
                    *slot = Some(meta.clone());
                }
            }
            if let Some(change) = &campaign_change {
                if p.campaign_id != *change {
                    p.marks.clear();
                }
                p.campaign_id = change.clone();
            }
            saved = p.clone();
            Ok(())
        })?;
        crate::clip_cmds::log_event(
            &ctx,
            &store,
            "clip.meta_set",
            json!({"clip_id": i.clip_id, "campaign_id": saved.campaign_id, "platforms": checked.iter().map(|(k, _)| *k).collect::<Vec<_>>()}),
        )?;
    }
    let clip = store.load_clip(&i.clip_id)?;
    let compliance = match compliance_of(&store, &clip, &saved) {
        Ok(Some((_, c))) => json!(c),
        Ok(None) => Value::Null,
        Err(e) => json!({"error": e.message}),
    };
    let pending: Vec<String> = match &compliance {
        Value::Object(_) => serde_json::from_value::<Compliance>(compliance.clone())
            .map(|c| c.rules.into_iter().filter(|r| r.status != crate::campaign_cmds::RuleStatus::Pass).map(|r| r.id).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(json!({
        "clip_id": i.clip_id,
        "saved_platforms": checked.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        "post": saved,
        "compliance_now": compliance,
        "dry_run": ctx.dry_run,
        "next": if pending.is_empty() {
            vec![format!("rustler qc run {id}, then rustler export clip {id}", id = i.clip_id)]
        } else {
            vec![format!("rules not passing yet: {} (machine rules: fix the clip or copy; agent rules: rustler campaign mark {} <rule_id> --pass true --reason \"...\")", pending.join(", "), i.clip_id)]
        },
    }))
}
