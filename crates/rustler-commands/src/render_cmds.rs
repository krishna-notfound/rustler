//! Commands: render.run, render.preview_frame (Task 10b).
//!
//! `render.run` builds the caption track (words -> TimeMap -> ASS), stages fonts and captions into
//! a per-render workdir, loads the `extras.json` sidecar (music bed, crop paths), compiles one
//! ffmpeg invocation and renders it. `render.preview_frame` runs the same graph for one frame.

use crate::clip_cmds::{ClipExtras, caption_words_for_clip, compile_extras, log_event};
use crate::see_cmds::find_fonts;
use crate::{audio_cmds, extras, fx_cmds, overlay_cmds};
use rustler_media::RenderReport;
use chrono::Utc;
use rustler_captions::write_ass;
use rustler_core::{
    CaptionScript, Clip, ClipId, ErrorCode, ProjectStore, RenderId, RenderQuality, RenderRecord, RustlerError,
    SourceId, TimeMap,
};
use rustler_media::compile::{CompileExtras, CompiledRender, SourceMedia, compile_with};
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Result<T> = rustler_core::Result<T>;

fn io_err(what: &str, p: &Path, e: std::io::Error) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, "render", format!("{what} {}: {e}", p.display()))
}

fn abs_root(store: &ProjectStore) -> PathBuf {
    std::path::absolute(&store.root).unwrap_or_else(|_| store.root.clone())
}

// ---------------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------------

/// Font family override for the ASS file: Devanagari captions need a Devanagari font; otherwise
/// the preset's own family is used (empty string).
pub fn caption_font_family(script: CaptionScript, font_files: &[PathBuf]) -> &'static str {
    let has_devanagari = font_files.iter().any(|p| p.to_string_lossy().to_lowercase().contains("devanagari"));
    if script == CaptionScript::Devanagari && has_devanagari { "Noto Sans Devanagari" } else { "" }
}

/// `out_t` clamped just inside the clip so the last frame can be previewed; Validation when it is
/// negative, not finite or clearly beyond the end.
pub fn clamp_preview_time(out_t: f64, duration: f64) -> Result<f64> {
    if !out_t.is_finite() || out_t < 0.0 {
        return Err(RustlerError::new(ErrorCode::Validation, "render.preview_frame", "out_t must be >= 0"));
    }
    if out_t > duration + 0.05 {
        return Err(RustlerError::new(
            ErrorCode::Validation,
            "render.preview_frame",
            format!("out_t {out_t:.2}s is past the end of the clip ({duration:.2}s)"),
        )
        .hint("`rustler clip show <clip_id>` lists the output timeline"));
    }
    Ok(out_t.min((duration - 0.04).max(0.0)))
}

// ---------------------------------------------------------------------------------------------
// Preparation shared by render.run and render.preview_frame
// ---------------------------------------------------------------------------------------------

struct Prepared {
    compiled: CompiledRender,
    encoder: String,
    warnings: Vec<String>,
}

fn source_media(store: &ProjectStore, clip: &Clip) -> Result<HashMap<SourceId, SourceMedia>> {
    let mut map = HashMap::new();
    for seg in &clip.segments {
        if map.contains_key(&seg.source_id) {
            continue;
        }
        let s = store.load_source(&seg.source_id)?;
        let path = store.resolve(&s.media_path);
        if !path.is_file() {
            return Err(RustlerError::new(
                ErrorCode::NotFound,
                "render",
                format!("source media is missing: {}", path.display()),
            )
            .hint("the original file was moved or deleted; re-import the source"));
        }
        map.insert(s.id.clone(), SourceMedia { id: s.id, path, probe: s.probe });
    }
    Ok(map)
}

/// Copies every font below `<home>/fonts/**` flat into `dest` (libass `fontsdir`).
fn stage_fonts(paths: &ToolPaths, dest: &Path) -> Result<Vec<PathBuf>> {
    let fonts = find_fonts(&paths.fonts);
    std::fs::create_dir_all(dest).map_err(|e| io_err("create", dest, e))?;
    for f in &fonts {
        if let Some(name) = f.file_name() {
            let to = dest.join(name);
            if !to.exists() {
                std::fs::copy(f, &to).map_err(|e| io_err("copy font", f, e))?;
            }
        }
    }
    Ok(fonts)
}

#[allow(clippy::too_many_arguments)]
async fn prepare(
    paths: &ToolPaths,
    store: &ProjectStore,
    clip: &Clip,
    quality: RenderQuality,
    prefer_gpu: bool,
    workdir: &Path,
    out: &Path,
    two_pass_loudness: bool,
) -> Result<Prepared> {
    std::fs::create_dir_all(workdir).map_err(|e| io_err("create", workdir, e))?;
    let sources = source_media(store, clip)?;
    let mut warnings = Vec::new();

    let fonts_dir = workdir.join("fonts");
    let mut ass: Option<PathBuf> = None;
    if clip.captions.enabled {
        let (words, w) = caption_words_for_clip(store, clip)?;
        warnings.extend(w);
        // Captions follow the picture through transitions (edit -> rendered timeline).
        let words = fx_cmds::map_caption_words(words, &fx_cmds::transition_map(store, clip)?);
        if words.is_empty() {
            warnings.push("captions are enabled but no transcript words fall inside the clip; rendering without captions".into());
        } else {
            let fonts = stage_fonts(paths, &fonts_dir)?;
            let family = caption_font_family(clip.captions.script, &fonts);
            let text = write_ass(&words, &clip.captions, &clip.canvas, family)?;
            let p = workdir.join("captions.ass");
            std::fs::write(&p, text).map_err(|e| io_err("write", &p, e))?;
            ass = Some(p);
        }
    }

    // extras.json through the shared helper: music + crop paths (typed view) and overlays.
    let map = extras::load(store, &clip.id)?;
    let typed_extras: ClipExtras = serde_json::from_value(serde_json::Value::Object(map.clone()))
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "render", format!("extras.json is invalid: {e}")))?;
    let mut cx: CompileExtras = compile_extras(&typed_extras, clip);
    cx.overlays = extras::from_map(&map, overlay_cmds::KEY)?.unwrap_or_default();
    cx.two_pass_loudness = two_pass_loudness;
    // Audio post (denoise, gains, sfx) and transitions / effects / progress bar.
    warnings.extend(audio_cmds::apply_to_compile(store, clip, &map, &mut cx)?);
    warnings.extend(fx_cmds::apply_to_compile(&map, clip, &mut cx)?);
    if !cx.overlays.is_empty() && ass.is_none() {
        // Overlays need the fonts even when there are no captions.
        stage_fonts(paths, &fonts_dir)?;
    }
    if let Some(m) = &cx.music {
        if !m.path.is_file() {
            return Err(RustlerError::new(ErrorCode::NotFound, "render", format!("music file is missing: {}", m.path.display()))
                .hint("`rustler audio remove-music <clip_id>` or re-add the music"));
        }
    }

    let enc = rustler_media::pick_encoder(paths, quality, prefer_gpu).await;
    let compiled = compile_with(clip, &sources, ass.as_deref(), &fonts_dir, &enc, out, workdir, &cx)?;
    Ok(Prepared { compiled, encoder: enc.name, warnings })
}

// ---------------------------------------------------------------------------------------------
// render.preview_frame
// ---------------------------------------------------------------------------------------------

/// Renders one preview frame of `clip_id` at output time `out_t`; returns the absolute jpg path.
pub(crate) async fn preview_frame_image(ctx: &Ctx, clip_id: &ClipId, out_t: f64) -> Result<PathBuf> {
    let store = ctx.store()?;
    let clip = store.load_clip(clip_id)?;
    let duration = TimeMap::from_segments(&clip.segments).duration();
    // out_t is on the edit timeline; the frame comes from the rendered (transition-shortened) one.
    let tmap = fx_cmds::transition_map(&store, &clip)?;
    let t = clamp_preview_time(out_t, duration)?;
    let t = tmap.map_time(t).min((tmap.duration() - 0.04).max(0.0));
    let dir = abs_root(&store).join("clips").join(clip.id.as_str()).join("preview");
    let work = dir.join("work");
    let jpg = dir.join(format!("frame_{}ms.jpg", (t * 1000.0).round() as u64));
    let paths = ToolPaths::from_home(&ctx.home);
    // Preview frames have no audio pass: two-pass loudness is irrelevant here.
    let prepared = prepare(&paths, &store, &clip, RenderQuality::Draft, false, &work, &work.join("preview.mp4"), false).await?;
    rustler_media::preview_frame(&paths, &prepared.compiled, t, &jpg).await?;
    Ok(jpg)
}

#[derive(Deserialize, JsonSchema)]
pub struct PreviewFrameIn {
    pub clip_id: ClipId,
    /// Output time in seconds (0 .. clip duration).
    pub out_t: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct PreviewFrameOut {
    /// Absolute path of the jpg; look at it before rendering.
    pub image: PathBuf,
    pub out_t: f64,
}

async fn render_preview_frame(ctx: Ctx, i: PreviewFrameIn) -> Result<PreviewFrameOut> {
    let image = preview_frame_image(&ctx, &i.clip_id, i.out_t).await?;
    Ok(PreviewFrameOut { image, out_t: i.out_t })
}

// ---------------------------------------------------------------------------------------------
// render.run
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct RenderRunIn {
    pub clip_id: ClipId,
    /// draft (fast, small) | final (default)
    pub quality: Option<RenderQuality>,
    /// Use a hardware encoder when one passes a probe (default true).
    pub prefer_gpu: Option<bool>,
    /// Measure loudness in a fast audio-only pass first so the clip lands on the LUFS target
    /// (default true; false = single-pass loudnorm).
    pub two_pass_loudness: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct RenderRunOut {
    pub render_id: RenderId,
    /// Absolute path of the mp4.
    pub path: PathBuf,
    pub encoder: String,
    /// Duration of the rendered file in seconds (edit duration minus transition overlaps).
    pub duration: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    /// How loudness was normalized (off | single-pass | two-pass) and the measured values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness: Option<RenderReport>,
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

async fn render_run(ctx: Ctx, i: RenderRunIn) -> Result<RenderRunOut> {
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    let quality = i.quality.unwrap_or(RenderQuality::Final);
    // Rendered length: transitions shorten the clip by their overlaps.
    let duration = fx_cmds::effective_duration_of(&store, &clip)?;
    let render_id = RenderId::new();
    let renders = abs_root(&store).join("clips").join(clip.id.as_str()).join("renders");
    let workdir = renders.join(render_id.as_str());
    let out = renders.join(format!("{render_id}.mp4"));
    let next = vec![format!("rustler qc run {}", clip.id)];

    ctx.progress.emit(ProgressEvent { stage: "render.prepare".into(), pct: Some(0.0), message: Some(clip.name.clone()) });
    let paths = ToolPaths::from_home(&ctx.home);
    if ctx.dry_run {
        let enc = rustler_media::pick_encoder(&paths, quality, i.prefer_gpu.unwrap_or(true)).await;
        return Ok(RenderRunOut {
            render_id,
            path: out,
            encoder: enc.name,
            duration,
            dry_run: Some(true),
            loudness: None,
            next,
            warnings: Vec::new(),
        });
    }
    let two_pass = i.two_pass_loudness.unwrap_or(true);
    let mut prepared =
        prepare(&paths, &store, &clip, quality, i.prefer_gpu.unwrap_or(true), &workdir, &out, two_pass).await?;
    let encoder = prepared.encoder.clone();
    let mut report =
        rustler_media::render_with_report(&paths, &prepared.compiled, duration, &ctx.cancel, &ctx.progress).await?;
    prepared.warnings.append(&mut report.warnings);

    // Re-load: the clip may have been edited while rendering; only the renders list changes here.
    let mut fresh = store.load_clip(&clip.id)?;
    let base = fresh.revision;
    fresh.renders.push(RenderRecord {
        id: render_id.clone(),
        path: PathBuf::from("clips").join(clip.id.as_str()).join("renders").join(format!("{render_id}.mp4")),
        quality,
        encoder: encoder.clone(),
        created_at: Utc::now(),
        qc: None,
    });
    store.save_clip(&mut fresh, base)?;
    log_event(
        &ctx,
        &store,
        "render.run",
        serde_json::json!({"clip_id": clip.id, "render_id": render_id, "encoder": encoder, "quality": quality}),
    )?;
    Ok(RenderRunOut {
        render_id,
        path: out,
        encoder,
        duration,
        dry_run: None,
        loudness: Some(report),
        next,
        warnings: prepared.warnings,
    })
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "render.run",
        "Render the clip to a 1080x1920 mp4 (captions, loudnorm, optional music bed) in one ffmpeg pass",
        &["clip_id"],
        Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: true },
        vec![
            Example { description: "fast draft render", cli: "rustler render run clp_x --quality draft" },
            Example { description: "final render on CPU only", cli: "rustler render run clp_x --prefer-gpu false" },
        ],
        render_run,
    ));
    reg.register(typed(
        "render.preview_frame",
        "One frame of the composed clip (layout + captions) at output time out_t as a jpg; look at it before rendering",
        &["clip_id"],
        Annotations { read_only: true, destructive: false, network: false, costs_money: false, long_running: false },
        vec![Example { description: "frame at 3.5 s", cli: "rustler render preview-frame clp_x --out-t 3.5" }],
        render_preview_frame,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_family_choice() {
        let deva = vec![PathBuf::from("/h/fonts/NotoSansDevanagari-Bold.ttf")];
        let latin = vec![PathBuf::from("/h/fonts/Anton-Regular.ttf")];
        assert_eq!(caption_font_family(CaptionScript::Devanagari, &deva), "Noto Sans Devanagari");
        assert_eq!(caption_font_family(CaptionScript::Devanagari, &latin), "");
        assert_eq!(caption_font_family(CaptionScript::Latin, &deva), "");
    }

    #[test]
    fn preview_time_clamping() {
        assert_eq!(clamp_preview_time(1.0, 10.0).unwrap(), 1.0);
        assert!((clamp_preview_time(10.0, 10.0).unwrap() - 9.96).abs() < 1e-9);
        assert!((clamp_preview_time(10.04, 10.0).unwrap() - 9.96).abs() < 1e-9);
        assert_eq!(clamp_preview_time(11.0, 10.0).unwrap_err().code, ErrorCode::Validation);
        assert_eq!(clamp_preview_time(-1.0, 10.0).unwrap_err().code, ErrorCode::Validation);
        assert_eq!(clamp_preview_time(f64::NAN, 10.0).unwrap_err().code, ErrorCode::Validation);
    }
}
