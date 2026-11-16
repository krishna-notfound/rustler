//! Commands: config.get, config.set, config.unset, config.list (v2 Task 0.6).
//!
//! Agent settings: typed, validated defaults the agent changes once instead of repeating a flag on every
//! call. Two scopes: **global** in `<home>/config.json` (the tools home, `RUSTLER_HOME`) and **project** in
//! `<project>/config.json`; the project value wins. Commands read a setting through [`setting`] and an
//! explicit command flag always wins over both.
//!
//! File shape: `{"schema": "rustler.config@1", "settings": {"download.quality": "1080"}}`.

use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

const STAGE: &str = "config";
const SCHEMA: &str = "rustler.config@1";
const FILE: &str = "config.json";

/// How a key is validated.
#[derive(Clone, Copy)]
enum Kind {
    /// One of the listed strings.
    Enum(&'static [&'static str]),
    /// Integer in `lo..=hi`.
    Int(i64, i64),
    Bool,
    /// Non-empty string.
    Text,
}

struct KeyDef {
    key: &'static str,
    kind: Kind,
    /// Documented default (what a command uses when nothing is set); `None` = no default.
    default: Option<fn() -> Value>,
    doc: &'static str,
}

fn d_quality() -> Value {
    json!("720")
}
fn d_parallel() -> Value {
    json!(2)
}
fn d_render_quality() -> Value {
    json!("final")
}
fn d_true() -> Value {
    json!(true)
}
fn d_false() -> Value {
    json!(false)
}

const KEYS: &[KeyDef] = &[
    KeyDef {
        key: "download.quality",
        kind: Kind::Enum(&["480", "720", "1080", "best"]),
        default: Some(d_quality),
        doc: "Max video height for `source import <url>` when --quality is not given.",
    },
    KeyDef {
        key: "render.parallel",
        kind: Kind::Int(1, 4),
        default: Some(d_parallel),
        doc: "How many clips `render batch` renders at once when --parallel is not given.",
    },
    KeyDef {
        key: "render.quality",
        kind: Kind::Enum(&["draft", "final"]),
        default: Some(d_render_quality),
        doc: "Default render quality for render commands that take --quality (draft = fast preview).",
    },
    KeyDef {
        key: "captions.style",
        kind: Kind::Text,
        default: None,
        doc: "Default caption style preset for new clips (see `rustler caption list-styles`).",
    },
    KeyDef {
        key: "captions.words_per_line",
        kind: Kind::Int(1, 10),
        default: None,
        doc: "Default maximum words per caption line for new clips.",
    },
    KeyDef {
        key: "template.default",
        kind: Kind::Text,
        default: None,
        doc: "Name of the template applied by default when a command takes an optional template.",
    },
    KeyDef {
        key: "clip.smart_defaults",
        kind: Kind::Bool,
        default: Some(d_true),
        doc: "Let clip creation apply smart defaults (layout, framing); used from v0.3.0 on.",
    },
    KeyDef {
        key: "qc.faces",
        kind: Kind::Bool,
        default: Some(d_false),
        doc: "Run the engine face checks on every `qc run` (slower; starts the Python engine).",
    },
];

fn find_key(key: &str) -> Result<&'static KeyDef> {
    KEYS.iter().find(|k| k.key == key).ok_or_else(|| {
        RustlerError::new(ErrorCode::Validation, STAGE, format!("unknown setting `{key}`"))
            .hint(format!("valid keys: {}", KEYS.iter().map(|k| k.key).collect::<Vec<_>>().join(", ")))
    })
}

fn allowed(kind: Kind) -> String {
    match kind {
        Kind::Enum(v) => format!("one of {}", v.join(", ")),
        Kind::Int(lo, hi) => format!("an integer {lo}..{hi}"),
        Kind::Bool => "true or false".to_string(),
        Kind::Text => "a non-empty string".to_string(),
    }
}

/// Validates and normalizes `raw` for `def`. Strings are coerced (`"3"` -> 3, `"true"` -> true, `720` -> "720").
fn coerce(def: &KeyDef, raw: &Value) -> Result<Value> {
    let bad = || {
        RustlerError::new(ErrorCode::Validation, STAGE, format!("bad value {raw} for `{}`", def.key))
            .hint(format!("`{}` must be {}", def.key, allowed(def.kind)))
    };
    let as_text = || match raw {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    match def.kind {
        Kind::Enum(options) => {
            let t = as_text().ok_or_else(bad)?.to_ascii_lowercase();
            options.contains(&t.as_str()).then(|| json!(t)).ok_or_else(bad)
        }
        Kind::Int(lo, hi) => {
            let n = match raw {
                Value::Number(n) => n.as_i64().filter(|_| n.is_i64() || n.as_f64().is_some_and(|f| f.fract() == 0.0)),
                Value::String(s) => s.trim().parse::<i64>().ok(),
                _ => None,
            }
            .ok_or_else(bad)?;
            (lo..=hi).contains(&n).then(|| json!(n)).ok_or_else(bad)
        }
        Kind::Bool => match raw {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Ok(json!(true)),
                "false" | "no" | "off" | "0" => Ok(json!(false)),
                _ => Err(bad()),
            },
            _ => Err(bad()),
        },
        Kind::Text => {
            let t = as_text().ok_or_else(bad)?;
            if t.is_empty() { Err(bad()) } else { Ok(json!(t)) }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Global,
    Project,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Project => "project",
        }
    }
}

fn path_of(ctx: &Ctx, scope: Scope) -> Result<PathBuf> {
    match scope {
        Scope::Global => Ok(ctx.home.join(FILE)),
        Scope::Project => ctx.project_dir.as_ref().map(|d| d.join(FILE)).ok_or_else(|| {
            RustlerError::new(ErrorCode::NotFound, STAGE, "no project for the project scope")
                .hint("pass --project, or use --scope global")
        }),
    }
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, STAGE, format!("cannot {what} {}: {e}", path.display()))
}

/// The `settings` object of one scope's file (empty when the file does not exist).
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    if !path.is_file() {
        return Ok(Map::new());
    }
    let bytes = std::fs::read(path).map_err(|e| io_err("read", path, e))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, STAGE, format!("invalid JSON in {}: {e}", path.display()))
            .hint("fix or delete the file")
    })?;
    Ok(v.get("settings").and_then(Value::as_object).cloned().unwrap_or_default())
}

fn write_settings(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| io_err("create", dir, e))?;
    let body = serde_json::to_vec_pretty(&json!({ "schema": SCHEMA, "settings": settings }))
        .map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, e.to_string()))?;
    let tmp = dir.join(format!("{FILE}.tmp{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| io_err("write", &tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err("replace", path, e)
    })
}

/// `(value, scope)` of a key: project first, then global. `None` when neither sets it.
fn lookup(ctx: &Ctx, key: &str) -> Result<Option<(Value, Scope)>> {
    if ctx.project_dir.is_some() {
        if let Some(v) = read_settings(&path_of(ctx, Scope::Project)?)?.get(key) {
            return Ok(Some((v.clone(), Scope::Project)));
        }
    }
    Ok(read_settings(&path_of(ctx, Scope::Global)?)?.get(key).map(|v| (v.clone(), Scope::Global)))
}

/// The configured value of `key` (project wins over global), deserialized; `None` when unset, unreadable or
/// of the wrong type. Defaults are not applied: an explicit command flag wins, then this, then the
/// command's own default. Use [`setting_or_default`] to include the documented default.
pub fn setting<T: DeserializeOwned>(ctx: &Ctx, key: &str) -> Option<T> {
    match lookup(ctx, key) {
        Ok(Some((v, _))) => serde_json::from_value(v).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("config: ignoring unreadable settings: {}", e.message);
            None
        }
    }
}

/// [`setting`], falling back to the key's documented default.
pub fn setting_or_default<T: DeserializeOwned>(ctx: &Ctx, key: &str) -> Option<T> {
    setting(ctx, key).or_else(|| {
        KEYS.iter().find(|k| k.key == key).and_then(|k| k.default).and_then(|d| serde_json::from_value(d()).ok())
    })
}

fn describe(ctx: &Ctx, def: &KeyDef) -> Result<Value> {
    let found = lookup(ctx, def.key)?;
    let default = def.default.map(|d| d());
    let (value, source) = match &found {
        Some((v, s)) => (v.clone(), s.label()),
        None => (default.clone().unwrap_or(Value::Null), if default.is_some() { "default" } else { "unset" }),
    };
    Ok(json!({
        "key": def.key,
        "value": value,
        "source": source,
        "default": default,
        "allowed": allowed(def.kind),
        "doc": def.doc,
    }))
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "config.get",
        "Read one setting (or all when no key): the effective value, where it comes from (project, global, default or unset), the default, the allowed values and what it does. With --scope only that scope's own value is returned.",
        &["key"],
        Annotations { read_only: true, ..Annotations::default() },
        vec![
            Example { description: "what download quality is used?", cli: "rustler config get download.quality" },
            Example { description: "the global value only", cli: "rustler config get download.quality --scope global" },
        ],
        config_get,
    ));
    reg.register(typed(
        "config.set",
        "Set a setting (validated; `config list` shows the keys). Scope `project` (default when a project is open) writes <project>/config.json and wins over `global` (<home>/config.json). Explicit command flags always win over settings.",
        &["key", "value"],
        Annotations::default(),
        vec![
            Example { description: "download 1080p for every project", cli: "rustler config set download.quality 1080 --scope global" },
            Example { description: "render three clips at once in this project", cli: "rustler config set render.parallel 3" },
        ],
        config_set,
    ));
    reg.register(typed(
        "config.unset",
        "Remove a setting from a scope so the next scope (or the default) applies again.",
        &["key"],
        Annotations { destructive: true, ..Annotations::default() },
        vec![Example { description: "back to the default quality", cli: "rustler config unset download.quality --scope global" }],
        config_unset,
    ));
    reg.register(typed(
        "config.list",
        "List every setting with its effective value, source, default, allowed values and description, plus the two file paths.",
        &[],
        Annotations { read_only: true, ..Annotations::default() },
        vec![Example { description: "see all settings", cli: "rustler config list" }],
        config_list,
    ));
}

#[derive(Deserialize, JsonSchema)]
pub struct GetIn {
    /// Setting key, e.g. `download.quality`. Omit to get all settings.
    pub key: Option<String>,
    /// `global` or `project`: return only that scope's own value (no fallback, no default).
    pub scope: Option<Scope>,
}

async fn config_get(ctx: Ctx, i: GetIn) -> Result<Value> {
    let Some(key) = i.key else {
        return config_list(ctx, ListIn {}).await;
    };
    let def = find_key(&key)?;
    match i.scope {
        None => describe(&ctx, def),
        Some(scope) => {
            let value = read_settings(&path_of(&ctx, scope)?)?.get(def.key).cloned();
            Ok(json!({ "key": def.key, "scope": scope.label(), "value": value, "set": value.is_some(), "allowed": allowed(def.kind) }))
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SetIn {
    /// Setting key (see `config list`).
    pub key: String,
    /// The new value; strings like "3" or "true" are converted to the key's type.
    #[schemars(with = "serde_json::Value")]
    pub value: Value,
    /// `project` (default when a project is open) or `global`.
    pub scope: Option<Scope>,
}

async fn config_set(ctx: Ctx, i: SetIn) -> Result<Value> {
    let def = find_key(&i.key)?;
    let value = coerce(def, &i.value)?;
    let scope = i.scope.unwrap_or(if ctx.project_dir.is_some() { Scope::Project } else { Scope::Global });
    let path = path_of(&ctx, scope)?;
    let previous = {
        let mut s = read_settings(&path)?;
        let prev = s.insert(def.key.to_string(), value.clone());
        if !ctx.dry_run {
            write_settings(&path, &s)?;
        }
        prev
    };
    Ok(json!({
        "key": def.key, "value": value, "scope": scope.label(), "previous": previous,
        "file": path, "dry_run": ctx.dry_run, "effective": describe(&ctx, def)?,
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct UnsetIn {
    pub key: String,
    /// `project` (default when a project is open) or `global`.
    pub scope: Option<Scope>,
}

async fn config_unset(ctx: Ctx, i: UnsetIn) -> Result<Value> {
    let def = find_key(&i.key)?;
    let scope = i.scope.unwrap_or(if ctx.project_dir.is_some() { Scope::Project } else { Scope::Global });
    let path = path_of(&ctx, scope)?;
    let mut s = read_settings(&path)?;
    let removed = s.remove(def.key);
    if removed.is_some() && !ctx.dry_run {
        write_settings(&path, &s)?;
    }
    Ok(json!({
        "key": def.key, "scope": scope.label(), "removed": removed, "was_set": removed.is_some(),
        "file": path, "dry_run": ctx.dry_run, "effective": describe(&ctx, def)?,
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct ListIn {}

async fn config_list(ctx: Ctx, _i: ListIn) -> Result<Value> {
    let settings = KEYS.iter().map(|k| describe(&ctx, k)).collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "settings": settings,
        "files": {
            "global": path_of(&ctx, Scope::Global)?,
            "project": path_of(&ctx, Scope::Project).ok(),
        },
        "next": ["change one: rustler config set <key> <value> [--scope global|project]"],
    }))
}
