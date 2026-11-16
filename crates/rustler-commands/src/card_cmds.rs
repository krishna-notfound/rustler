//! Commands: asset.render_card, text.create_style, text.styles. The agent designs its own graphics
//! (title cards, lower thirds, badges, CTA cards, logo plates) from a JSON `CardSpec`; the PNG is saved
//! as an `image` asset (license `agent-generated`, tag `created_by:agent`) that `image.add` can place.
//! Text looks are saved as `text_style` assets (JSON) and can be referenced from card texts by name.

use crate::asset_cmds::{AssetRow, Scope, asset_font_dirs, load_rows};
use crate::clip_cmds::log_event;
use rustler_core::{ErrorCode, RustlerError};
use rustler_media::card::{
    ANIM_KINDS, CardEnv, CardSpec, OutlineSpec, ShadowSpec, TextAnim, TextBox, TextStyle, Weight, check_font, parse_color, render_card,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use rustler_tools::assets::{AssetEntry, AssetKind, assets_root, entry_file, load_manifest, now_rfc3339, project_assets_root, slug, unique_id, update_manifest, upsert};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// License recorded on everything the agent generates itself.
pub const LICENSE_AGENT: &str = "agent-generated";
/// Tag marking agent-made assets (there is no `created_by` field in the frozen manifest entry).
pub const TAG_CREATED_BY_AGENT: &str = "created_by:agent";

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn io_err(stage: &str, what: &str, p: &Path, e: impl std::fmt::Display) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, stage, format!("{what} {}: {e}", p.display()))
}

fn scope_name(s: Scope) -> &'static str {
    match s {
        Scope::Project => "project",
        Scope::Global => "global",
    }
}

fn root_for(ctx: &Ctx, scope: Option<Scope>, stage: &str) -> Result<(Scope, PathBuf)> {
    let scope = scope.unwrap_or(if ctx.project_dir.is_some() { Scope::Project } else { Scope::Global });
    match scope {
        Scope::Global => Ok((scope, assets_root(&ctx.home))),
        Scope::Project => match &ctx.project_dir {
            Some(p) => Ok((scope, project_assets_root(p))),
            None => Err(RustlerError::new(ErrorCode::NotFound, stage, "no project is open for scope \"project\"").hint("pass --scope global, or --project <dir>")),
        },
    }
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn log(ctx: &Ctx, kind: &str, data: serde_json::Value) {
    if let Ok(store) = ctx.store() {
        let _ = log_event(ctx, &store, kind, data);
    }
}

/// Everything the renderer looks up, built from the merged asset library.
fn load_style_file(row: &AssetRow) -> Option<TextStyle> {
    let bytes = std::fs::read(row.file.as_ref()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn with_env<T>(ctx: &Ctx, f: impl FnOnce(&CardEnv) -> Result<T>) -> Result<T> {
    let project = ctx.project_dir.as_deref();
    let paths = ToolPaths::from_home(&ctx.home);
    let rows = load_rows(&ctx.home, project);
    let mut font_dirs = vec![paths.fonts.clone()];
    font_dirs.extend(asset_font_dirs(&ctx.home, project));
    let mut env = CardEnv::new(font_dirs, rustler_media::qc::default_font(&paths));
    {
        let rows = &rows;
        env.resolve_asset = Box::new(move |key: &str| {
            let k = key.trim();
            let by_id = rows.iter().find(|r| r.entry.id == k && matches!(r.entry.kind, AssetKind::Font | AssetKind::Image));
            let by_name = || rows.iter().find(|r| r.entry.kind == AssetKind::Font && r.entry.name.eq_ignore_ascii_case(k));
            by_id.or_else(by_name).and_then(|r| r.file.clone()).filter(|f| f.is_file())
        });
        env.resolve_style = Box::new(move |key: &str| {
            let k = key.trim();
            let want = slug(k);
            rows.iter().find(|r| r.entry.kind == AssetKind::TextStyle && (r.entry.id == k || r.entry.name == k || r.entry.name == want)).and_then(load_style_file)
        });
    }
    f(&env)
}

// ---------------------------------------------------------------------------------------------
// asset.render_card
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct RenderCardIn {
    /// Display name of the card asset (the id becomes `image-<name>`). Rendering a card with the same
    /// name again replaces the earlier agent-made card (same id, so placements keep working).
    pub name: String,
    /// The design: canvas px size, background, shapes, images, texts (see `rustler guide show assets`).
    pub spec: CardSpec,
    /// `project` (default when a project is open) or `global`.
    pub scope: Option<Scope>,
    pub tags: Option<Vec<String>>,
}

#[derive(Serialize, JsonSchema)]
pub struct RenderCardOut {
    pub asset_id: String,
    /// The PNG file.
    pub path: PathBuf,
    /// The same PNG, so vision agents see the result.
    pub image: PathBuf,
    pub width: u32,
    pub height: u32,
    pub scope: String,
    /// True when an earlier agent-made card of this name was overwritten.
    pub replaced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

async fn render_card_cmd(ctx: Ctx, i: RenderCardIn) -> Result<RenderCardOut> {
    const STAGE: &str = "asset.render_card";
    if !i.name.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(validation(STAGE, "name must contain letters or digits").hint("e.g. `rustler asset render-card pov-hook --input-json @card.json`"));
    }
    let (scope, root) = root_for(&ctx, i.scope, STAGE)?;
    let out = with_env(&ctx, |env| render_card(&i.spec, env))?;
    let (width, height) = (i.spec.width, i.spec.height);

    let existing = load_manifest(&root);
    let same = existing.iter().find(|e| e.kind == AssetKind::Image && e.license == LICENSE_AGENT && e.name.eq_ignore_ascii_case(i.name.trim())).cloned();
    let id = same.as_ref().map(|e| e.id.clone()).unwrap_or_else(|| unique_id(AssetKind::Image, &i.name, &existing));
    let rel = match &same {
        Some(e) => e.path.clone(),
        None => {
            let dir = root.join(AssetKind::Image.dir());
            let base = slug(&i.name);
            let mut n = 1;
            let mut file = format!("{base}.png");
            while dir.join(&file).exists() {
                n += 1;
                file = format!("{base}-{n}.png");
            }
            format!("{}/{file}", AssetKind::Image.dir())
        }
    };
    let mut tags = vec!["card".to_string(), TAG_CREATED_BY_AGENT.to_string()];
    tags.extend(i.tags.clone().unwrap_or_default());
    let mut entry = AssetEntry {
        id: id.clone(),
        kind: AssetKind::Image,
        name: i.name.trim().to_string(),
        path: rel,
        tags: clean_tags(&tags),
        license: LICENSE_AGENT.into(),
        license_url: None,
        source_url: None,
        attribution: None,
        added_at: now_rfc3339(),
        sha256: String::new(),
    };
    let file = entry_file(&root, &entry).unwrap_or_default();
    let next = vec![
        format!("rustler asset show {id}"),
        format!("rustler image add <clip_id> asset:{id} --at <seconds>   (place it on a clip)"),
    ];
    if ctx.dry_run {
        return Ok(RenderCardOut {
            asset_id: id,
            path: file.clone(),
            image: file,
            width,
            height,
            scope: scope_name(scope).into(),
            replaced: same.is_some(),
            dry_run: Some(true),
            warnings: out.warnings,
            next,
        });
    }
    out.save_png(&file)?;
    entry.sha256 = rustler_tools::download::sha256_file(&file).map_err(|e| io_err(STAGE, "read", &file, e))?;
    update_manifest(&root, |m| {
        upsert(m, entry);
        Ok(())
    })?;
    log(&ctx, "asset.render_card", serde_json::json!({"id": id, "scope": scope_name(scope), "width": width, "height": height}));
    Ok(RenderCardOut {
        asset_id: id,
        path: file.clone(),
        image: file,
        width,
        height,
        scope: scope_name(scope).into(),
        replaced: same.is_some(),
        dry_run: None,
        warnings: out.warnings,
        next,
    })
}

// ---------------------------------------------------------------------------------------------
// text.create_style / text.styles
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct CreateTextStyleIn {
    /// Style name (lowercased, spaces become `-`); the asset id is `text-<name>`. Creating the same name again replaces it.
    pub name: String,
    /// Font asset id, .ttf path or installed family name (must resolve now).
    pub font: String,
    /// Size in px of the canvas the text is drawn on (cards: card px; over 1080-wide video: px at 1080).
    pub size: f32,
    /// Text colour `#RRGGBB` / `#RRGGBBAA`.
    pub color: String,
    pub weight: Option<Weight>,
    /// Outline colour and visible width in px.
    pub outline: Option<OutlineSpec>,
    /// Rounded box behind the text: colour, corner radius, padding (px).
    #[serde(rename = "box")]
    pub box_: Option<TextBox>,
    pub shadow: Option<ShadowSpec>,
    pub uppercase: Option<bool>,
    /// Animation hint for text layers: kind pop | bounce | fade | typewriter | slide_up.
    pub anim: Option<TextAnim>,
    pub tags: Option<Vec<String>>,
    /// `project` (default when a project is open) or `global`.
    pub scope: Option<Scope>,
}

#[derive(Serialize, JsonSchema)]
pub struct CreateTextStyleOut {
    pub asset_id: String,
    pub style: TextStyle,
    pub scope: String,
    pub file: PathBuf,
    pub replaced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub next: Vec<String>,
}

fn check_colour(stage: &str, label: &str, v: &str) -> Result<()> {
    parse_color(v).map(|_| ()).ok_or_else(|| validation(stage, format!("{label} '{v}' is not a colour")).hint("use #RRGGBB or #RRGGBBAA"))
}

async fn create_text_style(ctx: Ctx, i: CreateTextStyleIn) -> Result<CreateTextStyleOut> {
    const STAGE: &str = "text.create_style";
    if !i.name.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(validation(STAGE, "name must contain letters or digits").hint("e.g. `rustler text create-style hook-yellow --font Anton --size 110 --color #000000`"));
    }
    if !(i.size.is_finite() && (1.0..=2000.0).contains(&i.size)) {
        return Err(validation(STAGE, "size must be 1..2000 px"));
    }
    check_colour(STAGE, "color", &i.color)?;
    if let Some(o) = &i.outline {
        check_colour(STAGE, "outline.color", &o.color)?;
    }
    if let Some(b) = &i.box_ {
        check_colour(STAGE, "box.color", &b.color)?;
    }
    if let Some(s) = &i.shadow {
        if let Some(c) = &s.color {
            check_colour(STAGE, "shadow.color", c)?;
        }
    }
    if let Some(a) = &i.anim {
        if !ANIM_KINDS.contains(&a.kind.as_str()) {
            return Err(validation(STAGE, format!("anim.kind '{}' is unknown", a.kind)).hint(format!("one of: {}", ANIM_KINDS.join(", "))));
        }
    }
    with_env(&ctx, |env| check_font(env, &i.font, i.weight.as_ref()))?;
    let (scope, root) = root_for(&ctx, i.scope, STAGE)?;
    let name = slug(&i.name);
    let style = TextStyle {
        name: name.clone(),
        font: i.font.trim().to_string(),
        size: i.size,
        weight: i.weight.clone(),
        color: i.color.trim().to_string(),
        outline: i.outline.clone(),
        box_: i.box_.clone(),
        shadow: i.shadow.clone(),
        uppercase: i.uppercase,
        anim: i.anim.clone(),
    };
    let id = format!("text-{name}");
    let rel = format!("{}/{name}.json", AssetKind::TextStyle.dir());
    let file = root.join(&rel);
    let replaced = load_manifest(&root).iter().any(|e| e.id == id);
    let next = vec![
        format!("rustler text styles"),
        format!("use it in a card: {{\"texts\":[{{\"text\":\"...\",\"style\":\"{name}\",\"rect\":{{...}}}}]}} with `rustler asset render-card`"),
    ];
    if ctx.dry_run {
        return Ok(CreateTextStyleOut { asset_id: id, style, scope: scope_name(scope).into(), file, replaced, dry_run: Some(true), next });
    }
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
    std::fs::create_dir_all(&dir).map_err(|e| io_err(STAGE, "create", &dir, e))?;
    let body = serde_json::to_vec_pretty(&style).map_err(|e| validation(STAGE, e.to_string()))?;
    std::fs::write(&file, &body).map_err(|e| io_err(STAGE, "write", &file, e))?;
    let mut tags = vec!["text".to_string(), TAG_CREATED_BY_AGENT.to_string()];
    tags.extend(i.tags.clone().unwrap_or_default());
    let entry = AssetEntry {
        id: id.clone(),
        kind: AssetKind::TextStyle,
        name: name.clone(),
        path: rel,
        tags: clean_tags(&tags),
        license: LICENSE_AGENT.into(),
        license_url: None,
        source_url: None,
        attribution: None,
        added_at: now_rfc3339(),
        sha256: rustler_tools::download::sha256_bytes(&body),
    };
    update_manifest(&root, |m| {
        upsert(m, entry);
        Ok(())
    })?;
    log(&ctx, "text.create_style", serde_json::json!({"name": name, "scope": scope_name(scope)}));
    Ok(CreateTextStyleOut { asset_id: id, style, scope: scope_name(scope).into(), file, replaced, dry_run: None, next })
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct TextStylesIn {}

#[derive(Serialize, JsonSchema)]
pub struct TextStyleRow {
    pub asset_id: String,
    pub name: String,
    pub scope: String,
    pub style: Option<TextStyle>,
    pub file: Option<PathBuf>,
}

#[derive(Serialize, JsonSchema)]
pub struct TextStylesOut {
    pub count: usize,
    pub styles: Vec<TextStyleRow>,
    pub next: Vec<String>,
}

async fn text_styles(ctx: Ctx, _i: TextStylesIn) -> Result<TextStylesOut> {
    let styles: Vec<TextStyleRow> = load_rows(&ctx.home, ctx.project_dir.as_deref())
        .into_iter()
        .filter(|r| r.entry.kind == AssetKind::TextStyle)
        .map(|r| TextStyleRow { style: load_style_file(&r), asset_id: r.entry.id.clone(), name: r.entry.name.clone(), scope: r.scope.clone(), file: r.file.clone() })
        .collect();
    let next = vec!["rustler text create-style <name> --font <family> --size 96 --color #FFFFFF".to_string()];
    Ok(TextStylesOut { count: styles.len(), styles, next })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let ro = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false };
    let write = Annotations { read_only: false, ..ro };
    reg.register(typed(
        "asset.render_card",
        "Design your own graphic (title card, lower third, badge, CTA card, logo plate) from a JSON spec: canvas px size, background (solid / linear_gradient / transparent), rounded shapes (fill, stroke, shadow), images from assets, text blocks (font asset id or installed family, size, weight, colour, outline, box, shadow, align, wrap). Saved as a PNG image asset (license agent-generated) and returned as an image so you can look at it; place it with image.add",
        &["name"],
        write,
        vec![
            Example { description: "a POV hook card from a spec file (use --input-json @card.json on Windows PowerShell)", cli: "rustler asset render-card pov-hook --input-json @card.json" },
            Example { description: "same name again replaces the card (same asset id)", cli: "rustler asset render-card pov-hook --input-json @card-v2.json" },
        ],
        render_card_cmd,
    ));
    reg.register(typed(
        "text.create_style",
        "Save a text look (font, size, colour, outline, rounded box, shadow, uppercase, animation hint) as a text_style asset; cards use it with texts[].style, same name replaces it",
        &["name"],
        write,
        vec![Example { description: "black bold text in a yellow rounded box", cli: "rustler text create-style hook-yellow --input-json @style.json" }],
        create_text_style,
    ));
    reg.register(typed(
        "text.styles",
        "List saved text styles (project and global) with their definitions",
        &[],
        ro,
        vec![Example { description: "all text styles", cli: "rustler text styles" }],
        text_styles,
    ));
}
