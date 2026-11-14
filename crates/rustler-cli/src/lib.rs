//! Library half of the `rustler` CLI (so integration tests can reach `args_to_json` and
//! `run_cli`). The CLI is a generic, schema-driven front over the registry: no feature logic.
//!
//! Grammar (frozen, see the slice-1 plan, Task 8):
//!
//! ```text
//! rustler <group> <action> [positional...] [--key value]... [--input-json <json | @file | ->]
//! rustler <group>                      list the group's commands
//! rustler <group> --help               same
//! rustler <group> <action> --help      same as `commands describe <group>.<action>`
//! rustler commands list [group] | commands describe <name> | help | --version
//! globals (anywhere): --project <dir> --pretty --json --dry-run --yes --actor agent|human
//!                     --max-seconds N -q
//! ```
//!
//! stdout carries exactly one JSON envelope per invocation; progress/logs go to stderr.

pub mod argmap;
pub mod output;

use argmap::{args_to_json, flag_name, is_bool_flag, known_properties, normalize_key, prop_schema, schema_types};
use rustler_core::{Actor, ErrorCode, RustlerError};
use rustler_registry::{Annotations, CommandSpec, Ctx, Envelope, ProgressSink, Registry};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// "Start here" text returned by `rustler help`.
pub const START: &str = "rustler doctor → rustler setup → rustler project create --name X → rustler source import <file|url> → rustler analyze quick <source-id> → rustler transcript get <source-id> → rustler see tiles … → rustler clip create … → rustler render run <clip-id> → rustler qc run <clip-id>";

/// Options for [`run_cli_with`].
#[derive(Clone)]
pub struct CliOptions {
    /// stdout is a terminal: still JSON, but pretty-printed (unless `--json`).
    pub tty: bool,
    /// Cancelling this token (the binary does it on Ctrl-C) cancels the running command.
    pub cancel: CancellationToken,
    /// How long a cancelled command gets to wind down before the CLI answers for it.
    pub grace: Duration,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            tty: false,
            cancel: CancellationToken::new(),
            grace: Duration::from_secs(5),
        }
    }
}

/// Installs the stderr logger: level from `RUSTLER_LOG` (default `warn`). Safe to call twice.
pub fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("RUSTLER_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}

/// Runs one CLI invocation. `args` excludes the program name. Returns `(stdout, exit_code)`;
/// `stdout` has no trailing newline and is empty only for commands that print their own output.
pub async fn run_cli(args: Vec<String>, reg: Registry) -> (String, i32) {
    run_cli_with(args, reg, CliOptions::default()).await
}

/// [`run_cli`] with explicit options (TTY pretty-printing, cancellation token).
pub async fn run_cli_with(args: Vec<String>, reg: Registry, opts: CliOptions) -> (String, i32) {
    init_logging();
    let (globals, reply) = match extract_globals(&args) {
        Ok((g, rest)) => {
            let reply = execute(&g, rest, &reg, &opts).await;
            (g, reply)
        }
        Err(e) => (Globals::default(), Reply::Env(Envelope::err(e))),
    };
    let pretty = output::want_pretty(globals.pretty, globals.json, opts.tty);
    match reply {
        Reply::Env(env) => (output::render_envelope(&env, pretty), env.exit_code()),
        Reply::Raw(v) => (output::render_value(&v, pretty), 0),
        Reply::Silent => (String::new(), 0),
    }
}

// ---------------------------------------------------------------------------------------
// Global flags
// ---------------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Globals {
    project: Option<PathBuf>,
    pretty: bool,
    json: bool,
    dry_run: bool,
    yes: bool,
    actor: Actor,
    max_seconds: Option<u64>,
    quiet: bool,
    help: bool,
    version: bool,
}

enum Reply {
    Env(Envelope),
    /// A bare JSON value (only `--version`).
    Raw(Value),
    /// The built-in printed its own output.
    #[allow(dead_code)]
    Silent,
}

fn usage(message: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Usage, "cli", message)
}

fn parse_bool(inline: &Option<String>) -> bool {
    !matches!(inline.as_deref(), Some("false" | "0" | "no"))
}

/// Pulls the global flags out of `args` (anywhere), returning the remaining tokens in order.
fn extract_globals(args: &[String]) -> rustler_core::Result<(Globals, Vec<String>)> {
    let mut g = Globals::default();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            rest.extend(args[i..].iter().cloned());
            break;
        }
        match a {
            "-q" => {
                g.quiet = true;
                i += 1;
                continue;
            }
            "-h" => {
                g.help = true;
                i += 1;
                continue;
            }
            "-V" => {
                g.version = true;
                i += 1;
                continue;
            }
            "-y" => {
                g.yes = true;
                i += 1;
                continue;
            }
            _ => {}
        }
        let Some(body) = a.strip_prefix("--") else {
            rest.push(args[i].clone());
            i += 1;
            continue;
        };
        let (key, inline) = match body.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (body, None),
        };
        match key {
            "project" | "actor" | "max-seconds" => {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        match args.get(i) {
                            Some(v) if !v.starts_with("--") => v.clone(),
                            _ => {
                                return Err(usage(format!("--{key} needs a value"))
                                    .hint(match key {
                                        "project" => "e.g. --project ./my-project",
                                        "actor" => "e.g. --actor agent (or human)",
                                        _ => "e.g. --max-seconds 120",
                                    }));
                            }
                        }
                    }
                };
                match key {
                    "project" => g.project = Some(PathBuf::from(value)),
                    "actor" => {
                        g.actor = match value.as_str() {
                            "agent" => Actor::Agent,
                            "human" => Actor::Human,
                            other => {
                                return Err(usage(format!("invalid --actor `{other}`"))
                                    .hint("use --actor agent or --actor human"));
                            }
                        }
                    }
                    _ => {
                        g.max_seconds = Some(value.parse::<u64>().map_err(|_| {
                            usage(format!("--max-seconds needs a whole number, got `{value}`"))
                                .hint("e.g. --max-seconds 120")
                        })?)
                    }
                }
            }
            "pretty" => g.pretty = parse_bool(&inline),
            "json" => g.json = parse_bool(&inline),
            "dry-run" => g.dry_run = parse_bool(&inline),
            "yes" => g.yes = parse_bool(&inline),
            "quiet" => g.quiet = parse_bool(&inline),
            "help" => g.help = parse_bool(&inline),
            "version" => g.version = parse_bool(&inline),
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    Ok((g, rest))
}

// ---------------------------------------------------------------------------------------
// Built-in specs and JSON views
// ---------------------------------------------------------------------------------------

fn builtin_specs() -> Vec<CommandSpec> {
    let ro = Annotations {
        read_only: true,
        ..Annotations::default()
    };
    vec![
        CommandSpec {
            name: "commands.list",
            summary: "List every command (name, summary, annotations); optional group filter",
            positional: &["group"],
            input_schema: json!({"type":"object","properties":{"group":{"type":["string","null"],"description":"only commands of this group, e.g. `source`"}}}),
            output_schema: json!({"type":"object","properties":{"commands":{"type":"array"}}}),
            annotations: ro,
            examples: vec![],
        },
        CommandSpec {
            name: "commands.describe",
            summary: "Full description of one command: input/output JSON schemas, flags, examples",
            positional: &["name"],
            input_schema: json!({"type":"object","properties":{"name":{"type":"string","description":"e.g. `source.import` or `source import`"}},"required":["name"]}),
            output_schema: json!({"type":"object"}),
            annotations: ro,
            examples: vec![],
        },
        CommandSpec {
            name: "help",
            summary: "Start-here workflow, global flags, exit codes and the command list",
            positional: &[],
            input_schema: json!({"type":"object","properties":{}}),
            output_schema: json!({"type":"object"}),
            annotations: ro,
            examples: vec![],
        },
    ]
}

fn cli_form(name: &str) -> String {
    format!("rustler {}", name.replace('.', " ").replace('_', "-"))
}

fn group_of(name: &str) -> &str {
    name.split_once('.').map(|(g, _)| g).unwrap_or(name)
}

fn action_of(name: &str) -> &str {
    name.split_once('.').map(|(_, a)| a).unwrap_or("")
}

fn list_entry(s: &CommandSpec) -> Value {
    json!({
        "name": s.name,
        "summary": s.summary,
        "annotations": s.annotations,
        "cli": cli_form(s.name),
    })
}

fn type_label(schema: &Value) -> String {
    let mut types = schema_types(schema);
    types.retain(|t| t != "null");
    types.dedup();
    if types.is_empty() {
        "any".to_string()
    } else {
        types.join("|")
    }
}

/// Flag table derived from the input schema.
fn flag_table(spec: &CommandSpec) -> Vec<Value> {
    let Some(props) = spec.input_schema.get("properties").and_then(Value::as_object) else {
        return vec![];
    };
    let required: Vec<&str> = spec
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    props
        .iter()
        .map(|(k, s)| {
            json!({
                "field": k,
                "flag": flag_name(k),
                "type": type_label(s),
                "required": required.contains(&k.as_str()),
                "positional": spec.positional.contains(&k.as_str()),
                "description": s.get("description").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn usage_line(spec: &CommandSpec) -> String {
    let mut line = cli_form(spec.name);
    for p in spec.positional {
        line.push_str(&format!(" <{p}>"));
    }
    for f in flag_table(spec) {
        let is_pos = f["positional"].as_bool().unwrap_or(false);
        if f["required"].as_bool().unwrap_or(false) && !is_pos {
            line.push_str(&format!(" {} <{}>", f["flag"].as_str().unwrap_or(""), f["type"].as_str().unwrap_or("value")));
        }
    }
    line.push_str(" [--flag value]... [--input-json <json|@file>]");
    line
}

fn describe_value(spec: &CommandSpec) -> Value {
    let mut v = serde_json::to_value(spec).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("cli".into(), json!(cli_form(spec.name)));
        m.insert("usage".into(), json!(usage_line(spec)));
        m.insert("flags".into(), Value::Array(flag_table(spec)));
        m.insert(
            "global_flags".into(),
            json!(["--project <dir>", "--pretty", "--dry-run", "--yes", "--actor agent|human", "--max-seconds N", "-q"]),
        );
    }
    v
}

fn global_flags_doc() -> Value {
    json!([
        {"flag": "--project <dir>", "meaning": "project folder (else env RUSTLER_PROJECT, else cwd if it holds project.json)"},
        {"flag": "--pretty", "meaning": "pretty-print the JSON envelope (default: one compact line; pretty automatically on a terminal unless --json)"},
        {"flag": "--json", "meaning": "force compact JSON even on a terminal"},
        {"flag": "--dry-run", "meaning": "describe what would change without changing it (command permitting)"},
        {"flag": "--yes", "meaning": "confirm destructive actions; Rustler never prompts"},
        {"flag": "--actor agent|human", "meaning": "who is acting, recorded in the project event log (default agent)"},
        {"flag": "--max-seconds N", "meaning": "time budget for long-running commands"},
        {"flag": "-q", "meaning": "silence stderr progress lines"},
        {"flag": "--input-json <json|@file|->", "meaning": "command input as JSON; explicit --flags override it"},
        {"flag": "--help", "meaning": "`rustler <group> --help` lists a group; `rustler <group> <action> --help` describes a command"},
        {"flag": "--version", "meaning": "print {\"version\":..}"}
    ])
}

fn exit_codes_doc() -> Value {
    json!({
        "0": "ok", "2": "usage/validation", "3": "not found", "4": "dependency missing",
        "5": "provider/network error", "6": "media/ffmpeg error", "7": "QC failed",
        "8": "cancelled", "9": "revision conflict", "10": "internal"
    })
}

// ---------------------------------------------------------------------------------------
// Resolution and suggestions
// ---------------------------------------------------------------------------------------

/// Up to `n` closest names to `candidate`.
fn closest(candidate: &str, names: &[String], n: usize) -> Vec<String> {
    let mut scored: Vec<(f64, &String)> = names
        .iter()
        .map(|nm| (strsim::normalized_levenshtein(candidate, nm), nm))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(b.1)));
    scored.into_iter().take(n).map(|(_, nm)| nm.clone()).collect()
}

fn all_names(specs: &[CommandSpec]) -> Vec<String> {
    let mut names: Vec<String> = specs.iter().map(|s| s.name.to_string()).collect();
    names.extend(builtin_specs().iter().map(|s| s.name.to_string()));
    names
}

fn suggestion_hint(near: &[String]) -> String {
    if near.is_empty() {
        return "run `rustler commands list` to see available commands".into();
    }
    let parts: Vec<String> = near.iter().map(|n| format!("{n} ({})", cli_form(n))).collect();
    format!(
        "closest commands: {}. Run `rustler commands list` to see all of them",
        parts.join(", ")
    )
}

fn not_found_command(specs: &[CommandSpec], candidate: &str) -> RustlerError {
    let near = closest(candidate, &all_names(specs), 3);
    RustlerError::new(ErrorCode::NotFound, "cli", format!("unknown command `{candidate}`"))
        .hint(suggestion_hint(&near))
}

fn unknown_flag_error(spec: &CommandSpec, key: &str, known: &[String]) -> RustlerError {
    let near = closest(key, known, 3);
    let valid: Vec<String> = known.iter().map(|k| flag_name(k)).collect();
    let did_you_mean = near
        .first()
        .map(|n| format!("did you mean {}? ", flag_name(n)))
        .unwrap_or_default();
    usage(format!("unknown flag {} for `{}`", flag_name(key), spec.name)).hint(format!(
        "{did_you_mean}valid flags: {}; see `rustler commands describe {}`",
        valid.join(" "),
        spec.name
    ))
}

// ---------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------

fn help_envelope(specs: &[CommandSpec]) -> Envelope {
    let commands: Vec<Value> = specs.iter().map(list_entry).collect();
    Envelope::ok(
        "help",
        json!({
            "start": START,
            "grammar": "rustler <group> <action> [positional...] [--key value]... [--input-json <json|@file>]",
            "discover": ["rustler commands list", "rustler commands describe <group.action>", "rustler <group>", "rustler <group> <action> --help"],
            "global_flags": global_flags_doc(),
            "exit_codes": exit_codes_doc(),
            "output": "stdout: exactly one JSON envelope {ok,schema,data,warnings} or {ok:false,error:{code,stage,message,hint,retryable}}; stderr: progress NDJSON and logs",
            "commands": commands,
        }),
    )
}

fn list_envelope(specs: &[CommandSpec], group: Option<&str>) -> Envelope {
    let commands: Vec<Value> = specs
        .iter()
        .filter(|s| group.is_none_or(|g| group_of(s.name) == g))
        .map(list_entry)
        .collect();
    let mut data = json!({ "commands": commands });
    if let Some(g) = group {
        data["group"] = json!(g);
        data["next"] = json!(format!("rustler {g} <action> --help"));
    }
    Envelope::ok("commands.list", data)
}

fn describe_envelope(spec: &CommandSpec) -> Envelope {
    Envelope::ok("commands.describe", describe_value(spec))
}

fn resolve_home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()));
    match base {
        Some(b) => PathBuf::from(b).join(".rustler"),
        None => PathBuf::from(".rustler"),
    }
}

fn resolve_project(g: &Globals) -> Option<PathBuf> {
    if let Some(p) = &g.project {
        return Some(p.clone());
    }
    if let Some(p) = std::env::var_os("RUSTLER_PROJECT").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let cwd = std::env::current_dir().ok()?;
    cwd.join("project.json").is_file().then_some(cwd)
}

/// Splits tokens into positionals and `(key, value)` flags, using the spec to know which
/// flags are booleans (so `--loud file.mp4` does not swallow the file).
fn split_args(
    spec: &CommandSpec,
    rest: &[String],
) -> rustler_core::Result<(Vec<String>, Vec<(String, Option<String>)>)> {
    let mut pos = Vec::new();
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let t = rest[i].as_str();
        if t == "--" {
            pos.extend(rest[i + 1..].iter().cloned());
            break;
        }
        if let Some(body) = t.strip_prefix("--") {
            if let Some((k, v)) = body.split_once('=') {
                flags.push((k.to_string(), Some(v.to_string())));
                i += 1;
                continue;
            }
            let value = if is_bool_flag(spec, body) {
                match rest.get(i + 1).map(String::as_str) {
                    Some("true" | "false") => {
                        i += 1;
                        Some(rest[i].clone())
                    }
                    _ => None,
                }
            } else {
                match rest.get(i + 1) {
                    Some(n) if !n.starts_with("--") => {
                        i += 1;
                        Some(n.clone())
                    }
                    _ => None,
                }
            };
            if value.is_none() {
                let nk = normalize_key(body);
                if nk != "input_json" {
                    if let Some(s) = prop_schema(spec, &nk) {
                        if !schema_types(s).iter().any(|t| t == "boolean") {
                            return Err(usage(format!("flag --{body} needs a value"))
                                .hint(format!(
                                    "e.g. {} <{}>; see `rustler commands describe {}`",
                                    flag_name(&nk),
                                    type_label(s),
                                    spec.name
                                )));
                        }
                    }
                }
            }
            flags.push((body.to_string(), value));
        } else if t.starts_with('-') && t.len() > 1 && t.parse::<f64>().is_err() {
            return Err(usage(format!("unknown short flag `{t}`"))
                .hint("only -q, -h, -V and -y exist; everything else is a --long-flag"));
        } else {
            pos.push(t.to_string());
        }
        i += 1;
    }
    Ok((pos, flags))
}

fn cancelled_error(extra: Option<&RustlerError>) -> RustlerError {
    let msg = match extra {
        Some(e) => format!("cancelled (Ctrl-C); the command reported: {}", e.message),
        None => "cancelled (Ctrl-C)".to_string(),
    };
    RustlerError::new(
        ErrorCode::Cancelled,
        extra.map(|e| e.stage.as_str()).unwrap_or("cli"),
        msg,
    )
    .hint("re-run the command; partial results already saved in the project are kept")
    .retryable()
}

async fn run_command(
    reg: &Registry,
    spec: &CommandSpec,
    rest: &[String],
    g: &Globals,
    opts: &CliOptions,
) -> Envelope {
    let (pos, flags) = match split_args(spec, rest) {
        Ok(x) => x,
        Err(e) => return Envelope::err(e),
    };
    let mut input = match args_to_json(spec, &pos, &flags) {
        Ok(v) => v,
        Err(e) => return Envelope::err(e),
    };
    // Typos must not be silently dropped by serde.
    if let (Some(known), Value::Object(m)) = (known_properties(spec), &input) {
        for k in m.keys() {
            if !known.contains(k) {
                return Envelope::err(unknown_flag_error(spec, k, &known));
            }
        }
    }
    // `--max-seconds` is global, but also feeds a same-named input field.
    if let (Some(ms), Value::Object(m)) = (g.max_seconds, &mut input) {
        if prop_schema(spec, "max_seconds").is_some() && !m.contains_key("max_seconds") {
            m.insert("max_seconds".into(), json!(ms));
        }
    }

    let mut ctx = Ctx::new(resolve_project(g), resolve_home());
    ctx.actor = g.actor;
    ctx.dry_run = g.dry_run;
    ctx.yes = g.yes;
    ctx.max_seconds = g.max_seconds;
    ctx.progress = if g.quiet { ProgressSink::noop() } else { ProgressSink::stderr_ndjson() };
    ctx.cancel = opts.cancel.clone();

    let name = spec.name;
    let fut = reg.dispatch(name, input, ctx);
    tokio::pin!(fut);
    let result = tokio::select! {
        r = &mut fut => r,
        _ = opts.cancel.cancelled() => {
            match tokio::time::timeout(opts.grace, &mut fut).await {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(e)) if e.code == ErrorCode::Cancelled => Err(e),
                Ok(Err(e)) => Err(cancelled_error(Some(&e))),
                Err(_) => Err(cancelled_error(None)),
            }
        }
    };
    match result {
        Ok(v) => Envelope::ok(name, v),
        Err(e) => Envelope::err(e),
    }
}

async fn execute(g: &Globals, rest: Vec<String>, reg: &Registry, opts: &CliOptions) -> Reply {
    let specs = reg.list();
    let mut rest = rest;

    let Some(gi) = rest.iter().position(|t| !t.starts_with('-')) else {
        if g.version {
            return Reply::Raw(json!({ "version": env!("CARGO_PKG_VERSION") }));
        }
        if rest.is_empty() {
            return Reply::Env(help_envelope(&specs));
        }
        return Reply::Env(Envelope::err(
            usage(format!("no command given (found only `{}`)", rest.join(" ")))
                .hint("run `rustler help` or `rustler commands list`"),
        ));
    };
    let group = rest.remove(gi);
    let action: Option<String> = rest
        .iter()
        .position(|t| !t.starts_with('-'))
        .map(|ai| rest.remove(ai));

    // ---- reserved built-ins -------------------------------------------------------------
    match group.as_str() {
        "help" => return Reply::Env(help_envelope(&specs)),
        // `mcp serve` and `gui` run as servers from the binary (main.rs); here we only answer
        // `--help` (or a mistyped action) for them.
        "mcp" | "gui" => {
            let info = if group == "mcp" {
                json!({
                    "command": "rustler mcp serve",
                    "summary": "Run the MCP server on stdio: every registry command as a tool, knowledge guides as resources, workflows as prompts.",
                    "flags": {"--project <dir>": "default project for tool calls (tools also accept a `project` argument)"},
                    "example_config": {"command": "rustler", "args": ["mcp", "serve"]}
                })
            } else {
                json!({
                    "command": "rustler gui",
                    "summary": "Start the local web GUI (127.0.0.1, token in the URL) over the same commands as the CLI.",
                    "flags": {"--project <dir>": "project to open", "--port <n>": "fixed port (default random)", "--no-open": "don't open a browser"}
                })
            };
            if group == "mcp" && action.as_deref().is_some_and(|a| a != "serve") {
                return Reply::Env(Envelope::err(
                    usage(format!("unknown command `mcp {}`", action.unwrap_or_default())).hint("did you mean `rustler mcp serve`?"),
                ));
            }
            return Reply::Env(Envelope::ok(&format!("{group}.help"), info));
        }
        "commands" => {
            return Reply::Env(commands_builtin(&specs, action.as_deref(), &rest, g));
        }
        _ => {}
    }

    // ---- registry commands --------------------------------------------------------------
    let in_group: Vec<CommandSpec> = specs
        .iter()
        .filter(|s| group_of(s.name) == group)
        .cloned()
        .collect();
    if in_group.is_empty() {
        let candidate = match &action {
            Some(a) => format!("{group}.{}", a.replace('-', "_")),
            None => group.clone(),
        };
        return Reply::Env(Envelope::err(not_found_command(&specs, &candidate)));
    }

    let spec = match &action {
        Some(a) => {
            let full = format!("{group}.{}", a.replace('-', "_"));
            match in_group.iter().find(|s| s.name == full) {
                Some(s) => s.clone(),
                None => {
                    let near = closest(&full, &all_names(&specs), 3);
                    let acts: Vec<&str> = in_group.iter().map(|s| action_of(s.name)).collect();
                    let mut hint = suggestion_hint(&near);
                    hint.push_str(&format!("; `{group}` actions: {}", acts.join(", ")));
                    return Reply::Env(Envelope::err(
                        RustlerError::new(ErrorCode::NotFound, "cli", format!("unknown command `{full}`")).hint(hint),
                    ));
                }
            }
        }
        None => {
            // `rustler doctor` with a lone `doctor.run` just runs it.
            if in_group.len() == 1 && action_of(in_group[0].name) == "run" && !g.help {
                in_group[0].clone()
            } else {
                return Reply::Env(list_envelope(&specs, Some(&group)));
            }
        }
    };

    if g.help {
        return Reply::Env(describe_envelope(&spec));
    }
    Reply::Env(run_command(reg, &spec, &rest, g, opts).await)
}

fn commands_builtin(specs: &[CommandSpec], action: Option<&str>, rest: &[String], g: &Globals) -> Envelope {
    let builtins = builtin_specs();
    let pos: Vec<&String> = rest.iter().filter(|t| !t.starts_with('-')).collect();
    match action {
        None => list_envelope(specs, None),
        Some("list") => {
            if g.help {
                return describe_envelope(&builtins[0]);
            }
            list_envelope(specs, pos.first().map(|s| s.as_str()))
        }
        Some("describe") => {
            if g.help {
                return describe_envelope(&builtins[1]);
            }
            if pos.is_empty() {
                return Envelope::err(
                    usage("`commands describe` needs a command name")
                        .hint("e.g. `rustler commands describe source.import`; list them with `rustler commands list`"),
                );
            }
            let name = if pos.len() == 1 {
                pos[0].replace('-', "_")
            } else {
                format!("{}.{}", pos[0], pos[1].replace('-', "_"))
            };
            if let Some(s) = specs.iter().find(|s| s.name == name) {
                return describe_envelope(s);
            }
            if let Some(s) = builtins.iter().find(|s| s.name == name) {
                return describe_envelope(s);
            }
            Envelope::err(not_found_command(specs, &name))
        }
        Some(other) => Envelope::err(
            RustlerError::new(ErrorCode::NotFound, "cli", format!("unknown command `commands {other}`"))
                .hint("did you mean `commands list` or `commands describe <name>`?"),
        ),
    }
}
