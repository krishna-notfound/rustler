# UI Context

This file describes the local web GUI (`crates/rustler-gui`): the shell (f24) and the editor built on it (f25–f27: source/region editor,
program monitor, multi-track timeline, inspector tabs, render queue; see `docs/features/f25_gui_editor.md`). It is not a Tauri app (deferred).
The sections below describe the original shell's theme and tokens, which the editor reuses. The page is plain HTML, CSS and JavaScript served by the Rust server; there is no
framework, no build step and no component library.

## Theme

Dark only. A quiet, dense tool look: dark panels, one blue accent, muted secondary text.

All colors are CSS custom properties defined on `:root` in `crates/rustler-gui/src/ui/style.css`. There is no
light theme and no dark-mode switch.

| Role | CSS Variable | Value |
| ---- | ------------ | ----- |
| Page background | `--bg` | `#0f1115` |
| Surface (cards, nav) | `--panel` | `#171a21` |
| Raised surface (buttons, chips, toasts) | `--panel2` | `#1e222b` |
| Border | `--line` | `#2a2f3a` |
| Primary text | `--text` | `#e6e8ee` |
| Muted text | `--muted` | `#8b93a3` |
| Accent (links, busy indicator, toast edge) | `--accent` | `#5b8cff` |
| Accent, pressed or darker | `--accent2` | `#3d6ae0` |
| Pass | `--pass` | `#3fb950` |
| Warning | `--warn` | `#d29922` |
| Fail | `--fail` | `#f85149` |

Components reference these variables only (`var(--...)`). Raw hex values belong in the `:root` block.

## Typography

| Role | Font | CSS Variable |
| ---- | ---- | ------------ |
| UI text | Browser default sans-serif (no web font is loaded) | none (inherits) |
| Mono (paths, times, JSON, unit ids) | `ui-monospace, SFMono-Regular, Consolas, Menlo, monospace` | `--mono` |

Scale (as set in `style.css`): page title `h1` 20px; section `h2` 15px; sub-label `h3` 13px uppercase with muted
color; body 14px by browser default; small text 11–12px (chips, notes, mono labels); brand 18px bold.

## Spacing, Radius, Shadow

| Context | Value in `style.css` |
| ------- | ------------- |
| Nav links, buttons, inputs, `pre`, toasts | `border-radius: 6px` |
| Cards, `details` blocks | `border-radius: 10px` (cards), `8px` (details, video) |
| Chips and badges | `border-radius: 99px` |
| Shadow | none defined |

## Component Library

None. Components are plain elements built by `app.js` (helper `h()` creates elements). Styling is in `style.css`.

## Layout Patterns

- One shell: left nav (`#nav`, `#navlinks`, `#navfoot`), top bar (`#topbar`, project chip `#projchip`, busy
  indicator `#busy`), main view (`#view`), toast stack (`#toasts`).
- Views in the nav: Project, Sources, Clips, Renders, Activity, Commands.
- Breakpoint: one `@media (max-width: 800px)` rule in `style.css`; the exact layout change is defined there.

## Required UI States

- Empty state: `.na` block (dashed border, muted italic text) for views with no data.
- Errors: toast with the command name, the error message and the hint (`app.js` `toast(..., 'bad')`).
- Warnings: toast with `warning:` prefix.
- Busy: `#busy` shows while a command runs.
- Loading skeleton: not built. Destructive-action confirmation in the GUI: not verified; the CLI requires `--yes`.

Rule for this page: every button calls a registry command through `POST /api/run`; no button may show a result
without calling the command.

## Accessibility

- Not audited. Known gap: no documented focus-ring or contrast check.
- Controls are native `button`, `input`, `select`, `textarea`, so keyboard focus works by default.

## Icons

None. Text labels only.

## Content / Copy Rules

- Use the command vocabulary: project, source, clip, render, QC, export, unit.
- Show command names in mono where they appear (for example `render.run`).
- Reloading with a `#view` hash opens that view (commit d77e638; not yet re-checked in a browser).
- Demo data: `crates/rustler-gui/examples/demo.rs` is a development example; the page itself reads project data through the server.
