//! Printing the single stdout envelope (compact or `--pretty`). (Task 8)
//!
//! Default output is compact single-line JSON (agents first). Pretty-printing is used for
//! `--pretty`, or when stdout is a terminal and `--json` was not given; it is still JSON.

use rustler_registry::Envelope;
use serde_json::Value;
use std::io::Write;

/// Whether to pretty-print: explicit `--pretty`, or a human terminal without `--json`.
pub fn want_pretty(pretty_flag: bool, json_flag: bool, stdout_is_tty: bool) -> bool {
    pretty_flag || (stdout_is_tty && !json_flag)
}

/// Serializes any JSON value (no trailing newline).
pub fn render_value(v: &Value, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    } else {
        v.to_string()
    }
}

/// Serializes an envelope (no trailing newline).
pub fn render_envelope(env: &Envelope, pretty: bool) -> String {
    render_value(&env.to_value(), pretty)
}

/// Writes `s` plus a newline to stdout, ignoring broken pipes.
pub fn print_stdout(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{s}");
    let _ = out.flush();
}
