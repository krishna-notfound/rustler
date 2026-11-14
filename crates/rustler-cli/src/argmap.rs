//! Generic CLI flags/positionals -> command input JSON. (Task 8)

use rustler_registry::CommandSpec;

/// `--kebab-key` -> `snake_key`; values parsed as JSON when they parse (numbers, bools,
/// arrays, objects) else strings; flag without value -> `true`; repeated key -> array;
/// positionals assigned to `spec.positional` in order (extra -> `Usage`);
/// `--input-json` merged first, flags override.
pub fn args_to_json(
    spec: &CommandSpec,
    positional: &[String],
    flags: &[(String, Option<String>)],
) -> rustler_core::Result<serde_json::Value> {
    let _ = (spec, positional, flags);
    todo!("Task 8: args_to_json")
}
