//! MCP `prompts/*` and `resources/*` backed by `rustler-knowledge` (guides, workflow
//! prompts), the command registry (`rustler://commands`) and the project folder
//! (`rustler://project/state`).

use rustler_registry::Registry;
use serde_json::{Map, Value, json};
use std::path::Path;

pub const GUIDE_URI_PREFIX: &str = "rustler://guides/";
pub const COMMANDS_URI: &str = "rustler://commands";
pub const PROJECT_STATE_URI: &str = "rustler://project/state";

/// JSON-RPC error code used for unknown resources (MCP convention).
pub const RESOURCE_NOT_FOUND: i64 = -32002;
pub const INVALID_PARAMS: i64 = -32602;

/// `(code, message)` of a JSON-RPC error.
pub type RpcErr = (i64, String);

// ------------------------------------------------------------------- prompts

pub fn prompts_list() -> Value {
    let prompts: Vec<Value> = rustler_knowledge::prompts()
        .into_iter()
        .map(|p| {
            json!({
                "name": p.name,
                "title": p.title,
                "description": p.description,
                "arguments": p.arguments.iter().map(|a| json!({
                    "name": a.name,
                    "description": a.description,
                    "required": a.required,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"prompts": prompts})
}

/// `prompts/get`: renders the template; the text goes out as one user-role message.
pub fn prompts_get(params: &Value) -> Result<Value, RpcErr> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "prompts/get requires a string `name`".to_string()))?;
    let args: Map<String, Value> = match params.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => {
            return Err((INVALID_PARAMS, "`arguments` must be an object of strings".into()));
        }
    };
    let text = rustler_knowledge::render_prompt(name, &args).map_err(|e| {
        let mut msg = e.message.clone();
        if let Some(h) = &e.hint {
            msg.push_str(&format!(" ({h})"));
        }
        (INVALID_PARAMS, msg)
    })?;
    let description = rustler_knowledge::prompts()
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| p.description)
        .unwrap_or_default();
    Ok(json!({
        "description": description,
        "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
    }))
}

// ----------------------------------------------------------------- resources

fn project_state_available(project_dir: Option<&Path>) -> bool {
    project_dir.is_some_and(|d| d.join("project.json").is_file())
}

pub fn resources_list(project_dir: Option<&Path>) -> Value {
    let mut resources: Vec<Value> = rustler_knowledge::guides()
        .into_iter()
        .map(|g| {
            json!({
                "uri": format!("{GUIDE_URI_PREFIX}{}", g.id),
                "name": g.id,
                "title": g.title,
                "description": g.summary,
                "mimeType": "text/markdown",
            })
        })
        .collect();
    resources.push(json!({
        "uri": COMMANDS_URI,
        "name": "commands",
        "title": "Rustler command reference",
        "description": "JSON array of every command spec: name, summary, input/output JSON schema, annotations, CLI examples.",
        "mimeType": "application/json",
    }));
    if project_state_available(project_dir) {
        resources.push(json!({
            "uri": PROJECT_STATE_URI,
            "name": "project-state",
            "title": "Current project state",
            "description": "project.json plus a summary of every clip in the current project.",
            "mimeType": "application/json",
        }));
    }
    json!({"resources": resources})
}

pub fn resource_templates_list() -> Value {
    json!({"resourceTemplates": [{
        "uriTemplate": format!("{GUIDE_URI_PREFIX}{{id}}"),
        "name": "guide",
        "title": "Rustler guide",
        "description": "A guide from the Rustler knowledge book by id (see resources/list or the `guide_list` tool).",
        "mimeType": "text/markdown",
    }]})
}

pub fn resources_read(
    params: &Value,
    registry: &Registry,
    project_dir: Option<&Path>,
) -> Result<Value, RpcErr> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "resources/read requires a string `uri`".to_string()))?;
    let not_found = |why: String| (RESOURCE_NOT_FOUND, format!("resource not found: {uri} ({why})"));

    let (mime, text) = if let Some(id) = uri.strip_prefix(GUIDE_URI_PREFIX) {
        let g = rustler_knowledge::guide(id).ok_or_else(|| {
            let ids: Vec<String> = rustler_knowledge::guides().into_iter().map(|g| g.id).collect();
            not_found(format!("available guides: {}", ids.join(", ")))
        })?;
        ("text/markdown", g.body)
    } else if uri == COMMANDS_URI {
        let specs = registry.list();
        (
            "application/json",
            serde_json::to_string_pretty(&specs).map_err(|e| (-32603, e.to_string()))?,
        )
    } else if uri == PROJECT_STATE_URI {
        let dir = project_dir
            .ok_or_else(|| not_found("no project directory is set for this server".into()))?;
        let state = project_state(dir).map_err(not_found)?;
        (
            "application/json",
            serde_json::to_string_pretty(&state).map_err(|e| (-32603, e.to_string()))?,
        )
    } else {
        return Err(not_found("unknown URI".into()));
    };

    Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
}

/// project.json verbatim + a summary per `clips/*/clip.json`.
fn project_state(dir: &Path) -> Result<Value, String> {
    let pj = dir.join("project.json");
    let text = std::fs::read_to_string(&pj)
        .map_err(|e| format!("cannot read {}: {e}; run project_create first", pj.display()))?;
    let project: Value =
        serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", pj.display()))?;

    let mut clips: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir.join("clips")) {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            let Ok(t) = std::fs::read_to_string(d.join("clip.json")) else {
                continue;
            };
            let Ok(c) = serde_json::from_str::<Value>(&t) else {
                continue;
            };
            let count = |k: &str| c.get(k).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            clips.push(json!({
                "id": c.get("id"),
                "name": c.get("name"),
                "revision": c.get("revision"),
                "segments": count("segments"),
                "renders": count("renders"),
                "last_render": c.get("renders").and_then(Value::as_array).and_then(|r| r.last()),
            }));
        }
    }
    Ok(json!({"project": project, "clips": clips}))
}
