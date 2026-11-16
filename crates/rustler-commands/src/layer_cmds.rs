//! Commands: image.add/list/edit/remove, mask.add/list/edit/remove, cutaway.add/list/edit/remove,
//! caption.word, caption.words_list, caption.words_clear.
//!
//! The v2 layer kinds (design §5.1), each stored under its own `extras.json` key through
//! [`crate::extras`] (`images`, `masks`, `cutaways`, `caption_word_overrides`; other keys untouched).
//! `render.run` composites them in the same single encode ([`apply_to_compile`],
//! [`apply_word_overrides`]).
//!
//! Times are on the *edit timeline* (what `clip show` prints), like overlays and effects; the
//! render maps them through the transitions. Instead of seconds an item can hang on transcript
//! words (`start_word` / `end_word` = `{source_id, word_i}`): those follow re-cuts, and an item
//! whose word was cut out is skipped with a warning (never shown at a wrong time).
//!
//! Every write takes an optional `base_revision` (fails with `revision_conflict` unless the clip
//! is at that revision), honours `--dry-run` (validates and returns the result, saves nothing)
//! and logs an event.

use crate::audio_cmds::{change, next_id};
use crate::clip_cmds::{load_transcript, log_event};
use crate::media_ref::{self, VIDEO_EXTS, Want, is_url};
use crate::{extras, fx_cmds};
use rustler_captions::{CaptionWord, WordPaint};
use rustler_core::{Clip, ClipId, ErrorCode, Fit, ProjectStore, Rect, RustlerError, SourceId, TimeMap, Word};
use rustler_media::compile::{CompileExtras, TransitionMap};
use rustler_media::layers::{
    Anchor, Anim, AnimKind, CAPTION_WORDS_KEY, CUTAWAYS_KEY, CaptionWordOverride, Cutaway, CutawayMedia, CutawayMode,
    IMAGE_EXTS, IMAGES_KEY, ImageLayer, MASKS_KEY, MaskKind, MaskLayer, Timing, check_cutaway, check_image, check_mask,
    check_word_override, ext_of, is_still, resolve_timing, resolved, word_spans,
};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

/// End may exceed the clip duration by this much (rounding).
const END_TOLERANCE: f64 = 0.05;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------------------------

/// A stored layer item (image, mask, cutaway).
pub trait LayerItem: Serialize + DeserializeOwned + JsonSchema + Clone + Send + 'static {
    const KEY: &'static str;
    const PREFIX: &'static str;
    const GROUP: &'static str;
    fn id(&self) -> &str;
    fn set_id(&mut self, id: String);
    fn timing(&self) -> &Timing;
    fn timing_mut(&mut self) -> &mut Timing;
    fn check(&self) -> std::result::Result<(), String>;
}

impl LayerItem for ImageLayer {
    const KEY: &'static str = IMAGES_KEY;
    const PREFIX: &'static str = "img";
    const GROUP: &'static str = "image";
    fn id(&self) -> &str {
        &self.id
    }
    fn set_id(&mut self, id: String) {
        self.id = id;
    }
    fn timing(&self) -> &Timing {
        &self.timing
    }
    fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }
    fn check(&self) -> std::result::Result<(), String> {
        check_image(self)
    }
}

impl LayerItem for MaskLayer {
    const KEY: &'static str = MASKS_KEY;
    const PREFIX: &'static str = "msk";
    const GROUP: &'static str = "mask";
    fn id(&self) -> &str {
        &self.id
    }
    fn set_id(&mut self, id: String) {
        self.id = id;
    }
    fn timing(&self) -> &Timing {
        &self.timing
    }
    fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }
    fn check(&self) -> std::result::Result<(), String> {
        check_mask(self)
    }
}

impl LayerItem for Cutaway {
    const KEY: &'static str = CUTAWAYS_KEY;
    const PREFIX: &'static str = "cut";
    const GROUP: &'static str = "cutaway";
    fn id(&self) -> &str {
        &self.id
    }
    fn set_id(&mut self, id: String) {
        self.id = id;
    }
    fn timing(&self) -> &Timing {
        &self.timing
    }
    fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }
    fn check(&self) -> std::result::Result<(), String> {
        check_cutaway(self)
    }
}

/// Fails with `revision_conflict` unless the clip is at `base`.
pub fn check_revision(stage: &str, clip: &Clip, base: Option<u64>) -> Result<()> {
    match base {
        Some(r) if r != clip.revision => Err(RustlerError::new(
            ErrorCode::RevisionConflict,
            stage,
            format!("clip {} is at revision {} but base_revision was {r}", clip.id, clip.revision),
        )
        .hint("reload the clip (`rustler clip show <clip_id>`) and retry with its revision")),
        _ => Ok(()),
    }
}

/// Transcript words of every source in `sources` (sources without a transcript are left out).
fn words_for(store: &ProjectStore, sources: impl IntoIterator<Item = SourceId>) -> Result<HashMap<SourceId, Vec<Word>>> {
    let mut words = HashMap::new();
    for s in sources {
        if !words.contains_key(&s)
            && let Some(t) = load_transcript(store, &s)?
        {
            words.insert(s, t.words);
        }
    }
    Ok(words)
}

fn anchors_of(t: &Timing) -> impl Iterator<Item = SourceId> + '_ {
    t.start_word.iter().chain(t.end_word.iter()).map(|a| a.source_id.clone())
}

/// The clip and its two timelines.
struct Env {
    store: ProjectStore,
    clip: Clip,
    tm: TimeMap,
    tmap: TransitionMap,
}

impl Env {
    fn load(ctx: &Ctx, stage: &str, clip_id: &ClipId, base_revision: Option<u64>) -> Result<Self> {
        let store = ctx.store()?;
        let clip = store.load_clip(clip_id)?;
        check_revision(stage, &clip, base_revision)?;
        let tm = TimeMap::from_segments(&clip.segments);
        let tmap = fx_cmds::transition_map(&store, &clip)?;
        Ok(Self { store, clip, tm, tmap })
    }

    fn duration(&self) -> f64 {
        self.tm.duration()
    }

    /// Resolved window; `Err(reason)` when it does not resolve.
    fn when(&self, t: &Timing) -> Result<std::result::Result<WhenView, String>> {
        let words = words_for(&self.store, anchors_of(t))?;
        Ok(resolve_timing(t, &self.tm, &words).and_then(|(s, e)| {
            let (rs, re) = self
                .tmap
                .map_interval(s, e)
                .ok_or_else(|| format!("{s:.2}..{e:.2}s lies inside the hidden half of a transition"))?;
            Ok(WhenView { start: s, end: e, rendered_start: rs, rendered_end: re })
        }))
    }

    /// Validation of a new or edited item's timing.
    fn check_timing(&self, stage: &str, t: &Timing) -> Result<WhenView> {
        let d = self.duration();
        if let Some(e) = t.out_end
            && t.end_word.is_none()
            && e > d + END_TOLERANCE
        {
            return Err(validation(stage, format!("end {e:.2}s is past the end of the clip ({d:.2}s)"))
                .hint("`rustler clip show <clip_id>` prints the edit timeline; leave end out for 'until the end'"));
        }
        self.when(t)?.map_err(|reason| {
            let hint = if t.has_anchor() {
                "`rustler transcript get <source_id>` lists word indices (word_i); the word must be inside the clip"
            } else {
                "times are edit-timeline seconds (what `rustler clip show <clip_id>` prints)"
            };
            validation(stage, reason).hint(hint)
        })
    }
}

/// Edit-timeline window of an item and where it lands in the rendered file.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct WhenView {
    /// Edit-timeline start (seconds).
    pub start: f64,
    pub end: f64,
    /// Rendered-file window (after transitions).
    pub rendered_start: f64,
    pub rendered_end: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct ItemView<T: JsonSchema> {
    #[serde(flatten)]
    pub item: T,
    /// Resolved window; absent when the item will not render (see `problem`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<WhenView>,
    /// Why the item will not render (e.g. its anchor word was cut out).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

fn item_view<T: LayerItem>(env: &Env, item: T) -> Result<ItemView<T>> {
    let (when, problem) = match env.when(item.timing())? {
        Ok(w) => (Some(w), None),
        Err(p) => (None, Some(p)),
    };
    Ok(ItemView { item, when, problem })
}

#[derive(Serialize, JsonSchema)]
pub struct ItemOut<T: JsonSchema> {
    pub clip_id: ClipId,
    pub id: String,
    pub item: ItemView<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ItemList<T: JsonSchema> {
    pub clip_id: ClipId,
    /// Edit-timeline duration (seconds).
    pub duration: f64,
    /// Rendered duration (after transitions).
    pub rendered_duration: f64,
    pub items: Vec<ItemView<T>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct RemoveOut {
    pub clip_id: ClipId,
    pub removed: String,
    /// Ids left.
    pub remaining: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

fn next_steps(clip: &ClipId, w: Option<&WhenView>) -> Vec<String> {
    let t = w.map_or(0.0, |w| ((w.start + w.end) / 2.0).min(w.start + 1.0));
    vec![
        format!("rustler render preview-frame {clip} --out-t {t:.2}"),
        format!("rustler render run {clip} --quality draft"),
    ]
}

fn not_found<T: LayerItem>(stage: &str, clip: &ClipId, id: &str, list: &[T]) -> RustlerError {
    let ids: Vec<&str> = list.iter().map(|o| o.id()).collect();
    RustlerError::new(ErrorCode::NotFound, stage, format!("clip {clip} has no {} '{id}'", T::GROUP)).hint(if ids.is_empty() {
        format!("the clip has no {}s; add one with `rustler {} add`", T::GROUP, T::GROUP)
    } else {
        format!("{}s: {}", T::GROUP, ids.join(", "))
    })
}

/// Validates and stores a new item (id assigned here).
fn add_item<T: LayerItem>(ctx: &Ctx, env: &Env, stage: &str, mut item: T, warnings: Vec<String>) -> Result<ItemOut<T>> {
    item.check().map_err(|m| validation(stage, m))?;
    let when = env.check_timing(stage, item.timing())?;
    let item = change(ctx, &env.store, &env.clip.id, T::KEY, Vec::is_empty, |list: &mut Vec<T>| {
        item.set_id(next_id(T::PREFIX, list.iter().map(|x| x.id())));
        list.push(item.clone());
        Ok(item.clone())
    })?;
    if !ctx.dry_run {
        log_event(ctx, &env.store, stage, serde_json::json!({"clip_id": env.clip.id, "id": item.id()}))?;
    }
    Ok(ItemOut {
        clip_id: env.clip.id.clone(),
        id: item.id().to_string(),
        next: next_steps(&env.clip.id, Some(&when)),
        item: ItemView { item, when: Some(when), problem: None },
        dry_run: ctx.dry_run.then_some(true),
        warnings,
    })
}

/// Applies `f` to item `id`, validates and stores it.
fn edit_item<T: LayerItem>(
    ctx: &Ctx,
    env: &Env,
    stage: &str,
    id: &str,
    f: impl FnOnce(&mut T) -> Result<()>,
) -> Result<ItemOut<T>> {
    let item = change(ctx, &env.store, &env.clip.id, T::KEY, Vec::is_empty, |list: &mut Vec<T>| {
        let Some(it) = list.iter_mut().find(|x| x.id() == id) else {
            return Err(not_found(stage, &env.clip.id, id, list));
        };
        let mut new = it.clone();
        f(&mut new)?;
        new.check().map_err(|m| validation(stage, m))?;
        env.check_timing(stage, new.timing())?;
        *it = new.clone();
        Ok(new)
    })?;
    if !ctx.dry_run {
        log_event(ctx, &env.store, stage, serde_json::json!({"clip_id": env.clip.id, "id": id}))?;
    }
    let v = item_view(env, item)?;
    Ok(ItemOut {
        clip_id: env.clip.id.clone(),
        id: id.to_string(),
        next: next_steps(&env.clip.id, v.when.as_ref()),
        item: v,
        dry_run: ctx.dry_run.then_some(true),
        warnings: Vec::new(),
    })
}

fn list_items<T: LayerItem>(env: &Env) -> Result<ItemList<T>> {
    let items: Vec<T> = extras::get(&env.store, &env.clip.id, T::KEY)?.unwrap_or_default();
    let mut warnings = Vec::new();
    let mut views = Vec::new();
    for it in items {
        let v = item_view(env, it)?;
        if let Some(p) = &v.problem {
            warnings.push(format!("{} will not render: {p}", v.item.id()));
        }
        views.push(v);
    }
    Ok(ItemList {
        clip_id: env.clip.id.clone(),
        duration: env.duration(),
        rendered_duration: env.tmap.duration(),
        items: views,
        warnings,
    })
}

fn remove_item<T: LayerItem>(ctx: &Ctx, env: &Env, stage: &str, id: &str) -> Result<RemoveOut> {
    let remaining = change(ctx, &env.store, &env.clip.id, T::KEY, Vec::is_empty, |list: &mut Vec<T>| {
        let Some(pos) = list.iter().position(|x| x.id() == id) else {
            return Err(not_found(stage, &env.clip.id, id, list));
        };
        list.remove(pos);
        Ok(list.iter().map(|x| x.id().to_string()).collect::<Vec<_>>())
    })?;
    if !ctx.dry_run {
        log_event(ctx, &env.store, stage, serde_json::json!({"clip_id": env.clip.id, "id": id}))?;
    }
    Ok(RemoveOut { clip_id: env.clip.id.clone(), removed: id.to_string(), remaining, dry_run: ctx.dry_run.then_some(true) })
}

/// Timing from command input: seconds or word anchors (a word wins over seconds on the same side).
fn timing_from(start: Option<f64>, end: Option<f64>, start_word: Option<Anchor>, end_word: Option<Anchor>) -> Timing {
    Timing {
        out_start: if start_word.is_some() { None } else { start },
        out_end: if end_word.is_some() { None } else { end },
        start_word,
        end_word,
    }
}

/// Edit of a timing: a new second value replaces that side's word anchor and vice versa.
fn edit_timing(t: &mut Timing, start: Option<f64>, end: Option<f64>, start_word: Option<Anchor>, end_word: Option<Anchor>) {
    if let Some(s) = start {
        t.out_start = Some(s);
        t.start_word = None;
    }
    if let Some(a) = start_word {
        t.start_word = Some(a);
        t.out_start = None;
    }
    if let Some(e) = end {
        t.out_end = Some(e);
        t.end_word = None;
    }
    if let Some(a) = end_word {
        t.end_word = Some(a);
        t.out_end = None;
    }
}

/// A visual file from exactly one of `asset` / `file` / `url`: `(asset id, absolute path)`.
/// In dry-run a URL is not downloaded (the path is where it would land).
async fn resolve_visual(
    ctx: &Ctx,
    stage: &str,
    asset: Option<&str>,
    file: Option<&str>,
    url: Option<&str>,
    exts: &[&str],
) -> Result<(Option<String>, PathBuf)> {
    let given = [asset.is_some(), file.is_some(), url.is_some()].iter().filter(|b| **b).count();
    if given != 1 {
        return Err(validation(stage, "give exactly one of asset, file or url")
            .hint("e.g. --file logo.png, --asset ast_x (`rustler asset list`), or --url https://.../logo.png"));
    }
    let (asset_id, spec) = match (asset, file, url) {
        (Some(a), _, _) => {
            let id = a.trim().trim_start_matches("asset:").trim().to_string();
            (Some(id.clone()), format!("asset:{id}"))
        }
        (_, Some(f), _) => (None, f.trim().to_string()),
        (_, _, Some(u)) => {
            if !is_url(u) {
                return Err(validation(stage, format!("url must be an http(s) link, got `{u}`")));
            }
            (None, u.trim().to_string())
        }
        _ => unreachable!(),
    };
    let r = media_ref::resolve(ctx, stage, &spec, Want::Any).await?;
    let path = std::path::absolute(&r.path).unwrap_or(r.path);
    if !r.pending && !path.is_file() {
        return Err(RustlerError::new(ErrorCode::NotFound, stage, format!("no such file: {}", path.display()))
            .hint("pass an existing file, a library asset id or an http(s) url"));
    }
    let ext = ext_of(&path);
    if !r.pending && !exts.contains(&ext.as_str()) {
        return Err(validation(stage, format!("unsupported file type '.{ext}' ({})", path.display()))
            .hint(format!("supported: {}", exts.join(", "))));
    }
    Ok((asset_id, path))
}

// ---------------------------------------------------------------------------------------------
// image.*
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct ImageAddIn {
    pub clip_id: ClipId,
    /// Library asset id (`rustler asset list`); give this, file or url.
    pub asset: Option<String>,
    /// Image file: png, jpg, webp, gif (animated, loops) or bmp.
    pub file: Option<String>,
    /// Direct http(s) link to an image (downloaded into the project's asset cache).
    pub url: Option<String>,
    /// Box on the canvas, normalized 0..1 `{x, y, w, h}`; w and h > 0.02. The image is fitted inside.
    pub rect: Rect,
    /// Edit-timeline start in seconds (default 0).
    pub start: Option<f64>,
    /// Edit-timeline end in seconds (default: end of the clip).
    pub end: Option<f64>,
    /// Start on a transcript word `{source_id, word_i}` instead (follows re-cuts).
    pub start_word: Option<Anchor>,
    /// End on a transcript word (default with start_word only: the end of that word).
    pub end_word: Option<Anchor>,
    /// Extra scale around the rect centre (0.05..5, default 1).
    pub scale: Option<f64>,
    /// 0..1 (default 1).
    pub opacity: Option<f64>,
    /// Degrees clockwise (default 0).
    pub rotation: Option<f64>,
    /// contain (default) | cover.
    pub fit: Option<Fit>,
    /// none (default) | fade | pop | slide_up
    pub anim_in: Option<AnimKind>,
    /// none (default) | fade | pop | slide_up
    pub anim_out: Option<AnimKind>,
    /// Seconds per animation (default 0.3).
    pub anim_duration: Option<f64>,
    /// Stacking order among images (higher on top; default 0).
    pub z: Option<i32>,
    /// Draw above the captions and text overlays (default false: below the captions).
    pub above_captions: Option<bool>,
    /// Fail with revision_conflict unless the clip is at this revision.
    pub base_revision: Option<u64>,
}

async fn image_add(ctx: Ctx, i: ImageAddIn) -> Result<ItemOut<ImageLayer>> {
    const STAGE: &str = "image.add";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let (asset, file) = resolve_visual(&ctx, STAGE, i.asset.as_deref(), i.file.as_deref(), i.url.as_deref(), &IMAGE_EXTS).await?;
    let anim = Anim {
        in_: i.anim_in.unwrap_or_default(),
        out: i.anim_out.unwrap_or_default(),
        duration: i.anim_duration.unwrap_or(Anim::default().duration),
    };
    let item = ImageLayer {
        id: String::new(),
        timing: timing_from(i.start, i.end, i.start_word, i.end_word),
        asset,
        file,
        rect: i.rect,
        scale: i.scale.unwrap_or(1.0),
        opacity: i.opacity.unwrap_or(1.0),
        rotation: i.rotation.unwrap_or(0.0),
        fit: i.fit.unwrap_or(Fit::Contain),
        anim,
        z: i.z.unwrap_or(0),
        above_captions: i.above_captions.unwrap_or(false),
    };
    let mut warnings = Vec::new();
    if item.opacity < 0.05 {
        warnings.push("opacity below 0.05 is practically invisible".into());
    }
    add_item(&ctx, &env, STAGE, item, warnings)
}

#[derive(Deserialize, JsonSchema)]
pub struct ClipIn {
    pub clip_id: ClipId,
}

async fn image_list(ctx: Ctx, i: ClipIn) -> Result<ItemList<ImageLayer>> {
    list_items(&Env::load(&ctx, "image.list", &i.clip_id, None)?)
}

#[derive(Deserialize, JsonSchema)]
pub struct ImageEditIn {
    pub clip_id: ClipId,
    pub image_id: String,
    /// Replace the image: asset, file or url (one).
    pub asset: Option<String>,
    pub file: Option<String>,
    pub url: Option<String>,
    pub rect: Option<Rect>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub start_word: Option<Anchor>,
    pub end_word: Option<Anchor>,
    pub scale: Option<f64>,
    pub opacity: Option<f64>,
    pub rotation: Option<f64>,
    pub fit: Option<Fit>,
    pub anim_in: Option<AnimKind>,
    pub anim_out: Option<AnimKind>,
    pub anim_duration: Option<f64>,
    pub z: Option<i32>,
    pub above_captions: Option<bool>,
    pub base_revision: Option<u64>,
}

async fn image_edit(ctx: Ctx, i: ImageEditIn) -> Result<ItemOut<ImageLayer>> {
    const STAGE: &str = "image.edit";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let media = if i.asset.is_some() || i.file.is_some() || i.url.is_some() {
        Some(resolve_visual(&ctx, STAGE, i.asset.as_deref(), i.file.as_deref(), i.url.as_deref(), &IMAGE_EXTS).await?)
    } else {
        None
    };
    edit_item(&ctx, &env, STAGE, &i.image_id, |im: &mut ImageLayer| {
        if let Some((asset, file)) = media {
            im.asset = asset;
            im.file = file;
        }
        edit_timing(&mut im.timing, i.start, i.end, i.start_word.clone(), i.end_word.clone());
        if let Some(r) = i.rect {
            im.rect = r;
        }
        if let Some(v) = i.scale {
            im.scale = v;
        }
        if let Some(v) = i.opacity {
            im.opacity = v;
        }
        if let Some(v) = i.rotation {
            im.rotation = v;
        }
        if let Some(v) = i.fit {
            im.fit = v;
        }
        if let Some(v) = i.anim_in {
            im.anim.in_ = v;
        }
        if let Some(v) = i.anim_out {
            im.anim.out = v;
        }
        if let Some(v) = i.anim_duration {
            im.anim.duration = v;
        }
        if let Some(v) = i.z {
            im.z = v;
        }
        if let Some(v) = i.above_captions {
            im.above_captions = v;
        }
        Ok(())
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct ImageRemoveIn {
    pub clip_id: ClipId,
    pub image_id: String,
    pub base_revision: Option<u64>,
}

async fn image_remove(ctx: Ctx, i: ImageRemoveIn) -> Result<RemoveOut> {
    let env = Env::load(&ctx, "image.remove", &i.clip_id, i.base_revision)?;
    remove_item::<ImageLayer>(&ctx, &env, "image.remove", &i.image_id)
}

// ---------------------------------------------------------------------------------------------
// mask.*
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct MaskAddIn {
    pub clip_id: ClipId,
    /// Area of the canvas to hide, normalized 0..1 `{x, y, w, h}`; w and h > 0.02.
    pub rect: Rect,
    /// blur | box | pixelate
    pub kind: MaskKind,
    /// Edit-timeline start in seconds (default 0).
    pub start: Option<f64>,
    /// Edit-timeline end in seconds (default: end of the clip).
    pub end: Option<f64>,
    pub start_word: Option<Anchor>,
    pub end_word: Option<Anchor>,
    /// blur: radius px (default 20); pixelate: block px (default 24); 1..100.
    pub strength: Option<f64>,
    /// box: `#RRGGBB` (default #000000).
    pub color: Option<String>,
    pub base_revision: Option<u64>,
}

async fn mask_add(ctx: Ctx, i: MaskAddIn) -> Result<ItemOut<MaskLayer>> {
    const STAGE: &str = "mask.add";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let mut warnings = Vec::new();
    if i.kind != MaskKind::Box && i.color.is_some() {
        warnings.push("color is only used by box masks".into());
    }
    if i.kind == MaskKind::Box && i.strength.is_some() {
        warnings.push("strength is not used by box masks".into());
    }
    let item = MaskLayer {
        id: String::new(),
        timing: timing_from(i.start, i.end, i.start_word, i.end_word),
        rect: i.rect,
        kind: i.kind,
        strength: i.strength,
        color: i.color,
    };
    add_item(&ctx, &env, STAGE, item, warnings)
}

async fn mask_list(ctx: Ctx, i: ClipIn) -> Result<ItemList<MaskLayer>> {
    list_items(&Env::load(&ctx, "mask.list", &i.clip_id, None)?)
}

#[derive(Deserialize, JsonSchema)]
pub struct MaskEditIn {
    pub clip_id: ClipId,
    pub mask_id: String,
    pub rect: Option<Rect>,
    pub kind: Option<MaskKind>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub start_word: Option<Anchor>,
    pub end_word: Option<Anchor>,
    pub strength: Option<f64>,
    pub color: Option<String>,
    pub base_revision: Option<u64>,
}

async fn mask_edit(ctx: Ctx, i: MaskEditIn) -> Result<ItemOut<MaskLayer>> {
    const STAGE: &str = "mask.edit";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    edit_item(&ctx, &env, STAGE, &i.mask_id, |m: &mut MaskLayer| {
        edit_timing(&mut m.timing, i.start, i.end, i.start_word.clone(), i.end_word.clone());
        if let Some(r) = i.rect {
            m.rect = r;
        }
        if let Some(k) = i.kind {
            m.kind = k;
        }
        if let Some(s) = i.strength {
            m.strength = Some(s);
        }
        if let Some(c) = i.color.clone() {
            m.color = Some(c);
        }
        Ok(())
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct MaskRemoveIn {
    pub clip_id: ClipId,
    pub mask_id: String,
    pub base_revision: Option<u64>,
}

async fn mask_remove(ctx: Ctx, i: MaskRemoveIn) -> Result<RemoveOut> {
    let env = Env::load(&ctx, "mask.remove", &i.clip_id, i.base_revision)?;
    remove_item::<MaskLayer>(&ctx, &env, "mask.remove", &i.mask_id)
}

// ---------------------------------------------------------------------------------------------
// cutaway.*
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct CutawayAddIn {
    pub clip_id: ClipId,
    /// An imported source (its video) as the cutaway; give this, asset, file or url.
    pub source_id: Option<SourceId>,
    /// Library asset id (image or video).
    pub asset: Option<String>,
    /// Video (mp4, mov, mkv, webm, m4v, avi) or image file.
    pub file: Option<String>,
    /// Direct http(s) link to a video or image (downloaded into the asset cache).
    pub url: Option<String>,
    /// Seconds into the media where the cutaway starts (default 0).
    pub src_start: Option<f64>,
    /// Edit-timeline start in seconds (or start_word).
    pub start: Option<f64>,
    /// Edit-timeline end in seconds (or end_word).
    pub end: Option<f64>,
    pub start_word: Option<Anchor>,
    pub end_word: Option<Anchor>,
    /// full (replaces the picture; speech keeps playing) | pip (inside rect)
    pub mode: CutawayMode,
    /// pip: box on the canvas, normalized 0..1.
    pub rect: Option<Rect>,
    /// cover (default) | contain.
    pub fit: Option<Fit>,
    /// Keep only the clip's speech (default true); false mixes the cutaway's own audio in.
    pub mute: Option<bool>,
    pub base_revision: Option<u64>,
}

async fn cutaway_add(ctx: Ctx, i: CutawayAddIn) -> Result<ItemOut<Cutaway>> {
    const STAGE: &str = "cutaway.add";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    if i.start.is_none() && i.start_word.is_none() || i.end.is_none() && i.end_word.is_none() {
        return Err(validation(STAGE, "a cutaway needs start (or start_word) and end (or end_word)")
            .hint("e.g. --start 3.2 --end 5.0 (edit-timeline seconds)"));
    }
    let mut warnings = Vec::new();
    let media_in = i.src_start.unwrap_or(0.0);
    let media = match &i.source_id {
        Some(sid) => {
            if i.asset.is_some() || i.file.is_some() || i.url.is_some() {
                return Err(validation(STAGE, "give exactly one of source_id, asset, file or url"));
            }
            let src = env.store.load_source(sid)?;
            if !src.probe.has_video {
                return Err(validation(STAGE, format!("source {sid} has no video")));
            }
            if media_in >= src.probe.duration {
                return Err(validation(STAGE, format!("src_start {media_in:.2}s is past the end of source {sid} ({:.2}s)", src.probe.duration)));
            }
            CutawayMedia { source_id: Some(sid.clone()), asset: None, file: None, in_: media_in }
        }
        None => {
            let exts: Vec<&str> = IMAGE_EXTS.iter().chain(VIDEO_EXTS.iter()).copied().collect();
            let (asset, file) = resolve_visual(&ctx, STAGE, i.asset.as_deref(), i.file.as_deref(), i.url.as_deref(), &exts).await?;
            CutawayMedia { source_id: None, asset, file: Some(file), in_: media_in }
        }
    };
    let still = media.file.as_deref().is_some_and(is_still);
    let mut mute = i.mute.unwrap_or(true);
    if !mute && still {
        warnings.push("an image has no audio; the cutaway stays muted".into());
        mute = true;
    }
    if i.mode == CutawayMode::Full && i.rect.is_some() {
        warnings.push("rect is only used by pip cutaways".into());
    }
    let item = Cutaway {
        id: String::new(),
        timing: timing_from(i.start, i.end, i.start_word, i.end_word),
        media,
        mode: i.mode,
        rect: if i.mode == CutawayMode::Pip { i.rect } else { None },
        fit: i.fit.unwrap_or(Fit::Cover),
        mute,
    };
    add_item(&ctx, &env, STAGE, item, warnings)
}

async fn cutaway_list(ctx: Ctx, i: ClipIn) -> Result<ItemList<Cutaway>> {
    list_items(&Env::load(&ctx, "cutaway.list", &i.clip_id, None)?)
}

#[derive(Deserialize, JsonSchema)]
pub struct CutawayEditIn {
    pub clip_id: ClipId,
    pub cutaway_id: String,
    pub src_start: Option<f64>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub start_word: Option<Anchor>,
    pub end_word: Option<Anchor>,
    pub mode: Option<CutawayMode>,
    pub rect: Option<Rect>,
    pub fit: Option<Fit>,
    pub mute: Option<bool>,
    pub base_revision: Option<u64>,
}

async fn cutaway_edit(ctx: Ctx, i: CutawayEditIn) -> Result<ItemOut<Cutaway>> {
    const STAGE: &str = "cutaway.edit";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    edit_item(&ctx, &env, STAGE, &i.cutaway_id, |c: &mut Cutaway| {
        edit_timing(&mut c.timing, i.start, i.end, i.start_word.clone(), i.end_word.clone());
        if let Some(v) = i.src_start {
            c.media.in_ = v;
        }
        if let Some(v) = i.mode {
            c.mode = v;
        }
        if let Some(r) = i.rect {
            c.rect = Some(r);
        }
        if c.mode == CutawayMode::Full {
            c.rect = None;
        }
        if let Some(v) = i.fit {
            c.fit = v;
        }
        if let Some(v) = i.mute {
            if !v && c.media.file.as_deref().is_some_and(is_still) {
                return Err(validation(STAGE, "an image cutaway has no audio to unmute"));
            }
            c.mute = v;
        }
        Ok(())
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct CutawayRemoveIn {
    pub clip_id: ClipId,
    pub cutaway_id: String,
    pub base_revision: Option<u64>,
}

async fn cutaway_remove(ctx: Ctx, i: CutawayRemoveIn) -> Result<RemoveOut> {
    let env = Env::load(&ctx, "cutaway.remove", &i.clip_id, i.base_revision)?;
    remove_item::<Cutaway>(&ctx, &env, "cutaway.remove", &i.cutaway_id)
}

// ---------------------------------------------------------------------------------------------
// caption.word / caption.words_list / caption.words_clear
// ---------------------------------------------------------------------------------------------

/// Lower-case word text without surrounding punctuation (for `match`).
pub fn norm_word(s: &str) -> String {
    s.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

#[derive(Deserialize, JsonSchema)]
pub struct CaptionWordIn {
    pub clip_id: ClipId,
    /// Source of the word (required with word_i; with match it limits the search).
    pub source_id: Option<SourceId>,
    /// Word index in the source transcript (`rustler transcript get <source_id>`).
    pub word_i: Option<u32>,
    /// Every word in the clip with this text (case and punctuation ignored), instead of word_i.
    #[serde(rename = "match")]
    pub match_text: Option<String>,
    /// Text colour `#RRGGBB`.
    pub color: Option<String>,
    /// Size factor 0.5..2.5 (1 = preset size).
    pub scale: Option<f64>,
    /// Appended after the word, e.g. "🔥".
    pub emoji: Option<String>,
    /// true leaves the word out of the captions; false shows it again.
    pub hide: Option<bool>,
    pub base_revision: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct WordOverrideView {
    #[serde(flatten)]
    pub item: CaptionWordOverride,
    /// Transcript text of the word.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Edit-timeline starts of the word in the clip (a word can appear more than once).
    pub at: Vec<f64>,
    /// Why it will not show (e.g. the word was cut out).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct CaptionWordOut {
    pub clip_id: ClipId,
    /// The overrides created or changed by this call.
    pub changed: Vec<WordOverrideView>,
    /// All overrides of the clip now.
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

fn word_view(o: CaptionWordOverride, tm: &TimeMap, words: &HashMap<SourceId, Vec<Word>>) -> WordOverrideView {
    let text = words.get(&o.word_anchor.source_id).and_then(|ws| {
        ws.iter().find(|w| w.i == o.word_anchor.word_i).or_else(|| ws.get(o.word_anchor.word_i as usize)).map(|w| w.text.trim().to_string())
    });
    let (at, problem) = match word_spans(&o.word_anchor, tm, words) {
        Ok(spans) => (spans.iter().map(|s| s.0).collect(), None),
        Err(p) => (Vec::new(), Some(p)),
    };
    WordOverrideView { item: o, text, at, problem }
}

async fn caption_word(ctx: Ctx, i: CaptionWordIn) -> Result<CaptionWordOut> {
    const STAGE: &str = "caption.word";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    let clip_sources: Vec<SourceId> = env.clip.segments.iter().map(|s| s.source_id.clone()).collect();
    let words = words_for(&env.store, clip_sources.iter().cloned().chain(i.source_id.clone()))?;
    let mut warnings = Vec::new();

    // The words this call targets.
    let anchors: Vec<Anchor> = match (&i.word_i, &i.match_text) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(validation(STAGE, "give word_i (with source_id) or match")
                .hint("e.g. --source-id src_x --word-i 42, or --match \"million\""));
        }
        (Some(wi), None) => {
            let Some(sid) = &i.source_id else {
                return Err(validation(STAGE, "word_i needs source_id"));
            };
            let a = Anchor { source_id: sid.clone(), word_i: *wi };
            word_spans(&a, &env.tm, &words).map_err(|reason| {
                validation(STAGE, reason).hint("`rustler transcript get <source_id>` lists word indices; the word must be inside the clip")
            })?;
            vec![a]
        }
        (None, Some(m)) => {
            let target = norm_word(m);
            if target.is_empty() {
                return Err(validation(STAGE, "match is empty"));
            }
            let mut out = Vec::new();
            let mut sources: Vec<&SourceId> = words.keys().filter(|s| clip_sources.contains(s)).collect();
            sources.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            for sid in sources.into_iter().filter(|s| i.source_id.as_ref().is_none_or(|x| x == *s)) {
                for w in &words[sid] {
                    if norm_word(&w.text) != target {
                        continue;
                    }
                    let a = Anchor { source_id: sid.clone(), word_i: w.i };
                    if word_spans(&a, &env.tm, &words).is_ok() {
                        out.push(a);
                    }
                }
            }
            if out.is_empty() {
                return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("no word '{m}' inside clip {}", env.clip.id))
                    .hint("`rustler clip show <clip_id>` prints the clip's text; match ignores case and punctuation"));
            }
            out
        }
    };

    let probe = CaptionWordOverride {
        word_anchor: anchors[0].clone(),
        color: i.color.clone(),
        scale: i.scale,
        emoji: i.emoji.clone(),
        hide: i.hide.unwrap_or(false),
    };
    if i.color.is_none() && i.scale.is_none() && i.emoji.is_none() && i.hide.is_none() {
        return Err(validation(STAGE, "nothing to change: give color, scale, emoji or hide"));
    }
    if probe.color.is_some() || probe.scale.is_some() || probe.emoji.is_some() || probe.hide {
        check_word_override(&probe).map_err(|m| validation(STAGE, m))?;
    }

    let (changed, total) = change(&ctx, &env.store, &env.clip.id, CAPTION_WORDS_KEY, Vec::is_empty, |list: &mut Vec<CaptionWordOverride>| {
        let mut changed = Vec::new();
        for a in &anchors {
            let idx = match list.iter().position(|o| &o.word_anchor == a) {
                Some(k) => k,
                None => {
                    list.push(CaptionWordOverride { word_anchor: a.clone(), color: None, scale: None, emoji: None, hide: false });
                    list.len() - 1
                }
            };
            let o = &mut list[idx];
            if let Some(c) = &i.color {
                o.color = Some(c.clone());
            }
            if let Some(s) = i.scale {
                o.scale = Some(s);
            }
            if let Some(e) = &i.emoji {
                o.emoji = Some(e.clone());
            }
            if let Some(h) = i.hide {
                o.hide = h;
            }
            if o.color.is_none() && o.scale.is_none() && o.emoji.is_none() && !o.hide {
                // `hide: false` on a word with nothing else: the override is gone.
                list.remove(idx);
            } else {
                changed.push(list[idx].clone());
            }
        }
        Ok((changed, list.len()))
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &env.store, STAGE, serde_json::json!({"clip_id": env.clip.id, "words": anchors.len()}))?;
    }
    if !env.clip.captions.enabled {
        warnings.push(format!("captions are off for this clip; turn them on with `rustler caption set {} --enabled true`", env.clip.id));
    }
    let changed: Vec<WordOverrideView> = changed.into_iter().map(|o| word_view(o, &env.tm, &words)).collect();
    let t = changed.iter().flat_map(|v| v.at.first().copied()).next().unwrap_or(0.0);
    Ok(CaptionWordOut {
        clip_id: env.clip.id.clone(),
        changed,
        total,
        dry_run: ctx.dry_run.then_some(true),
        warnings,
        next: vec![
            format!("rustler render preview-frame {} --out-t {:.2}", env.clip.id, t + 0.1),
            format!("rustler caption words-list {}", env.clip.id),
        ],
    })
}

#[derive(Serialize, JsonSchema)]
pub struct CaptionWordsListOut {
    pub clip_id: ClipId,
    pub overrides: Vec<WordOverrideView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

async fn caption_words_list(ctx: Ctx, i: ClipIn) -> Result<CaptionWordsListOut> {
    let env = Env::load(&ctx, "caption.words_list", &i.clip_id, None)?;
    let list: Vec<CaptionWordOverride> = extras::get(&env.store, &env.clip.id, CAPTION_WORDS_KEY)?.unwrap_or_default();
    let words = words_for(&env.store, list.iter().map(|o| o.word_anchor.source_id.clone()))?;
    let overrides: Vec<WordOverrideView> = list.into_iter().map(|o| word_view(o, &env.tm, &words)).collect();
    let warnings = overrides
        .iter()
        .filter_map(|v| v.problem.as_ref().map(|p| format!("override on {}#{} does not show: {p}", v.item.word_anchor.source_id, v.item.word_anchor.word_i)))
        .collect();
    Ok(CaptionWordsListOut { clip_id: env.clip.id, overrides, warnings })
}

#[derive(Deserialize, JsonSchema)]
pub struct CaptionWordsClearIn {
    pub clip_id: ClipId,
    /// Only this source's overrides (all when left out).
    pub source_id: Option<SourceId>,
    /// Only this word (needs source_id).
    pub word_i: Option<u32>,
    pub base_revision: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct CaptionWordsClearOut {
    pub clip_id: ClipId,
    pub removed: usize,
    pub remaining: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

async fn caption_words_clear(ctx: Ctx, i: CaptionWordsClearIn) -> Result<CaptionWordsClearOut> {
    const STAGE: &str = "caption.words_clear";
    let env = Env::load(&ctx, STAGE, &i.clip_id, i.base_revision)?;
    if i.word_i.is_some() && i.source_id.is_none() {
        return Err(validation(STAGE, "word_i needs source_id"));
    }
    let (removed, remaining) = change(&ctx, &env.store, &env.clip.id, CAPTION_WORDS_KEY, Vec::is_empty, |list: &mut Vec<CaptionWordOverride>| {
        let before = list.len();
        list.retain(|o| {
            let src_match = i.source_id.as_ref().is_none_or(|s| s == &o.word_anchor.source_id);
            let word_match = i.word_i.is_none_or(|w| w == o.word_anchor.word_i);
            !(src_match && word_match)
        });
        let removed = before - list.len();
        if removed == 0 && i.word_i.is_some() {
            return Err(RustlerError::new(ErrorCode::NotFound, STAGE, "no override on that word").hint("`rustler caption words-list <clip_id>`"));
        }
        Ok((removed, list.len()))
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &env.store, STAGE, serde_json::json!({"clip_id": env.clip.id, "removed": removed}))?;
    }
    Ok(CaptionWordsClearOut { clip_id: env.clip.id, removed, remaining, dry_run: ctx.dry_run.then_some(true) })
}

// ---------------------------------------------------------------------------------------------
// Render integration
// ---------------------------------------------------------------------------------------------

/// Applies the clip's caption word overrides to its caption words (edit timeline): hidden words
/// are removed, emoji appended, colours/sizes returned as paints (speaker colours are the base;
/// an override colour wins). `paints` is `None` when the clip has no overrides (callers keep the
/// plain / speaker-coloured path). Overrides whose word was cut out are skipped with a warning.
#[allow(clippy::type_complexity)]
pub fn apply_word_overrides(
    store: &ProjectStore,
    clip: &Clip,
    words: Vec<CaptionWord>,
    colors: Option<Vec<Option<String>>>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<CaptionWord>, Option<Vec<Option<String>>>, Option<Vec<WordPaint>>)> {
    let list: Vec<CaptionWordOverride> = extras::get(store, &clip.id, CAPTION_WORDS_KEY)?.unwrap_or_default();
    if list.is_empty() {
        return Ok((words, colors, None));
    }
    let tm = TimeMap::from_segments(&clip.segments);
    let transcript = words_for(store, list.iter().map(|o| o.word_anchor.source_id.clone()))?;
    let mut paints: Vec<WordPaint> = (0..words.len())
        .map(|k| WordPaint { colour: colors.as_ref().and_then(|c| c.get(k).cloned().flatten()), scale: None })
        .collect();
    let mut words = words;
    let mut hidden = vec![false; words.len()];
    for o in &list {
        let spans = match word_spans(&o.word_anchor, &tm, &transcript) {
            Ok(s) => s,
            Err(reason) => {
                warnings.push(format!("caption override on {}#{} skipped: {reason}", o.word_anchor.source_id, o.word_anchor.word_i));
                continue;
            }
        };
        for (s, _) in spans {
            let hits: Vec<usize> = (0..words.len()).filter(|k| (words[*k].out_start - s).abs() < 1e-4).collect();
            for k in hits {
                if o.hide {
                    hidden[k] = true;
                }
                if let Some(c) = &o.color {
                    paints[k].colour = Some(c.clone());
                }
                if let Some(f) = o.scale {
                    paints[k].scale = Some(f);
                }
                if let Some(e) = &o.emoji
                    && !words[k].text.ends_with(e.as_str())
                {
                    words[k].text = format!("{} {e}", words[k].text);
                }
            }
        }
    }
    let (words, paints): (Vec<CaptionWord>, Vec<WordPaint>) =
        words.into_iter().zip(paints).zip(hidden).filter(|(_, h)| !h).map(|(wp, _)| wp).unzip();
    Ok((words, None, Some(paints)))
}

/// Caption words + paints moved onto the rendered timeline (words hidden in a transition dropped).
pub fn map_words_with_paints(words: Vec<CaptionWord>, paints: Vec<WordPaint>, tmap: &TransitionMap) -> (Vec<CaptionWord>, Vec<WordPaint>) {
    if tmap.is_identity() {
        return (words, paints);
    }
    words
        .into_iter()
        .zip(paints)
        .filter_map(|(w, p)| {
            let (s, e) = tmap.map_interval(w.out_start, w.out_end)?;
            Some((CaptionWord { out_start: s, out_end: e, ..w }, p))
        })
        .unzip()
}

/// Items of one key; entries that do not parse are skipped with a warning (a broken entry must
/// not block renders or preview frames).
fn parse_list<T: LayerItem>(map: &serde_json::Map<String, serde_json::Value>, warnings: &mut Vec<String>) -> Vec<T> {
    let Some(serde_json::Value::Array(items)) = map.get(T::KEY) else { return Vec::new() };
    items
        .iter()
        .enumerate()
        .filter_map(|(n, v)| match serde_json::from_value::<T>(v.clone()) {
            Ok(e) => Some(e),
            Err(err) => {
                let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                warnings.push(format!("{} entry {n} ({id}) in extras.json is invalid and ignored: {err}", T::GROUP));
                None
            }
        })
        .collect()
}

fn missing(what: &str, path: &Path, clip: &ClipId, group: &str, id: &str) -> RustlerError {
    RustlerError::new(ErrorCode::NotFound, "render", format!("{what} file is missing: {}", path.display()))
        .hint(format!("`rustler {group} remove {clip} {id}` or restore the file"))
}

/// Re-resolves a library asset (it may have moved); falls back to the stored path.
fn asset_path(store: &ProjectStore, paths: &ToolPaths, asset: Option<&str>, stored: Option<&Path>) -> Option<PathBuf> {
    if let Some(a) = asset
        && let Ok(p) = crate::asset_cmds::resolve_asset_path(store, &paths.home, &format!("asset:{a}"))
    {
        return Some(p);
    }
    stored.map(Path::to_path_buf)
}

/// Images, masks and cutaways into the compile extras: word anchors resolved (items whose word
/// was cut out are skipped with a warning), files resolved and checked, cutaway audio probed.
pub(crate) async fn apply_to_compile(
    store: &ProjectStore,
    paths: &ToolPaths,
    clip: &Clip,
    map: &serde_json::Map<String, serde_json::Value>,
    cx: &mut CompileExtras,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let images: Vec<ImageLayer> = parse_list(map, &mut warnings);
    let masks: Vec<MaskLayer> = parse_list(map, &mut warnings);
    let cutaways: Vec<Cutaway> = parse_list(map, &mut warnings);
    if images.is_empty() && masks.is_empty() && cutaways.is_empty() {
        return Ok(warnings);
    }
    let tm = TimeMap::from_segments(&clip.segments);
    let words = words_for(
        store,
        images
            .iter()
            .map(|x| &x.timing)
            .chain(masks.iter().map(|x| &x.timing))
            .chain(cutaways.iter().map(|x| &x.timing))
            .flat_map(|t| anchors_of(t).collect::<Vec<_>>()),
    )?;
    let mut resolve = |group: &str, id: &str, t: &Timing| -> Option<Timing> {
        match resolved(t, &tm, &words) {
            Ok(r) => Some(r),
            Err(reason) => {
                warnings.push(format!("{group} {id} skipped: {reason}"));
                None
            }
        }
    };
    let mut out_images = Vec::new();
    for mut im in images {
        let Some(t) = resolve("image", &im.id, &im.timing) else { continue };
        im.timing = t;
        let p = asset_path(store, paths, im.asset.as_deref(), Some(&im.file)).unwrap_or_default();
        if !p.is_file() {
            return Err(missing("image", &p, &clip.id, "image", &im.id));
        }
        im.file = p;
        out_images.push(im);
    }
    let mut out_masks = Vec::new();
    for mut m in masks {
        let Some(t) = resolve("mask", &m.id, &m.timing) else { continue };
        m.timing = t;
        out_masks.push(m);
    }
    let mut pending = Vec::new();
    for mut c in cutaways {
        let Some(t) = resolve("cutaway", &c.id, &c.timing) else { continue };
        c.timing = t;
        pending.push(c);
    }
    let mut out_cut = Vec::new();
    for mut c in pending {
        let (path, has_audio) = match &c.media.source_id {
            Some(sid) => {
                let s = store.load_source(sid)?;
                (store.resolve(&s.media_path), Some(s.probe.has_audio))
            }
            None => (asset_path(store, paths, c.media.asset.as_deref(), c.media.file.as_deref()).unwrap_or_default(), None),
        };
        if !path.is_file() {
            return Err(missing("cutaway", &path, &clip.id, "cutaway", &c.id));
        }
        if !c.mute {
            let audio = match has_audio {
                Some(a) => a,
                None if is_still(&path) => false,
                None => rustler_media::probe(paths, &path).await.map(|p| p.has_audio).unwrap_or(false),
            };
            if !audio {
                warnings.push(format!("cutaway {} has no audio track; rendered muted", c.id));
                c.mute = true;
            }
        }
        c.media.file = Some(std::path::absolute(&path).unwrap_or(path));
        out_cut.push(c);
    }
    cx.images = out_images;
    cx.masks = out_masks;
    cx.cutaways = out_cut;
    Ok(warnings)
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    let read = Annotations { read_only: true, ..write };
    let remove = Annotations { destructive: true, ..write };
    let fetch = Annotations { network: true, ..write };

    reg.register(typed(
        "image.add",
        "Add an image layer (logo, sticker, PNG card, animated GIF) in a canvas rect, with opacity, z-order and fade/pop/slide_up animation; times or transcript words",
        &["clip_id"],
        fetch,
        vec![
            Example {
                description: "logo in the top-right corner for the whole clip, fading in",
                cli: "rustler image add clp_x --file logo.png --rect '{\"x\":0.74,\"y\":0.03,\"w\":0.22,\"h\":0.08}' --anim-in fade",
            },
            Example {
                description: "library sticker that pops on a word (PowerShell: use --input-json @image.json)",
                cli: "rustler image add clp_x --asset ast_fire --rect '{\"x\":0.4,\"y\":0.55,\"w\":0.2,\"h\":0.12}' --start-word '{\"source_id\":\"src_x\",\"word_i\":42}' --anim-in pop",
            },
        ],
        image_add,
    ));
    reg.register(typed(
        "image.list",
        "List a clip's image layers with their resolved and rendered times (and why one will not render)",
        &["clip_id"],
        read,
        vec![Example { description: "image layers of a clip", cli: "rustler image list clp_x" }],
        image_list,
    ));
    reg.register(typed(
        "image.edit",
        "Change an image layer: file, rect, times/words, scale, opacity, rotation, fit, animation, z, above_captions",
        &["clip_id", "image_id"],
        fetch,
        vec![Example { description: "make the logo half transparent", cli: "rustler image edit clp_x img_1 --opacity 0.5" }],
        image_edit,
    ));
    reg.register(typed(
        "image.remove",
        "Remove an image layer from a clip",
        &["clip_id", "image_id"],
        remove,
        vec![Example { description: "remove an image", cli: "rustler image remove clp_x img_1" }],
        image_remove,
    ));

    reg.register(typed(
        "mask.add",
        "Hide part of the source picture (pop-up, logo, burned-in subtitles) with a blur, solid box or pixelation in a canvas rect",
        &["clip_id"],
        write,
        vec![
            Example {
                description: "blur a channel logo in the top-right corner for the whole clip",
                cli: "rustler mask add clp_x --kind blur --rect '{\"x\":0.75,\"y\":0.02,\"w\":0.23,\"h\":0.07}'",
            },
            Example {
                description: "black box over burned-in subtitles from 2 s to 9 s",
                cli: "rustler mask add clp_x --kind box --rect '{\"x\":0.05,\"y\":0.78,\"w\":0.9,\"h\":0.08}' --start 2 --end 9",
            },
        ],
        mask_add,
    ));
    reg.register(typed(
        "mask.list",
        "List a clip's masks with their resolved and rendered times",
        &["clip_id"],
        read,
        vec![Example { description: "masks of a clip", cli: "rustler mask list clp_x" }],
        mask_list,
    ));
    reg.register(typed(
        "mask.edit",
        "Change a mask: rect, kind, times/words, strength, colour",
        &["clip_id", "mask_id"],
        write,
        vec![Example { description: "stronger pixelation", cli: "rustler mask edit clp_x msk_1 --kind pixelate --strength 40" }],
        mask_edit,
    ));
    reg.register(typed(
        "mask.remove",
        "Remove a mask from a clip",
        &["clip_id", "mask_id"],
        remove,
        vec![Example { description: "remove a mask", cli: "rustler mask remove clp_x msk_1" }],
        mask_remove,
    ));

    reg.register(typed(
        "cutaway.add",
        "Add a cutaway (B-roll): another source, asset, file or URL shown full-frame (speech keeps playing) or picture-in-picture in a rect",
        &["clip_id"],
        fetch,
        vec![
            Example {
                description: "full-frame B-roll from another source, 3.0-5.5 s of the clip, from 12 s into the source",
                cli: "rustler cutaway add clp_x --source-id src_broll --src-start 12 --start 3 --end 5.5 --mode full",
            },
            Example {
                description: "picture-in-picture screenshot bottom-right",
                cli: "rustler cutaway add clp_x --file chart.png --mode pip --rect '{\"x\":0.52,\"y\":0.58,\"w\":0.44,\"h\":0.25}' --start 6 --end 9",
            },
        ],
        cutaway_add,
    ));
    reg.register(typed(
        "cutaway.list",
        "List a clip's cutaways with their resolved and rendered times",
        &["clip_id"],
        read,
        vec![Example { description: "cutaways of a clip", cli: "rustler cutaway list clp_x" }],
        cutaway_list,
    ));
    reg.register(typed(
        "cutaway.edit",
        "Change a cutaway: times/words, src_start, mode, rect, fit, mute",
        &["clip_id", "cutaway_id"],
        write,
        vec![Example { description: "turn a full cutaway into a PiP", cli: "rustler cutaway edit clp_x cut_1 --mode pip --rect '{\"x\":0.5,\"y\":0.6,\"w\":0.45,\"h\":0.25}'" }],
        cutaway_edit,
    ));
    reg.register(typed(
        "cutaway.remove",
        "Remove a cutaway from a clip",
        &["clip_id", "cutaway_id"],
        remove,
        vec![Example { description: "remove a cutaway", cli: "rustler cutaway remove clp_x cut_1" }],
        cutaway_remove,
    ));

    reg.register(typed(
        "caption.word",
        "Style one caption word (source_id + word_i) or every match of a text: colour, size, emoji after it, or hide it. Follows re-cuts",
        &["clip_id"],
        write,
        vec![
            Example { description: "every 'million' in yellow and 30% bigger", cli: "rustler caption word clp_x --match million --color \"#FFE000\" --scale 1.3" },
            Example { description: "fire emoji after word 42", cli: "rustler caption word clp_x --source-id src_x --word-i 42 --emoji \"🔥\"" },
            Example { description: "hide a filler word in the captions", cli: "rustler caption word clp_x --source-id src_x --word-i 17 --hide true" },
        ],
        caption_word,
    ));
    reg.register(typed(
        "caption.words_list",
        "List a clip's caption word overrides with the word text and where it appears",
        &["clip_id"],
        read,
        vec![Example { description: "keyword overrides of a clip", cli: "rustler caption words-list clp_x" }],
        caption_words_list,
    ));
    reg.register(typed(
        "caption.words_clear",
        "Remove caption word overrides: all, one source's, or one word",
        &["clip_id"],
        remove,
        vec![
            Example { description: "remove all overrides", cli: "rustler caption words-clear clp_x" },
            Example { description: "remove one word's override", cli: "rustler caption words-clear clp_x --source-id src_x --word-i 42" },
        ],
        caption_words_clear,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_and_timing_edits() {
        assert_eq!(norm_word("  Million!! "), "million");
        assert_eq!(norm_word("\"don't,\""), "don't");
        let a = Anchor { source_id: SourceId::new(), word_i: 3 };
        let mut t = timing_from(Some(1.0), Some(2.0), None, None);
        assert_eq!(t, Timing::out(1.0, 2.0));
        edit_timing(&mut t, None, None, Some(a.clone()), None);
        assert_eq!((t.out_start, t.start_word.as_ref()), (None, Some(&a)));
        edit_timing(&mut t, Some(0.5), None, None, None);
        assert_eq!((t.out_start, t.start_word.is_none()), (Some(0.5), true));
        let t = timing_from(Some(1.0), None, Some(a.clone()), None);
        assert_eq!(t.out_start, None, "a word wins over seconds");
    }
}
