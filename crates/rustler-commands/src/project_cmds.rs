//! Commands: project.create, project.show, project.set_permissions, project.set_settings (Task 10a).
//!
//! Also home of small helpers shared by the other command modules: [`Outcome`] (dry-run
//! envelope), [`log_event`], [`update_project`], time formatting and flexible input parsing.

use rustler_core::{
    CaptionScript, ErrorCode, Event, Permissions, Project, ProjectStore, RustlerError, SourceId,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------------------

/// Result of a mutating command: either the real output or, with `--dry-run`, a description
/// of what would have happened (nothing is written).
#[derive(Serialize, JsonSchema, Debug)]
#[serde(untagged)]
pub enum Outcome<T> {
    Done(T),
    DryRun(DryRunOut),
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct DryRunOut {
    pub dry_run: bool,
    pub would: Value,
}

impl<T> Outcome<T> {
    pub fn dry_run(would: Value) -> Self {
        Outcome::DryRun(DryRunOut { dry_run: true, would })
    }
}

/// Appends an event (`events.ndjson`) attributed to the invoking actor.
pub fn log_event(ctx: &Ctx, store: &ProjectStore, kind: &str, data: Value) -> rustler_core::Result<()> {
    let project_revision = store.load_project().map(|p| p.revision).unwrap_or(0);
    store.append_event(&Event {
        ts: chrono::Utc::now(),
        actor: ctx.actor,
        kind: kind.to_string(),
        project_revision,
        data,
    })
}

/// Loads the project, applies `f`, saves with a revision check; retries a few times on a
/// revision conflict (another command saved in between).
pub fn update_project(
    store: &ProjectStore,
    mut f: impl FnMut(&mut Project) -> rustler_core::Result<()>,
) -> rustler_core::Result<Project> {
    let mut last = None;
    for _ in 0..4 {
        let mut p = store.load_project()?;
        let base = p.revision;
        f(&mut p)?;
        match store.save_project(&mut p, base) {
            Ok(()) => return Ok(p),
            Err(e) if e.code == ErrorCode::RevisionConflict => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.expect("loop ran at least once"))
}

/// `MM:SS` (or `H:MM:SS` from one hour up).
pub fn mmss(t: f64) -> String {
    let total = t.max(0.0).floor() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// `HH:MM:SS.t` with one decimal.
pub fn hmsd(t: f64) -> String {
    let tenths = (t.max(0.0) * 10.0).floor() as u64;
    let (h, rem) = (tenths / 36000, tenths % 36000);
    let (m, rem) = (rem / 600, rem % 600);
    format!("{h:02}:{m:02}:{:02}.{}", rem / 10, rem % 10)
}

/// Accepts `"a"`, `["a","b"]` or `"a,b"` (CLI flags give a bare string for a single value).
pub fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<V>::deserialize(d)? {
        None => None,
        Some(V::One(s)) => Some(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()),
        Some(V::Many(v)) => Some(v),
    })
}

/// `true` when `path` lies under one of `roots` (empty roots = anything is allowed).
pub fn path_allowed(roots: &[PathBuf], path: &Path) -> bool {
    if roots.is_empty() {
        return true;
    }
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let target = canon(path);
    roots.iter().any(|r| target.starts_with(canon(r)))
}

/// Validation error unless the project permits network access (and downloads when asked).
pub fn require_network(project: &Project, downloads: bool, what: &str) -> rustler_core::Result<()> {
    let p = &project.permissions;
    if !p.network || (downloads && !p.downloads) {
        let flag = if !p.network { "--network true" } else { "--downloads true" };
        return Err(RustlerError::new(
            ErrorCode::Validation,
            "permissions",
            format!("{what} is not allowed: this project's permissions forbid {}", if !p.network { "network access" } else { "downloads" }),
        )
        .hint(format!("enable it with `rustler project set-permissions {flag}` or import a local file instead")));
    }
    Ok(())
}

pub(crate) fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "project".to_string() } else { s }
}

pub(crate) fn validation(stage: &str, msg: impl Into<String>, hint: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg).hint(hint)
}

// ---------------------------------------------------------------------------------------
// project.create
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct CreateIn {
    /// Project name.
    pub name: String,
    /// Directory to create the project in. Default: `--project`, else `./<slug of name>`.
    pub dir: Option<PathBuf>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct CreateOut {
    pub project_dir: PathBuf,
    pub project: Project,
    pub next: Vec<String>,
}

pub async fn create(ctx: Ctx, input: CreateIn) -> rustler_core::Result<Outcome<CreateOut>> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(validation("project.create", "project name is empty", "pass a name: `rustler project create \"My project\"`"));
    }
    let dir = match (&input.dir, &ctx.project_dir) {
        (Some(d), _) => d.clone(),
        (None, Some(d)) => d.clone(),
        (None, None) => PathBuf::from(slug(&name)),
    };
    let dir = std::path::absolute(&dir).unwrap_or(dir);
    if ctx.dry_run {
        if dir.join("project.json").exists() {
            return Err(already_exists(&dir));
        }
        return Ok(Outcome::dry_run(json!({"create_project": name, "project_dir": dir})));
    }
    if dir.join("project.json").exists() {
        return Err(already_exists(&dir));
    }
    let (store, project) = ProjectStore::create(&dir, &name)?;
    log_event(&ctx, &store, "project.created", json!({"name": name}))?;
    let d = dir.display();
    Ok(Outcome::Done(CreateOut {
        next: vec![
            format!("rustler --project \"{d}\" source import <file-or-url>"),
            "for a YouTube URL scan first: `rustler source info <url>` then `rustler source fetch-transcript <url>`".to_string(),
        ],
        project_dir: dir,
        project,
    }))
}

fn already_exists(dir: &Path) -> RustlerError {
    validation(
        "project.create",
        format!("a project already exists in {}", dir.display()),
        "pass --dir <new-directory> to create another, or use the existing one with --project",
    )
}

// ---------------------------------------------------------------------------------------
// project.show
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug, Default)]
pub struct ShowIn {}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SourceBrief {
    pub id: SourceId,
    pub name: String,
    pub duration: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub transcribed: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Counts {
    pub sources: usize,
    pub clips: usize,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ShowOut {
    pub project_dir: PathBuf,
    pub project: Project,
    pub counts: Counts,
    pub sources: Vec<SourceBrief>,
    pub next: Vec<String>,
}

pub async fn show(ctx: Ctx, _input: ShowIn) -> rustler_core::Result<ShowOut> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let mut sources = Vec::new();
    for id in &project.sources {
        // A source folder that went missing should not make `project show` unusable.
        if let Ok(s) = store.load_source(id) {
            sources.push(SourceBrief {
                id: s.id.clone(),
                name: s.name,
                duration: s.probe.duration,
                has_video: s.probe.has_video,
                has_audio: s.probe.has_audio,
                transcribed: store.analysis_dir(&s.id).join("transcript.json").is_file(),
            });
        }
    }
    let mut next = Vec::new();
    if sources.is_empty() {
        next.push("rustler source import <file-or-url>".to_string());
    } else if let Some(s) = sources.iter().find(|s| !s.transcribed && s.has_audio) {
        next.push(format!("rustler analyze quick {}", s.id));
    } else if project.clips.is_empty() {
        next.push("rustler transcript get <source-id>  (then `rustler clip create --start-unit N --end-unit M`)".to_string());
    } else {
        next.push("rustler clip list".to_string());
    }
    Ok(ShowOut {
        project_dir: store.root.clone(),
        counts: Counts { sources: project.sources.len(), clips: project.clips.len() },
        project,
        sources,
        next,
    })
}

// ---------------------------------------------------------------------------------------
// project.set_permissions / project.set_settings
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SetPermissionsIn {
    pub network: Option<bool>,
    pub downloads: Option<bool>,
    /// Restrict local file imports to these directories (empty list = any path).
    #[serde(default, deserialize_with = "paths_one_or_many")]
    #[schemars(with = "Option<Vec<String>>")]
    pub fs_roots: Option<Vec<PathBuf>>,
    /// Path of your own cookies file for yt-dlp; empty string clears it.
    pub cookies_file: Option<String>,
}

fn paths_one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<PathBuf>>, D::Error> {
    Ok(one_or_many(d)?.map(|v| v.into_iter().map(PathBuf::from).collect()))
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct PermissionsOut {
    pub permissions: Permissions,
    pub project_revision: u64,
}

pub async fn set_permissions(ctx: Ctx, input: SetPermissionsIn) -> rustler_core::Result<Outcome<PermissionsOut>> {
    let store = ctx.store()?;
    if let Some(c) = input.cookies_file.as_deref().filter(|c| !c.is_empty()) {
        if !Path::new(c).is_file() {
            return Err(validation(
                "project.set_permissions",
                format!("cookies file not found: {c}"),
                "export cookies in Netscape format from your own browser session and pass the path",
            ));
        }
    }
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({
            "network": input.network, "downloads": input.downloads,
            "fs_roots": input.fs_roots, "cookies_file": input.cookies_file,
        })));
    }
    let p = update_project(&store, |p| {
        if let Some(v) = input.network {
            p.permissions.network = v;
        }
        if let Some(v) = input.downloads {
            p.permissions.downloads = v;
        }
        if let Some(v) = &input.fs_roots {
            p.permissions.fs_roots = v.clone();
        }
        if let Some(c) = &input.cookies_file {
            p.permissions.cookies_file = if c.is_empty() { None } else { Some(PathBuf::from(c)) };
        }
        Ok(())
    })?;
    // The cookies path is deliberately not logged.
    log_event(&ctx, &store, "project.permissions_set", json!({"network": p.permissions.network, "downloads": p.permissions.downloads, "fs_roots": p.permissions.fs_roots}))?;
    Ok(Outcome::Done(PermissionsOut { permissions: p.permissions, project_revision: p.revision }))
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct SetSettingsIn {
    /// `latin` (default, romanized Hinglish), `devanagari` or `as_transcribed`.
    pub caption_script: Option<CaptionScript>,
    /// faster-whisper model name, e.g. `tiny`, `small`, `medium`.
    pub default_whisper_model: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SettingsOut {
    pub caption_script: CaptionScript,
    pub default_whisper_model: String,
    pub project_revision: u64,
}

pub async fn set_settings(ctx: Ctx, input: SetSettingsIn) -> rustler_core::Result<Outcome<SettingsOut>> {
    let store = ctx.store()?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({
            "caption_script": input.caption_script, "default_whisper_model": input.default_whisper_model,
        })));
    }
    let p = update_project(&store, |p| {
        if let Some(s) = input.caption_script {
            p.settings.caption_script = s;
        }
        if let Some(m) = &input.default_whisper_model {
            p.settings.default_whisper_model = m.clone();
        }
        Ok(())
    })?;
    log_event(&ctx, &store, "project.settings_set", json!({"caption_script": p.settings.caption_script, "default_whisper_model": p.settings.default_whisper_model}))?;
    Ok(Outcome::Done(SettingsOut {
        caption_script: p.settings.caption_script,
        default_whisper_model: p.settings.default_whisper_model,
        project_revision: p.revision,
    }))
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "project.create",
        "Create a project folder (sources/, analysis/, clips/, exports/) and project.json",
        &["name"],
        Annotations::default(),
        vec![Example { description: "New project in ./my-podcast", cli: "rustler project create \"My podcast\"" }],
        create,
    ));
    reg.register(typed(
        "project.show",
        "Show the project, its sources with transcription status, counts and the suggested next command",
        &[],
        Annotations { read_only: true, ..Default::default() },
        vec![Example { description: "Overview", cli: "rustler project show --project ./my-podcast" }],
        show,
    ));
    reg.register(typed(
        "project.set_permissions",
        "Set network/download permissions, allowed local folders (fs_roots) and your cookies file",
        &[],
        Annotations::default(),
        vec![Example { description: "Forbid network", cli: "rustler project set-permissions --network false" }],
        set_permissions,
    ));
    reg.register(typed(
        "project.set_settings",
        "Set caption script (latin|devanagari|as_transcribed) and the default whisper model",
        &[],
        Annotations::default(),
        vec![Example { description: "Faster, rougher transcripts", cli: "rustler project set-settings --default-whisper-model tiny" }],
        set_settings,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formats() {
        assert_eq!(mmss(65.9), "01:05");
        assert_eq!(mmss(3725.0), "1:02:05");
        assert_eq!(hmsd(2825.25), "00:47:05.2");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My Podcast #1!"), "my-podcast-1");
        assert_eq!(slug("!!!"), "project");
    }

    #[test]
    fn fs_roots_empty_allows_all_and_roots_restrict() {
        let dir = std::env::temp_dir();
        assert!(path_allowed(&[], &dir));
        let inner = dir.join("x_not_there.mp4");
        assert!(path_allowed(&[dir.clone()], &inner));
        assert!(!path_allowed(&[dir.join("some_other_root_dir")], &inner));
    }

    #[test]
    fn one_or_many_accepts_string_and_array() {
        #[derive(Deserialize)]
        struct T {
            #[serde(default, deserialize_with = "one_or_many")]
            v: Option<Vec<String>>,
        }
        let a: T = serde_json::from_value(json!({"v": "1:00-2:00"})).unwrap();
        assert_eq!(a.v.unwrap(), vec!["1:00-2:00"]);
        let b: T = serde_json::from_value(json!({"v": ["a", "b"]})).unwrap();
        assert_eq!(b.v.unwrap().len(), 2);
        let c: T = serde_json::from_value(json!({"v": "a, b"})).unwrap();
        assert_eq!(c.v.unwrap(), vec!["a", "b"]);
        let d: T = serde_json::from_value(json!({})).unwrap();
        assert!(d.v.is_none());
    }
}
