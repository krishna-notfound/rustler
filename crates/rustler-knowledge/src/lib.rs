//! Agent-facing knowledge book for Rustler: guides and prompt templates written as
//! markdown files under `knowledge/` at the repo root and embedded into the binary.
//!
//! Formats (frozen):
//! - Guides: `knowledge/guides/<id>.md`, YAML frontmatter `id`, `title`, `summary`,
//!   `order` (int), `tags` (list), then a markdown body.
//! - Prompts: `knowledge/prompts/<name>.md`, YAML frontmatter `name`, `title`,
//!   `description`, `arguments` (list of `{name, description, required}`); the body is
//!   the template and uses `{{arg}}` placeholders.

use include_dir::{Dir, include_dir};
use rustler_core::{ErrorCode, RustlerError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::OnceLock;

static KNOWLEDGE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../knowledge");

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Guide {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub order: i64,
    pub tags: Vec<String>,
    /// Markdown body (frontmatter removed).
    pub body: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PromptArg {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PromptDef {
    pub name: String,
    pub title: String,
    pub description: String,
    pub arguments: Vec<PromptArg>,
    /// Body with `{{arg}}` placeholders.
    pub template: String,
}

#[derive(Deserialize)]
struct GuideFm {
    id: String,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    order: i64,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct PromptFm {
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    arguments: Vec<PromptArg>,
}

/// Splits `---\n<yaml>\n---\n<body>`; tolerant of CRLF and a BOM.
fn split_frontmatter(src: &str) -> Result<(String, String), String> {
    let src = src.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = src
        .strip_prefix("---\n")
        .ok_or("missing opening `---` frontmatter fence")?;
    let (fm, body) = if let Some(i) = rest.find("\n---\n") {
        (&rest[..i], &rest[i + 5..])
    } else if let Some(fm) = rest.strip_suffix("\n---") {
        (fm, "")
    } else {
        return Err("missing closing `---` frontmatter fence".into());
    };
    Ok((fm.to_string(), body.trim_start_matches('\n').to_string()))
}

/// Parses one guide file (frontmatter + markdown body).
pub fn parse_guide(src: &str) -> Result<Guide, String> {
    let (fm, body) = split_frontmatter(src)?;
    let fm: GuideFm = serde_yaml::from_str(&fm).map_err(|e| format!("guide frontmatter: {e}"))?;
    Ok(Guide {
        id: fm.id,
        title: fm.title,
        summary: fm.summary,
        order: fm.order,
        tags: fm.tags,
        body,
    })
}

/// Parses one prompt file (frontmatter + template body).
pub fn parse_prompt(src: &str) -> Result<PromptDef, String> {
    let (fm, body) = split_frontmatter(src)?;
    let fm: PromptFm = serde_yaml::from_str(&fm).map_err(|e| format!("prompt frontmatter: {e}"))?;
    Ok(PromptDef {
        name: fm.name,
        title: fm.title,
        description: fm.description,
        arguments: fm.arguments,
        template: body,
    })
}

fn load_dir<T>(sub: &str, parse: impl Fn(&str) -> Result<T, String>) -> Vec<T> {
    let mut out = Vec::new();
    let Some(dir) = KNOWLEDGE.get_dir(sub) else {
        return out;
    };
    for f in dir.files() {
        if f.path().extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(text) = f.contents_utf8() else {
            eprintln!("rustler-knowledge: {} is not UTF-8, skipped", f.path().display());
            continue;
        };
        match parse(text) {
            Ok(v) => out.push(v),
            // Pinned by the `embedded_files_all_parse` test, so this never ships silently.
            Err(e) => eprintln!("rustler-knowledge: {}: {e}", f.path().display()),
        }
    }
    out
}

fn all_guides() -> &'static Vec<Guide> {
    static CELL: OnceLock<Vec<Guide>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut v = load_dir("guides", parse_guide);
        v.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        v
    })
}

fn all_prompts() -> &'static Vec<PromptDef> {
    static CELL: OnceLock<Vec<PromptDef>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut v = load_dir("prompts", parse_prompt);
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    })
}

/// All guides, sorted by `order` (then id).
pub fn guides() -> Vec<Guide> {
    all_guides().clone()
}

pub fn guide(id: &str) -> Option<Guide> {
    all_guides().iter().find(|g| g.id == id).cloned()
}

/// Case-insensitive search. Every whitespace-separated term must occur somewhere in a
/// guide (id, title, summary, tags or body); the second tuple element lists the body
/// lines (trimmed) containing at least one term. Best matches (most lines) first.
pub fn search(query: &str) -> Vec<(Guide, Vec<String>)> {
    search_in(all_guides(), query)
}

fn search_in(guides: &[Guide], query: &str) -> Vec<(Guide, Vec<String>)> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(Guide, Vec<String>)> = Vec::new();
    for g in guides {
        let hay = format!(
            "{}\n{}\n{}\n{}\n{}",
            g.id,
            g.title,
            g.summary,
            g.tags.join(" "),
            g.body
        )
        .to_lowercase();
        if !terms.iter().all(|t| hay.contains(t.as_str())) {
            continue;
        }
        let lines: Vec<String> = g
            .body
            .lines()
            .filter(|l| {
                let l = l.to_lowercase();
                terms.iter().any(|t| l.contains(t.as_str()))
            })
            .map(|l| l.trim().to_string())
            .collect();
        hits.push((g.clone(), lines));
    }
    hits.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.order.cmp(&b.0.order)));
    hits
}

/// All prompt templates, sorted by name.
pub fn prompts() -> Vec<PromptDef> {
    all_prompts().clone()
}

/// Renders the named embedded prompt. `NotFound` for unknown names; `Validation` when a
/// required argument is missing or empty.
pub fn render_prompt(name: &str, args: &Map<String, Value>) -> Result<String, RustlerError> {
    let def = all_prompts().iter().find(|p| p.name == name).ok_or_else(|| {
        let names: Vec<&str> = all_prompts().iter().map(|p| p.name.as_str()).collect();
        RustlerError::new(ErrorCode::NotFound, "prompt", format!("unknown prompt `{name}`"))
            .hint(format!("available prompts: {}", names.join(", ")))
    })?;
    render_def(def, args)
}

/// Renders a prompt definition (used by `render_prompt`; public for tests/tools).
pub fn render_def(def: &PromptDef, args: &Map<String, Value>) -> Result<String, RustlerError> {
    let to_text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut missing = Vec::new();
    for a in &def.arguments {
        let present = args
            .get(&a.name)
            .map(|v| !to_text(v).trim().is_empty())
            .unwrap_or(false);
        if a.required && !present {
            missing.push(a.name.clone());
        }
    }
    if !missing.is_empty() {
        return Err(RustlerError::new(
            ErrorCode::Validation,
            "prompt",
            format!(
                "prompt `{}` is missing required argument(s): {}",
                def.name,
                missing.join(", ")
            ),
        )
        .hint("pass them as {\"args\": {\"<name>\": \"<value>\"}}"));
    }
    let mut out = String::with_capacity(def.template.len());
    let mut rest = def.template.as_str();
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let key = after[..end].trim();
                let declared = def.arguments.iter().any(|a| a.name == key);
                match args.get(key) {
                    Some(v) => out.push_str(&to_text(v)),
                    // Declared-but-optional and not given: empty. Unknown placeholder: keep.
                    None if declared => {}
                    None => {
                        out.push_str("{{");
                        out.push_str(&after[..end]);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GUIDE_SRC: &str = "---\nid: g1\ntitle: Guide One\nsummary: About clips\norder: 2\ntags: [a, b]\n---\n# Heading\n\nUse `clip.create` then `render.run`.\nSecond line about Captions.\n";
    const PROMPT_SRC: &str = "---\nname: p1\ntitle: P One\ndescription: Does a thing\narguments:\n  - name: url\n    description: The URL\n    required: true\n  - name: note\n    description: Optional\n    required: false\n---\nImport {{url}} now. Note: [{{ note }}] keep {{unknown}}.\n";

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn parses_guide_in_memory() {
        let g = parse_guide(GUIDE_SRC).unwrap();
        assert_eq!(g.id, "g1");
        assert_eq!(g.title, "Guide One");
        assert_eq!(g.order, 2);
        assert_eq!(g.tags, vec!["a", "b"]);
        assert!(g.body.starts_with("# Heading"));
        assert!(!g.body.contains("---"));
    }

    #[test]
    fn parses_crlf_and_rejects_bad_frontmatter() {
        let crlf = GUIDE_SRC.replace('\n', "\r\n");
        assert_eq!(parse_guide(&crlf).unwrap().id, "g1");
        assert!(parse_guide("# no frontmatter").is_err());
        assert!(parse_guide("---\nid: x\n").is_err());
        assert!(parse_guide("---\ntitle: no id\n---\nbody").is_err());
    }

    #[test]
    fn parses_prompt_and_renders() {
        let p = parse_prompt(PROMPT_SRC).unwrap();
        assert_eq!(p.arguments.len(), 2);
        assert!(p.arguments[0].required && !p.arguments[1].required);
        let out = render_def(&p, &args(json!({"url": "http://x"}))).unwrap();
        assert_eq!(out.trim(), "Import http://x now. Note: [] keep {{unknown}}.");
        let out = render_def(&p, &args(json!({"url": "u", "note": 5}))).unwrap();
        assert!(out.contains("[5]"));
    }

    #[test]
    fn render_requires_required_args() {
        let p = parse_prompt(PROMPT_SRC).unwrap();
        let e = render_def(&p, &Map::new()).unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation);
        assert!(e.message.contains("url"));
        let e = render_def(&p, &args(json!({"url": "  "}))).unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation);
    }

    #[test]
    fn search_matches_terms_and_lines() {
        let g = parse_guide(GUIDE_SRC).unwrap();
        let hits = search_in(std::slice::from_ref(&g), "CLIP.create");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, vec!["Use `clip.create` then `render.run`."]);
        // all terms must be present somewhere
        assert!(search_in(std::slice::from_ref(&g), "clip zebra").is_empty());
        assert!(search_in(std::slice::from_ref(&g), "   ").is_empty());
        // metadata-only match yields the guide with no body lines
        let hits = search_in(std::slice::from_ref(&g), "about clips");
        assert_eq!(hits.len(), 1);
    }

    // ---- tests over the embedded knowledge/ folder (must hold for any real content) ----

    #[test]
    fn embedded_files_all_parse() {
        for sub in ["guides", "prompts"] {
            let dir = KNOWLEDGE.get_dir(sub).unwrap_or_else(|| panic!("knowledge/{sub} missing"));
            for f in dir.files() {
                let text = f.contents_utf8().expect("utf8");
                let r = if sub == "guides" {
                    parse_guide(text).map(|_| ())
                } else {
                    parse_prompt(text).map(|_| ())
                };
                r.unwrap_or_else(|e| panic!("{}: {e}", f.path().display()));
            }
        }
    }

    #[test]
    fn embedded_guides_sorted_and_start_here_exists() {
        let gs = guides();
        assert!(!gs.is_empty());
        assert!(gs.windows(2).all(|w| w[0].order <= w[1].order));
        let ids: std::collections::HashSet<_> = gs.iter().map(|g| &g.id).collect();
        assert_eq!(ids.len(), gs.len(), "duplicate guide ids");
        let g = guide("start-here").expect("start-here guide");
        assert!(!g.body.is_empty());
        assert!(guide("does-not-exist").is_none());
    }

    #[test]
    fn embedded_search_finds_start_here_by_id() {
        let hits = search("start-here");
        assert!(hits.iter().any(|(g, _)| g.id == "start-here"));
        assert!(search("").is_empty());
    }

    #[test]
    fn embedded_prompts_render_with_dummy_args() {
        let ps = prompts();
        assert!(!ps.is_empty());
        for p in &ps {
            let mut a = Map::new();
            for arg in &p.arguments {
                a.insert(arg.name.clone(), json!("DUMMY"));
            }
            let out = render_prompt(&p.name, &a).unwrap();
            for arg in &p.arguments {
                assert!(
                    !out.contains(&format!("{{{{{}}}}}", arg.name)),
                    "prompt {} left {{{{{}}}}} unreplaced",
                    p.name,
                    arg.name
                );
            }
            // missing required args are rejected
            if p.arguments.iter().any(|a| a.required) {
                assert_eq!(
                    render_prompt(&p.name, &Map::new()).unwrap_err().code,
                    ErrorCode::Validation
                );
            }
        }
        assert_eq!(
            render_prompt("no-such-prompt", &Map::new()).unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}
