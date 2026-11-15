//! Rustler GUI: a tiny local web server (127.0.0.1 + random token) serving one static page
//! (plain JS modules under `ui/js/`, no build step) that drives the SAME command registry as
//! the CLI through `POST /api/run`, plus a few helpers for the editor:
//! - `POST /api/jobs`, `GET /api/jobs`, `POST /api/jobs/cancel`: run a command in the background
//!   with live progress (render queue);
//! - `GET /api/frame?source_id&at&w`: a source frame as JPEG (via `see.frame`);
//! - `GET /api/clip_state?clip_id`: cheap clip revision poll for live sync.

use anyhow::Context as _;
use axum::{
    Json, Router,
    body::Body,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rustler_core::{Actor, ErrorCode, RustlerError};
use rustler_registry::{Ctx, Envelope, ProgressEvent, ProgressSink, Registry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const INDEX_HTML: &str = include_str!("ui/index.html");
const STYLE_CSS: &str = include_str!("ui/style.css");

/// JS modules, served at `/js/<name>`; index.html loads them in this order.
const JS_MODULES: &[(&str, &str)] = &[
    ("core.js", include_str!("ui/js/core.js")),
    ("forms.js", include_str!("ui/js/forms.js")),
    ("project.js", include_str!("ui/js/project.js")),
    ("sources.js", include_str!("ui/js/sources.js")),
    ("clips.js", include_str!("ui/js/clips.js")),
    ("renders.js", include_str!("ui/js/renders.js")),
    ("activity.js", include_str!("ui/js/activity.js")),
    ("commands.js", include_str!("ui/js/commands.js")),
    ("editor.js", include_str!("ui/js/editor.js")),
    ("monitor.js", include_str!("ui/js/monitor.js")),
    ("regions.js", include_str!("ui/js/regions.js")),
    ("timeline.js", include_str!("ui/js/timeline.js")),
    ("inspector.js", include_str!("ui/js/inspector.js")),
    ("panels.js", include_str!("ui/js/panels.js")),
    ("assets.js", include_str!("ui/js/assets.js")),
    ("brief.js", include_str!("ui/js/brief.js")),
    ("boot.js", include_str!("ui/js/boot.js")),
];

/// Names of the JS modules in load order (index.html references each).
pub fn js_modules() -> Vec<&'static str> {
    JS_MODULES.iter().map(|(n, _)| *n).collect()
}

const MAX_JOBS: usize = 60;

#[derive(Clone, serde::Serialize)]
struct Job {
    id: String,
    command: String,
    input: Value,
    project: Option<String>,
    /// running | ok | error | cancelled
    status: String,
    progress: Option<ProgressEvent>,
    started_ms: u128,
    finished_ms: Option<u128>,
    /// The envelope once finished.
    result: Option<Value>,
    #[serde(skip)]
    cancel: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Clone, Debug, Default)]
pub struct GuiOptions {
    /// `None` or `Some(0)` = random free port.
    pub port: Option<u16>,
    pub open_browser: bool,
}

struct AppState {
    registry: Arc<Registry>,
    base: Ctx,
    token: String,
    /// Project the UI is currently working on (also an allowed file root).
    current: Mutex<Option<PathBuf>>,
    jobs: Mutex<Vec<Job>>,
}

impl AppState {
    fn current_project(&self) -> Option<PathBuf> {
        self.current.lock().ok().and_then(|g| g.clone())
    }
    fn set_current(&self, p: PathBuf) {
        if let Ok(mut g) = self.current.lock() {
            *g = Some(p);
        }
    }
}

/// A started server (used by tests; [`serve`] wraps it).
pub struct RunningGui {
    pub url: String,
    pub port: u16,
    pub token: String,
    task: tokio::task::JoinHandle<()>,
}

impl RunningGui {
    /// Runs until the server task ends.
    pub async fn wait(self) -> anyhow::Result<()> {
        self.task.await.context("gui server task")?;
        Ok(())
    }
    pub fn abort(&self) {
        self.task.abort();
    }
}

fn random_token() -> String {
    let n: u128 = rand::random();
    format!("{n:032x}")
}

/// Binds and starts serving in the background; does not print or open a browser.
pub async fn start(
    registry: Arc<Registry>,
    base_ctx: Ctx,
    opts: &GuiOptions,
) -> anyhow::Result<RunningGui> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", opts.port.unwrap_or(0)))
        .await
        .context("bind 127.0.0.1")?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let state = Arc::new(AppState {
        registry,
        current: Mutex::new(base_ctx.project_dir.clone()),
        jobs: Mutex::new(Vec::new()),
        base: base_ctx,
        token: token.clone(),
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/style.css", get(style_css))
        .route("/js/{name}", get(js_module))
        .route("/api/commands", get(commands))
        .route("/api/run", post(run))
        .route("/api/jobs", get(jobs_list).post(jobs_start))
        .route("/api/jobs/cancel", post(jobs_cancel))
        .route("/api/frame", get(frame))
        .route("/api/clip_state", get(clip_state))
        .route("/api/font/{name}", get(font))
        .route("/api/file", get(file))
        .route("/api/events", get(events))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state);
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("rustler-gui: server error: {e}");
        }
    });
    Ok(RunningGui {
        url: format!("http://127.0.0.1:{port}/?t={token}"),
        port,
        token,
        task,
    })
}

/// Starts the server, prints one JSON line with the URL, optionally opens the browser, then
/// serves until the process is stopped.
pub async fn serve(registry: Arc<Registry>, base_ctx: Ctx, opts: GuiOptions) -> anyhow::Result<()> {
    let gui = start(registry, base_ctx, &opts).await?;
    println!(
        "{}",
        json!({"ok": true, "schema": "rustler.gui@1", "data": {"url": gui.url}})
    );
    if opts.open_browser {
        if let Err(e) = open::that(&gui.url) {
            eprintln!("rustler-gui: could not open a browser ({e}); open the URL manually");
        }
    }
    gui.wait().await
}

// ---------------------------------------------------------------- guard

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn host_ok(h: &str) -> bool {
    let name = h.rsplit_once(':').map(|(n, _)| n).unwrap_or(h);
    name == "127.0.0.1" || name == "localhost"
}

fn plain(status: StatusCode, msg: &str) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], msg.to_string()).into_response()
}

async fn guard(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !host_ok(host) {
        return plain(StatusCode::FORBIDDEN, "bad host");
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let o = origin.split("://").nth(1).unwrap_or(origin);
        if !host_ok(o) {
            return plain(StatusCode::FORBIDDEN, "bad origin");
        }
    }
    let from_query = Query::<HashMap<String, String>>::try_from_uri(req.uri())
        .ok()
        .and_then(|q| q.0.get("t").cloned());
    let from_header = headers
        .get("x-rustler-token")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let ok = [from_query, from_header]
        .into_iter()
        .flatten()
        .any(|t| ct_eq(&t, &st.token));
    if !ok {
        return plain(StatusCode::UNAUTHORIZED, "missing or invalid token");
    }
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp.headers_mut()
        .insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    resp
}

// ---------------------------------------------------------------- static

/// `RUSTLER_GUI_DEV_UI=<dir>` (the `src/ui` folder): serve the page, CSS and JS from disk on every
/// request instead of the copies embedded at build time (UI development without rebuilding).
fn dev_ui(rel: &str) -> Option<String> {
    let dir = std::env::var_os("RUSTLER_GUI_DEV_UI")?;
    std::fs::read_to_string(PathBuf::from(dir).join(rel)).ok()
}

async fn index(State(st): State<Arc<AppState>>) -> Response {
    let scripts: String = JS_MODULES
        .iter()
        .map(|(n, _)| format!("<script src=\"/js/{n}?t={}\"></script>\n", st.token))
        .collect();
    let html = dev_ui("index.html")
        .unwrap_or_else(|| INDEX_HTML.to_string())
        .replace("__SCRIPTS__", &scripts)
        .replace("__TOKEN__", &st.token);
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}
async fn js_module(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    match JS_MODULES.iter().find(|(n, _)| *n == name) {
        Some((n, body)) => {
            let body = dev_ui(&format!("js/{n}")).unwrap_or_else(|| (*body).to_string());
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], body).into_response()
        }
        None => plain(StatusCode::NOT_FOUND, "no such module"),
    }
}
async fn style_css() -> Response {
    let css = dev_ui("style.css").unwrap_or_else(|| STYLE_CSS.to_string());
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], css).into_response()
}

// ---------------------------------------------------------------- api

async fn commands(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(serde_json::to_value(st.registry.list()).unwrap_or(Value::Array(vec![])))
}

#[derive(Deserialize)]
struct RunReq {
    command: String,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    project: Option<String>,
}

async fn run(State(st): State<Arc<AppState>>, body: Result<Json<RunReq>, axum::extract::rejection::JsonRejection>) -> Response {
    let req = match body {
        Ok(Json(r)) => r,
        Err(e) => {
            let env = Envelope::err(
                RustlerError::new(ErrorCode::Usage, "gui", format!("bad request body: {e}"))
                    .hint("POST {\"command\": \"group.action\", \"input\": {...}, \"project\": \"<dir>\"}"),
            );
            return Json(env.to_value()).into_response();
        }
    };
    let (ctx, project, input) = human_ctx(&st, &req);
    let env = dispatch(&st, &req.command, input, ctx, project).await;
    Json(env.to_value()).into_response()
}

fn req_project(req: &RunReq) -> Option<PathBuf> {
    req.project
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// A human-actor ctx for the requested (or current) project.
fn human_ctx(st: &AppState, req: &RunReq) -> (Ctx, Option<PathBuf>, Value) {
    let project = req_project(req);
    let mut ctx = st.base.clone();
    ctx.actor = Actor::Human;
    ctx.project_dir = project.clone().or_else(|| st.current_project());
    let input = match &req.input {
        Some(Value::Null) | None => json!({}),
        Some(v) => v.clone(),
    };
    (ctx, project, input)
}

async fn dispatch(st: &AppState, command: &str, input: Value, ctx: Ctx, project: Option<PathBuf>) -> Envelope {
    match st.registry.dispatch(command, input, ctx).await {
        Ok(data) => {
            // Remember which project the UI works on (for file serving and events).
            if let Some(p) = project {
                st.set_current(p);
            } else if let Some(d) = data.get("project_dir").and_then(Value::as_str) {
                st.set_current(PathBuf::from(d));
            }
            Envelope::ok(command, data)
        }
        Err(e) => Envelope::err(e),
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn usage_env(msg: String, hint: &str) -> Value {
    Envelope::err(RustlerError::new(ErrorCode::Usage, "gui", msg).hint(hint.to_string())).to_value()
}

// ---------------------------------------------------------------- jobs (background runs)

/// Starts `command` in the background; returns `{ok, data: {job_id}}` at once. Progress events
/// of the command land in the job (polled with `GET /api/jobs`).
async fn jobs_start(
    State(st): State<Arc<AppState>>,
    body: Result<Json<RunReq>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let req = match body {
        Ok(Json(r)) => r,
        Err(e) => {
            return Json(usage_env(format!("bad request body: {e}"), "POST {\"command\": \"render.run\", \"input\": {...}}"))
                .into_response();
        }
    };
    if st.registry.describe(&req.command).is_none() {
        return Json(usage_env(format!("unknown command '{}'", req.command), "see GET /api/commands")).into_response();
    }
    let (mut ctx, project, input) = human_ctx(&st, &req);
    let id = format!("job_{}", &random_token()[..12]);
    let cancel = tokio_util::sync::CancellationToken::new();
    ctx.cancel = cancel.clone();
    {
        let st2 = st.clone();
        let jid = id.clone();
        ctx.progress = ProgressSink::new(move |e: ProgressEvent| {
            if let Ok(mut jobs) = st2.jobs.lock() {
                if let Some(j) = jobs.iter_mut().find(|j| j.id == jid) {
                    j.progress = Some(e);
                }
            }
        });
    }
    let job = Job {
        id: id.clone(),
        command: req.command.clone(),
        input: input.clone(),
        project: ctx.project_dir.as_ref().map(|p| p.display().to_string()),
        status: "running".into(),
        progress: None,
        started_ms: now_ms(),
        finished_ms: None,
        result: None,
        cancel: Some(cancel.clone()),
    };
    if let Ok(mut jobs) = st.jobs.lock() {
        jobs.push(job);
        // Drop the oldest finished jobs beyond the cap.
        while jobs.len() > MAX_JOBS {
            match jobs.iter().position(|j| j.status != "running") {
                Some(ix) => {
                    jobs.remove(ix);
                }
                None => break,
            }
        }
    }
    let st2 = st.clone();
    let jid = id.clone();
    let command = req.command.clone();
    tokio::spawn(async move {
        let env = dispatch(&st2, &command, input, ctx, project).await;
        let v = env.to_value();
        let status = if v.get("ok").and_then(Value::as_bool) == Some(true) {
            "ok"
        } else if cancel.is_cancelled() {
            "cancelled"
        } else {
            "error"
        };
        if let Ok(mut jobs) = st2.jobs.lock() {
            if let Some(j) = jobs.iter_mut().find(|j| j.id == jid) {
                j.status = status.into();
                j.finished_ms = Some(now_ms());
                j.result = Some(v);
                j.cancel = None;
            }
        }
    });
    Json(json!({"ok": true, "data": {"job_id": id}})).into_response()
}

async fn jobs_list(State(st): State<Arc<AppState>>) -> Json<Value> {
    let jobs = st.jobs.lock().map(|j| j.clone()).unwrap_or_default();
    Json(json!({"now_ms": now_ms(), "jobs": jobs}))
}

#[derive(Deserialize)]
struct CancelReq {
    job_id: String,
}

async fn jobs_cancel(
    State(st): State<Arc<AppState>>,
    body: Result<Json<CancelReq>, axum::extract::rejection::JsonRejection>,
) -> Json<Value> {
    let Ok(Json(req)) = body else {
        return Json(usage_env("bad request body".into(), "POST {\"job_id\": \"job_...\"}"));
    };
    let token = st
        .jobs
        .lock()
        .ok()
        .and_then(|jobs| jobs.iter().find(|j| j.id == req.job_id).and_then(|j| j.cancel.clone()));
    match token {
        Some(t) => {
            t.cancel();
            Json(json!({"ok": true, "data": {"job_id": req.job_id, "cancel_requested": true}}))
        }
        None => Json(usage_env(format!("no running job '{}'", req.job_id), "list jobs with GET /api/jobs")),
    }
}

// ---------------------------------------------------------------- editor helpers

/// Ids are `[A-Za-z0-9_-]+` (no path tricks).
fn safe_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn query_project(st: &AppState, q: &HashMap<String, String>) -> Option<PathBuf> {
    q.get("project")
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| st.current_project())
}

/// `GET /api/frame?source_id=..&at=<seconds>&w=..` -> the JPEG from `see.frame` (cached on disk
/// by it). The time is `at` because `t` is the auth token.
async fn frame(State(st): State<Arc<AppState>>, Query(q): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    let Some(sid) = q.get("source_id").filter(|s| safe_id(s)) else {
        return plain(StatusCode::BAD_REQUEST, "missing or bad source_id");
    };
    let Some(t) = q.get("at").and_then(|t| t.parse::<f64>().ok()).filter(|t| t.is_finite() && *t >= 0.0) else {
        return plain(StatusCode::BAD_REQUEST, "missing or bad at (source seconds)");
    };
    let w = q.get("w").and_then(|w| w.parse::<u32>().ok()).unwrap_or(640).clamp(16, 1920);
    // Millisecond resolution keeps see.frame's cache file names stable.
    let t = (t * 1000.0).round() / 1000.0;
    let mut ctx = st.base.clone();
    ctx.actor = Actor::Human;
    ctx.project_dir = query_project(&st, &q);
    let input = json!({"source_id": sid, "t": t, "width": w});
    match st.registry.dispatch("see.frame", input, ctx).await {
        Ok(data) => match data.get("image").and_then(Value::as_str) {
            Some(img) => serve_path(&st, PathBuf::from(img), &headers).await,
            None => plain(StatusCode::INTERNAL_SERVER_ERROR, "see.frame returned no image"),
        },
        Err(e) => plain(StatusCode::UNPROCESSABLE_ENTITY, &format!("see.frame: {}", e.message)),
    }
}

/// Caption/overlay fonts installed by `rustler setup` (`<home>/fonts/...`), so the live canvas
/// preview draws with the renderer's fonts. Fixed allowlist: name -> relative file.
const FONTS: &[(&str, &str)] = &[
    ("anton", "fonts/Anton/Anton-Regular.ttf"),
    ("poppins-extrabold", "fonts/Poppins/Poppins-ExtraBold.ttf"),
    ("poppins-bold", "fonts/Poppins/Poppins-Bold.ttf"),
    ("noto-devanagari", "fonts/NotoSansDevanagari/NotoSansDevanagari[wdth,wght].ttf"),
];

/// Names served by `GET /api/font/{name}`.
pub fn font_names() -> Vec<&'static str> {
    FONTS.iter().map(|(n, _)| *n).collect()
}

async fn font(State(st): State<Arc<AppState>>, axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    let Some((_, rel)) = FONTS.iter().find(|(n, _)| *n == name) else {
        return plain(StatusCode::NOT_FOUND, "unknown font");
    };
    match tokio::fs::read(st.base.home.join(rel)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "font/ttf")], bytes).into_response(),
        Err(_) => plain(StatusCode::NOT_FOUND, "font not installed (run `rustler setup`)"),
    }
}

/// `GET /api/clip_state?clip_id=..` -> `{clip_id, revision, extras_mtime_ms}`: a cheap poll
/// the editor uses to notice edits made by an agent (CLI/MCP) and reload.
async fn clip_state(State(st): State<Arc<AppState>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some(id) = q.get("clip_id").filter(|s| safe_id(s)) else {
        return plain(StatusCode::BAD_REQUEST, "missing or bad clip_id");
    };
    let Some(proj) = query_project(&st, &q) else {
        return plain(StatusCode::NOT_FOUND, "no project");
    };
    let dir = proj.join("clips").join(id);
    let Ok(text) = tokio::fs::read_to_string(dir.join("clip.json")).await else {
        return plain(StatusCode::NOT_FOUND, "no such clip");
    };
    let revision = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("revision").and_then(Value::as_u64));
    let extras_mtime_ms = tokio::fs::metadata(dir.join("extras.json"))
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    Json(json!({"clip_id": id, "revision": revision, "extras_mtime_ms": extras_mtime_ms})).into_response()
}

fn content_type(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("json") => "application/json",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("ogg") | Some("oga") => "audio/ogg",
        Some("opus") => "audio/ogg; codecs=opus",
        Some("flac") => "audio/flac",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("woff2") => "font/woff2",
        Some("ass") | Some("srt") | Some("vtt") | Some("ndjson") | Some("txt") => {
            "text/plain; charset=utf-8"
        }
        _ => "application/octet-stream",
    }
}

fn canon(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// `bytes=a-b` | `bytes=a-` | `bytes=-n` -> inclusive (start, end). `Err` = unsatisfiable.
fn parse_range(h: &str, len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = h.trim().strip_prefix("bytes=") else {
        return Ok(None);
    };
    let first = spec.split(',').next().unwrap_or("").trim();
    let Some((a, b)) = first.split_once('-') else {
        return Ok(None);
    };
    if len == 0 {
        return Err(());
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let s: u64 = a.parse().map_err(|_| ())?;
        let e: u64 = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>().map_err(|_| ())?.min(len - 1)
        };
        (s, e)
    };
    if start > end || start >= len {
        return Err(());
    }
    Ok(Some((start, end)))
}

async fn file(
    State(st): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(raw) = q.get("path").filter(|p| !p.is_empty()) else {
        return plain(StatusCode::BAD_REQUEST, "missing path");
    };
    let resp = serve_path(&st, PathBuf::from(raw), &headers).await;
    // `optional=1`: a missing file is an expected state (e.g. an analysis not run yet), so answer
    // 204 instead of 404 and keep the browser console clean.
    if resp.status() == StatusCode::NOT_FOUND && q.get("optional").map(String::as_str) == Some("1") {
        return (StatusCode::NO_CONTENT, "").into_response();
    }
    resp
}

/// Serves a file under the current project, the start project or `~/.rustler`, with Range support.
async fn serve_path(st: &AppState, mut p: PathBuf, headers: &HeaderMap) -> Response {
    let proj = st.current_project();
    if p.is_relative() {
        match &proj {
            Some(d) => p = d.join(p),
            None => return plain(StatusCode::NOT_FOUND, "relative path but no project"),
        }
    }
    let Some(real) = canon(&p) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let roots: Vec<PathBuf> = [proj, st.base.project_dir.clone(), Some(st.base.home.clone())]
        .into_iter()
        .flatten()
        .filter_map(|r| canon(&r))
        .collect();
    if !roots.iter().any(|r| real.starts_with(r)) {
        return plain(StatusCode::FORBIDDEN, "path outside the project and ~/.rustler");
    }
    let meta = match tokio::fs::metadata(&real).await {
        Ok(m) if m.is_file() => m,
        _ => return plain(StatusCode::NOT_FOUND, "not a file"),
    };
    let len = meta.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|h| parse_range(h, len));
    let (status, start, count) = match range {
        None | Some(Ok(None)) => (StatusCode::OK, 0, len),
        Some(Ok(Some((s, e)))) => (StatusCode::PARTIAL_CONTENT, s, e - s + 1),
        Some(Err(())) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{len}"))],
            )
                .into_response();
        }
    };
    let mut f = match tokio::fs::File::open(&real).await {
        Ok(f) => f,
        Err(_) => return plain(StatusCode::NOT_FOUND, "cannot open"),
    };
    if start > 0 && f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "seek failed");
    }
    let body = Body::from_stream(ReaderStream::new(f.take(count)));
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(&real)));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(count));
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {}-{}/{}", start, start + count - 1, len)) {
            h.insert(header::CONTENT_RANGE, v);
        }
    }
    resp
}

async fn events(
    State(st): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<Value> {
    let dir = q
        .get("project")
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| st.current_project());
    let Some(dir) = dir else {
        return Json(json!([]));
    };
    let text = tokio::fs::read_to_string(dir.join("events.ndjson"))
        .await
        .unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(200)..];
    let out: Vec<Value> = tail
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    Json(Value::Array(out))
}
