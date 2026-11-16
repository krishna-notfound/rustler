//! Commands: campaign.create, campaign.show, campaign.list, campaign.delete, campaign.mark (v2 Task 1.7).
//!
//! A *campaign* is one job's rules (platform, payout, required hashtags/mentions, duration range and a list
//! of rules). A project can hold many (`<project>/campaigns/<id>.json`); the single-brief commands
//! (`brief.*`) keep working beside them. A clip is attached to a campaign by `clip.meta_set --campaign-id`.
//!
//! Rules are `machine` (Rustler checks them from a `check`: duration, hashtag, mention, banned word, text in
//! the description, title/hashtag limits) or `agent` (judgement calls Rustler cannot make, e.g. "must show
//! the product"). The agent records an agent rule's outcome with `campaign.mark <clip> <rule_id> --pass
//! true|false --reason "<what you saw>"`; an unmarked agent rule counts as not yet compliant.
//!
//! [`evaluate`] is the one place a clip is checked against a campaign; QC (`campaign_compliant`) and the
//! export's `<name>.post.json` both use it.

use crate::brief_cmds::{Payout, Platform};
use crate::post_cmds::{Mark, PlatformMeta, PostData, load_post, update_post};
use crate::transcript_cmds::normalize;
use chrono::{DateTime, Utc};
use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

pub const SCHEMA: &str = "rustler.campaign@1";
const DIR: &str = "campaigns";
/// Tolerance when comparing durations to a range (seconds).
const DURATION_EPS: f64 = 0.05;
/// Rule ids generated from the campaign's own fields; user rules may not reuse them.
const IMPLICIT_IDS: [&str; 3] = ["duration", "hashtags_required", "mentions_required"];

fn validation(msg: impl Into<String>, hint: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, "campaign", msg).hint(hint)
}

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// Rustler checks it from `check`.
    Machine,
    /// The agent checks it and records the outcome with `campaign.mark`.
    Agent,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CheckType {
    /// Clip duration (seconds) >= value.
    DurationMin,
    /// Clip duration (seconds) <= value.
    DurationMax,
    /// The hashtag (value, with or without #) is in the hashtags or the description.
    Hashtag,
    /// The mention (value, with or without @) is in the mentions or the description.
    Mention,
    /// The word or phrase (value) is in none of the title, description, hashtags or the spoken words.
    BannedWord,
    /// The text (value) is in the description (case-insensitive).
    DescriptionContains,
    /// The title has at most `value` characters.
    TitleMaxChars,
    /// At most `value` hashtags.
    HashtagsMax,
}

impl CheckType {
    fn wants_number(self) -> bool {
        matches!(self, Self::DurationMin | Self::DurationMax | Self::TitleMaxChars | Self::HashtagsMax)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Check {
    #[serde(rename = "type")]
    pub kind: CheckType,
    /// A number (duration_*, title_max_chars, hashtags_max) or a string (the rest).
    #[schemars(with = "serde_json::Value")]
    pub value: Value,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Rule {
    /// Short id, letters/digits/`_`/`-`, unique in the campaign (used by `campaign.mark`).
    pub id: String,
    /// The requirement in the brief's words; shown to the agent and written to the post JSON.
    pub text: String,
    pub kind: RuleKind,
    /// Required for `machine` rules, not allowed for `agent` rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Check>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug, Default)]
pub struct DurationRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// `<project>/campaigns/<id>.json`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Campaign {
    pub schema: String,
    pub id: String,
    pub name: String,
    pub platform: Platform,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payout: Option<Payout>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashtags_required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions_required: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<DurationRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Campaign {
    /// Every requirement as one sentence: the user rules plus the ones implied by the fields.
    pub fn requirements(&self) -> Vec<String> {
        let mut out: Vec<String> = self.rules.iter().map(|r| r.text.clone()).collect();
        if let Some(d) = &self.duration {
            out.push(duration_text(d));
        }
        if !self.hashtags_required.is_empty() {
            out.push(format!("Hashtags required: {}", self.hashtags_required.join(" ")));
        }
        if !self.mentions_required.is_empty() {
            out.push(format!("Mentions required: {}", self.mentions_required.join(" ")));
        }
        out
    }
}

fn duration_text(d: &DurationRange) -> String {
    match (d.min, d.max) {
        (Some(a), Some(b)) => format!("Duration {a}-{b} s"),
        (Some(a), None) => format!("Duration at least {a} s"),
        (None, Some(b)) => format!("Duration at most {b} s"),
        (None, None) => "Duration".to_string(),
    }
}

pub fn campaign_rel(id: &str) -> PathBuf {
    Path::new(DIR).join(format!("{id}.json"))
}

fn valid_id(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// One campaign; `NotFound` (listing the ids that exist) when there is none with that id.
pub fn load_campaign(store: &ProjectStore, id: &str) -> Result<Campaign> {
    if valid_id(id, 48) && store.resolve(&campaign_rel(id)).is_file() {
        return store.read_json(&campaign_rel(id));
    }
    let ids: Vec<String> = list_campaigns(store)?.into_iter().map(|c| c.id).collect();
    Err(RustlerError::new(ErrorCode::NotFound, "campaign", format!("no campaign `{id}`")).hint(if ids.is_empty() {
        "create one: rustler campaign create <id> --name \"...\" (see `rustler campaign create --help`)".to_string()
    } else {
        format!("campaigns in this project: {}", ids.join(", "))
    }))
}

/// All campaigns of the project, by id.
pub fn list_campaigns(store: &ProjectStore) -> Result<Vec<Campaign>> {
    let dir = store.resolve(Path::new(DIR));
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_some_and(|x| x == "json") {
            if let Some(rel) = path.file_name() {
                out.push(store.read_json::<Campaign>(&Path::new(DIR).join(rel))?);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn tags(items: &[String], sigil: char, what: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for s in items {
        let s = s.trim().trim_start_matches(sigil);
        if s.is_empty() || s.chars().any(char::is_whitespace) {
            return Err(validation(format!("'{sigil}{s}' is not a valid {what} (no spaces)"), format!("write it as {sigil}name")));
        }
        let t = format!("{sigil}{s}");
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    Ok(out)
}

fn check_rule(r: &Rule) -> Result<()> {
    if !valid_id(&r.id, 48) {
        return Err(validation(format!("bad rule id `{}`", r.id), "use letters, digits, `_` or `-` (max 48)"));
    }
    if IMPLICIT_IDS.contains(&r.id.as_str()) {
        return Err(validation(
            format!("rule id `{}` is reserved", r.id),
            "`duration`, `hashtags_required` and `mentions_required` are generated from the campaign's own fields",
        ));
    }
    if r.text.trim().is_empty() {
        return Err(validation(format!("rule `{}` has no text", r.id), "put the requirement in the brief's words"));
    }
    match (r.kind, &r.check) {
        (RuleKind::Machine, None) => Err(validation(
            format!("machine rule `{}` needs a check", r.id),
            "add {\"type\": \"hashtag\", \"value\": \"#ad\"} (types: duration_min, duration_max, hashtag, mention, banned_word, description_contains, title_max_chars, hashtags_max), or make it an agent rule",
        )),
        (RuleKind::Agent, Some(_)) => Err(validation(
            format!("agent rule `{}` takes no check", r.id),
            "agent rules are judged by you and recorded with `campaign.mark`; drop `check` or make it a machine rule",
        )),
        (RuleKind::Machine, Some(c)) => {
            let ok = if c.kind.wants_number() {
                c.value.as_f64().is_some_and(|n| n >= 0.0)
            } else {
                c.value.as_str().is_some_and(|s| !s.trim().is_empty())
            };
            if ok {
                Ok(())
            } else {
                Err(validation(
                    format!("rule `{}`: check value {} does not fit type {:?}", r.id, c.value, c.kind),
                    if c.kind.wants_number() { "this check takes a number" } else { "this check takes a non-empty string" },
                ))
            }
        }
        (RuleKind::Agent, None) => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------
// Compliance
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RuleStatus {
    Pass,
    Fail,
    /// An agent rule nobody marked yet.
    Pending,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct RuleResult {
    pub id: String,
    pub text: String,
    pub kind: RuleKind,
    pub status: RuleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct Compliance {
    pub passed: bool,
    pub rules: Vec<RuleResult>,
}

impl Compliance {
    /// `id: reason` of every rule that is not passing.
    pub fn problems(&self) -> Vec<String> {
        self.rules
            .iter()
            .filter(|r| r.status != RuleStatus::Pass)
            .map(|r| {
                let why = r.reason.clone().unwrap_or_else(|| "not checked".to_string());
                format!("{} ({}): {why}", r.id, r.text)
            })
            .collect()
    }
}

/// What a campaign is checked against.
pub struct Subject<'a> {
    /// Effective clip duration in seconds.
    pub duration: f64,
    pub post: &'a PostData,
    /// Normalized spoken words of the clip (for `banned_word`).
    pub spoken: &'a str,
}

fn has_tag(meta: &PlatformMeta, tag: &str, sigil: char) -> bool {
    let want = format!("{sigil}{}", tag.trim().trim_start_matches(sigil)).to_lowercase();
    let list = if sigil == '#' { &meta.hashtags } else { &meta.mentions };
    list.iter().any(|t| t.to_lowercase() == want)
        || meta
            .description
            .to_lowercase()
            .split(|c: char| c.is_whitespace() || ",.;:!?()[]".contains(c))
            .any(|w| w == want)
}

/// Runs one text check against one platform's copy; `Err(reason)` when it fails.
fn text_check(c: &Check, meta: &PlatformMeta, spoken: &str) -> std::result::Result<(), String> {
    let s = c.value.as_str().unwrap_or_default();
    let n = c.value.as_f64().unwrap_or(0.0);
    match c.kind {
        CheckType::Hashtag if !has_tag(meta, s, '#') => Err(format!("hashtag #{} is missing", s.trim_start_matches('#'))),
        CheckType::Mention if !has_tag(meta, s, '@') => Err(format!("mention @{} is missing", s.trim_start_matches('@'))),
        CheckType::DescriptionContains if !meta.description.to_lowercase().contains(&s.trim().to_lowercase()) => {
            Err(format!("the description does not contain \"{s}\""))
        }
        CheckType::TitleMaxChars if meta.title.chars().count() as f64 > n => {
            Err(format!("title has {} characters, max {n}", meta.title.chars().count()))
        }
        CheckType::HashtagsMax if meta.hashtags.len() as f64 > n => Err(format!("{} hashtags, max {n}", meta.hashtags.len())),
        CheckType::BannedWord => {
            let word = normalize(s);
            let padded = |t: &str| format!(" {} ", normalize(t));
            let needle = format!(" {word} ");
            let in_copy = [&meta.title, &meta.description, &meta.hashtags.join(" ")].iter().any(|t| padded(t).contains(&needle));
            if in_copy {
                Err(format!("banned \"{s}\" appears in the title/description/hashtags"))
            } else if padded(spoken).contains(&needle) {
                Err(format!("banned \"{s}\" is spoken in the clip"))
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

/// The copy a campaign's text checks run on: its platform's, or every written platform for `all`.
fn targets<'a>(c: &Campaign, post: &'a PostData) -> std::result::Result<Vec<&'a PlatformMeta>, String> {
    let written = post.platforms.written();
    match c.platform {
        Platform::All => {
            if written.is_empty() {
                Err("no posting copy yet (clip.meta_set)".to_string())
            } else {
                Ok(written.into_iter().map(|(_, m)| m).collect())
            }
        }
        p => {
            let key = crate::post_cmds::platform_key(p).unwrap_or_default();
            written
                .into_iter()
                .find(|(k, _)| *k == key)
                .map(|(_, m)| vec![m])
                .ok_or_else(|| format!("no posting copy for {key} yet (clip.meta_set)"))
        }
    }
}

fn push_rule(rules: &mut Vec<RuleResult>, id: &str, text: String, kind: RuleKind, outcome: std::result::Result<(), String>) {
    let (status, reason) = match outcome {
        Ok(()) => (RuleStatus::Pass, None),
        Err(r) => (RuleStatus::Fail, Some(r)),
    };
    rules.push(RuleResult { id: id.to_string(), text, kind, status, reason });
}

/// Checks `subject` against `campaign`: user rules, then the rules implied by the campaign's own fields.
pub fn evaluate(campaign: &Campaign, subject: &Subject<'_>) -> Compliance {
    let copy = targets(campaign, subject.post);
    let mut rules: Vec<RuleResult> = Vec::new();
    let duration_ok = |min: Option<f64>, max: Option<f64>| -> std::result::Result<(), String> {
        let d = subject.duration;
        if min.is_some_and(|m| d + DURATION_EPS < m) {
            return Err(format!("duration {d:.1} s is below {} s", min.unwrap_or_default()));
        }
        if max.is_some_and(|m| d - DURATION_EPS > m) {
            return Err(format!("duration {d:.1} s is above {} s", max.unwrap_or_default()));
        }
        Ok(())
    };
    let over_copy = |f: &dyn Fn(&PlatformMeta) -> std::result::Result<(), String>| -> std::result::Result<(), String> {
        match &copy {
            Err(e) => Err(e.clone()),
            Ok(list) => list.iter().try_for_each(|m| f(m)),
        }
    };

    for r in &campaign.rules {
        match (r.kind, &r.check) {
            (RuleKind::Machine, Some(c)) => {
                let outcome = match c.kind {
                    CheckType::DurationMin => duration_ok(c.value.as_f64(), None),
                    CheckType::DurationMax => duration_ok(None, c.value.as_f64()),
                    _ => over_copy(&|m| text_check(c, m, subject.spoken)),
                };
                push_rule(&mut rules, &r.id, r.text.clone(), r.kind, outcome);
            }
            (RuleKind::Machine, None) => push_rule(&mut rules, &r.id, r.text.clone(), r.kind, Err("rule has no check".to_string())),
            (RuleKind::Agent, _) => {
                let (status, reason) = match subject.post.marks.get(&r.id) {
                    Some(m) if m.pass => (RuleStatus::Pass, Some(m.reason.clone())),
                    Some(m) => (RuleStatus::Fail, Some(m.reason.clone())),
                    None => (
                        RuleStatus::Pending,
                        Some("not checked yet: look at the clip, then `campaign.mark` with --pass and --reason".to_string()),
                    ),
                };
                rules.push(RuleResult { id: r.id.clone(), text: r.text.clone(), kind: r.kind, status, reason });
            }
        }
    }
    if let Some(d) = &campaign.duration {
        push_rule(&mut rules, "duration", duration_text(d), RuleKind::Machine, duration_ok(d.min, d.max));
    }
    if !campaign.hashtags_required.is_empty() {
        let outcome = over_copy(&|m| {
            let missing: Vec<&String> = campaign.hashtags_required.iter().filter(|h| !has_tag(m, h, '#')).collect();
            if missing.is_empty() { Ok(()) } else { Err(format!("missing {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "))) }
        });
        push_rule(&mut rules, "hashtags_required", format!("Hashtags required: {}", campaign.hashtags_required.join(" ")), RuleKind::Machine, outcome);
    }
    if !campaign.mentions_required.is_empty() {
        let outcome = over_copy(&|m| {
            let missing: Vec<&String> = campaign.mentions_required.iter().filter(|h| !has_tag(m, h, '@')).collect();
            if missing.is_empty() { Ok(()) } else { Err(format!("missing {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "))) }
        });
        push_rule(&mut rules, "mentions_required", format!("Mentions required: {}", campaign.mentions_required.join(" ")), RuleKind::Machine, outcome);
    }
    let passed = rules.iter().all(|r| r.status == RuleStatus::Pass);
    Compliance { passed, rules }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "campaign.create",
        "Create a campaign: one job's rules (platform, payout, required hashtags/mentions, duration range, machine and agent rules). A project can hold many; brief.* keeps working. Machine rules carry a check Rustler tests (duration_min, duration_max, hashtag, mention, banned_word, description_contains, title_max_chars, hashtags_max); agent rules you judge and record with campaign.mark. Pass --input-json @file for the rules list.",
        &["id"],
        Annotations::default(),
        vec![Example {
            description: "a per-view campaign (rules from a file)",
            cli: "rustler campaign create spring-promo --input-json @spring.json",
        }],
        campaign_create,
    ));
    reg.register(typed(
        "campaign.show",
        "Show one campaign with every requirement (rules plus the ones implied by duration/hashtags/mentions).",
        &["id"],
        Annotations { read_only: true, ..Annotations::default() },
        vec![Example { description: "read the rules before editing", cli: "rustler campaign show spring-promo" }],
        campaign_show,
    ));
    reg.register(typed(
        "campaign.list",
        "List the project's campaigns.",
        &[],
        Annotations { read_only: true, ..Annotations::default() },
        vec![Example { description: "which campaigns exist?", cli: "rustler campaign list" }],
        campaign_list,
    ));
    reg.register(typed(
        "campaign.delete",
        "Delete a campaign file. Clips attached to it keep the id and fail QC `campaign_compliant` until re-attached.",
        &["id"],
        Annotations { destructive: true, ..Annotations::default() },
        vec![Example { description: "remove a finished campaign", cli: "rustler campaign delete spring-promo" }],
        campaign_delete,
    ));
    reg.register(typed(
        "campaign.mark",
        "Record the outcome of an agent-checked rule for a clip: pass or fail plus the reason (what you saw). The clip must be attached to the campaign (clip.meta_set --campaign-id). Machine rules are checked by Rustler and cannot be marked.",
        &["clip_id", "rule_id"],
        Annotations::default(),
        vec![Example {
            description: "the product is on screen at 0:07",
            cli: "rustler campaign mark clp_X shows-product --pass true --reason \"product visible 0:05-0:09, checked the contact sheet\"",
        }],
        campaign_mark,
    ));
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateIn {
    /// Short id: letters, digits, `_` or `-` (max 48), e.g. `spring-promo`.
    pub id: String,
    /// Display name. Default: the id.
    pub name: Option<String>,
    /// tiktok | reels | shorts | all (default all). A specific platform makes that platform's copy the one checked.
    pub platform: Option<Platform>,
    /// Rules `{id, text, kind: machine|agent, check?: {type, value}}`.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// `{model: per_view|per_clip, rate?, currency?}`.
    pub payout: Option<Payout>,
    /// Hashtags every post must carry (a leading # is added when missing).
    #[serde(default)]
    pub hashtags_required: Vec<String>,
    /// Mentions every post must carry (a leading @ is added when missing).
    #[serde(default)]
    pub mentions_required: Vec<String>,
    /// Allowed clip duration in seconds `{min?, max?}`.
    pub duration: Option<DurationRange>,
    pub notes: Option<String>,
    /// Overwrite an existing campaign with this id.
    #[serde(default)]
    pub replace: bool,
}

async fn campaign_create(ctx: Ctx, i: CreateIn) -> Result<Value> {
    let store = ctx.store()?;
    let id = i.id.trim().to_string();
    if !valid_id(&id, 48) {
        return Err(validation(format!("bad campaign id `{}`", i.id), "use letters, digits, `_` or `-` (max 48), e.g. spring-promo"));
    }
    if store.resolve(&campaign_rel(&id)).is_file() && !i.replace {
        return Err(validation(format!("campaign `{id}` already exists"), "use --replace true to overwrite it, or pick another id"));
    }
    let mut seen: Vec<&str> = Vec::new();
    for r in &i.rules {
        check_rule(r)?;
        if seen.contains(&r.id.as_str()) {
            return Err(validation(format!("duplicate rule id `{}`", r.id), "rule ids must be unique in a campaign"));
        }
        seen.push(&r.id);
    }
    if let Some(d) = &i.duration {
        let bad = |v: Option<f64>| v.is_some_and(|x| !x.is_finite() || x <= 0.0);
        if bad(d.min) || bad(d.max) || d.min.zip(d.max).is_some_and(|(a, b)| a > b) {
            return Err(validation("duration needs 0 < min <= max (seconds)", "e.g. {\"min\": 20, \"max\": 60}"));
        }
    }
    if let Some(Payout { rate: Some(r), .. }) = &i.payout {
        if !r.is_finite() || *r < 0.0 {
            return Err(validation("payout rate must be >= 0", "per 1000 views (per_view) or per accepted clip (per_clip)"));
        }
    }
    let campaign = Campaign {
        schema: SCHEMA.to_string(),
        name: i.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone()),
        id: id.clone(),
        platform: i.platform.unwrap_or(Platform::All),
        rules: i.rules,
        payout: i.payout,
        hashtags_required: tags(&i.hashtags_required, '#', "hashtag")?,
        mentions_required: tags(&i.mentions_required, '@', "mention")?,
        duration: i.duration,
        notes: i.notes.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        created_at: Utc::now(),
    };
    let replaced = store.resolve(&campaign_rel(&id)).is_file();
    if !ctx.dry_run {
        store.write_json(&campaign_rel(&id), &campaign)?;
        crate::clip_cmds::log_event(&ctx, &store, "campaign.create", json!({"campaign_id": id, "replaced": replaced}))?;
    }
    Ok(json!({
        "campaign": campaign,
        "requirements": campaign.requirements(),
        "replaced": replaced,
        "dry_run": ctx.dry_run,
        "next": [
            format!("attach clips and write their copy: rustler clip meta-context <clip_id>, then rustler clip meta-set <clip_id> --input-json @meta.json (with campaign_id \"{id}\")"),
        ],
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct IdIn {
    pub id: String,
}

async fn campaign_show(ctx: Ctx, i: IdIn) -> Result<Value> {
    let store = ctx.store()?;
    let c = load_campaign(&store, i.id.trim())?;
    Ok(json!({ "requirements": c.requirements(), "campaign": c }))
}

#[derive(Deserialize, JsonSchema)]
pub struct ListIn {}

async fn campaign_list(ctx: Ctx, _i: ListIn) -> Result<Value> {
    let store = ctx.store()?;
    let list = list_campaigns(&store)?;
    let rows: Vec<Value> = list
        .iter()
        .map(|c| {
            json!({
                "id": c.id, "name": c.name, "platform": c.platform, "rules": c.rules.len(),
                "agent_rules": c.rules.iter().filter(|r| r.kind == RuleKind::Agent).count(),
                "payout": c.payout, "duration": c.duration,
            })
        })
        .collect();
    Ok(json!({
        "count": rows.len(),
        "campaigns": rows,
        "next": if rows.is_empty() { vec!["create one: rustler campaign create <id> --input-json @campaign.json"] } else { vec!["read the rules: rustler campaign show <id>"] },
    }))
}

async fn campaign_delete(ctx: Ctx, i: IdIn) -> Result<Value> {
    let store = ctx.store()?;
    let id = i.id.trim();
    let c = load_campaign(&store, id)?;
    if !ctx.dry_run {
        std::fs::remove_file(store.resolve(&campaign_rel(&c.id))).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "campaign", format!("cannot delete campaign `{id}`: {e}"))
        })?;
        crate::clip_cmds::log_event(&ctx, &store, "campaign.delete", json!({"campaign_id": c.id}))?;
    }
    Ok(json!({ "deleted": c.id, "dry_run": ctx.dry_run }))
}

#[derive(Deserialize, JsonSchema)]
pub struct MarkIn {
    pub clip_id: ClipId,
    /// The agent rule's id (see `campaign show`).
    pub rule_id: String,
    /// Did the clip satisfy the rule?
    pub pass: bool,
    /// What you checked and saw (required): this is the audit trail in the post JSON.
    pub reason: String,
}

async fn campaign_mark(ctx: Ctx, i: MarkIn) -> Result<Value> {
    let store = ctx.store()?;
    store.load_clip(&i.clip_id)?;
    let reason = i.reason.trim().to_string();
    if reason.is_empty() {
        return Err(validation(
            "a reason is required",
            "say what you checked and saw, e.g. --reason \"product visible 0:05-0:09 in the contact sheet\"",
        ));
    }
    let post = load_post(&store, &i.clip_id)?;
    let Some(cid) = post.campaign_id.clone() else {
        return Err(validation(
            format!("clip {} is not attached to a campaign", i.clip_id),
            "attach it first: rustler clip meta-set <clip_id> --input-json @meta.json (with campaign_id)",
        ));
    };
    let campaign = load_campaign(&store, &cid)?;
    let Some(rule) = campaign.rules.iter().find(|r| r.id == i.rule_id) else {
        let ids: Vec<&str> = campaign.rules.iter().filter(|r| r.kind == RuleKind::Agent).map(|r| r.id.as_str()).collect();
        return Err(validation(
            format!("campaign `{cid}` has no rule `{}`", i.rule_id),
            format!("agent rules you can mark: {}", if ids.is_empty() { "(none)".to_string() } else { ids.join(", ") }),
        ));
    };
    if rule.kind == RuleKind::Machine {
        return Err(validation(
            format!("rule `{}` is a machine rule", rule.id),
            "Rustler checks machine rules itself (QC `campaign_compliant`); only agent rules are marked",
        ));
    }
    let mark = Mark { pass: i.pass, reason, at: Utc::now() };
    if !ctx.dry_run {
        update_post(&store, &i.clip_id, |p| {
            p.marks.insert(rule.id.clone(), mark.clone());
            Ok(())
        })?;
        crate::clip_cmds::log_event(
            &ctx,
            &store,
            "campaign.mark",
            json!({"clip_id": i.clip_id, "campaign_id": cid, "rule_id": rule.id, "pass": i.pass}),
        )?;
    }
    let marks: BTreeMap<String, Mark> = {
        let mut m = post.marks;
        m.insert(rule.id.clone(), mark.clone());
        m
    };
    let pending: Vec<&str> = campaign
        .rules
        .iter()
        .filter(|r| r.kind == RuleKind::Agent && !marks.contains_key(&r.id))
        .map(|r| r.id.as_str())
        .collect();
    Ok(json!({
        "clip_id": i.clip_id, "campaign_id": cid, "rule_id": rule.id, "mark": mark,
        "pending_agent_rules": pending, "dry_run": ctx.dry_run,
        "next": if pending.is_empty() { vec!["all agent rules are marked: rustler qc run <clip_id>".to_string()] } else { vec![format!("still to check: {}", pending.join(", "))] },
    }))
}
