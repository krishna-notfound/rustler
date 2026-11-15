# Rustler desktop app

`rustler desktop` (or the standalone `rustler-desktop` binary) opens the Rustler editor in a **native
desktop window** with a native menu bar, like a Premiere-style application. It is not a second UI and not
a second engine: the window hosts the existing GUI (`crates/rustler-gui`, same page, same 14 JS modules)
served **in-process** over the **same command registry** as the CLI and MCP server.

```text
rustler desktop [--project <dir>] [--view <route>]
rustler-desktop [--project <dir>] [--view <route>]
```

| Flag / env | Meaning |
|---|---|
| `--project <dir>` | project the server starts with (default: `RUSTLER_PROJECT`, else the current folder if it holds `project.json`, else none: the Project page lets you open one) |
| `--view <route>` | GUI route shown first, appended as the URL hash: `editor/<clip_id>`, `sources`, `clips`, `renders`, ... |
| `RUSTLER_HOME` | data folder (default `~/.rustler`): `desktop.json`, `projects.json`, webview profile |
| `RUSTLER_DESKTOP_OPEN=<dir>` | after the first page load, open this project through the **same function** File > Open project... calls after the folder picker (for tests and scripted launches) |

`rustler desktop --help` prints the help envelope (`rustler.desktop.help@1`). The process prints one JSON
line per lifecycle event on stdout (`rustler.desktop@1`): `started` (origin, project, state file),
`project_opened` / `project_open_failed`, `closed` (whether the window state was saved). The token is never
printed. Release builds of `rustler-desktop.exe` are Windows GUI-subsystem apps (no console window); start
errors are shown in a native message box.

## What you get

- **Window:** title `Rustler` (`Rustler — <project folder>` once a project is open), default inner size
  1440x900, minimum 1280x800, app icon drawn in code (`crates/rustler-desktop/src/icon.rs`: orange rounded
  square with a white play triangle; window and taskbar icon). Size, maximized flag and zoom are remembered in
  `<RUSTLER_HOME>/desktop.json`. When the remembered size plus window chrome does not fit the primary
  screen (e.g. 1440x900 on a 1920x1080 display at 125 % scaling), the window starts maximized; otherwise it is
  centred.
- **Menu (native, dark on Windows):**
  - **File:** Open project... (Ctrl+O, native folder picker via `rfd`), Open Recent (up to 10 projects from
    `<RUSTLER_HOME>/projects.json`, newest first, only folders that still contain `project.json`; refreshed
    after every successful open), Quit (Ctrl+Q).
  - **Edit:** Undo / Redo: evaluated in the page as `R.E().undo()` / `R.E().redo()`, the same functions as the
    editor toolbar buttons (`history.undo` / `history.restore`). Outside the clip editor the page shows a toast.
    These items register no accelerator on purpose: the editor page already binds Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y,
    and a second binding would apply each edit twice.
  - **View:** Reload (F5), Zoom In / Zoom Out / Actual Size (Ctrl+= / Ctrl+- / Ctrl+0, 50 %–200 %, remembered),
    Toggle DevTools (F12, **debug builds only**).
  - **Help:** Open docs folder (a `docs/README.md` found next to or above the executable, else the source tree the
    binary was built from), About Rustler.
- **Open project:** the folder must contain `project.json` (otherwise a warning box). The shell then calls the
  GUI's own `R.openProject(dir)` (which runs `project.open` and records the project in `projects.json`), waits
  for the page to confirm over IPC, updates the title and the Recent menu.

### Keyboard shortcuts while the page has focus

While the webview has keyboard focus, key presses go to the browser engine, not to the native menu's
accelerators. An initialization script (`js::INIT_SCRIPT`) therefore catches Ctrl+O, Ctrl+Q, Ctrl+= / Ctrl+-
/ Ctrl+0 in the page and posts them to the shell over `window.ipc`, which runs the same menu actions. F5 and
F12 are handled by WebView2 itself. Native Ctrl+wheel / Ctrl+± zoom of the webview is disabled
(`with_hotkeys_zoom(false)`) so there is one zoom level, the one the menu shows and remembers.

## Security

- The GUI server binds `127.0.0.1` on a random port with a random per-run token (unchanged `rustler-gui`
  guard: token, Host and Origin checks). The webview loads `http://127.0.0.1:<port>/?t=<token>`.
- **Only that origin loads in the window** (`with_navigation_handler`). Any other `http(s):` / `mailto:` link
  opens in the system browser; other schemes (`file:`, `javascript:`, ...) are refused. `window.open` /
  `target=_blank` requests never create a second webview window: they open in the system browser.
- IPC messages are accepted only from the app origin and only as three typed messages (`menu`, `opened`,
  `open_failed`); the page cannot make the shell evaluate code or open arbitrary paths.
- The webview profile (cookies, localStorage, cache) lives in `<RUSTLER_HOME>/desktop-webview`, not next to the
  executable. Because the port is random, per-origin page storage starts fresh each run; the project to show
  comes from `--project` / Open Recent instead.
- **Shutdown:** closing the window (or File > Quit / Ctrl+Q) ends the event loop, saves `desktop.json`, drops
  the webview and stops the in-process server task; the process then exits (verified: the port stops listening).

## Why wry + tao instead of Tauri

The design spec (§5.10) named Tauri. The desktop app uses **wry + tao + muda** directly, the same crates Tauri
itself is built on (wry = WebView2 / WebKitGTK webview, tao = windowing, muda = native menus), plus `rfd` for
native dialogs. Reasons:

- The UI already exists as a plain-JS page served by an axum server with its own security guard; nothing of
  Tauri's command/IPC/capability system, bundler or frontend pipeline would be used.
- Tauri 2 requires `tauri-build` in a build script, a `tauri.conf.json`, an icon set on disk and its CLI for
  bundling; wry + tao need none of that, so `cargo build --workspace` stays the only build step and no Node/npm
  toolchain is involved.
- Far fewer dependencies and a shorter build (`rustler-desktop` debug build from cold: about 8 minutes with
  `CARGO_BUILD_JOBS=2` on this laptop, most of it the shared workspace).

Pinned versions: `wry 0.57.2` (feature `devtools`), `tao 0.37.2`, `muda 0.21.2`, `rfd 0.18.0`.

## Platforms

- **Windows 10/11:** uses the WebView2 runtime (preinstalled on Windows 11). Verified on this laptop (Windows 11).
- **Linux:** the same code builds against WebKitGTK 4.1 + GTK 3, but those system packages are not on every
  machine (they were not on the WSL Ubuntu used for `docs/linux.md`). So on Linux the webview stack is behind
  the `linux-webview` feature of `rustler-desktop`; without it `rustler-desktop` / `rustler desktop` build and
  exit with an error that points to `rustler gui`, and `cargo build --workspace` keeps working on headless
  machines. To build the real window on Linux:

  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev
  cargo build --release -p rustler-desktop --features linux-webview
  cargo build --release -p rustler-cli --features rustler-desktop/linux-webview   # `rustler desktop`
  ```

- **macOS:** compiles the same way (WKWebView, `init_for_nsapp` menu) but was not built or tested.

## Files

| File | Purpose |
|---|---|
| `crates/rustler-desktop/src/lib.rs` | options, URL policy, docs folder, project/home resolution, platform gate |
| `crates/rustler-desktop/src/app.rs` | window, menu, webview, event loop, shutdown (`run`) |
| `crates/rustler-desktop/src/js.rs` | page scripts (init shortcuts, open project, undo/redo) and IPC message types |
| `crates/rustler-desktop/src/state.rs` | `desktop.json` load/save, recent projects from `projects.json` |
| `crates/rustler-desktop/src/icon.rs` | app icon pixels |
| `crates/rustler-desktop/src/main.rs` | `rustler-desktop` binary |
| `crates/rustler-cli/src/main.rs` | `rustler desktop` built-in (runs outside `block_on`; the window owns the main thread) |

## Verification (2026-11-15, Windows 11, this laptop)

- `cargo test -p rustler-desktop`: unit tests for window-state round trip/clamping, recent-projects ordering
  and filtering, URL allowlist, external-link schemes, JS path escaping, IPC parsing, accelerators, icon pixels,
  docs folder lookup.
- Launched `rustler-desktop --project %TEMP%\rustler-stream --view editor/<clip>`: native window with menu bar,
  editor (source monitor with regions, 9:16 program, inspector, timeline) inside: `.playwright-mcp/desktop-editor.png`.
- `RUSTLER_DESKTOP_OPEN=%TEMP%\rustler-stream rustler desktop` (no `--project`): stdout `project_opened`, title
  `Rustler — rustler-stream`, Project page shows the project: `.playwright-mcp/desktop-open-project.png`.
- File menu and Open Recent submenu (real entries from `projects.json`): `.playwright-mcp/desktop-menu-file.png`,
  `.playwright-mcp/desktop-menu-recent.png`. Choosing a recent project opened it (`project_opened`, title changed);
  Edit > Undo outside the editor shows the page toast: `.playwright-mcp/desktop-recent-undo.png`.
- Ctrl+O with focus in the page opens the native folder picker parented to the window:
  `.playwright-mcp/desktop-open-dialog.png`; Ctrl+= zooms the page: `.playwright-mcp/desktop-zoom.png`.
- Closing the window: stdout `closed` with `state_saved: true`, `desktop.json` written, process exited, nothing
  listening on the server port afterwards.
