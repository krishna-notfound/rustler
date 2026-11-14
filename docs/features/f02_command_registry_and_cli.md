# f02 — Command Registry and CLI

**Status:** Verified  |  **Depends on:** f01  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

Every Rustler capability is declared once in a command registry. The `rustler` CLI is a generic,
schema-driven front over that registry: a command `group.action` is reached as `rustler group action`.
An agent can discover commands, read their schema, and call them with JSON output and documented exit codes.

## Scope

**In:**
- Registry: `Command` trait, `typed()` helper (typed JSON input/output), `CommandSpec`, annotations, examples.
- Output envelope: `{ok, schema: "rustler.<cmd>@1", data, warnings}`; errors `{ok: false, error: {code, stage, message, hint, retryable}}`.
- Exit codes: 0 ok, 2 usage/validation, 3 not found, 4 dependency missing, 5 provider, 6 media, 7 QC failed, 8 cancelled, 9 revision conflict, 10 internal.
- CLI: generic argument mapping (`--flag value` to JSON), `commands list`, `commands describe <name>`, `help`, global flags `--project <dir>`, `--pretty`, `--dry-run`, `--yes`, `--actor agent|human`, `--max-seconds N`, `-q`.
- Progress events on stderr (`ProgressSink`).

**Out (explicitly):**
- Long-running job API (`job.start/status/wait/cancel`) from spec §5.2 is not built; long commands run inline with progress.
- Provider configuration (`rustler config provider add`), dropped with f13.
- Exit code list in `docs/superpowers/specs/2026-11-15-rustler-design.md` §5.2 omits 9; the implemented list includes it.

## User-facing Behavior

- `rustler commands list` lists registered commands; `rustler commands describe <name>` prints schema and examples.
- `rustler <group>` with several actions and no action word returns the list for that group.
- Unknown command returns `not_found` with a "did you mean" hint built from close names.
- `rustler mcp serve` and `rustler gui` are intercepted in `main.rs` before the generic parser (f10, f24).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-registry/src/lib.rs`, `registry.rs`, `spec.rs`, `ctx.rs`, `envelope.rs`, `progress.rs`
- `crates/rustler-cli/src/main.rs`, `lib.rs`, `argmap.rs`, `output.rs`
- `crates/rustler-core/src/error.rs` (error codes and exit mapping)

**Read-only dependencies:**
- `crates/rustler-commands/src/lib.rs` — `register_all` is the single entry point the CLI calls.

## Data and Interfaces

- `trait Command { fn spec(&self) -> CommandSpec; async fn run(&self, ctx: &Ctx, input: Value) -> Result<Value>; }`
- `typed(name, description, required_args, annotations, examples, handler)` builds a `Command` from typed input/output.
- `Ctx { project_dir, home (RUSTLER_HOME), actor, dry_run, yes, progress, cancel }`.
- Envelope JSON: `ok`, `schema`, `data`, `warnings`; error object carries `code`, `stage`, `message`, `hint`, `retryable`.

## Rules and Edge Cases

1. Every command goes through the registry; CLI, MCP and GUI never call handlers directly.
2. Flag names map `-` to `_` (`--start-unit` becomes `start_unit`).
3. Input is validated against the schema before the handler runs (`typed_command_validates_input`).
4. A lone `*.run` command in a group runs without an action word (`rustler doctor`).
5. Stdout carries exactly one JSON envelope per invocation; progress and logs go to stderr.

## Tasks

- [x] Registry, spec, ctx, envelope, progress (`crates/rustler-registry`)
- [x] Generic CLI front and argument mapping (`crates/rustler-cli`)
- [x] `commands list` / `describe`, `help` built-ins

## Acceptance Criteria

- [x] Registry validates typed input and lists specs (`crates/rustler-registry/tests/registry.rs`)
- [x] Argument mapping is correct (`crates/rustler-cli/tests/argmap.rs`, 10 tests)
- [x] CLI end-to-end behaviour: envelope, exit codes, help (`crates/rustler-cli/tests/cli.rs`, 26 tests)
- [x] Full project flow via the real CLI passed (progress log, 2026-11-15)

## Verification

- `cargo test -p rustler-registry -p rustler-cli` (PowerShell).
- `rustler commands list` returns a non-empty list; `rustler commands describe clip.create` returns a schema.
- Pass = exit 0 and `ok: true` for read commands; a bad flag returns exit 2 with a hint.

## Status Log

- 2026-11-15 — Registry and CLI front merged (CLI tests 36 across `argmap.rs` and `cli.rs`); end-to-end run through the real CLI passed (project create to export clip).
- 2026-11-15 — Fix committed (d77e638): `--help` on `gui` or `mcp serve` returns the help envelope instead of starting a server (found when an agent's `rustler gui --help` hung).
- 2026-11-15 — A full workspace run had one outdated test that expected `mcp`/`gui` to say "not built yet"; it was replaced with a real `mcp serve` stdio test. CLI suite then passed 26/26. The progress log does not record a final full-workspace count after that fix.
