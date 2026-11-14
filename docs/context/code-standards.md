# Code Standards

## General

- Keep modules small and single-purpose; one crate per responsibility (see `architecture-context.md`).
- Fix root causes — do not layer workarounds.
- Respect the boundaries in `architecture-context.md`. Commands glue; domain logic lives in its crate.
- No dead code, no commented-out code, no TODOs for core functionality.
- Match the style, naming, and comment density of the surrounding code.
- Frozen contracts (`rustler-core` models, ids, errors, events; `rustler-registry`) change only with a recorded decision.

## Language: Rust (edition 2024)

- Errors: return `RustlerError` (`crates/rustler-core/src/error.rs`) with code, stage, message and hint. The
  error code decides the exit code (see `docs/features/f02_command_registry_and_cli.md`).
- Input from outside (CLI args, JSON input, engine responses, URLs) is validated against the command's schema.
- Use the registry `typed()` helper for commands; do not hand-roll JSON parsing in a command.
- Keep public signatures of frozen contracts unchanged; fill bodies, not signatures.

## Language: Python (engine)

- Python 3.11 to 3.12 (`engine/pyproject.toml`); managed by uv.
- The JSON-RPC contract is `engine/PROTOCOL.md`. Change the protocol document first, then both sides.
- Engine methods return typed dicts documented in `PROTOCOL.md`; no undocumented fields.

## Subprocess tools (FFmpeg, ffprobe, yt-dlp, uv)

- Call them through the subprocess helpers (`crates/rustler-tools/src/run.rs`, `crates/rustler-media/src/proc.rs`).
- Never build FFmpeg filtergraphs in a command; the render compiler (`compile.rs`) owns them.
- Paths with spaces, brackets and Devanagari must work (tested in render and probe tests).

## Styling (GUI)

- Colors, radius and spacing come from the variables in `ui-context.md` (`style.css` `:root`).
- No hardcoded colors in `app.js`.

## Data and State

- All project writes go through `ProjectStore` (`crates/rustler-core/src/store.rs`); no command writes project JSON directly.
- Writes are atomic: temp file, then rename.
- Every clip change that matters for undo writes a history snapshot (`history/<rev>.clip.json`).
- The GUI keeps no server-side state beyond the token; project state is read through commands.

## Naming and File Organization

- Name files after the responsibility they contain (`probe.rs`, `qc.rs`, `facepath.rs`), not the technology.
- Command groups map to files: `source_cmds.rs`, `analyze_cmds.rs`, `see_cmds.rs`, `clip_cmds.rs`, `render_cmds.rs`, `qc_cmds.rs`, `edit_cmds.rs`, `guide_cmds.rs`, `setup_cmds.rs`, `project_cmds.rs`, `transcript_cmds.rs`.
- Command names are `group.action` in snake_case; the CLI maps `-` to `_`.
- Engine test files are `engine/tests/test_<area>.py`; Rust tests are `crates/<crate>/tests/<area>.rs`.

## Testing

- Rust: `cargo test --workspace`. Engine: `~/.rustler/bin/uv run --project engine pytest -q`.
- Tests needing ffmpeg, yt-dlp, uv or the real engine skip with a printed reason when the tool is missing.
- Live and network tests are `#[ignore]`-gated and are run on purpose, not on every test run.
- Tests that touch the user's home write to a temp `RUSTLER_HOME`.
- A feature is not done until its tests pass **and** the result was observed, not assumed. Record the run in
  `docs/progress-log.md`.

## Definition of Done (per unit of work)

1. Works end to end within the feature file's scope.
2. `cargo build --workspace` and `cargo test --workspace` pass (real output seen). Engine tests pass when the engine changed. Lint is not configured.
3. No invariant in `architecture-context.md` violated.
4. Feature file *Status Log* and `docs/features/README.md` updated; dated entry in `docs/progress-log.md`.
