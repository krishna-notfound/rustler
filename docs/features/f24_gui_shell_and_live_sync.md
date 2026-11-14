# f24 — GUI Shell (simple local web page)

**Status:** Verified  |  **Depends on:** f02, f03  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

**Scope note:** the feature index planned a Tauri shell. The built GUI is a simple local web page served by
`rustler gui`, as chosen for the 5-hour window (plan, 5-HOUR SCOPE). The Tauri editor is deferred to f25–f27.

## Purpose

A human can look at what the agent is doing and check results without learning the CLI. The page shows the
project, its sources, clips, renders and activity, and can run any registry command. It uses the same
registry as the CLI and MCP, so the GUI can never do something the agent cannot.

## Scope

**In:**
- `rustler gui` starts an axum server on `127.0.0.1` (random port unless `--port`), prints a URL with a random token, and opens the browser unless `--no-open` is passed.
- One static page (plain HTML, CSS and JS, no framework) with views: Project, Sources, Clips, Renders, Activity, Commands.
- Routes: `GET /`, `GET /app.js`, `GET /style.css`, `GET /api/commands`, `POST /api/run`, `GET /api/file`, `GET /api/events`.
- Renders view plays the render, shows the QC table and the contact sheet.

**Out (explicitly):**
- Tauri shell and the Premiere-style timeline, program monitor, caption editor and render queue (f25, f26, f27).
- Remote or multi-user access; the server is local only.
- Editing layouts by dragging regions (f15, f26).

## User-facing Behavior

- `rustler gui [--port N] [--no-open]` prints the URL and serves until stopped.
- Requests without the token are refused (`token_required`).
- Views: Project (project summary), Sources, Clips, Renders (player, QC table, contact sheet), Activity, Commands (run any command).
- Reloading with a `#view` hash opens that view (fix in commit d77e638; the progress log still lists it as a nit, and the fix is not yet re-checked in a browser).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-gui/src/lib.rs`
- `crates/rustler-gui/src/ui/index.html`, `app.js`, `style.css`
- `crates/rustler-gui/examples/demo.rs`
- `crates/rustler-gui/tests/gui.rs`

**Read-only dependencies:**
- `crates/rustler-registry` — commands listed and run through `POST /api/run` (f02).
- `crates/rustler-cli/src/main.rs` — `gui` is intercepted and handed to `rustler_gui::serve` (shared with f02, f10).

## Data and Interfaces

- `rustler_gui::serve(reg, ctx, GuiOptions { port, open_browser })`.
- `POST /api/run {command, input}` dispatches to the registry; the response is the standard envelope.
- `GET /api/file` serves files (test `file_serving_and_events`); `GET /api/events` serves events (same test). The exact file set served is in `crates/rustler-gui/src/lib.rs`.
- Styling uses CSS custom properties on `:root` (dark palette: background `#0f1115`, accent `#5b8cff`).

## Rules and Edge Cases

1. The server binds to `127.0.0.1` only.
2. Every GUI action is a registry command; the GUI has no private code path.
3. The token is random per launch and required on API routes.
4. The GUI must not show a state that does nothing: buttons call real commands.

## Tasks

- [x] Axum server, token, static page (`crates/rustler-gui/src/lib.rs`)
- [x] Views and command runner (`app.js`)
- [ ] Tauri shell and timeline editor (deferred, f25–f27)

## Acceptance Criteria

- [x] GUI tests (`crates/rustler-gui/tests/gui.rs`, 4 tests): `token_required`, `page_references_app_js`, `commands_and_run`, `file_serving_and_events`
- [x] Real browser check on the e2e project: Project page, Renders page plays the NVENC render, QC table (12 pass, -14.3 LUFS), contact sheet; no console errors (progress log)

## Verification

- `cargo test -p rustler-gui` (PowerShell).
- `rustler gui --no-open --port 8787` in a project, open the printed URL with the token; pass = Project view loads and Commands view lists registry commands.

## Status Log

- 2026-11-15 — Simple web GUI merged (axum, token, plain JS). Verified in a real browser on the e2e project: Renders page plays the render, QC table and contact sheet shown, no console errors. Known nit at that point: `#view` hash reload landed on Project; fixed in commit d77e638 (not yet re-verified in a browser).
