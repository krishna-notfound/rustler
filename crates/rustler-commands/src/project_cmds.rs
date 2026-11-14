//! Commands: project.create, project.show, project.set_permissions, project.set_settings (Task 10a).
//!
//! Also home of small helpers shared by the other command modules: [`Outcome`] (dry-run
//! envelope), [`log_event`], [`update_project`], time formatting and flexible input parsing.

use rustler_core::{
    CaptionScript, ErrorCode, Event, Permissions, Project, ProjectStore, RustlerError, SourceId,
};
use chrono::{DateTime, Utc};
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
    // Canonicalize the longest existing ancestor so not-yet-existing paths still compare right.
    let canon = |p: &Path| {
        let mut rest = Vec::new();
        let mut cur = p.to_path_buf();
        loop {
            if let Ok(c) = std::fs::canonicalize(&cur) {
                return rest.iter().rev().fold(c, |acc, part: &std::ffi::OsString| acc.join(part));
            }
            match (cur.file_name().map(|n| n.to_owned()), cur.parent().map(|p| p.to_path_buf())) {
                (Some(n), Some(par)) => {
                    rest.push(n);
                    cur = par;
                }
                _ => return p.to_path_buf(),
            }
        }
    };
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
    record_recent(&ctx.home, &dir, &name);
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
    if !ctx.dry_run {
        record_recent(&ctx.home, &store.root, &project.name);
    }
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

// ---------------------------------------------------------------------------------------
// recent-projects registry: <RUSTLER_HOME>/projects.json
// ---------------------------------------------------------------------------------------

const REGISTRY_FILE: &str = "projects.json";
const REGISTRY_MAX: usize = 50;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct RecentEntry {
    path: PathBuf,
    name: String,
    last_opened: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct RecentFile {
    #[serde(default)]
    projects: Vec<RecentEntry>,
}

/// Absolute, canonical (when the path exists) path without the Windows `\\?\` prefix.
fn norm_path(p: &Path) -> PathBuf {
    let a = std::fs::canonicalize(p).unwrap_or_else(|_| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    let s = a.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => a,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn read_registry(home: &Path) -> RecentFile {
    std::fs::read(home.join(REGISTRY_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn unique_suffix() -> String {
    format!("{}-{}", std::process::id(), Utc::now().timestamp_nanos_opt().unwrap_or(0))
}

fn write_registry(home: &Path, f: &RecentFile) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let tmp = home.join(format!("{REGISTRY_FILE}.tmp{}", unique_suffix()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(f).unwrap_or_default())?;
    std::fs::rename(&tmp, home.join(REGISTRY_FILE)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Records `dir` as most recently opened in `<home>/projects.json`. Best effort: a registry that
/// cannot be written never makes a command fail. Keeps the newest [`REGISTRY_MAX`] entries
/// (entries whose folder vanished are dropped first).
pub fn record_recent(home: &Path, dir: &Path, name: &str) {
    let path = norm_path(dir);
    let mut f = read_registry(home);
    f.projects.retain(|e| !same_path(&e.path, &path));
    f.projects.push(RecentEntry { path, name: name.to_string(), last_opened: Utc::now() });
    if f.projects.len() > REGISTRY_MAX {
        f.projects.retain(|e| e.path.join("project.json").is_file());
        f.projects.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
        f.projects.truncate(REGISTRY_MAX);
    }
    if let Err(e) = write_registry(home, &f) {
        tracing::warn!("could not update {}: {e}", home.join(REGISTRY_FILE).display());
    }
}

// ---------------------------------------------------------------------------------------
// project.list / project.open
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug, Default)]
pub struct ListIn {
    /// Also drop projects whose folder no longer exists from the registry. Default false.
    pub prune_missing: Option<bool>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ListItem {
    pub path: PathBuf,
    pub name: String,
    pub last_opened: DateTime<Utc>,
    pub sources: usize,
    pub clips: usize,
    /// false when the folder (or its project.json) is gone (moved drive, deleted).
    pub exists: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ListOut {
    pub projects: Vec<ListItem>,
    /// The registry file.
    pub registry: PathBuf,
    pub next: Vec<String>,
}

pub async fn list(ctx: Ctx, input: ListIn) -> rustler_core::Result<ListOut> {
    let mut f = read_registry(&ctx.home);
    if input.prune_missing.unwrap_or(false) && !ctx.dry_run {
        let before = f.projects.len();
        f.projects.retain(|e| e.path.join("project.json").is_file());
        if f.projects.len() != before {
            let _ = write_registry(&ctx.home, &f);
        }
    }
    f.projects.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    let projects: Vec<ListItem> = f
        .projects
        .into_iter()
        .map(|e| {
            let v: Option<Value> = std::fs::read(e.path.join("project.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
            let count = |k: &str| v.as_ref().and_then(|v| v[k].as_array()).map_or(0, |a| a.len());
            ListItem {
                name: v.as_ref().and_then(|v| v["name"].as_str().map(String::from)).unwrap_or(e.name),
                sources: count("sources"),
                clips: count("clips"),
                exists: v.is_some(),
                last_opened: e.last_opened,
                path: e.path,
            }
        })
        .collect();
    let mut next = Vec::new();
    if let Some(p) = projects.iter().find(|p| p.exists) {
        next.push(format!("rustler project status --project \"{}\"", p.path.display()));
    } else {
        next.push("rustler project create \"My project\"".to_string());
    }
    Ok(ListOut { registry: ctx.home.join(REGISTRY_FILE), projects, next })
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct OpenIn {
    /// Project directory (contains project.json).
    pub dir: PathBuf,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct OpenOut {
    pub project_dir: PathBuf,
    pub name: String,
    pub revision: u64,
    pub sources: usize,
    pub clips: usize,
    pub next: Vec<String>,
}

pub async fn open(ctx: Ctx, input: OpenIn) -> rustler_core::Result<OpenOut> {
    let dir = norm_path(&input.dir);
    let store = ProjectStore::open(&dir)?;
    let p = store.load_project()?;
    if !ctx.dry_run {
        record_recent(&ctx.home, &dir, &p.name);
    }
    let d = dir.display();
    Ok(OpenOut {
        name: p.name.clone(),
        revision: p.revision,
        sources: p.sources.len(),
        clips: p.clips.len(),
        next: vec![format!("rustler project status --project \"{d}\"")],
        project_dir: dir,
    })
}

// ---------------------------------------------------------------------------------------
// project.pack / project.unpack
// ---------------------------------------------------------------------------------------

const PACK_MANIFEST: &str = "rustler-pack.json";

enum Payload {
    File(PathBuf),
    Bytes(Vec<u8>),
}

struct PackEntry {
    name: String,
    payload: Payload,
}

struct PackPlan {
    entries: Vec<PackEntry>,
    warnings: Vec<String>,
}

fn pretty(v: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap_or_default()
}

/// Files below `dir`, as (zip name, path), sorted; temp files from atomic writes are skipped.
fn walk_files(dir: &Path, prefix: &str, skip: &dyn Fn(&str) -> bool, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut items: Vec<_> = rd.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for e in items {
        let fname = e.file_name().to_string_lossy().into_owned();
        let name = format!("{prefix}/{fname}");
        if fname.contains(".tmp") || skip(&name) {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            walk_files(&path, &name, skip, out);
        } else if path.is_file() {
            out.push((name, path));
        }
    }
}

fn plan_pack(store: &ProjectStore, include_media: bool, include_renders: bool, include_analysis: bool, skip_file: Option<&Path>) -> rustler_core::Result<PackPlan> {
    let root = &store.root;
    let mut entries: Vec<PackEntry> = Vec::new();
    let mut warnings = Vec::new();

    // project.json without the cookies path (a local path that means nothing elsewhere).
    let mut pj: Value = store.read_json(Path::new("project.json"))?;
    if let Some(c) = pj.pointer_mut("/permissions/cookies_file") {
        *c = Value::Null;
    }
    entries.push(PackEntry { name: "project.json".into(), payload: Payload::Bytes(pretty(&pj)) });
    for f in ["brief.json", "events.ndjson"] {
        if root.join(f).is_file() {
            entries.push(PackEntry { name: f.into(), payload: Payload::File(root.join(f)) });
        }
    }

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let no_skip = |_: &str| false;
    for d in ["history", "layouts", "assets"] {
        walk_files(&root.join(d), d, &no_skip, &mut files);
    }
    if include_analysis {
        walk_files(&root.join("analysis"), "analysis", &no_skip, &mut files);
    }
    let renders_skip = |n: &str| !include_renders && n.rsplit('/').next() == Some("renders");
    walk_files(&root.join("clips"), "clips", &renders_skip, &mut files);
    if include_renders {
        walk_files(&root.join("exports"), "exports", &no_skip, &mut files);
    } else if std::fs::read_dir(root.join("clips")).into_iter().flatten().flatten().any(|c| c.path().join("renders").is_dir()) {
        warnings.push("renders and exports are not included: re-render with `rustler render run <clip>` after unpacking".to_string());
    }
    files.retain(|(_, p)| skip_file.is_none_or(|s| !same_path(&norm_path(p), s)));
    entries.extend(files.into_iter().map(|(name, p)| PackEntry { name, payload: Payload::File(p) }));

    // Sources: source.json always (media paths may be rewritten); other files only with media.
    let mut source_dirs: Vec<_> = std::fs::read_dir(root.join("sources")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    source_dirs.sort();
    for sd in source_dirs {
        let sid = sd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let json_path = sd.join("source.json");
        if !json_path.is_file() {
            continue;
        }
        let mut src: rustler_core::Source = match std::fs::read(&json_path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(s) => s,
            None => {
                warnings.push(format!("sources/{sid}/source.json is unreadable: packed as-is"));
                entries.push(PackEntry { name: format!("sources/{sid}/source.json"), payload: Payload::File(json_path) });
                continue;
            }
        };
        let media = store.resolve(&src.media_path);
        let inside = media.starts_with(&sd);
        if include_media {
            if !media.is_file() {
                warnings.push(format!("source {sid} ({}): media file is missing at {}", src.name, media.display()));
            } else if !inside {
                let fname = media.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "media".into());
                let zip_name = format!("media/{sid}_{fname}");
                src.media_path = PathBuf::from(&zip_name);
                entries.push(PackEntry { name: zip_name, payload: Payload::File(media) });
            }
            let mut inner = Vec::new();
            walk_files(&sd, &format!("sources/{sid}"), &|n: &str| n.ends_with("/source.json"), &mut inner);
            entries.extend(inner.into_iter().map(|(name, p)| PackEntry { name, payload: Payload::File(p) }));
        } else if !inside {
            warnings.push(format!("source {sid} ({}): media is not included (external file {}); pass --include-media to bundle it", src.name, media.display()));
        } else {
            warnings.push(format!("source {sid} ({}): media, proxy and audio are not included; pass --include-media to bundle them", src.name));
        }
        entries.push(PackEntry { name: format!("sources/{sid}/source.json"), payload: Payload::Bytes(pretty(&src)) });
    }
    Ok(PackPlan { entries, warnings })
}

fn deflate_ok(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    !matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "mov" | "m4a" | "mp3" | "opus" | "ogg" | "aac" | "flac" | "jpg" | "jpeg" | "png" | "zip")
}

fn write_zip(out: &Path, plan: &PackPlan, manifest: Value, cancel: &tokio_util::sync::CancellationToken) -> rustler_core::Result<usize> {
    let ioe = |what: &str, e: &dyn std::fmt::Display| RustlerError::new(ErrorCode::Internal, "project.pack", format!("{what}: {e}"));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ioe(&format!("cannot create {}", parent.display()), &e))?;
    }
    let tmp = out.with_extension(format!("zip.tmp{}", unique_suffix()));
    let result = (|| -> rustler_core::Result<usize> {
        let f = std::fs::File::create(&tmp).map_err(|e| ioe(&format!("cannot create {}", tmp.display()), &e))?;
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
        let base = zip::write::SimpleFileOptions::default();
        let mut n = 0;
        for e in &plan.entries {
            if cancel.is_cancelled() {
                return Err(RustlerError::new(ErrorCode::Cancelled, "project.pack", "cancelled"));
            }
            let method = if deflate_ok(&e.name) { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored };
            let mut opts = base.compression_method(method);
            match &e.payload {
                Payload::File(p) => {
                    let len = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                    opts = opts.large_file(len >= 0xF000_0000);
                    z.start_file(&e.name, opts).map_err(|x| ioe(&format!("zip {}", e.name), &x))?;
                    let mut r = std::io::BufReader::new(std::fs::File::open(p).map_err(|x| ioe(&format!("cannot read {}", p.display()), &x))?);
                    std::io::copy(&mut r, &mut z).map_err(|x| ioe(&format!("zip {}", e.name), &x))?;
                }
                Payload::Bytes(b) => {
                    z.start_file(&e.name, opts).map_err(|x| ioe(&format!("zip {}", e.name), &x))?;
                    std::io::Write::write_all(&mut z, b).map_err(|x| ioe(&format!("zip {}", e.name), &x))?;
                }
            }
            n += 1;
        }
        z.start_file(PACK_MANIFEST, base).map_err(|x| ioe("zip manifest", &x))?;
        std::io::Write::write_all(&mut z, &pretty(&manifest)).map_err(|x| ioe("zip manifest", &x))?;
        let mut w = z.finish().map_err(|x| ioe("finish zip", &x))?;
        std::io::Write::flush(&mut w).map_err(|x| ioe("finish zip", &x))?;
        Ok(n + 1)
    })();
    match result {
        Ok(n) => {
            let _ = std::fs::remove_file(out);
            std::fs::rename(&tmp, out).map_err(|e| ioe(&format!("cannot write {}", out.display()), &e))?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[derive(Deserialize, JsonSchema, Debug, Default)]
pub struct PackIn {
    /// Archive path. Default `<project>/../<project-name-slug>.rustler.zip`.
    pub out: Option<PathBuf>,
    /// Also bundle source media (proxy, audio, downloaded media; local files go to `media/`). Default false.
    pub include_media: Option<bool>,
    /// Include rendered videos (`clips/*/renders`, `exports/`). Default true.
    pub include_renders: Option<bool>,
    /// Include `analysis/` (transcripts, scenes, ...). Default true.
    pub include_analysis: Option<bool>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct PackOut {
    pub path: PathBuf,
    pub size: u64,
    pub files: usize,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

pub async fn pack(ctx: Ctx, input: PackIn) -> rustler_core::Result<Outcome<PackOut>> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let (include_media, include_renders, include_analysis) =
        (input.include_media.unwrap_or(false), input.include_renders.unwrap_or(true), input.include_analysis.unwrap_or(true));
    let out = match input.out {
        Some(o) => o,
        None => {
            let root = norm_path(&store.root);
            root.parent().unwrap_or(&root).join(format!("{}.rustler.zip", slug(&project.name)))
        }
    };
    let out = std::path::absolute(&out).unwrap_or(out);
    let plan = plan_pack(&store, include_media, include_renders, include_analysis, Some(&norm_path(&out)))?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"pack_to": out, "files": plan.entries.len() + 1, "warnings": plan.warnings})));
    }
    let manifest = json!({
        "schema": "rustler.pack@1", "project_id": project.id, "project_name": project.name,
        "packed_at": Utc::now(), "include_media": include_media, "include_renders": include_renders,
        "include_analysis": include_analysis, "files": plan.entries.len(),
    });
    let cancel = ctx.cancel.clone();
    let out2 = out.clone();
    let (files, warnings) = tokio::task::spawn_blocking(move || write_zip(&out2, &plan, manifest, &cancel).map(|n| (n, plan.warnings)))
        .await
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "project.pack", format!("pack task failed: {e}")))??;
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    log_event(&ctx, &store, "project.packed", json!({"path": out, "files": files, "size": size, "include_media": include_media}))?;
    Ok(Outcome::Done(PackOut {
        next: vec![format!("rustler project unpack \"{}\" <new-dir>   (on the other machine)", out.display())],
        path: out,
        size,
        files,
        warnings,
    }))
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct UnpackIn {
    /// A `.rustler.zip` made by `project.pack`.
    pub archive: PathBuf,
    /// Directory to extract the project into.
    pub dir: PathBuf,
    /// Extract over an existing project in `dir` (same-named files are replaced). Default false.
    pub force: Option<bool>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct UnpackOut {
    pub project_dir: PathBuf,
    pub name: String,
    pub sources: usize,
    pub clips: usize,
    pub files: usize,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn bad_archive(path: &Path, why: impl std::fmt::Display) -> RustlerError {
    validation(
        "project.unpack",
        format!("{} is not a usable Rustler project archive: {why}", path.display()),
        "pass a .rustler.zip created by `rustler project pack`",
    )
}

/// Reads and checks `project.json` inside the archive.
fn read_pack_project(zip: &mut zip::ZipArchive<std::fs::File>, archive: &Path) -> rustler_core::Result<Project> {
    let mut entry = zip.by_name("project.json").map_err(|_| bad_archive(archive, "it has no project.json at its top level"))?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| bad_archive(archive, e))?;
    let p: Project = serde_json::from_slice(&buf).map_err(|e| bad_archive(archive, format!("project.json is invalid: {e}")))?;
    if !p.schema.starts_with("rustler.project@") {
        return Err(bad_archive(archive, format!("unknown project schema `{}`", p.schema)));
    }
    Ok(p)
}

pub async fn unpack(ctx: Ctx, input: UnpackIn) -> rustler_core::Result<Outcome<UnpackOut>> {
    let archive = input.archive.clone();
    if !archive.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, "project.unpack", format!("archive not found: {}", archive.display()))
            .hint("pass the path of a .rustler.zip"));
    }
    let dir = std::path::absolute(&input.dir).unwrap_or_else(|_| input.dir.clone());
    let force = input.force.unwrap_or(false);
    if dir.join("project.json").exists() && !force {
        return Err(validation(
            "project.unpack",
            format!("a project already exists in {}", dir.display()),
            "unpack into a new directory, or pass --force to extract over the existing project",
        ));
    }
    let dry = ctx.dry_run;
    let cancel = ctx.cancel.clone();
    let (dir2, archive2) = (dir.clone(), archive.clone());
    let (project, files, mut warnings) = tokio::task::spawn_blocking(move || -> rustler_core::Result<(Project, usize, Vec<String>)> {
        let f = std::fs::File::open(&archive2).map_err(|e| bad_archive(&archive2, e))?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| bad_archive(&archive2, e))?;
        let project = read_pack_project(&mut zip, &archive2)?;
        let mut warnings = Vec::new();
        let mut n = 0;
        let werr = |dest: &Path, e: std::io::Error| RustlerError::new(ErrorCode::Internal, "project.unpack", format!("cannot write {}: {e}", dest.display()));
        for i in 0..zip.len() {
            if cancel.is_cancelled() {
                return Err(RustlerError::new(ErrorCode::Cancelled, "project.unpack", "cancelled"));
            }
            let mut entry = zip.by_index(i).map_err(|e| bad_archive(&archive2, e))?;
            if !entry.is_file() {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else {
                warnings.push(format!("skipped an entry with an unsafe path: {}", entry.name().map(|n| n.into_owned()).unwrap_or_default()));
                continue;
            };
            if rel == Path::new(PACK_MANIFEST) {
                continue;
            }
            n += 1;
            if dry {
                continue;
            }
            let dest = dir2.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| werr(parent, e))?;
            }
            let mut out = std::fs::File::create(&dest).map_err(|e| werr(&dest, e))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| werr(&dest, e))?;
        }
        Ok((project, n, warnings))
    })
    .await
    .map_err(|e| RustlerError::new(ErrorCode::Internal, "project.unpack", format!("unpack task failed: {e}")))??;

    if dry {
        return Ok(Outcome::dry_run(json!({"unpack_to": dir, "project": project.name, "files": files, "sources": project.sources.len(), "clips": project.clips.len()})));
    }

    // Packed local media (`media/<file>`) becomes an absolute path under the new directory.
    let store = ProjectStore::open(&dir)?;
    for sid in &project.sources {
        let Ok(mut s) = store.load_source(sid) else {
            warnings.push(format!("source {sid}: source.json missing in the archive"));
            continue;
        };
        if s.media_path.is_relative() && s.media_path.components().next().is_some_and(|c| c.as_os_str() == "media") {
            s.media_path = dir.join(&s.media_path);
            store.save_source(&s)?;
        }
        let m = store.resolve(&s.media_path);
        if !m.is_file() {
            warnings.push(format!("source {sid} ({}): media not found at {}; re-import it or repack with --include-media", s.name, m.display()));
        }
    }
    record_recent(&ctx.home, &dir, &project.name);
    log_event(&ctx, &store, "project.unpacked", json!({"archive": archive, "files": files}))?;
    let d = dir.display();
    Ok(Outcome::Done(UnpackOut {
        name: project.name.clone(),
        sources: project.sources.len(),
        clips: project.clips.len(),
        files,
        warnings,
        next: vec![format!("rustler project status --project \"{d}\"")],
        project_dir: dir,
    }))
}

// ---------------------------------------------------------------------------------------
// project.status
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug, Default)]
pub struct StatusIn {}

#[derive(Serialize, JsonSchema, Debug)]
pub struct SourceStatus {
    pub id: SourceId,
    pub name: String,
    pub duration: f64,
    /// `file` or `url`.
    pub origin: String,
    pub media_present: bool,
    /// A transcript exists.
    pub analyzed: bool,
    pub transcript_words: usize,
    pub units: usize,
    pub scenes: bool,
    pub audio_events: bool,
    /// Any of ocr / events / faces / understanding.
    pub deep_analysis: bool,
    pub deep_parts: Vec<String>,
    /// `analysis/<id>/source_info.json` (heatmap, chapters) saved.
    pub source_info: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct RenderStatus {
    pub id: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub file_exists: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ClipQcStatus {
    pub passed: bool,
    pub verdict: Option<String>,
    pub report: PathBuf,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ClipBriefStatus {
    /// pass | warn | fail (last `brief.check`).
    pub status: String,
    pub checked_at: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ClipStatus {
    pub id: String,
    pub name: String,
    pub revision: u64,
    /// Sum of segment lengths in seconds.
    pub duration: f64,
    pub segments: usize,
    pub renders: usize,
    pub last_render: Option<RenderStatus>,
    /// The clip (or its extras: overlays, music, ...) changed after its last render.
    pub edits_newer_than_render: bool,
    pub stale_reason: Option<String>,
    pub qc: Option<ClipQcStatus>,
    /// A brief exists for the project.
    pub has_brief: bool,
    pub brief_check: Option<ClipBriefStatus>,
    /// The last render was copied to `exports/`.
    pub exported: bool,
    /// An export exists but of an older render.
    pub exported_older_render: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct StatusOut {
    pub project_dir: PathBuf,
    pub name: String,
    pub revision: u64,
    pub sources: Vec<SourceStatus>,
    pub clips: Vec<ClipStatus>,
    /// Everything that looks unfinished, most important first.
    pub suggestions: Vec<String>,
    /// The first few suggestions as commands to run.
    pub next: Vec<String>,
}

const CONTENT_IGNORE: [&str; 4] = ["renders", "revision", "created_at", "name"];

/// The clip's editable content (everything except bookkeeping), for comparing snapshots.
fn clip_content(c: &rustler_core::Clip) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        for k in CONTENT_IGNORE {
            o.remove(k);
        }
    }
    v
}

fn mtime(p: &Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().map(DateTime::<Utc>::from)
}

/// Why the clip no longer matches its last render, if it doesn't. `save_clip` snapshots the previous
/// state into `history/` on every save, so the earliest snapshot written after the render record
/// was created holds the clip as it was rendered; that is compared with the current clip.
fn stale_reason(store: &ProjectStore, clip: &rustler_core::Clip, render: &rustler_core::RenderRecord) -> Option<String> {
    let hist = store.clip_dir(&clip.id).join("history");
    let first_after = std::fs::read_dir(&hist)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| mtime(&e.path()).map(|t| (t, e.path())))
        .filter(|(t, _)| *t >= render.created_at)
        .min_by_key(|(t, _)| *t);
    if let Some((_, p)) = first_after {
        if let Some(old) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<rustler_core::Clip>(&b).ok()) {
            if clip_content(&old) != clip_content(clip) {
                return Some("segments, captions, canvas or audio settings changed after the render".to_string());
            }
        }
    }
    let extras = store.clip_dir(&clip.id).join("extras.json");
    if mtime(&extras).is_some_and(|t| t > render.created_at) {
        return Some("overlays/music/effects (extras.json) changed after the render".to_string());
    }
    None
}

/// clip id -> ids of renders copied to `exports/` (from `exports/*.report.json`).
fn exported_renders(store: &ProjectStore) -> std::collections::HashMap<String, Vec<String>> {
    let mut m: std::collections::HashMap<String, Vec<String>> = Default::default();
    for e in std::fs::read_dir(store.exports_dir()).into_iter().flatten().flatten() {
        let p = e.path();
        if !p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".report.json")) {
            continue;
        }
        let Some(v) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        if let (Some(c), Some(r)) = (v["clip"]["id"].as_str(), v["render"]["id"].as_str()) {
            m.entry(c.to_string()).or_default().push(r.to_string());
        }
    }
    m
}

pub async fn status(ctx: Ctx, _input: StatusIn) -> rustler_core::Result<StatusOut> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let mut sources = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();
    for id in &project.sources {
        let Ok(s) = store.load_source(id) else {
            suggestions.push(format!("source {id} is listed in project.json but its source.json is missing"));
            continue;
        };
        let a = store.analysis_dir(id);
        let has = |f: &str| a.join(f).is_file();
        let words = std::fs::read(a.join("transcript.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v["words"].as_array().map(|w| w.len()))
            .unwrap_or(0);
        let units = std::fs::read(a.join("units.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_array().map(|u| u.len()))
            .unwrap_or(0);
        let deep_parts: Vec<String> =
            ["ocr", "events", "faces", "understanding"].iter().filter(|p| has(&format!("{p}.json"))).map(|p| p.to_string()).collect();
        let st = SourceStatus {
            id: s.id.clone(),
            name: s.name.clone(),
            duration: s.probe.duration,
            origin: if matches!(s.origin, rustler_core::SourceOrigin::Url { .. }) { "url" } else { "file" }.to_string(),
            media_present: store.resolve(&s.media_path).is_file(),
            analyzed: has("transcript.json"),
            transcript_words: words,
            units,
            scenes: has("scenes.json"),
            audio_events: has("audio_events.json"),
            deep_analysis: !deep_parts.is_empty(),
            deep_parts,
            source_info: has("source_info.json"),
        };
        if !st.media_present {
            suggestions.push(format!("source {} ({}): media file is missing; re-import it", st.id, st.name));
        }
        if !st.analyzed && s.probe.has_audio {
            suggestions.push(format!("rustler analyze quick {}   (source {} has no transcript yet)", st.id, st.name));
        } else if st.analyzed && project.clips.is_empty() {
            suggestions.push(format!("rustler highlights candidates {}   (find moments to clip)", st.id));
        }
        sources.push(st);
    }

    let has_brief = store.root.join("brief.json").is_file();
    let exported = exported_renders(&store);
    let mut clips = Vec::new();
    for cid in &project.clips {
        let Ok(clip) = store.load_clip(cid) else {
            suggestions.push(format!("clip {cid} is listed in project.json but its clip.json is missing"));
            continue;
        };
        let last = clip.renders.last();
        let stale = last.and_then(|r| stale_reason(&store, &clip, r));
        let qc = last.and_then(|r| r.qc.as_ref()).and_then(|q| {
            let v: Value = store.read_json(q).ok()?;
            Some(ClipQcStatus { passed: v["passed"].as_bool()?, verdict: v["verdict"].as_str().map(String::from), report: q.clone() })
        });
        let brief_check = crate::brief_cmds::brief_status(&store, &clip.id).and_then(|v| {
            Some(ClipBriefStatus { status: v["status"].as_str()?.to_string(), checked_at: v["checked_at"].as_str().map(String::from) })
        });
        let ex = exported.get(clip.id.as_str());
        let exported_latest = last.is_some_and(|r| ex.is_some_and(|e| e.iter().any(|x| x == r.id.as_str())));
        let cs = ClipStatus {
            id: clip.id.to_string(),
            name: clip.name.clone(),
            revision: clip.revision,
            duration: (clip.segments.iter().map(|s| s.end - s.start).sum::<f64>() * 100.0).round() / 100.0,
            segments: clip.segments.len(),
            renders: clip.renders.len(),
            last_render: last.map(|r| RenderStatus {
                id: r.id.to_string(),
                path: r.path.clone(),
                created_at: r.created_at,
                file_exists: store.resolve(&r.path).is_file(),
            }),
            edits_newer_than_render: stale.is_some(),
            stale_reason: stale,
            qc,
            has_brief,
            brief_check,
            exported: exported_latest,
            exported_older_render: ex.is_some() && !exported_latest,
        };
        let id = &cs.id;
        if cs.last_render.is_none() {
            suggestions.push(format!("rustler render run {id}   (clip {} was never rendered)", cs.name));
        } else {
            if cs.last_render.as_ref().is_some_and(|r| !r.file_exists) {
                suggestions.push(format!("rustler render run {id}   (the last render file of clip {} is missing)", cs.name));
            }
            if let Some(why) = &cs.stale_reason {
                suggestions.push(format!("rustler render run {id}   (clip {} has edits newer than its render: {why})", cs.name));
            }
            match &cs.qc {
                None => suggestions.push(format!("rustler qc run {id}   (last render of {} has no QC report)", cs.name)),
                Some(q) if !q.passed => suggestions.push(format!("rustler fix suggest {id}   (QC failed on {})", cs.name)),
                _ => {}
            }
        }
        if has_brief {
            match &cs.brief_check {
                None => suggestions.push(format!("rustler brief check {id}   (clip {} was never checked against the brief)", cs.name)),
                Some(b) if b.status == "fail" => suggestions.push(format!("rustler brief check {id}   (brief check failed on {}: fix, then re-check)", cs.name)),
                _ => {}
            }
        }
        if cs.qc.as_ref().is_some_and(|q| q.passed) && !cs.edits_newer_than_render && !cs.exported {
            let older = if cs.exported_older_render { "; an older render was exported" } else { "" };
            suggestions.push(format!("rustler export clip {id}   ({} passed QC and is not exported{older})", cs.name));
        }
        clips.push(cs);
    }
    if project.sources.is_empty() {
        suggestions.push("rustler source import <file-or-url>".to_string());
    }
    let next = suggestions
        .iter()
        .filter(|s| s.starts_with("rustler "))
        .take(5)
        .map(|s| s.split("   (").next().unwrap_or(s).to_string())
        .collect();
    Ok(StatusOut { project_dir: store.root.clone(), name: project.name, revision: project.revision, sources, clips, suggestions, next })
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "project.list",
        "List recently used projects (registry in RUSTLER_HOME/projects.json): path, name, last opened, source/clip counts, whether the folder still exists",
        &[],
        Annotations { read_only: true, ..Default::default() },
        vec![Example { description: "Recent projects", cli: "rustler project list" }],
        list,
    ));
    reg.register(typed(
        "project.open",
        "Validate a project folder and record it as most recently opened",
        &["dir"],
        Annotations::default(),
        vec![Example { description: "Open a project", cli: "rustler project open C:\\clips\\my-podcast" }],
        open,
    ));
    reg.register(typed(
        "project.status",
        "One-glance summary: per source (transcript words, units, deep analysis) and per clip (revision, last render, QC, brief check, exported), what is unfinished, next commands",
        &[],
        Annotations { read_only: true, ..Default::default() },
        vec![Example { description: "Where am I?", cli: "rustler project status --project ./my-podcast" }],
        status,
    ));
    reg.register(typed(
        "project.pack",
        "Zip the project (json, analysis, clips, layouts, assets, exports, history; media optional) into <name>.rustler.zip without touching the original",
        &["out"],
        Annotations::default(),
        vec![
            Example { description: "Pack without media", cli: "rustler project pack" },
            Example { description: "Pack with local media, no renders", cli: "rustler project pack --out D:\\share\\proj.rustler.zip --include-media true --include-renders false" },
        ],
        pack,
    ));
    reg.register(typed(
        "project.unpack",
        "Extract a .rustler.zip into a new directory, fix packed media paths and register it in the recent list; refuses an existing project unless --force",
        &["archive", "dir"],
        Annotations::default(),
        vec![Example { description: "Unpack", cli: "rustler project unpack proj.rustler.zip C:\\clips\\proj" }],
        unpack,
    ));
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
