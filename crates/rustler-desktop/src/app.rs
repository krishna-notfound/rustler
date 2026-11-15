//! The native window: tao window + muda menu + wry webview hosting the in-process GUI server,
//! and the event loop that ties them together. Compiled on Windows/macOS, and on Linux with the
//! `linux-webview` feature (see `docs/desktop.md`).

use crate::state::{self, RecentProject, WindowState};
use crate::{DesktopOptions, RECENT_LIMIT, WINDOW_CHROME, docs_dir, emit, icon, is_app_url, is_external_openable, js, window_title};
use anyhow::Context as _;
use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator};
use rustler_registry::{Ctx, Registry};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tao::dpi::{LogicalSize, PhysicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Icon, Window, WindowBuilder};
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};
#[derive(Debug)]
enum UserEvent {
    Menu(String),
    Ipc(String),
    PageLoaded(String),
}

fn accel(s: &str) -> Option<Accelerator> {
    s.parse().ok()
}

/// The native menu bar and the parts that change at runtime.
struct AppMenu {
    bar: Menu,
    recent: Submenu,
    recent_items: Vec<MenuItem>,
    recent_paths: Vec<PathBuf>,
}

impl AppMenu {
    fn build(recents: &[RecentProject]) -> anyhow::Result<Self> {
        let recent = Submenu::new("Open &Recent", true);
        let file = Submenu::with_items(
            "&File",
            true,
            &[
                &MenuItem::with_id("open", "&Open project\u{2026}", true, accel("CmdOrCtrl+O")),
                &recent,
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("quit", "&Quit", true, accel("CmdOrCtrl+Q")),
            ],
        )?;
        // Undo/redo have no registered accelerator: the editor page binds Ctrl+Z / Ctrl+Shift+Z
        // itself, and a second binding here would apply each edit twice.
        let edit = Submenu::with_items(
            "&Edit",
            true,
            &[
                &MenuItem::with_id("undo", "&Undo\tCtrl+Z", true, None),
                &MenuItem::with_id("redo", "&Redo\tCtrl+Shift+Z", true, None),
            ],
        )?;
        let view = Submenu::with_items(
            "&View",
            true,
            &[
                &MenuItem::with_id("reload", "&Reload", true, accel("F5")),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("zoom_in", "Zoom &In", true, accel("CmdOrCtrl+=")),
                &MenuItem::with_id("zoom_out", "Zoom &Out", true, accel("CmdOrCtrl+-")),
                &MenuItem::with_id("zoom_reset", "&Actual Size", true, accel("CmdOrCtrl+0")),
            ],
        )?;
        if cfg!(debug_assertions) {
            view.append_items(&[
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("devtools", "Toggle &DevTools", true, accel("F12")),
            ])?;
        }
        let help = Submenu::with_items(
            "&Help",
            true,
            &[
                &MenuItem::with_id("docs", "Open &docs folder", true, None),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("about", "&About Rustler", true, None),
            ],
        )?;
        let bar = Menu::with_items(&[&file, &edit, &view, &help])?;
        let mut m = Self { bar, recent, recent_items: Vec::new(), recent_paths: Vec::new() };
        m.set_recent(recents);
        Ok(m)
    }

    fn set_recent(&mut self, recents: &[RecentProject]) {
        for it in self.recent_items.drain(..) {
            let _ = self.recent.remove(&it);
        }
        self.recent_paths.clear();
        if recents.is_empty() {
            let it = MenuItem::with_id("recent_none", "No recent projects", false, None);
            let _ = self.recent.append(&it);
            self.recent_items.push(it);
            return;
        }
        for (i, r) in recents.iter().enumerate() {
            // `&&` shows a literal ampersand instead of a mnemonic.
            let label = format!("{}  \u{2014}  {}", r.name, r.path.display()).replace('&', "&&");
            let it = MenuItem::with_id(format!("recent:{i}"), label, true, None);
            let _ = self.recent.append(&it);
            self.recent_items.push(it);
            self.recent_paths.push(r.path.clone());
        }
    }

    #[cfg(target_os = "windows")]
    fn attach(&self, window: &Window) -> anyhow::Result<()> {
        use tao::platform::windows::WindowExtWindows;
        // SAFETY: the hwnd belongs to `window`, which outlives the menu (both live until the
        // event loop returns).
        unsafe { self.bar.init_for_hwnd_with_theme(window.hwnd(), muda::MenuTheme::Dark) }?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn attach(&self, window: &Window) -> anyhow::Result<()> {
        use tao::platform::unix::WindowExtUnix;
        self.bar.init_for_gtk_window(window.gtk_window(), window.default_vbox())?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn attach(&self, _window: &Window) -> anyhow::Result<()> {
        self.bar.init_for_nsapp();
        Ok(())
    }
}

/// Shows a native message box (blocking).
fn message(level: rfd::MessageLevel, title: &str, text: &str) {
    rfd::MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// The live app: window, webview, server and menu.
struct App {
    window: Window,
    webview: WebView,
    menu: AppMenu,
    home: PathBuf,
    origin: String,
    state: WindowState,
    project: Option<PathBuf>,
    devtools_open: bool,
}

impl App {
    /// File > Open project... after the picker, and [`OPEN_ENV`]: hands the folder to the GUI's
    /// own open-project function; the page confirms over IPC.
    fn open_project(&mut self, dir: &Path) -> anyhow::Result<()> {
        if !state::is_project(dir) {
            anyhow::bail!("{} is not a Rustler project (no project.json)", dir.display());
        }
        self.webview.evaluate_script(&js::open_project(dir)).context("send open-project to the page")?;
        Ok(())
    }

    fn pick_and_open(&mut self) {
        let mut dlg = rfd::FileDialog::new().set_title("Open Rustler project").set_parent(&self.window);
        if let Some(start) = self.project.clone().or_else(|| state::recent_projects(&self.home, 1).pop().map(|r| r.path)) {
            dlg = dlg.set_directory(start);
        }
        if let Some(dir) = dlg.pick_folder() {
            self.open_or_report(&dir);
        }
    }

    fn open_or_report(&mut self, dir: &Path) {
        if let Err(e) = self.open_project(dir) {
            emit("project_open_failed", json!({"dir": dir.display().to_string(), "reason": e.to_string()}));
            message(rfd::MessageLevel::Warning, "Open project", &format!("{e:#}"));
        }
    }

    fn set_zoom(&mut self, z: f64) {
        self.state.zoom = (z * 100.0).round() / 100.0;
        self.state = self.state.clone().sanitized();
        let _ = self.webview.zoom(self.state.zoom);
    }

    fn on_menu(&mut self, id: &str, cf: &mut ControlFlow) {
        match id {
            "open" => self.pick_and_open(),
            "quit" => *cf = ControlFlow::Exit,
            "undo" | "redo" => {
                let _ = self.webview.evaluate_script(&js::history(id));
            }
            "reload" => {
                let _ = self.webview.reload();
            }
            "zoom_in" => self.set_zoom(self.state.zoom + 0.1),
            "zoom_out" => self.set_zoom(self.state.zoom - 0.1),
            "zoom_reset" => self.set_zoom(1.0),
            "devtools" if cfg!(debug_assertions) => {
                if self.devtools_open {
                    self.webview.close_devtools();
                } else {
                    self.webview.open_devtools();
                }
                self.devtools_open = !self.devtools_open;
            }
            "docs" => match docs_dir() {
                Some(d) => {
                    if let Err(e) = open::that(&d) {
                        message(rfd::MessageLevel::Error, "Open docs folder", &format!("{}: {e}", d.display()));
                    }
                }
                None => message(rfd::MessageLevel::Info, "Open docs folder", "No docs folder was found next to this executable."),
            },
            "about" => message(
                rfd::MessageLevel::Info,
                "About Rustler",
                &format!(
                    "Rustler {}\n\nLocal, agent-native clipping and editing studio.\nThis window hosts the Rustler GUI served in-process on {} (loopback only, per-run token).\n\nData: {}",
                    env!("CARGO_PKG_VERSION"),
                    self.origin,
                    self.home.display()
                ),
            ),
            other => {
                if let Some(i) = other.strip_prefix("recent:").and_then(|n| n.parse::<usize>().ok()) {
                    if let Some(p) = self.menu.recent_paths.get(i).cloned() {
                        self.open_or_report(&p);
                    }
                }
            }
        }
    }

    fn on_ipc(&mut self, body: &str, cf: &mut ControlFlow) {
        match js::parse_ipc(body) {
            Some(js::IpcMsg::Menu { id }) => self.on_menu(&id, cf),
            Some(js::IpcMsg::Opened { dir }) => {
                let p = PathBuf::from(&dir);
                self.window.set_title(&window_title(Some(&p)));
                self.project = Some(p);
                self.menu.set_recent(&state::recent_projects(&self.home, RECENT_LIMIT));
                emit("project_opened", json!({"dir": dir}));
            }
            Some(js::IpcMsg::OpenFailed { dir, reason }) => {
                emit("project_open_failed", json!({"dir": dir, "reason": reason}));
            }
            None => {}
        }
    }

    fn remember_size(&mut self, size: PhysicalSize<u32>) {
        let max = self.window.is_maximized();
        self.state.maximized = max;
        if max || self.window.is_minimized() || size.width == 0 || size.height == 0 {
            return;
        }
        let l: LogicalSize<f64> = size.to_logical(self.window.scale_factor());
        self.state.width = l.width;
        self.state.height = l.height;
    }
}

/// Starts the GUI server on `rt`, opens the window and runs the native event loop on the calling
/// thread (must be the main thread, and not inside an async context) until the window closes.
/// Returns the process exit code.
pub fn run(registry: Registry, opts: DesktopOptions, rt: &tokio::runtime::Handle) -> anyhow::Result<i32> {
    let ctx = Ctx::new(opts.project.clone(), opts.home.clone());
    let gui = rt
        .block_on(rustler_gui::start(
            Arc::new(registry),
            ctx,
            &rustler_gui::GuiOptions { port: None, open_browser: false },
        ))
        .context("start the GUI server")?;
    let origin = format!("http://127.0.0.1:{}", gui.port);
    let mut url = gui.url.clone();
    if let Some(v) = opts.view.as_deref().map(|v| v.trim_start_matches('#')).filter(|v| !v.is_empty()) {
        url.push('#');
        url.push_str(v);
    }

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy: EventLoopProxy<UserEvent> = event_loop.create_proxy();

    let st = WindowState::load(&opts.home);
    let icon = Icon::from_rgba(icon::rgba(64), 64, 64).context("app icon")?;
    // A screen too small for the remembered size (title bar and menu included) starts maximized;
    // otherwise the window is centred on the primary monitor.
    let mut maximized = st.maximized;
    let mut position = None;
    if let Some(m) = event_loop.primary_monitor() {
        let sf = m.scale_factor();
        let screen: LogicalSize<f64> = m.size().to_logical(sf);
        let origin_pt: tao::dpi::LogicalPosition<f64> = m.position().to_logical(sf);
        if st.width + 16.0 > screen.width || st.height + WINDOW_CHROME > screen.height {
            maximized = true;
        } else {
            position = Some(tao::dpi::LogicalPosition::new(
                origin_pt.x + (screen.width - st.width) / 2.0,
                origin_pt.y + ((screen.height - st.height - WINDOW_CHROME) / 2.0).max(0.0),
            ));
        }
    }
    let mut wb = WindowBuilder::new()
        .with_title(window_title(opts.project.as_deref()))
        .with_inner_size(LogicalSize::new(st.width, st.height))
        .with_min_inner_size(LogicalSize::new(state::MIN_SIZE.0, state::MIN_SIZE.1))
        .with_maximized(maximized);
    if let Some(p) = position {
        wb = wb.with_position(p);
    }
    let window = wb
        .with_window_icon(Some(icon))
        .build(&event_loop)
        .context("create the window")?;

    let menu = AppMenu::build(&state::recent_projects(&opts.home, RECENT_LIMIT)).context("build the menu")?;
    menu.attach(&window).context("attach the menu")?;
    {
        // muda wants a Send + Sync handler; the tao proxy is Send, so guard it.
        let p = Mutex::new(proxy.clone());
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            if let Ok(p) = p.lock() {
                let _ = p.send_event(UserEvent::Menu(e.id.0));
            }
        }));
    }

    let mut web_ctx = WebContext::new(Some(opts.home.join("desktop-webview")));
    let ipc_proxy = proxy.clone();
    let ipc_origin = origin.clone();
    let nav_origin = origin.clone();
    let load_proxy = proxy.clone();
    let builder = WebViewBuilder::new_with_web_context(&mut web_ctx)
        .with_url(&url)
        .with_background_color((22, 24, 29, 255))
        .with_initialization_script(js::INIT_SCRIPT)
        .with_devtools(cfg!(debug_assertions))
        .with_hotkeys_zoom(false)
        .with_ipc_handler(move |req| {
            // Only the app's own page may drive the shell.
            if is_app_url(&req.uri().to_string(), &ipc_origin) {
                let _ = ipc_proxy.send_event(UserEvent::Ipc(req.body().clone()));
            }
        })
        .with_navigation_handler(move |u| {
            if is_app_url(&u, &nav_origin) || u == "about:blank" {
                return true;
            }
            if is_external_openable(&u) {
                let _ = open::that(&u);
            }
            false
        })
        .with_new_window_req_handler(|u, _features| {
            // Same-origin pop-ups (e.g. a rendered file) and external links: system browser.
            if is_external_openable(&u) {
                let _ = open::that(&u);
            }
            NewWindowResponse::Deny
        })
        .with_on_page_load_handler(move |ev, u| {
            if matches!(ev, PageLoadEvent::Finished) {
                let _ = load_proxy.send_event(UserEvent::PageLoaded(u));
            }
        });
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window).context("create the webview (is WebView2 installed?)")?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().context("gtk vbox")?;
        builder.build_gtk(vbox).context("create the webview (WebKitGTK)")?
    };
    if (st.zoom - 1.0).abs() > f64::EPSILON {
        let _ = webview.zoom(st.zoom);
    }

    emit(
        "started",
        json!({"origin": origin, "project": opts.project.as_ref().map(|p| p.display().to_string()), "state_file": opts.home.join(state::STATE_FILE).display().to_string()}),
    );

    let mut app = App {
        window,
        webview,
        menu,
        home: opts.home.clone(),
        origin: origin.clone(),
        state: st,
        project: opts.project.clone(),
        devtools_open: false,
    };
    let mut pending_open = opts.open_after_load.clone();

    let code = event_loop.run_return(|event, _target, cf| {
        *cf = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *cf = ControlFlow::Exit,
            Event::WindowEvent { event: WindowEvent::Resized(size), .. } => app.remember_size(size),
            Event::UserEvent(UserEvent::Menu(id)) => app.on_menu(&id, cf),
            Event::UserEvent(UserEvent::Ipc(body)) => app.on_ipc(&body, cf),
            Event::UserEvent(UserEvent::PageLoaded(u)) => {
                if is_app_url(&u, &app.origin) {
                    if let Some(dir) = pending_open.take() {
                        app.open_or_report(&dir);
                    }
                }
            }
            _ => {}
        }
    });

    // Graceful shutdown: persist the window, close the webview, stop the server.
    muda::MenuEvent::set_event_handler(None::<fn(muda::MenuEvent)>);
    let saved = app.state.save(&app.home);
    drop(app);
    gui.abort();
    emit("closed", json!({"state_saved": saved.is_ok()}));
    Ok(code)
}


#[cfg(test)]
mod tests {
    #[test]
    fn accelerators_parse() {
        for a in ["CmdOrCtrl+O", "CmdOrCtrl+Q", "F5", "CmdOrCtrl+=", "CmdOrCtrl+-", "CmdOrCtrl+0", "F12"] {
            assert!(super::accel(a).is_some(), "{a}");
        }
    }
}