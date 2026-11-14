# f10 — MCP Server

**Status:** Verified  |  **Depends on:** f02  |  **Owner:** orchestrator + implementation agents (see `docs/progress-log.md`, 2026-11-15)

## Purpose

Agents that speak MCP (Claude Code, Claude Desktop and others) get the same capabilities as the CLI without
a shell. `rustler mcp serve` runs an MCP server over stdio. Its tools, prompts and resources are generated from
the same command registry and knowledge book, so a command added to the registry appears to MCP clients too.

## Scope

**In:**
- `rustler mcp serve`: MCP over stdio (`crates/rustler-mcp`).
- Tools: one per registry command, schema from `CommandSpec`; a help tool lists guides and prompts.
- Images returned as image content (for vision-capable agents), via `images.rs`.
- Prompts (`prompts/list`, `prompts/get`) and resources (`resources/list`, `resources/read`, templates) from the knowledge book (f11).
- `initialize` instructions for the client.
- Project state resource `rustler://project/state` when a project directory is set.

**Out (explicitly):**
- Loopback HTTP transport with bearer token and Host/Origin checks (spec §5.12). Only stdio is built.
- Network exposure of any kind.
- Any tool that is not a registry command.

## User-facing Behavior

- `rustler mcp serve` with no project set still serves the tools, guides and prompts; the project-state resource appears only when a project is set.
- Resource URIs: `rustler://guides/<id>`, `rustler://commands`, `rustler://project/state`.
- A stdio check as an agent client reported 55 tools, 8 prompts and 14 resources, and `prompts/get make_viral_clips` rendered (progress log).

## Routes / Files

**Owned files (this feature may create/edit only these):**
- `crates/rustler-mcp/src/lib.rs`, `images.rs`, `knowledge.rs`
- `crates/rustler-mcp/tests/mcp.rs`, `crates/rustler-mcp/tests/knowledge.rs`

**Read-only dependencies:**
- `crates/rustler-registry` — the command list and `CommandSpec` used to build tools (f02).
- `crates/rustler-knowledge` — guide and prompt content embedded in the binary (f11).
- `crates/rustler-cli/src/main.rs` — intercepts `mcp serve` and calls `rustler_mcp::serve_stdio` (shared with f02 and f24).

## Data and Interfaces

- Entry point: `rustler_mcp::serve_stdio(reg: Arc<Registry>, ctx: Ctx)`.
- Protocol methods exercised by tests: `initialize`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get`, `resources/list`, `resources/read`, and resource templates.
- Project directory from `--project <dir>`, `RUSTLER_PROJECT`, or the current directory when it contains `project.json`.

## Rules and Edge Cases

1. MCP tools are generated from the registry only; no handler is duplicated for MCP.
2. Image results are returned as image content, not as file paths only.
3. A `--project` flag before `gui` or `mcp` must not be read as the command name (bug found and fixed in real use; progress log).
4. The MCP server uses stdout for protocol messages; logs go to stderr.

## Tasks

- [x] Stdio server, tool generation from the registry (`crates/rustler-mcp/src/lib.rs`)
- [x] Image content (`images.rs`)
- [x] Prompts, resources and initialize instructions (`knowledge.rs`)

## Acceptance Criteria

- [x] MCP protocol tests (`crates/rustler-mcp/tests/mcp.rs`, 5 tests)
- [x] Prompt, resource and help-tool tests (`crates/rustler-mcp/tests/knowledge.rs`, 5 tests), including `initialize_advertises_prompts_resources_and_instructions` and `prompts_list_and_get`
- [x] CLI wiring for `mcp serve` (`crates/rustler-cli/tests/cli.rs`; `mcp serve` replaced the "not built yet" test)
- [x] Stdio check as an agent: 55 tools, 8 prompts, 14 resources (progress log)

## Verification

- `cargo test -p rustler-mcp` (PowerShell).
- Start `rustler mcp serve` and send `initialize` then `tools/list` over stdin; pass = a non-empty tool list and a valid `initialize` result.

## Status Log

- 2026-11-15 — MCP server merged (stdio, tools from registry, images). Prompts and resources merged after the knowledge loader. Checked over stdio like an agent: 55 tools, 8 prompts, 14 resources; `prompts/get make_viral_clips` renders. Loopback HTTP and the spec's hardening items are not built.
