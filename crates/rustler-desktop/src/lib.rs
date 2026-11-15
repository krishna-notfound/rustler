//! Rustler desktop app: a native window (tao) with a native menu (muda) hosting the existing
//! Rustler GUI in the system webview (wry: WebView2 on Windows, WebKitGTK on Linux).
//!
//! There is no second UI and no second engine: [`run`] starts the same `rustler-gui` axum server
//! in-process on `127.0.0.1` (random port, random token) over the caller's registry and points the
//! webview at it. Only that origin may load inside the window; every other link opens in the
//! system browser. Closing the window stops the server.
//!
//! Platform gate: the webview stack is always built on Windows and macOS; on Linux it needs
//! WebKitGTK/GTK development packages, so it is behind the `linux-webview` feature and without it
//! [`run`] returns an error pointing to `rustler gui` (keeps headless Linux builds working).

#![cfg_attr(all(target_os = "linux", not(feature = "linux-webview")), allow(dead_code))]

pub mod icon;
pub mod js;
pub mod state;

#[cfg(any(not(target_os = "linux"), feature = "linux-webview"))]
mod app;
#[cfg(any(not(target_os = "linux"), feature = "linux-webview"))]
pub use app::run;

use serde_json::json;
use std::path::{Path, PathBuf};

/// Env var: after the first page load, open this project folder through the same function the
/// File > Open project... folder picker uses (for tests and scripted launches).
pub const OPEN_ENV: &str = "RUSTLER_DESKTOP_OPEN";
pub(crate) const RECENT_LIMIT: usize = 10;
pub(crate) const BASE_TITLE: &str = "Rustler";
/// Logical height of the title bar + menu bar + taskbar allowance, used to decide whether the
/// remembered size fits the screen.
pub(crate) const WINDOW_CHROME: f64 = 110.0;

/// What to open.
#[derive(Debug, Clone, Default)]
pub struct DesktopOptions {
    /// Project the server starts with (the GUI's "server default project").
    pub project: Option<PathBuf>,
    /// `RUSTLER_HOME` (holds `desktop.json`, `projects.json` and the webview profile).
    pub home: PathBuf,
    /// Optional GUI route appended as the URL hash, e.g. `editor/clp_...` or `sources`.
    pub view: Option<String>,
    /// Project to open after the first page load (see [`OPEN_ENV`]).
    pub open_after_load: Option<PathBuf>,
}


/// Prints one JSON line on stdout (`rustler.desktop@1`), like the other servers do.
pub(crate) fn emit(event: &str, data: serde_json::Value) {
    let mut d = data;
    if let Some(o) = d.as_object_mut() {
        o.insert("event".into(), json!(event));
    }
    println!("{}", json!({"ok": true, "schema": "rustler.desktop@1", "data": d}));
}

/// True when `url` belongs to the in-process GUI server (`origin` is `http://127.0.0.1:<port>`).
pub fn is_app_url(url: &str, origin: &str) -> bool {
    url == origin || url.strip_prefix(origin).is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('#'))
}

/// Links worth handing to the system browser (never `file:`, `javascript:` etc.).
pub fn is_external_openable(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")
}

/// The docs folder for Help > Open docs folder: a `docs/README.md` next to (or above) the
/// executable, else the source tree this binary was built from.
pub fn docs_dir() -> Option<PathBuf> {
    let has = |d: &Path| d.join("README.md").is_file();
    if let Ok(exe) = std::env::current_exe() {
        for a in exe.ancestors().skip(1) {
            let d = a.join("docs");
            if has(&d) {
                return Some(d);
            }
        }
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("docs");
    has(&built).then(|| built.canonicalize().unwrap_or(built))
}

pub(crate) fn window_title(project: Option<&Path>) -> String {
    match project {
        Some(p) => format!("{BASE_TITLE} \u{2014} {}", state::folder_name(p)),
        None => BASE_TITLE.to_string(),
    }
}


/// Whether this build contains the native window (always on Windows/macOS; Linux needs the
/// `linux-webview` feature).
pub const HAS_WEBVIEW: bool = cfg!(any(not(target_os = "linux"), feature = "linux-webview"));

/// Linux build without the `linux-webview` feature: no window available.
#[cfg(all(target_os = "linux", not(feature = "linux-webview")))]
pub fn run(
    _registry: rustler_registry::Registry,
    _opts: DesktopOptions,
    _rt: &tokio::runtime::Handle,
) -> anyhow::Result<i32> {
    anyhow::bail!(
        "this build has no desktop webview: on Linux install libwebkit2gtk-4.1-dev, libgtk-3-dev and libxdo-dev, then rebuild with `--features rustler-desktop/linux-webview` (see docs/desktop.md); `rustler gui` serves the same editor in a browser"
    )
}

/// Shows a native error box where the window stack is available, and always prints to stderr.
pub fn error_dialog(msg: &str) {
    eprintln!("rustler-desktop: {msg}");
    #[cfg(any(not(target_os = "linux"), feature = "linux-webview"))]
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Rustler")
        .set_description(msg)
        .show();
}
/// `RUSTLER_HOME`, else `~/.rustler` (same rule as the CLI).
pub fn resolve_home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME") {
        return h.into();
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".rustler")
}

/// `--project`, else `RUSTLER_PROJECT`, else the current folder when it holds a `project.json`.
pub fn resolve_project(flag: Option<PathBuf>) -> Option<PathBuf> {
    if flag.is_some() {
        return flag;
    }
    if let Some(p) = std::env::var_os("RUSTLER_PROJECT") {
        return Some(p.into());
    }
    let cwd = std::env::current_dir().ok()?;
    state::is_project(&cwd).then_some(cwd)
}

/// [`OPEN_ENV`] as a path, when set and non-empty.
pub fn open_env() -> Option<PathBuf> {
    std::env::var_os(OPEN_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_server_origin_is_an_app_url() {
        let o = "http://127.0.0.1:5123";
        assert!(is_app_url("http://127.0.0.1:5123/?t=abc#editor/x", o));
        assert!(is_app_url("http://127.0.0.1:5123", o));
        assert!(is_app_url("http://127.0.0.1:5123/api/file?path=x", o));
        assert!(!is_app_url("http://127.0.0.1:51234/", o));
        assert!(!is_app_url("http://127.0.0.1:5123.evil.com/", o));
        assert!(!is_app_url("https://example.com/", o));
    }

    #[test]
    fn external_links_are_web_or_mail_only() {
        assert!(is_external_openable("https://github.com/"));
        assert!(is_external_openable("HTTP://x"));
        assert!(is_external_openable("mailto:a@b.c"));
        assert!(!is_external_openable("file:///C:/Windows"));
        assert!(!is_external_openable("javascript:alert(1)"));
    }

    #[test]
    fn titles() {
        assert_eq!(window_title(None), "Rustler");
        assert_eq!(window_title(Some(Path::new("C:/x/rustler-stream"))), "Rustler \u{2014} rustler-stream");
    }


    #[test]
    fn docs_dir_found_from_source_tree() {
        let d = docs_dir().expect("docs dir");
        assert!(d.join("README.md").is_file());
    }
}
