//! JavaScript the desktop shell injects into the GUI page, and the IPC messages it gets back.
//!
//! The page is the unmodified `rustler-gui` UI. The shell only calls functions that UI already
//! exposes on `window.R` (`R.openProject`, `R.E().undo/redo`, `R.toast`) so there is one code path
//! for humans in a browser and in the desktop window.

use serde::Deserialize;
use std::path::Path;

/// Runs before the page's own scripts on every load. Keyboard shortcuts for app-level actions are
/// caught here because, while the webview has keyboard focus, key presses never reach the native
/// menu's accelerators (WebView2 / WebKitGTK consume them). Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y are left
/// to the editor page, which already binds them.
pub const INIT_SCRIPT: &str = r#"(function () {
  if (window.__rustlerDesktop) return;
  window.__rustlerDesktop = true;
  function send(msg) { try { window.ipc.postMessage(JSON.stringify(msg)); } catch (e) { /* not in the desktop shell */ } }
  window.addEventListener('keydown', function (e) {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    var k = e.key, id = null;
    if (k === 'o' || k === 'O') id = 'open';
    else if (k === 'q' || k === 'Q') id = 'quit';
    else if (k === '=' || k === '+') id = 'zoom_in';
    else if (k === '-' || k === '_') id = 'zoom_out';
    else if (k === '0') id = 'zoom_reset';
    if (!id) return;
    e.preventDefault(); e.stopPropagation();
    send({ kind: 'menu', id: id });
  }, true);
})();"#;

/// Opens `dir` through the GUI's own "Open project" path (`R.openProject` -> `project.open`),
/// waiting until the page has loaded its command list; reports back over IPC.
pub fn open_project(dir: &Path) -> String {
    let d = serde_json::to_string(&dir.display().to_string()).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(function (dir) {{
  function send(m) {{ try {{ window.ipc.postMessage(JSON.stringify(m)); }} catch (e) {{}} }}
  var tries = 0;
  (function go() {{
    var R = window.R;
    if (!R || !R.openProject || !R.has || !R.has('project.open')) {{
      if (++tries < 400) return setTimeout(go, 50);
      return send({{ kind: 'open_failed', dir: dir, reason: 'the GUI page did not finish loading' }});
    }}
    Promise.resolve(R.openProject(dir)).then(function (d) {{
      send(d ? {{ kind: 'opened', dir: dir }} : {{ kind: 'open_failed', dir: dir, reason: 'project.open failed (see the page toast)' }});
    }}, function (e) {{ send({{ kind: 'open_failed', dir: dir, reason: String(e) }}); }});
  }})();
}})({d});"#
    )
}

/// Undo (`which = "undo"`) or redo in the clip editor, the same functions as the toolbar buttons.
pub fn history(which: &str) -> String {
    let w = if which == "redo" { "redo" } else { "undo" };
    format!(
        r#"(function () {{
  var R = window.R, E = R && R.E && R.E();
  if (E && E.{w}) {{ E.{w}(); return; }}
  if (R && R.toast) R.toast('{label} works in the clip editor: open a clip first', 'bad');
}})();"#,
        label = if w == "redo" { "Redo" } else { "Undo" }
    )
}

/// A message the page sends with `window.ipc.postMessage(JSON.stringify(..))`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IpcMsg {
    /// A keyboard shortcut for a menu action (same ids as the menu).
    Menu { id: String },
    Opened { dir: String },
    OpenFailed { dir: String, #[serde(default)] reason: String },
}

pub fn parse_ipc(body: &str) -> Option<IpcMsg> {
    serde_json::from_str(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_project_escapes_the_path() {
        let js = open_project(Path::new(r#"C:\Users\x\it's "q"\proj"#));
        assert!(js.contains(r#"("C:\\Users\\x\\it's \"q\"\\proj")"#), "{js}");
        assert!(js.contains("R.openProject(dir)"));
    }

    #[test]
    fn history_calls_editor_functions() {
        assert!(history("undo").contains("E.undo()"));
        assert!(history("redo").contains("E.redo()"));
        assert!(history("anything").contains("E.undo()"));
    }

    #[test]
    fn ipc_messages_parse() {
        assert_eq!(parse_ipc(r#"{"kind":"menu","id":"open"}"#), Some(IpcMsg::Menu { id: "open".into() }));
        assert_eq!(parse_ipc(r#"{"kind":"opened","dir":"C:\\p"}"#), Some(IpcMsg::Opened { dir: "C:\\p".into() }));
        assert_eq!(
            parse_ipc(r#"{"kind":"open_failed","dir":"x"}"#),
            Some(IpcMsg::OpenFailed { dir: "x".into(), reason: String::new() })
        );
        assert_eq!(parse_ipc("nope"), None);
        assert_eq!(parse_ipc(r#"{"kind":"eval","code":"x"}"#), None);
    }
}
