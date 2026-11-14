//! Commands: guide.list, guide.show, guide.search, prompt.list, prompt.show.
//!
//! The agent-facing "knowledge book": markdown guides and prompt templates embedded from
//! `knowledge/` by `rustler-knowledge`. All read-only; none need a project.

use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Command groups that exist; used to recognize command names in guide text.
const COMMAND_GROUPS: &[&str] = &[
    "setup", "doctor", "project", "source", "analyze", "transcript", "see", "clip", "render",
    "qc", "export", "guide", "prompt", "edit", "history", "crop", "audio", "models",
];

/// Things that look like `group.action` but are file names.
const FILE_EXTS: &[&str] = &[
    "json", "ndjson", "md", "mp4", "mp3", "wav", "jpg", "jpeg", "png", "txt", "toml", "yaml",
    "yml", "ass", "srt", "vtt", "py",
];

#[derive(Deserialize, JsonSchema)]
struct NoInput {}

#[derive(Serialize, JsonSchema)]
struct GuideSummary {
    id: String,
    title: String,
    summary: String,
    tags: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
struct GuideListOut {
    guides: Vec<GuideSummary>,
    /// Suggested first read.
    start_with: String,
}

#[derive(Deserialize, JsonSchema)]
struct GuideShowIn {
    /// Guide id, e.g. `start-here` (see `guide.list`).
    id: String,
}

#[derive(Serialize, JsonSchema)]
struct GuideShowOut {
    id: String,
    title: String,
    summary: String,
    tags: Vec<String>,
    /// Full markdown body.
    markdown: String,
    /// Command names (`group.action`) mentioned in backticks in the guide.
    related_commands: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GuideSearchIn {
    /// Words to look for (all must appear in a guide).
    query: String,
}

#[derive(Serialize, JsonSchema)]
struct GuideHit {
    id: String,
    title: String,
    summary: String,
    /// Matching lines of the guide body (at most 8).
    matches: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
struct GuideSearchOut {
    query: String,
    hits: Vec<GuideHit>,
}

#[derive(Serialize, JsonSchema)]
struct PromptArgInfo {
    name: String,
    description: String,
    required: bool,
}

#[derive(Serialize, JsonSchema)]
struct PromptInfo {
    name: String,
    title: String,
    description: String,
    arguments: Vec<PromptArgInfo>,
}

#[derive(Serialize, JsonSchema)]
struct PromptListOut {
    prompts: Vec<PromptInfo>,
}

#[derive(Deserialize, JsonSchema)]
struct PromptShowIn {
    /// Prompt name (see `prompt.list`).
    name: String,
    /// Values for the prompt's arguments, e.g. `{"url": "https://..."}`.
    #[serde(default)]
    args: Option<Map<String, Value>>,
}

#[derive(Serialize, JsonSchema)]
struct PromptShowOut {
    name: String,
    title: String,
    description: String,
    /// The rendered prompt text: follow it as your instructions.
    text: String,
}

/// Command names (`group.action`) found inside backtick spans: either `group.action`
/// itself or the CLI form `rustler group action ...`.
pub(crate) fn related_commands(markdown: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    for (i, span) in markdown.split('`').enumerate() {
        if i % 2 == 0 {
            continue; // outside backticks
        }
        // Fenced blocks are one span with many lines; look at each line.
        for line in span.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.first() == Some(&"rustler") {
                if let (Some(g), Some(a)) = (words.get(1), words.get(2)) {
                    if let Some(name) = command_name(g, a) {
                        found.insert(name);
                    }
                }
            }
            for w in &words {
                let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_');
                if let Some((g, a)) = w.split_once('.') {
                    if let Some(name) = command_name(g, a) {
                        found.insert(name);
                    }
                }
            }
        }
    }
    found.into_iter().collect()
}

fn command_name(group: &str, action: &str) -> Option<String> {
    let action = action.replace('-', "_");
    let action = action.as_str();
    let ok_ident = |s: &str| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    };
    if COMMAND_GROUPS.contains(&group) && ok_ident(action) && !FILE_EXTS.contains(&action) {
        Some(format!("{group}.{action}"))
    } else {
        None
    }
}

fn ro() -> Annotations {
    Annotations {
        read_only: true,
        ..Default::default()
    }
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "guide.list",
        "List the built-in guides (the knowledge book) with id, title, summary and tags. Start with `guide.show start-here`.",
        &[],
        ro(),
        vec![Example {
            description: "list every guide",
            cli: "rustler guide list",
        }],
        |_ctx: Ctx, _i: NoInput| async move {
            let guides = rustler_knowledge::guides()
                .into_iter()
                .map(|g| GuideSummary {
                    id: g.id,
                    title: g.title,
                    summary: g.summary,
                    tags: g.tags,
                })
                .collect();
            Ok(GuideListOut {
                guides,
                start_with: "start-here".into(),
            })
        },
    ));

    reg.register(typed(
        "guide.show",
        "Show one guide in full (markdown) plus the commands it mentions.",
        &["id"],
        ro(),
        vec![Example {
            description: "read the orientation guide",
            cli: "rustler guide show start-here",
        }],
        |_ctx: Ctx, i: GuideShowIn| async move {
            let g = rustler_knowledge::guide(&i.id).ok_or_else(|| {
                let ids: Vec<String> = rustler_knowledge::guides().into_iter().map(|g| g.id).collect();
                RustlerError::new(
                    ErrorCode::NotFound,
                    "guide.show",
                    format!("no guide with id `{}`", i.id),
                )
                .hint(format!("available guides: {}", ids.join(", ")))
            })?;
            let related_commands = related_commands(&g.body);
            Ok(GuideShowOut {
                id: g.id,
                title: g.title,
                summary: g.summary,
                tags: g.tags,
                markdown: g.body,
                related_commands,
            })
        },
    ));

    reg.register(typed(
        "guide.search",
        "Search all guides for words (case-insensitive; every word must appear). Returns matching lines.",
        &["query"],
        ro(),
        vec![Example {
            description: "find guidance about captions",
            cli: "rustler guide search captions",
        }],
        |_ctx: Ctx, i: GuideSearchIn| async move {
            if i.query.trim().is_empty() {
                return Err(RustlerError::new(
                    ErrorCode::Validation,
                    "guide.search",
                    "query must not be empty",
                ));
            }
            let hits = rustler_knowledge::search(&i.query)
                .into_iter()
                .map(|(g, mut lines)| {
                    lines.truncate(8);
                    GuideHit {
                        id: g.id,
                        title: g.title,
                        summary: g.summary,
                        matches: lines,
                    }
                })
                .collect();
            Ok(GuideSearchOut {
                query: i.query,
                hits,
            })
        },
    ));

    reg.register(typed(
        "prompt.list",
        "List the built-in workflow prompts (reusable step-by-step instructions) and their arguments.",
        &[],
        ro(),
        vec![Example {
            description: "list prompts",
            cli: "rustler prompt list",
        }],
        |_ctx: Ctx, _i: NoInput| async move {
            let prompts = rustler_knowledge::prompts()
                .into_iter()
                .map(|p| PromptInfo {
                    name: p.name,
                    title: p.title,
                    description: p.description,
                    arguments: p
                        .arguments
                        .into_iter()
                        .map(|a| PromptArgInfo {
                            name: a.name,
                            description: a.description,
                            required: a.required,
                        })
                        .collect(),
                })
                .collect();
            Ok(PromptListOut { prompts })
        },
    ));

    reg.register(typed(
        "prompt.show",
        "Render a workflow prompt with arguments and return its text; follow it as instructions.",
        &["name"],
        ro(),
        vec![Example {
            description: "render a prompt with arguments",
            cli: "rustler prompt show <name> --args '{\"url\":\"https://example.com/v\"}'",
        }],
        |_ctx: Ctx, i: PromptShowIn| async move {
            let args = i.args.unwrap_or_default();
            let text = rustler_knowledge::render_prompt(&i.name, &args)?;
            let def = rustler_knowledge::prompts()
                .into_iter()
                .find(|p| p.name == i.name)
                .expect("render_prompt succeeded so the prompt exists");
            Ok(PromptShowOut {
                name: def.name,
                title: def.title,
                description: def.description,
                text,
            })
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reg() -> Registry {
        let mut r = Registry::new();
        register(&mut r);
        r
    }

    fn ctx() -> Ctx {
        Ctx::new(None, std::env::temp_dir())
    }

    #[test]
    fn related_commands_extracts_from_backticks_only() {
        let md = "Use `clip.create` and `rustler render run --clip x`; also `project.json` (a file),\nand plain source.import outside backticks, `see.tiles`, `rustler clip set-layout x`.";
        assert_eq!(
            related_commands(md),
            vec!["clip.create", "clip.set_layout", "render.run", "see.tiles"]
        );
    }

    #[tokio::test]
    async fn guide_list_and_show() {
        let r = reg();
        let v = r.dispatch("guide.list", json!({}), ctx()).await.unwrap();
        let guides = v["guides"].as_array().unwrap();
        assert!(guides.iter().any(|g| g["id"] == "start-here"));
        assert!(guides[0]["title"].is_string() && guides[0]["tags"].is_array());

        let v = r
            .dispatch("guide.show", json!({"id": "start-here"}), ctx())
            .await
            .unwrap();
        assert!(!v["markdown"].as_str().unwrap().is_empty());
        assert!(v["related_commands"].is_array());

        let e = r
            .dispatch("guide.show", json!({"id": "nope"}), ctx())
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert!(e.hint.unwrap().contains("start-here"));
    }

    #[tokio::test]
    async fn guide_search_and_validation() {
        let r = reg();
        let v = r
            .dispatch("guide.search", json!({"query": "start-here"}), ctx())
            .await
            .unwrap();
        assert!(v["hits"].as_array().unwrap().iter().any(|h| h["id"] == "start-here"));
        let e = r
            .dispatch("guide.search", json!({"query": " "}), ctx())
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation);
    }

    #[tokio::test]
    async fn prompt_list_and_show() {
        let r = reg();
        let v = r.dispatch("prompt.list", json!({}), ctx()).await.unwrap();
        let prompts = v["prompts"].as_array().unwrap();
        assert!(!prompts.is_empty());
        let p = &prompts[0];
        let name = p["name"].as_str().unwrap();
        let mut args = Map::new();
        for a in p["arguments"].as_array().unwrap() {
            args.insert(a["name"].as_str().unwrap().into(), json!("VALUE"));
        }
        let v = r
            .dispatch("prompt.show", json!({"name": name, "args": args}), ctx())
            .await
            .unwrap();
        assert!(!v["text"].as_str().unwrap().is_empty());

        let e = r
            .dispatch("prompt.show", json!({"name": "nope"}), ctx())
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
    }

    #[test]
    fn all_read_only_with_schemas() {
        for s in reg().list() {
            assert!(s.annotations.read_only, "{}", s.name);
            assert_eq!(s.output_schema["type"], "object", "{}", s.name);
        }
        assert_eq!(reg().list().len(), 5);
    }
}
