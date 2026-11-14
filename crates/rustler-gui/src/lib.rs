//! Rustler GUI: a tiny local web server (127.0.0.1 + random token) serving one static page
//! that drives the SAME command registry as the CLI through `POST /api/run`.

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
use rustler_registry::{Ctx, Envelope, Registry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const INDEX_HTML: &str = include_str!("ui/index.html");
const APP_JS: &str = include_str!("ui/app.js");
const STYLE_CSS: &str = include_str!("ui/style.css");

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
        base: base_ctx,
        token: token.clone(),
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/api/commands", get(commands))
        .route("/api/run", post(run))
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

async fn index(State(st): State<Arc<AppState>>) -> Response {
    let html = INDEX_HTML.replace("__TOKEN__", &st.token);
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}
async fn app_js() -> Response {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], APP_JS).into_response()
}
async fn style_css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLE_CSS).into_response()
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
    let project = req
        .project
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let mut ctx = st.base.clone();
    ctx.actor = Actor::Human;
    ctx.project_dir = project.clone().or_else(|| st.current_project());
    let input = match req.input {
        Some(Value::Null) | None => json!({}),
        Some(v) => v,
    };
    let result = st.registry.dispatch(&req.command, input, ctx).await;
    let env = match result {
        Ok(data) => {
            // Remember which project the UI works on (for file serving and events).
            if let Some(p) = project {
                st.set_current(p);
            } else if let Some(d) = data.get("project_dir").and_then(Value::as_str) {
                st.set_current(PathBuf::from(d));
            }
            Envelope::ok(&req.command, data)
        }
        Err(e) => Envelope::err(e),
    };
    Json(env.to_value()).into_response()
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
        Some("m4a") => "audio/mp4",
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
    let proj = st.current_project();
    let mut p = PathBuf::from(raw);
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
