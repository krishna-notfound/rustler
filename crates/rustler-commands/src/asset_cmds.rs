//! Commands: asset.list, asset.search, asset.show, asset.add, asset.remove, caption.create_style,
//! caption.styles. The creative asset library: fonts, music, SFX, images, caption styles, layouts.
//!
//! Two stores with the same schema (`manifest.json` + files, see `rustler_tools::assets`): the
//! global library `<home>/assets/` and a per-project `<project>/assets/`. They are merged at read
//! time (project overrides global overrides built-in, by id). Every entry carries license metadata;
//! `asset.add` takes anything the agent or user supplies (a file, or `--url`): a missing license is
//! recorded as "unspecified (added by user/agent)" with a warning, never refused.
//!
//! Helpers other command modules use: [`resolve_asset_path`] (`asset:<id>` in place of a path),
//! [`asset_font_dirs`] (render font staging), [`custom_presets`] / [`check_caption_style`]
//! (custom caption styles), [`style_infos`].

use crate::clip_cmds::log_event;
use crate::media_ref::{self, Want, is_url};
use crate::see_cmds::find_fonts;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use imageproc::drawing::draw_text_mut;
use rustler_captions::presets::PRESET_TABLE;
use rustler_captions::{ActiveKind, CaptionWord, CustomStyle, Preset, find_preset_in, load_custom_presets, write_ass_with};
use rustler_core::{Canvas, CaptionTrack, ErrorCode, ProjectStore, RustlerError};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use rustler_tools::assets::{
    AssetEntry, AssetKind, assets_root, builtin_entries, entry_file, load_manifest, now_rfc3339,
    project_assets_root, slug, unique_id, update_manifest, upsert,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

type Result<T> = rustler_core::Result<T>;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

fn io_err(stage: &str, what: &str, p: &Path, e: impl std::fmt::Display) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, stage, format!("{what} {}: {e}", p.display()))
}

// ---------------------------------------------------------------------------------------------
// Merged library view
// ---------------------------------------------------------------------------------------------

/// Which store an asset comes from.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Project,
    Global,
}

/// A manifest entry plus where it lives.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct AssetRow {
    #[serde(flatten)]
    pub entry: AssetEntry,
    /// `built-in`, `global` or `project`.
    pub scope: String,
    /// Absolute file path (null for built-in styles/layouts).
    pub file: Option<PathBuf>,
    /// False when the manifest lists a file that is gone.
    pub exists: bool,
}

fn rows_of(root: &Path, scope: &str, entries: Vec<AssetEntry>) -> Vec<AssetRow> {
    entries
        .into_iter()
        .map(|entry| {
            let file = entry_file(root, &entry);
            let exists = file.as_ref().is_none_or(|f| f.is_file());
            let scope = if entry.is_builtin() { "built-in" } else { scope };
            AssetRow { entry, scope: scope.to_string(), file, exists }
        })
        .collect()
}

/// Built-in + global + project assets merged by id (later stores override earlier ones).
pub fn load_rows(home: &Path, project: Option<&Path>) -> Vec<AssetRow> {
    let mut out: Vec<AssetRow> = Vec::new();
    let groot = assets_root(home);
    let mut sources = vec![rows_of(&groot, "built-in", builtin_entries()), rows_of(&groot, "global", load_manifest(&groot))];
    if let Some(p) = project {
        let proot = project_assets_root(p);
        sources.push(rows_of(&proot, "project", load_manifest(&proot)));
    }
    for r in sources.into_iter().flatten() {
        match out.iter_mut().find(|x| x.entry.id == r.entry.id) {
            Some(old) => *old = r,
            None => out.push(r),
        }
    }
    out
}

fn find_row(home: &Path, project: Option<&Path>, id: &str) -> Result<AssetRow> {
    load_rows(home, project).into_iter().find(|r| r.entry.id == id).ok_or_else(|| {
        RustlerError::new(ErrorCode::NotFound, "asset", format!("no asset with id '{id}'"))
            .hint("`rustler asset list` or `rustler asset search <words>` shows the ids")
    })
}

/// Resolves `asset:<id>` to the asset's file, or returns `s` unchanged as a path. Use it wherever a
/// command takes an audio/image path (`audio.add_music`, `audio.add_sfx`).
pub fn resolve_asset_path(store: &ProjectStore, home: &Path, s: &str) -> Result<PathBuf> {
    resolve_asset_path_in(Some(&store.root), home, s)
}

/// [`resolve_asset_path`] without a project store (global library only when `project` is None).
pub fn resolve_asset_path_in(project: Option<&Path>, home: &Path, s: &str) -> Result<PathBuf> {
    let Some(id) = s.trim().strip_prefix("asset:") else { return Ok(PathBuf::from(s)) };
    let row = find_row(home, project, id.trim())?;
    match &row.file {
        Some(f) if f.is_file() => Ok(f.clone()),
        Some(f) => Err(RustlerError::new(ErrorCode::NotFound, "asset", format!("the file of asset '{}' is missing: {}", row.entry.id, f.display()))
            .hint("remove the entry with `rustler asset remove` and add it again")),
        None => Err(validation("asset", format!("asset '{}' ({}) has no file to use as a path", row.entry.id, row.entry.kind.as_str()))),
    }
}

/// Directories holding the font files of `font` assets (global and project), to be staged next to
/// the built-in fonts for libass. Project directories come first.
pub fn asset_font_dirs(home: &Path, project: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for r in load_rows(home, project).into_iter().rev() {
        if r.entry.kind != AssetKind::Font {
            continue;
        }
        if let Some(dir) = r.file.as_ref().filter(|f| f.is_file()).and_then(|f| f.parent()) {
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    dirs
}

/// Custom caption presets (project styles first, then global) for `write_ass_with`.
pub fn custom_presets(home: &Path, project: Option<&Path>) -> Vec<Preset> {
    let mut out = Vec::new();
    if let Some(p) = project {
        out.extend(load_custom_presets(&project_assets_root(p).join(AssetKind::CaptionStyle.dir())));
    }
    out.extend(load_custom_presets(&assets_root(home).join(AssetKind::CaptionStyle.dir())));
    out
}

/// `Validation` unless `name` is a built-in or a saved custom caption style.
pub fn check_caption_style(ctx: &Ctx, stage: &str, name: &str) -> Result<()> {
    let custom = custom_presets(&ctx.home, ctx.project_dir.as_deref());
    if find_preset_in(name, &custom).is_some() {
        return Ok(());
    }
    let names: Vec<&str> = rustler_captions::PRESETS.iter().copied().chain(custom.iter().map(|p| p.name)).collect();
    Err(validation(stage, format!("unknown caption style '{name}'"))
        .hint(format!("available styles: {} (see `rustler caption styles`; make one with `rustler caption create-style`)", names.join(", "))))
}

// ---------------------------------------------------------------------------------------------
// Style listing
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct StyleInfo {
    pub name: String,
    pub description: String,
    pub font_family: String,
    pub built_in: bool,
    /// `built-in`, `global` or `project`.
    pub scope: String,
    /// Asset id (`style-<name>`).
    pub asset_id: String,
    pub active_kind: ActiveKind,
    pub uppercase: bool,
    pub primary: String,
    pub active_colour: String,
}

fn style_info(p: &Preset, scope: &str) -> StyleInfo {
    let s = CustomStyle::from_preset(p);
    StyleInfo {
        name: p.name.to_string(),
        description: p.description.to_string(),
        font_family: p.font_family.to_string(),
        built_in: scope == "built-in",
        scope: scope.to_string(),
        asset_id: format!("style-{}", p.name),
        active_kind: s.active_kind,
        uppercase: p.uppercase,
        primary: s.primary,
        active_colour: s.active_colour,
    }
}

/// Built-in styles, then the project's, then the global custom styles.
pub fn style_infos(home: &Path, project: Option<&Path>) -> Vec<StyleInfo> {
    let mut out: Vec<StyleInfo> = PRESET_TABLE.iter().map(|p| style_info(p, "built-in")).collect();
    if let Some(p) = project {
        for c in load_custom_presets(&project_assets_root(p).join(AssetKind::CaptionStyle.dir())) {
            if !out.iter().any(|i| i.name == c.name) {
                out.push(style_info(&c, "project"));
            }
        }
    }
    for c in load_custom_presets(&assets_root(home).join(AssetKind::CaptionStyle.dir())) {
        if !out.iter().any(|i| i.name == c.name) {
            out.push(style_info(&c, "global"));
        }
    }
    out
}

/// Name/description/family of every style, built-in and custom (for `caption.list_styles`).
pub fn preset_infos(home: &Path, project: Option<&Path>) -> Vec<rustler_captions::PresetInfo> {
    rustler_captions::list_presets_with(&custom_presets(home, project))
}

// ---------------------------------------------------------------------------------------------
// asset.list / asset.search
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ListIn {
    /// font | music | sfx | image | caption_style | layout | template | text_style
    pub kind: Option<AssetKind>,
    /// Only assets carrying this tag.
    pub tag: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOut {
    pub count: usize,
    pub assets: Vec<AssetRow>,
    pub next: Vec<String>,
}

fn has_tag(r: &AssetRow, tag: &str) -> bool {
    let t = tag.trim().to_lowercase();
    r.entry.tags.iter().any(|x| x.to_lowercase() == t)
}

async fn asset_list(ctx: Ctx, i: ListIn) -> Result<ListOut> {
    let mut rows = load_rows(&ctx.home, ctx.project_dir.as_deref());
    if let Some(k) = i.kind {
        rows.retain(|r| r.entry.kind == k);
    }
    if let Some(t) = i.tag.as_deref().filter(|t| !t.trim().is_empty()) {
        rows.retain(|r| has_tag(r, t));
    }
    let next = rows.first().map(|r| vec![format!("rustler asset show {}", r.entry.id)]).unwrap_or_default();
    Ok(ListOut { count: rows.len(), assets: rows, next })
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchIn {
    /// Words to look for in names, tags and ids ("whoosh", "impact heavy", "devanagari").
    pub query: String,
    pub kind: Option<AssetKind>,
}

#[derive(Serialize, JsonSchema)]
pub struct SearchHit {
    pub score: f64,
    #[serde(flatten)]
    pub asset: AssetRow,
}

#[derive(Serialize, JsonSchema)]
pub struct SearchOut {
    pub count: usize,
    pub hits: Vec<SearchHit>,
    pub next: Vec<String>,
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// 0..1: mean over the query words of the best match against the asset's words.
fn match_score(query: &[String], r: &AssetRow) -> f64 {
    let mut hay = words(&r.entry.name);
    for t in &r.entry.tags {
        hay.extend(words(t));
    }
    hay.extend(words(&r.entry.id));
    hay.extend(words(r.entry.kind.as_str()));
    if query.is_empty() || hay.is_empty() {
        return 0.0;
    }
    let total: f64 = query
        .iter()
        .map(|q| {
            hay.iter()
                .map(|h| {
                    if h == q {
                        1.0
                    } else if q.len() >= 3 && (h.contains(q.as_str()) || (h.len() >= 3 && q.contains(h.as_str()))) {
                        0.85
                    } else {
                        let jw = strsim::jaro_winkler(h, q);
                        let prefix = h.chars().zip(q.chars()).take_while(|(a, b)| a == b).count();
                        // shared stem ("swipe" / "swiping") scores like a close fuzzy match
                        let stem = if prefix >= 4 && prefix * 10 >= h.chars().count().min(q.chars().count()) * 6 { 0.7 } else { 0.0 };
                        (if jw >= 0.88 { jw * 0.8 } else { 0.0 }).max(stem)
                    }
                })
                .fold(0.0, f64::max)
        })
        .sum();
    total / query.len() as f64
}

async fn asset_search(ctx: Ctx, i: SearchIn) -> Result<SearchOut> {
    let q = words(&i.query);
    if q.is_empty() {
        return Err(validation("asset.search", "query is empty").hint("e.g. `rustler asset search whoosh`"));
    }
    let mut hits: Vec<SearchHit> = load_rows(&ctx.home, ctx.project_dir.as_deref())
        .into_iter()
        .filter(|r| i.kind.is_none_or(|k| r.entry.kind == k))
        .map(|r| SearchHit { score: (match_score(&q, &r) * 1000.0).round() / 1000.0, asset: r })
        .filter(|h| h.score >= 0.5)
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.asset.entry.name.cmp(&b.asset.entry.name)));
    let next = hits.first().map(|h| vec![format!("rustler asset show {}", h.asset.entry.id)]).unwrap_or_else(|| {
        vec!["rustler asset list".to_string(), "add your own: rustler asset add <file> --kind sfx --license <license>".to_string()]
    });
    Ok(SearchOut { count: hits.len(), hits, next })
}

// ---------------------------------------------------------------------------------------------
// asset.show (with previews)
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct IdIn {
    pub id: String,
}

#[derive(Serialize, JsonSchema, Default)]
pub struct Preview {
    /// image | audio | none
    pub kind: String,
    /// Sample PNG (fonts, caption styles); look at it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Why no preview could be made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ShowOut {
    pub asset: AssetRow,
    pub preview: Preview,
    /// Resolved style definition (caption styles).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<CustomStyle>,
    pub next: Vec<String>,
}

fn text_width(font: &FontArc, scale: PxScale, text: &str) -> f32 {
    let sf = font.as_scaled(scale);
    text.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum()
}

/// Draws `text` at the largest size <= `max` that fits `width`.
fn draw_fitted(img: &mut RgbImage, font: &FontArc, text: &str, y: i32, max: f32, width: f32, colour: Rgb<u8>) -> f32 {
    let mut size = max;
    while size > 14.0 && text_width(font, PxScale::from(size), text) > width {
        size -= 2.0;
    }
    draw_text_mut(img, colour, 20, y, PxScale::from(size), font, text);
    size
}

/// Renders a specimen of `file` to `out`.
fn font_preview(file: &Path, out: &Path, title: &str, devanagari: bool) -> Result<()> {
    const STAGE: &str = "asset.show";
    let data = std::fs::read(file).map_err(|e| io_err(STAGE, "read", file, e))?;
    let font = FontArc::try_from_vec(data).map_err(|e| validation(STAGE, format!("cannot parse font {}: {e}", file.display())))?;
    let mut img = RgbImage::from_pixel(1080, 420, Rgb([255, 255, 255]));
    let (l1, l2) = if devanagari {
        ("\u{0915}\u{0916}\u{0917}\u{0918} \u{091A}\u{091B}\u{091C}\u{091D} \u{0924}\u{0925}\u{0926}", "0123456789 Hinglish ABC abc")
    } else {
        ("Make every word count", "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789")
    };
    let black = Rgb([20, 20, 20]);
    let y1 = 20;
    let s1 = draw_fitted(&mut img, &font, l1, y1, 120.0, 1040.0, black);
    let y2 = y1 + s1 as i32 + 24;
    let s2 = draw_fitted(&mut img, &font, l2, y2, 44.0, 1040.0, black);
    draw_fitted(&mut img, &font, &format!("{title} - {l2}"), y2 + s2 as i32 + 30, 30.0, 1040.0, Rgb([90, 90, 90]));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_err(STAGE, "create", dir, e))?;
    }
    img.save(out).map_err(|e| io_err(STAGE, "write", out, e))
}

/// Copies every font from `dirs` (flat) into `dest`.
fn stage_font_dirs(dirs: &[PathBuf], dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|e| io_err("asset", "create", dest, e))?;
    for d in dirs {
        for f in find_fonts(d) {
            if let Some(name) = f.file_name() {
                let to = dest.join(name);
                if !to.exists() {
                    std::fs::copy(&f, &to).map_err(|e| io_err("asset", "copy font", &f, e))?;
                }
            }
        }
    }
    Ok(())
}

/// Burns sample captions in style `style` onto a gray 1080x1920 frame (the same ASS path as
/// `render.run`/`render.preview_frame`) and saves it as PNG at `out`.
pub async fn caption_style_preview(home: &Path, project: Option<&Path>, style: &str, out: &Path) -> Result<()> {
    const STAGE: &str = "asset.show";
    let paths = ToolPaths::from_home(home);
    if !paths.ffmpeg.is_file() {
        return Err(RustlerError::new(ErrorCode::DependencyMissing, STAGE, format!("ffmpeg not found at {}", paths.ffmpeg.display()))
            .hint("run `rustler setup`"));
    }
    let custom = custom_presets(home, project);
    let sample = ["Make", "every", "word", "count"];
    let words: Vec<CaptionWord> = sample
        .iter()
        .enumerate()
        .map(|(k, w)| CaptionWord { text: (*w).to_string(), out_start: k as f64 * 0.4, out_end: k as f64 * 0.4 + 0.4 })
        .collect();
    let track = CaptionTrack { preset: style.to_string(), max_words_per_line: 4, ..Default::default() };
    let ass = write_ass_with(&words, &track, &Canvas::default(), "", &custom)?;

    let work = std::env::temp_dir().join(format!(
        "rustler-style-preview-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&work).map_err(|e| io_err(STAGE, "create", &work, e))?;
    let result = async {
        let mut dirs = vec![paths.fonts.clone()];
        dirs.extend(asset_font_dirs(home, project));
        stage_font_dirs(&dirs, &work.join("fonts"))?;
        std::fs::write(work.join("captions.ass"), ass).map_err(|e| io_err(STAGE, "write", &work, e))?;
        let c = Canvas::default();
        let args: Vec<std::ffi::OsString> = [
            "-hide_banner".to_string(), "-loglevel".into(), "error".into(), "-nostdin".into(), "-y".into(),
            "-f".into(), "lavfi".into(), "-i".into(), format!("color=c=0x707070:s={}x{}:r=30:d=2", c.width, c.height),
            "-vf".into(), "ass=captions.ass:fontsdir=fonts".into(),
            // second word of the sample is active at 0.5 s
            "-ss".into(), "0.5".into(), "-frames:v".into(), "1".into(), "preview.png".into(),
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        rustler_tools::run(&paths.ffmpeg, &args, Some(&work), &CancellationToken::new()).await?;
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io_err(STAGE, "create", dir, e))?;
        }
        std::fs::copy(work.join("preview.png"), out).map_err(|e| io_err(STAGE, "write", out, e))?;
        Ok::<(), RustlerError>(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn preview_path(home: &Path, row: &AssetRow) -> PathBuf {
    assets_root(home).join("previews").join(format!("{}-{}.png", row.scope.replace('-', ""), row.entry.id))
}

async fn asset_show(ctx: Ctx, i: IdIn) -> Result<ShowOut> {
    let project = ctx.project_dir.as_deref();
    let row = find_row(&ctx.home, project, i.id.trim())?;
    let mut preview = Preview { kind: "none".into(), ..Default::default() };
    let mut style = None;
    let mut next = Vec::new();
    match row.entry.kind {
        AssetKind::Font => match row.file.as_ref().filter(|f| f.is_file()) {
            Some(f) => {
                let out = preview_path(&ctx.home, &row);
                let deva = row.entry.tags.iter().any(|t| t == "devanagari") || row.entry.name.to_lowercase().contains("devanagari");
                match font_preview(f, &out, &row.entry.name, deva) {
                    Ok(()) => {
                        preview.kind = "image".into();
                        preview.image = Some(out);
                    }
                    Err(e) => preview.note = Some(e.message),
                }
                next.push("use it in a style: rustler caption create-style <name> --font-family \"<family>\"".to_string());
            }
            None => preview.note = Some("font file is missing".into()),
        },
        AssetKind::Music | AssetKind::Sfx => match row.file.as_ref().filter(|f| f.is_file()) {
            Some(f) => {
                preview.kind = "audio".into();
                let paths = ToolPaths::from_home(&ctx.home);
                match rustler_media::probe(&paths, f).await {
                    Ok(p) => {
                        preview.duration_s = Some((p.duration * 1000.0).round() / 1000.0);
                        preview.sample_rate = p.sample_rate;
                        preview.channels = p.channels;
                        preview.codec = p.audio_codec;
                    }
                    Err(e) => preview.note = Some(format!("ffprobe failed: {}", e.message)),
                }
                let verb = if row.entry.kind == AssetKind::Sfx { "add-sfx" } else { "add-music" };
                next.push(format!("rustler audio {verb} <clip_id> asset:{}", row.entry.id));
            }
            None => preview.note = Some("audio file is missing".into()),
        },
        AssetKind::Image => match row.file.as_ref().filter(|f| f.is_file()) {
            Some(f) => {
                preview.kind = "image".into();
                preview.image = Some(f.clone());
                if let Ok((w, h)) = image::image_dimensions(f) {
                    preview.width = Some(w);
                    preview.height = Some(h);
                }
            }
            None => preview.note = Some("image file is missing".into()),
        },
        AssetKind::CaptionStyle => {
            let custom = custom_presets(&ctx.home, project);
            match find_preset_in(&row.entry.name, &custom) {
                Some(p) => {
                    style = Some(CustomStyle::from_preset(p));
                    let out = preview_path(&ctx.home, &row);
                    match caption_style_preview(&ctx.home, project, &row.entry.name, &out).await {
                        Ok(()) => {
                            preview.kind = "image".into();
                            preview.image = Some(out);
                            preview.width = Some(1080);
                            preview.height = Some(1920);
                        }
                        Err(e) => preview.note = Some(e.message),
                    }
                    next.push(format!("rustler clip set-captions <clip_id> --style {}", row.entry.name));
                }
                None => preview.note = Some("the style file is missing or invalid".into()),
            }
        }
        AssetKind::Layout => {
            next.push(format!("rustler clip set-layout <clip_id> {}", row.entry.name));
        }
        AssetKind::Lut => {
            next.push(format!("rustler grade set <clip_id> --lut asset:{} --lut-strength 0.6", row.entry.id));
            next.push("rustler grade preview <clip_id> --out-t 1".to_string());
        }
        AssetKind::Template | AssetKind::TextStyle => {}
    }
    Ok(ShowOut { asset: row, preview, style, next })
}

// ---------------------------------------------------------------------------------------------
// asset.add
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct AddIn {
    /// Local file to copy into the library (or `source:<source_id>[@start-end]` to take the audio of an
    /// imported video as a music/sfx asset). Give this or `--url`.
    pub path: Option<PathBuf>,
    /// Direct `http(s)` link to an audio/image/font/video file: downloaded into the library (needs the
    /// project's network+downloads permission, like `source import`). A video file added as music/sfx
    /// is reduced to its audio track.
    pub url: Option<String>,
    pub kind: AssetKind,
    /// Display name (default: the file name).
    pub name: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Optional: the license the file is under (`CC0-1.0`, `CC-BY-4.0`, `OFL-1.1`, `own work`,
    /// `licensed: <where from>`, anything). Missing = stored as "unspecified (added by user/agent)" with a warning.
    #[serde(default)]
    pub license: String,
    pub license_url: Option<String>,
    pub source_url: Option<String>,
    /// Credit line to show when the license requires attribution.
    pub attribution: Option<String>,
    /// `project` (default when a project is open) or `global`.
    pub scope: Option<Scope>,
}

#[derive(Serialize, JsonSchema)]
pub struct AddOut {
    pub asset: AssetRow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

const NO_LICENSE: &[&str] = &["", "unknown", "none", "n/a", "na", "unlicensed", "todo", "tbd", "?", "-", "unspecified"];

/// License recorded when none (or a placeholder such as `unknown`) is given.
pub const LICENSE_UNSPECIFIED: &str = "unspecified (added by user/agent)";

/// The license text to store, and a warning when it is unspecified. Never refuses: the agent/user
/// decides what may be used; Rustler only records what it was told.
pub fn normalize_license(license: &str) -> (String, Option<String>) {
    let l = license.trim();
    if NO_LICENSE.contains(&l.to_lowercase().as_str()) {
        (
            LICENSE_UNSPECIFIED.to_string(),
            Some("no license given: stored as \"unspecified (added by user/agent)\". You are responsible for the right to use this file; pass --license/--source-url (and --attribution) when you know them".to_string()),
        )
    } else {
        (l.to_string(), None)
    }
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}

fn check_extension(kind: AssetKind, path: &Path) -> Result<()> {
    let ext = ext_of(path);
    let ok: &[&str] = match kind {
        AssetKind::Font => &["ttf", "otf"],
        AssetKind::Music | AssetKind::Sfx => &["mp3", "wav", "ogg", "m4a", "aac", "flac", "opus"],
        AssetKind::Image => &["png", "jpg", "jpeg", "webp", "gif"],
        AssetKind::CaptionStyle | AssetKind::Layout | AssetKind::TextStyle => &["json"],
        AssetKind::Lut => &["cube"],
        AssetKind::Template => &[],
    };
    if ok.is_empty() || ok.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(validation("asset.add", format!("a {} asset must be a .{} file, got '.{ext}'", kind.as_str(), ok.join("/.")))
            .hint("libass only loads ttf/otf fonts; convert other formats first"))
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

fn root_for(ctx: &Ctx, scope: Scope, stage: &str) -> Result<PathBuf> {
    match scope {
        Scope::Global => Ok(assets_root(&ctx.home)),
        Scope::Project => match &ctx.project_dir {
            Some(p) => Ok(project_assets_root(p)),
            None => Err(RustlerError::new(ErrorCode::NotFound, stage, "no project is open for scope \"project\"")
                .hint("pass --scope global, or --project <dir>")),
        },
    }
}

fn default_scope(ctx: &Ctx) -> Scope {
    if ctx.project_dir.is_some() { Scope::Project } else { Scope::Global }
}

fn scope_name(s: Scope) -> &'static str {
    match s {
        Scope::Project => "project",
        Scope::Global => "global",
    }
}

fn log(ctx: &Ctx, kind: &str, data: serde_json::Value) {
    if let Ok(store) = ctx.store() {
        let _ = log_event(ctx, &store, kind, data);
    }
}

fn sha256_of(path: &Path) -> Result<String> {
    rustler_tools::download::sha256_file(path).map_err(|e| io_err("asset.add", "read", path, e))
}

async fn asset_add(ctx: Ctx, i: AddIn) -> Result<AddOut> {
    const STAGE: &str = "asset.add";
    let (license, license_warning) = normalize_license(&i.license);
    let want = if matches!(i.kind, AssetKind::Music | AssetKind::Sfx) { Want::Audio } else { Want::Any };
    let (src, source_url_auto, fetch_note, pending) = match (&i.path, &i.url) {
        (Some(_), Some(_)) => {
            return Err(validation(STAGE, "give a path or --url, not both").hint("e.g. `rustler asset add song.mp3 --kind music` or `rustler asset add --url https://.../song.mp3 --kind music`"));
        }
        (None, None) => {
            return Err(validation(STAGE, "nothing to add: pass a file path or --url").hint("e.g. `rustler asset add song.mp3 --kind music` or `rustler asset add --url https://.../song.mp3 --kind music`"));
        }
        (None, Some(u)) => {
            if !is_url(u) {
                return Err(validation(STAGE, format!("--url must be an http(s) link, got `{u}`")).hint("use a direct link to the file, or pass a local path"));
            }
            let r = media_ref::resolve(&ctx, STAGE, u, want).await?;
            (r.path, Some(u.trim().to_string()), r.note, r.pending)
        }
        (Some(p), None) => {
            let s = p.to_string_lossy().into_owned();
            if is_url(&s) || s.starts_with("source:") {
                let r = media_ref::resolve(&ctx, STAGE, &s, want).await?;
                (r.path, is_url(&s).then(|| s.trim().to_string()), r.note, r.pending)
            } else {
                let r = media_ref::resolve(&ctx, STAGE, &s, want).await?;
                (r.path, None, r.note, false)
            }
        }
    };
    if !pending && !src.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("no such file: {}", src.display()))
            .hint("pass an existing local file, or --url <http(s) link>"));
    }
    check_extension(i.kind, &src)?;
    if i.kind == AssetKind::Lut {
        let text = std::fs::read_to_string(&src).map_err(|e| io_err(STAGE, "read", &src, e))?;
        rustler_media::grade::check_cube(&text).map_err(|m| validation(STAGE, format!("{}: {m}", src.display())))?;
    }
    // From here on `i.path` is the resolved local file.
    let i = AddIn { path: Some(src), source_url: i.source_url.or(source_url_auto), ..i };
    let i_path: PathBuf = i.path.clone().unwrap_or_default();
    let scope = i.scope.unwrap_or_else(|| default_scope(&ctx));
    let root = root_for(&ctx, scope, STAGE)?;
    let sha = if pending { String::new() } else { sha256_of(&i_path)? };
    let stem = i_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut name = i.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&stem).to_string();
    let mut warnings = Vec::new();
    warnings.extend(license_warning);
    warnings.extend(fetch_note);
    if license.to_lowercase().contains("by") && !license.to_lowercase().contains("cc0") && i.attribution.as_deref().is_none_or(|a| a.trim().is_empty()) {
        warnings.push("this license usually requires attribution; add --attribution \"<credit line>\"".to_string());
    }

    // Caption styles are validated and stored through the same path as caption.create_style.
    let mut style_json: Option<Vec<u8>> = None;
    if i.kind == AssetKind::CaptionStyle {
        let bytes = std::fs::read(&i_path).map_err(|e| io_err(STAGE, "read", &i_path, e))?;
        let mut s: CustomStyle = serde_json::from_slice(&bytes)
            .map_err(|e| validation(STAGE, format!("not a caption style file: {e}")).hint("create one with `rustler caption create-style`, or copy the shape of `rustler asset show style-bold-karaoke`"))?;
        if i.name.is_some() {
            s.name = slug(&name);
        }
        s.name = slug(&s.name);
        s.to_preset().map_err(|e| validation(STAGE, format!("invalid caption style: {e}")))?;
        if rustler_captions::presets::find_preset(&s.name).is_some() {
            return Err(validation(STAGE, format!("'{}' is a built-in caption style name", s.name)).hint("rename it with --name"));
        }
        name = s.name.clone();
        style_json = Some(serde_json::to_vec_pretty(&s).map_err(|e| validation(STAGE, e.to_string()))?);
    } else if i.kind == AssetKind::Layout {
        let bytes = std::fs::read(&i_path).map_err(|e| io_err(STAGE, "read", &i_path, e))?;
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| validation(STAGE, format!("not valid JSON: {e}")))?;
    }

    let existing = load_manifest(&root);
    if let Some(dup) = existing.iter().find(|e| e.kind == i.kind && !e.sha256.is_empty() && e.sha256 == sha) {
        return Err(validation(STAGE, format!("this file is already in the {} library as '{}'", scope_name(scope), dup.id))
            .hint(format!("`rustler asset show {}`; remove it first to change its license or tags", dup.id)));
    }
    let id = if i.kind == AssetKind::CaptionStyle { format!("style-{}", slug(&name)) } else { unique_id(i.kind, &name, &existing) };
    if i.kind == AssetKind::CaptionStyle && existing.iter().any(|e| e.id == id) {
        return Err(validation(STAGE, format!("a caption style called '{name}' already exists in the {} library", scope_name(scope)))
            .hint("pick another --name or `rustler asset remove` it first"));
    }

    let dir = root.join(i.kind.dir());
    let file_name = if i.kind == AssetKind::CaptionStyle {
        format!("{}.json", slug(&name))
    } else if i.kind == AssetKind::Font {
        i_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("{id}.ttf"))
    } else {
        format!("{}.{}", slug(&stem), ext_of(&i_path))
    };
    let mut dest = dir.join(&file_name);
    let mut n = 2;
    while dest.exists() && i.kind != AssetKind::CaptionStyle {
        let p = Path::new(&file_name);
        dest = dir.join(format!("{}-{n}.{}", p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), ext_of(p)));
        n += 1;
    }
    let rel = format!("{}/{}", i.kind.dir(), dest.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
    let entry = AssetEntry {
        id: id.clone(),
        kind: i.kind,
        name,
        path: rel,
        tags: clean_tags(&i.tags.unwrap_or_default()),
        license,
        license_url: i.license_url.filter(|s| !s.trim().is_empty()),
        source_url: i.source_url.filter(|s| !s.trim().is_empty()),
        attribution: i.attribution.filter(|s| !s.trim().is_empty()),
        added_at: now_rfc3339(),
        sha256: sha,
    };
    let next = vec![
        format!("rustler asset show {id}"),
        match i.kind {
            AssetKind::Music => format!("rustler audio add-music <clip_id> asset:{id}"),
            AssetKind::Sfx => format!("rustler audio add-sfx <clip_id> asset:{id}"),
            AssetKind::CaptionStyle => format!("rustler clip set-captions <clip_id> --style {}", entry.name),
            AssetKind::Lut => format!("rustler grade set <clip_id> --lut asset:{id} --lut-strength 0.6"),
            _ => "rustler asset list".to_string(),
        },
    ];
    let row = |entry: AssetEntry| {
        let file = entry_file(&root, &entry);
        AssetRow { entry, scope: scope_name(scope).into(), exists: !ctx.dry_run, file }
    };
    if ctx.dry_run {
        return Ok(AddOut { asset: row(entry), dry_run: Some(true), warnings, next });
    }
    std::fs::create_dir_all(&dir).map_err(|e| io_err(STAGE, "create", &dir, e))?;
    match style_json {
        Some(bytes) => std::fs::write(&dest, bytes).map_err(|e| io_err(STAGE, "write", &dest, e))?,
        None => {
            std::fs::copy(&i_path, &dest).map_err(|e| io_err(STAGE, "copy to", &dest, e))?;
        }
    }
    update_manifest(&root, |m| {
        upsert(m, entry.clone());
        Ok(())
    })?;
    log(&ctx, "asset.add", serde_json::json!({"id": id, "kind": entry.kind, "license": entry.license, "scope": scope_name(scope)}));
    let mut out = row(entry);
    out.exists = true;
    Ok(AddOut { asset: out, dry_run: None, warnings, next })
}

// ---------------------------------------------------------------------------------------------
// asset.remove
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct RemoveIn {
    pub id: String,
    /// Which library to remove from when the id exists in both (default: project first).
    pub scope: Option<Scope>,
}

#[derive(Serialize, JsonSchema)]
pub struct RemoveOut {
    pub removed: String,
    pub scope: String,
    /// The file was deleted (files outside the library, like registered fonts, are never deleted).
    pub file_deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn asset_remove(ctx: Ctx, i: RemoveIn) -> Result<RemoveOut> {
    const STAGE: &str = "asset.remove";
    let id = i.id.trim().to_string();
    let candidates: Vec<Scope> = match i.scope {
        Some(s) => vec![s],
        None => if ctx.project_dir.is_some() { vec![Scope::Project, Scope::Global] } else { vec![Scope::Global] },
    };
    for scope in candidates {
        let root = root_for(&ctx, scope, STAGE)?;
        let Some(entry) = load_manifest(&root).into_iter().find(|e| e.id == id) else { continue };
        if entry.is_builtin() {
            return Err(validation(STAGE, format!("'{id}' is a built-in asset and cannot be removed")));
        }
        let file = entry_file(&root, &entry);
        // Only files inside the library root are deleted.
        let inside = file.as_ref().filter(|f| f.starts_with(&root) && f.is_file()).cloned();
        if ctx.dry_run {
            return Ok(RemoveOut { removed: id, scope: scope_name(scope).into(), file_deleted: inside.is_some(), dry_run: Some(true) });
        }
        update_manifest(&root, |m| {
            m.retain(|e| e.id != id);
            Ok(())
        })?;
        let mut deleted = false;
        if let Some(f) = inside {
            deleted = std::fs::remove_file(&f).is_ok();
        }
        let _ = std::fs::remove_file(preview_path(&ctx.home, &AssetRow { entry: entry.clone(), scope: scope_name(scope).into(), file: None, exists: false }));
        log(&ctx, "asset.remove", serde_json::json!({"id": id, "scope": scope_name(scope)}));
        return Ok(RemoveOut { removed: id, scope: scope_name(scope).into(), file_deleted: deleted, dry_run: None });
    }
    // Not in a manifest: say whether it is a built-in, else not found.
    if builtin_entries().iter().any(|e| e.id == id) {
        return Err(validation(STAGE, format!("'{id}' is a built-in asset and cannot be removed")));
    }
    Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("no asset with id '{id}' in the selected library"))
        .hint("`rustler asset list` shows ids and scopes"))
}

// ---------------------------------------------------------------------------------------------
// caption.create_style / caption.styles
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct CreateStyleIn {
    /// Name of the new style (lowercased, spaces/underscores become `-`); cannot be a built-in name.
    pub name: String,
    /// Style to start from: a built-in or custom style name (default bold-karaoke).
    pub base: Option<String>,
    /// Font family name as libass sees it (e.g. "Anton", "Poppins ExtraBold").
    pub font_family: Option<String>,
    /// Font size as a fraction of the canvas width (0.02..0.15; 0.072 is Anton-sized).
    pub size_factor: Option<f64>,
    /// Text colour `#RRGGBB`.
    pub primary: Option<String>,
    /// Outline colour `#RRGGBB`.
    pub outline: Option<String>,
    /// Outline width in px at 1080 wide.
    pub outline_width: Option<f64>,
    /// Shadow depth in px.
    pub shadow: Option<f64>,
    pub uppercase: Option<bool>,
    /// How the spoken word is shown: recolor | box | pop.
    pub active_kind: Option<ActiveKind>,
    /// Spoken-word colour `#RRGGBB` (recolor/pop).
    pub active_color: Option<String>,
    /// Box colour `#RRGGBB` (active_kind box).
    pub box_color: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    /// `project` (default when a project is open) or `global`.
    pub scope: Option<Scope>,
}

#[derive(Serialize, JsonSchema)]
pub struct CreateStyleOut {
    pub style: CustomStyle,
    pub asset_id: String,
    pub scope: String,
    pub file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn font_known(home: &Path, project: Option<&Path>, family: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    let want = norm(family);
    let mut dirs = vec![ToolPaths::from_home(home).fonts];
    dirs.extend(asset_font_dirs(home, project));
    dirs.iter().flat_map(|d| find_fonts(d)).any(|f| f.file_stem().is_some_and(|st| norm(&st.to_string_lossy()).contains(&want)))
}

fn colour(stage: &str, label: &str, v: &str) -> Result<String> {
    rustler_captions::custom::parse_hex_colour(v)
        .map(|_| v.trim().to_string())
        .ok_or_else(|| validation(stage, format!("{label} '{v}' is not a colour")).hint("use #RRGGBB, e.g. #FFE000"))
}

async fn caption_create_style(ctx: Ctx, i: CreateStyleIn) -> Result<CreateStyleOut> {
    const STAGE: &str = "caption.create_style";
    let name = slug(&i.name);
    if !i.name.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(validation(STAGE, "name must contain letters or digits").hint("e.g. `rustler caption create-style neon-pop --base hormozi-pop --active-color #00FFFF`"));
    }
    if rustler_captions::presets::find_preset(&name).is_some() {
        return Err(validation(STAGE, format!("'{name}' is a built-in style name")).hint("choose another name; built-in styles cannot be overwritten"));
    }
    let scope = i.scope.unwrap_or_else(|| default_scope(&ctx));
    let root = root_for(&ctx, scope, STAGE)?;
    let project = ctx.project_dir.as_deref();

    let custom = custom_presets(&ctx.home, project);
    let base_name = i.base.as_deref().map(str::trim).filter(|b| !b.is_empty()).unwrap_or("bold-karaoke");
    let base = find_preset_in(base_name, &custom).ok_or_else(|| {
        validation(STAGE, format!("unknown base style '{base_name}'")).hint(format!("`rustler caption styles` lists them; built-in: {}", rustler_captions::PRESETS.join(", ")))
    })?;
    let mut s = CustomStyle::from_preset(base);
    s.name = name.clone();
    s.base = Some(base_name.to_string());
    s.description = i.description.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| format!("Custom style based on {base_name}."));
    let mut warnings = Vec::new();
    if let Some(f) = i.font_family.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        s.font_family = f.to_string();
    }
    if let Some(v) = i.size_factor {
        s.size_factor = v;
    }
    if let Some(v) = &i.primary {
        s.primary = colour(STAGE, "primary", v)?;
    }
    if let Some(v) = &i.outline {
        s.outline_colour = colour(STAGE, "outline", v)?;
    }
    if let Some(v) = i.outline_width {
        s.outline = v;
    }
    if let Some(v) = i.shadow {
        s.shadow = v;
    }
    if let Some(v) = i.uppercase {
        s.uppercase = v;
    }
    let kind = i.active_kind.unwrap_or(s.active_kind);
    let base_kind = s.active_kind;
    s.active_kind = kind;
    match kind {
        ActiveKind::Box => {
            if let Some(v) = i.box_color.as_ref().or(i.active_color.as_ref()) {
                s.active_colour = colour(STAGE, "box_color", v)?;
            }
            if s.box_padding <= 0.0 {
                s.box_padding = 12.0;
            }
        }
        _ => {
            if let Some(v) = &i.active_color {
                s.active_colour = colour(STAGE, "active_color", v)?;
            }
            if i.box_color.is_some() {
                warnings.push("box_color is only used with active_kind box; ignored".to_string());
            }
        }
    }
    if kind != base_kind && i.active_color.is_none() && i.box_color.is_none() {
        warnings.push(format!("active_kind changed from {base_kind:?} to {kind:?}; the base's active colour was kept (set --active-color / --box-color)"));
    }
    s.to_preset().map_err(|e| validation(STAGE, format!("invalid style: {e}")).hint("check the colours (#RRGGBB) and numeric ranges"))?;
    if !font_known(&ctx.home, project, &s.font_family) {
        warnings.push(format!(
            "font family '{}' was not found among the installed/asset fonts; libass will substitute another font. Add it with `rustler asset add <file.ttf> --kind font --license <license>`",
            s.font_family
        ));
    }

    let id = format!("style-{name}");
    let rel = format!("{}/{name}.json", AssetKind::CaptionStyle.dir());
    let file = root.join(&rel);
    let mut tags = vec!["caption".to_string(), "custom".to_string(), s.font_family.to_lowercase()];
    tags.extend(i.tags.clone().unwrap_or_default());
    let entry = AssetEntry {
        id: id.clone(),
        kind: AssetKind::CaptionStyle,
        name: name.clone(),
        path: rel,
        tags: clean_tags(&tags),
        license: "user-created".into(),
        license_url: None,
        source_url: None,
        attribution: None,
        added_at: now_rfc3339(),
        sha256: String::new(),
    };
    let next = vec![
        format!("rustler asset show {id}"),
        format!("rustler clip set-captions <clip_id> --style {name}"),
    ];
    if ctx.dry_run {
        return Ok(CreateStyleOut { style: s, asset_id: id, scope: scope_name(scope).into(), file, dry_run: Some(true), warnings, next });
    }
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
    std::fs::create_dir_all(&dir).map_err(|e| io_err(STAGE, "create", &dir, e))?;
    let body = serde_json::to_vec_pretty(&s).map_err(|e| validation(STAGE, e.to_string()))?;
    std::fs::write(&file, &body).map_err(|e| io_err(STAGE, "write", &file, e))?;
    let mut entry = entry;
    entry.sha256 = rustler_tools::download::sha256_bytes(&body);
    update_manifest(&root, |m| {
        upsert(m, entry);
        Ok(())
    })?;
    log(&ctx, "caption.create_style", serde_json::json!({"name": name, "scope": scope_name(scope)}));
    Ok(CreateStyleOut { style: s, asset_id: id, scope: scope_name(scope).into(), file, dry_run: None, warnings, next })
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct StylesIn {}

#[derive(Serialize, JsonSchema)]
pub struct StylesOut {
    pub styles: Vec<StyleInfo>,
    pub next: Vec<String>,
}

async fn caption_styles(ctx: Ctx, _i: StylesIn) -> Result<StylesOut> {
    let styles = style_infos(&ctx.home, ctx.project_dir.as_deref());
    let next = vec![
        "rustler asset show style-<name>   (renders a preview frame)".to_string(),
        "rustler caption create-style <name> --base hormozi-pop --active-color #00FFFF".to_string(),
    ];
    Ok(StylesOut { styles, next })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

pub fn register(reg: &mut Registry) {
    let ro = Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false };
    let write = Annotations { read_only: false, ..ro };
    // `asset.add --url` downloads
    let write_net = Annotations { network: true, ..write };
    reg.register(typed(
        "asset.list",
        "List library assets (fonts, music, sfx, images, caption styles, layouts) with license metadata; global + project merged",
        &[],
        ro,
        vec![
            Example { description: "all sound effects", cli: "rustler asset list --kind sfx" },
            Example { description: "everything tagged whoosh", cli: "rustler asset list --tag whoosh" },
        ],
        asset_list,
    ));
    reg.register(typed(
        "asset.search",
        "Fuzzy-search assets by name, tag and id",
        &["query"],
        ro,
        vec![Example { description: "find an impact hit", cli: "rustler asset search impact punch --kind sfx" }],
        asset_search,
    ));
    reg.register(typed(
        "asset.show",
        "Show one asset with a preview: a sample PNG for fonts and caption styles (burned on a gray 1080x1920 frame), duration for audio",
        &["id"],
        // Writes a preview PNG into the library's previews/ cache.
        write,
        vec![Example { description: "preview a caption style", cli: "rustler asset show style-hormozi-pop" }],
        asset_show,
    ));
    reg.register(typed(
        "asset.add",
        "Add anything to the library: a local file, or --url <direct http(s) link> (audio, image, font, or a video whose audio becomes music/sfx), or source:<id>[@start-end] (audio of an imported video). --license is optional (missing = recorded as unspecified, with a warning)",
        &["path"],
        write_net,
        vec![
            Example { description: "your own music, no license metadata needed", cli: "rustler asset add song.mp3 --kind music --tags chill" },
            Example { description: "music from a direct link", cli: "rustler asset add --url https://example.org/lofi.mp3 --kind music --tags lofi --license CC-BY-4.0 --attribution \"Artist - Title\"" },
            Example { description: "reuse the audio of an imported video as music", cli: "rustler asset add source:src_01k...@30-75 --kind music --name stream-bgm" },
            Example { description: "a CC0 whoosh into the project", cli: "rustler asset add whoosh.wav --kind sfx --license CC0-1.0 --tags whoosh,transition --source-url https://example.org/whoosh" },
            Example { description: "an OFL font for everyone", cli: "rustler asset add Bangers-Regular.ttf --kind font --license OFL-1.1 --scope global" },
        ],
        asset_add,
    ));
    reg.register(typed(
        "asset.remove",
        "Remove an asset from the project or global library (built-in assets cannot be removed)",
        &["id"],
        Annotations { destructive: true, ..write },
        vec![Example { description: "remove a sound", cli: "rustler asset remove sfx-whoosh" }],
        asset_remove,
    ));
    reg.register(typed(
        "caption.create_style",
        "Create a custom caption style from a base preset (font, size, colours, outline, active-word look); saved as an asset and usable by name in clip.set_captions and render",
        &["name"],
        write,
        vec![
            Example { description: "cyan spoken word on the hormozi look", cli: "rustler caption create-style neon-pop --base hormozi-pop --active-color #00FFFF" },
            Example { description: "yellow box behind the spoken word", cli: "rustler caption create-style yellow-box --base boxed-highlight --box-color #FFD400 --primary #000000 --outline #FFFFFF" },
        ],
        caption_create_style,
    ));
    reg.register(typed(
        "caption.styles",
        "List caption styles: the built-in presets plus saved custom styles (with scope, font and active-word kind)",
        &[],
        ro,
        vec![Example { description: "all styles", cli: "rustler caption styles" }],
        caption_styles,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, tags: &[&str]) -> AssetRow {
        AssetRow {
            entry: AssetEntry {
                id: format!("sfx-{}", slug(name)),
                kind: AssetKind::Sfx,
                name: name.into(),
                path: "x.ogg".into(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                license: "CC0-1.0".into(),
                license_url: None,
                source_url: None,
                attribution: None,
                added_at: String::new(),
                sha256: String::new(),
            },
            scope: "global".into(),
            file: None,
            exists: true,
        }
    }

    #[test]
    fn fuzzy_scores() {
        let r = row("Kenney Swipe up", &["swipe", "whoosh"]);
        let q = |s: &str| words(s);
        assert!(match_score(&q("whoosh"), &r) >= 0.99);
        assert!(match_score(&q("swiping"), &r) >= 0.5, "stem-ish match");
        assert!(match_score(&q("woosh"), &r) >= 0.5, "typo");
        assert!(match_score(&q("guitar"), &r) < 0.5);
    }

    #[test]
    fn license_is_optional_and_never_refused() {
        assert_eq!(normalize_license(" CC0-1.0 "), ("CC0-1.0".to_string(), None));
        assert_eq!(normalize_license("anything goes: bought on a stock site").0, "anything goes: bought on a stock site");
        for blank in ["", "  ", "Unknown", "N/A", "unlicensed"] {
            let (l, w) = normalize_license(blank);
            assert_eq!(l, "unspecified (added by user/agent)", "{blank}");
            assert!(w.unwrap().contains("responsible"), "{blank}");
        }
    }

    #[test]
    fn builtin_license_constant_is_builtin() {
        assert!(builtin_entries().iter().all(|e| e.license == rustler_tools::assets::LICENSE_BUILTIN));
    }
}
