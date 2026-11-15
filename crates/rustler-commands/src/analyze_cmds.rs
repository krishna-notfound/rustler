//! Commands: analyze.transcribe, analyze.scenes, analyze.audio, analyze.quick (Task 10a).
//!
//! Outputs live in `analysis/<source_id>/`: `transcript.json`, `units.json` (bare array of
//! `Unit`), `transcript.raw.json` (only when Devanagari was romanized), `scenes.json`,
//! `audio_events.json`, `transcript.parts/` (resumable whisper chunks).

use crate::project_cmds::{Outcome, log_event, mmss, validation};
use crate::source_cmds::load_source_checked;
use crate::transcript_cmds::{save_transcript, unit_at, units_preview};
use rustler_core::{
    AudioEvents, CaptionScript, ErrorCode, Project, ProjectStore, RustlerError, Scenes, Source, SourceId, SourceOrigin,
    Transcript, Unit, Word, WordTiming,
};
use rustler_engine_bridge::EngineClient;
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

// ---------------------------------------------------------------------------------------
// engine helpers (shared with source_cmds)
// ---------------------------------------------------------------------------------------

/// Starts the Python engine. Development/test override: if `RUSTLER_ENGINE_PROJECT` names an
/// engine source dir, the engine runs as `uv run --project <dir> --extra ocr --extra audio-events
/// --extra speakers python -m rustler_engine` (every analyzer, like `--profile full`) instead of the
/// venv created by `rustler setup`. `RUSTLER_HIDE_EXTRAS=ocr,speakers` makes the engine behave as if
/// those extras were not installed (lite simulation).
pub async fn spawn_engine(paths: &ToolPaths) -> rustler_core::Result<EngineClient> {
    if let Some(dir) = std::env::var_os("RUSTLER_ENGINE_PROJECT") {
        let dir = PathBuf::from(dir);
        let hf = std::env::var("HF_HOME").unwrap_or_else(|_| paths.models.join("hf").to_string_lossy().into_owned());
        let args: Vec<String> = [
            "run", "--project", &dir.to_string_lossy(), "--extra", "ocr", "--extra", "audio-events", "--extra", "speakers",
            "python", "-m", "rustler_engine",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        return EngineClient::spawn_command(&paths.uv, &args, &dir, &[("HF_HOME".to_string(), hf)]).await;
    }
    EngineClient::spawn(paths).await
}

fn is_devanagari(c: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&c)
}

/// True if any text contains Devanagari characters.
pub fn has_devanagari<'a>(texts: impl IntoIterator<Item = &'a str>) -> bool {
    texts.into_iter().any(|t| t.chars().any(is_devanagari))
}

/// Romanizes Devanagari texts through the engine (`translit.to_latin`); other texts pass
/// through untouched (and are not sent).
pub async fn romanize(engine: &EngineClient, ctx: &Ctx, texts: Vec<String>) -> rustler_core::Result<Vec<String>> {
    let idx: Vec<usize> = texts.iter().enumerate().filter(|(_, t)| t.chars().any(is_devanagari)).map(|(i, _)| i).collect();
    let mut out = texts;
    for chunk in idx.chunks(2000) {
        let batch: Vec<&str> = chunk.iter().map(|&i| out[i].as_str()).collect();
        #[derive(Deserialize)]
        struct R {
            texts: Vec<String>,
        }
        let r: R = engine.call("translit.to_latin", json!({"texts": batch}), &ctx.progress, &ctx.cancel).await?;
        if r.texts.len() != chunk.len() {
            return Err(RustlerError::new(ErrorCode::Internal, "romanize", "engine returned a different number of texts"));
        }
        for (&i, t) in chunk.iter().zip(r.texts) {
            out[i] = t;
        }
    }
    Ok(out)
}

fn progress(ctx: &Ctx, stage: &str, pct: f32, msg: &str) {
    ctx.progress.emit(ProgressEvent { stage: stage.into(), pct: Some(pct), message: Some(msg.to_string()) });
}

fn path_for(source: &Source, store: &ProjectStore, rel: &Option<PathBuf>) -> Option<PathBuf> {
    let _ = source;
    rel.as_ref().map(|p| store.resolve(p))
}

// ---------------------------------------------------------------------------------------
// analyze.transcribe
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct TranscribeIn {
    pub source_id: SourceId,
    /// whisper model (default: project setting, `small`).
    pub model: Option<String>,
    /// `auto` (default), `en`, `hi`.
    pub language: Option<String>,
    /// `auto` (default), `cpu`, `cuda`.
    pub device: Option<String>,
    /// Delete saved chunk parts and re-transcribe from scratch (use after changing model/language).
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct TranscribeOut {
    pub source_id: SourceId,
    /// false when `--max-seconds` ran out: rerun the same command to continue.
    pub complete: bool,
    pub language: Option<String>,
    pub engine: Option<String>,
    pub words: usize,
    pub units: usize,
    pub transcript_path: Option<PathBuf>,
    pub units_path: Option<PathBuf>,
    pub text_preview: Option<String>,
    /// First 30 units as `#id [mm:ss] text`.
    pub units_preview: Vec<String>,
    pub resume: Option<String>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Transcribes a source (shared by `analyze.transcribe` and `analyze.quick`).
pub async fn transcribe_source(
    ctx: &Ctx,
    store: &ProjectStore,
    project: &Project,
    paths: &ToolPaths,
    source: &Source,
    input: &TranscribeIn,
) -> rustler_core::Result<(TranscribeOut, Vec<Unit>)> {
    let wav = path_for(source, store, &source.audio_wav).ok_or_else(|| {
        validation(
            "analyze.transcribe",
            format!("source {} has no audio track to transcribe", source.id),
            "this is a silent video: use `rustler see tiles` to find moments and pass --start/--end to `clip create`",
        )
    })?;
    let id = source.id.to_string();
    let adir_rel = std::path::Path::new("analysis").join(&id);
    let parts_rel = adir_rel.join("transcript.parts");
    if input.force {
        for rel in [parts_rel.clone(), adir_rel.join("transcript.json"), adir_rel.join("units.json"), adir_rel.join("transcript.raw.json")] {
            let p = store.resolve(&rel);
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
    let model = input.model.clone().unwrap_or_else(|| project.settings.default_whisper_model.clone());
    let language = input.language.clone().unwrap_or_else(|| "auto".to_string());
    let device = input.device.clone().unwrap_or_else(|| "auto".to_string());
    progress(ctx, "transcribe", 0.0, &format!("starting whisper `{model}` ({device})"));
    let engine = spawn_engine(paths).await?;
    #[derive(Deserialize)]
    struct RawWord {
        text: String,
        start: f64,
        end: f64,
        conf: Option<f32>,
    }
    #[derive(Deserialize)]
    struct Resp {
        language: String,
        words: Vec<RawWord>,
        complete: bool,
    }
    let params = json!({
        "audio_path": wav, "model": model, "language": language, "device": device,
        "parts_dir": store.resolve(&parts_rel), "chunk_seconds": 600, "max_seconds": ctx.max_seconds,
    });
    let resp: rustler_core::Result<Resp> = engine.call("transcribe.run", params, &ctx.progress, &ctx.cancel).await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            engine.shutdown().await;
            return Err(e);
        }
    };
    if !resp.complete {
        engine.shutdown().await;
        return Ok((
            TranscribeOut {
                source_id: source.id.clone(),
                complete: false,
                language: Some(resp.language),
                engine: Some(format!("faster-whisper:{model}")),
                words: resp.words.len(),
                units: 0,
                transcript_path: None,
                units_path: None,
                text_preview: None,
                units_preview: vec![],
                resume: Some(format!("rerun the same command: `rustler analyze transcribe {id}` (finished chunks are kept)")),
                warnings: vec!["time budget reached before the whole file was transcribed".to_string()],
                next: vec![format!("rustler analyze transcribe {id}")],
            },
            vec![],
        ));
    }

    let mut words: Vec<Word> = resp
        .words
        .into_iter()
        .filter(|w| !w.text.trim().is_empty())
        .enumerate()
        .map(|(i, w)| Word { i: i as u32, text: w.text.trim().to_string(), start: w.start, end: w.end.max(w.start), conf: w.conf, speaker: None })
        .collect();
    let mut warnings = Vec::new();
    let mut script = CaptionScript::Latin;
    if has_devanagari(words.iter().map(|w| w.text.as_str())) {
        if project.settings.caption_script == CaptionScript::Latin {
            // Keep the original next to the romanized transcript.
            let raw = Transcript {
                schema: "rustler.transcript@1".to_string(),
                source_id: source.id.clone(),
                language: resp.language.clone(),
                engine: format!("faster-whisper:{model}"),
                script: CaptionScript::Devanagari,
                words: words.clone(),
                word_timing: WordTiming::Exact,
            };
            store.write_json(&adir_rel.join("transcript.raw.json"), &raw)?;
            let rom = romanize(&engine, ctx, words.iter().map(|w| w.text.clone()).collect()).await;
            match rom {
                Ok(t) => {
                    for (w, t) in words.iter_mut().zip(t) {
                        w.text = t;
                    }
                }
                Err(e) => {
                    engine.shutdown().await;
                    return Err(e);
                }
            }
        } else {
            script = CaptionScript::Devanagari;
        }
    }
    engine.shutdown().await;
    if words.is_empty() {
        warnings.push("no speech detected (music, silence or a language the model could not read); transcript is empty. Use `see tiles` and pass --start/--end to `clip create`, or retry with --language en|hi --force".to_string());
    }
    let transcript = Transcript {
        schema: "rustler.transcript@1".to_string(),
        source_id: source.id.clone(),
        language: resp.language,
        engine: format!("faster-whisper:{model}"),
        script,
        words,
        word_timing: WordTiming::Exact,
    };
    let units = save_transcript(store, &id, &transcript)?;
    log_event(ctx, store, "analyze.transcribed", json!({"source_id": id, "words": transcript.words.len(), "units": units.len(), "model": model}))?;
    let preview = transcript.words.iter().take(80).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    let next = if units.is_empty() {
        vec![format!("rustler see tiles {id}")]
    } else {
        vec![format!("rustler transcript get {id}"), format!("rustler transcript search {id} \"phrase\"")]
    };
    Ok((
        TranscribeOut {
            source_id: source.id.clone(),
            complete: true,
            language: Some(transcript.language.clone()),
            engine: Some(transcript.engine.clone()),
            words: transcript.words.len(),
            units: units.len(),
            transcript_path: Some(adir_rel.join("transcript.json")),
            units_path: Some(adir_rel.join("units.json")),
            text_preview: Some(preview),
            units_preview: units_preview(&units, 30),
            resume: None,
            warnings,
            next,
        },
        units,
    ))
}

pub async fn transcribe(ctx: Ctx, input: TranscribeIn) -> rustler_core::Result<Outcome<TranscribeOut>> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if source.audio_wav.is_none() {
        return Err(validation(
            "analyze.transcribe",
            format!("source {} has no audio track to transcribe", source.id),
            "this is a silent video: use `rustler see tiles` and pass --start/--end to `clip create`",
        ));
    }
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"transcribe": source.id, "model": input.model, "force": input.force})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let (out, _) = transcribe_source(&ctx, &store, &project, &paths, &source, &input).await?;
    Ok(Outcome::Done(out))
}

// ---------------------------------------------------------------------------------------
// analyze.scenes
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct ScenesIn {
    pub source_id: SourceId,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ScenesOut {
    pub source_id: SourceId,
    pub scene_count: usize,
    /// First 20 scenes as `[start, end]` seconds.
    pub scenes_preview: Vec<[f64; 2]>,
    pub path: PathBuf,
    pub next: Vec<String>,
}

async fn scenes_source(ctx: &Ctx, store: &ProjectStore, paths: &ToolPaths, source: &Source) -> rustler_core::Result<ScenesOut> {
    if !source.probe.has_video {
        return Err(validation(
            "analyze.scenes",
            format!("source {} has no video stream", source.id),
            "scene detection needs video; this source is audio-only (use `analyze audio` / `analyze transcribe`)",
        ));
    }
    let video = path_for(source, store, &source.proxy).unwrap_or_else(|| store.resolve(&source.media_path));
    let engine = spawn_engine(paths).await?;
    let r: rustler_core::Result<Scenes> =
        engine.call("scenes.detect", json!({"video_path": video, "threshold": 27.0}), &ctx.progress, &ctx.cancel).await;
    engine.shutdown().await;
    let scenes = r?;
    let rel = std::path::Path::new("analysis").join(source.id.as_str()).join("scenes.json");
    store.write_json(&rel, &scenes)?;
    log_event(ctx, store, "analyze.scenes", json!({"source_id": source.id, "scenes": scenes.scenes.len()}))?;
    Ok(ScenesOut {
        source_id: source.id.clone(),
        scene_count: scenes.scenes.len(),
        scenes_preview: scenes.scenes.iter().take(20).map(|s| [s.start, s.end]).collect(),
        path: rel,
        next: vec![format!("rustler see tiles {}", source.id)],
    })
}

pub async fn scenes(ctx: Ctx, input: ScenesIn) -> rustler_core::Result<Outcome<ScenesOut>> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"detect_scenes": source.id})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    Ok(Outcome::Done(scenes_source(&ctx, &store, &paths, &source).await?))
}

// ---------------------------------------------------------------------------------------
// analyze.audio
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema, Debug)]
pub struct AudioIn {
    pub source_id: SourceId,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct Peak {
    pub t: f64,
    pub mmss: String,
    pub lufs: f64,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct AudioOut {
    pub source_id: SourceId,
    pub duration: f64,
    pub buckets: usize,
    /// Top 10 loudest peaks (highest first).
    pub peaks: Vec<Peak>,
    pub peak_count: usize,
    pub silences_total: usize,
    /// First 20 silences as `[start, end]`.
    pub silences_preview: Vec<[f64; 2]>,
    pub path: PathBuf,
    pub next: Vec<String>,
}

/// Top `n` peaks by loudness.
pub fn top_peaks(ev: &AudioEvents, n: usize) -> Vec<Peak> {
    let mut p: Vec<_> = ev.peaks.iter().collect();
    p.sort_by(|a, b| b.lufs.partial_cmp(&a.lufs).unwrap_or(std::cmp::Ordering::Equal));
    p.into_iter().take(n).map(|p| Peak { t: p.t, mmss: mmss(p.t), lufs: (p.lufs * 10.0).round() / 10.0 }).collect()
}

async fn audio_source(ctx: &Ctx, store: &ProjectStore, paths: &ToolPaths, source: &Source) -> rustler_core::Result<(AudioOut, AudioEvents)> {
    if !source.probe.has_audio {
        return Err(validation(
            "analyze.audio",
            format!("source {} has no audio track", source.id),
            "nothing to measure; use `analyze scenes` and `see tiles` for silent video",
        ));
    }
    let media = store.resolve(&source.media_path);
    let ev = rustler_media::analyze_audio(paths, &media, &ctx.cancel, &ctx.progress).await?;
    let rel = std::path::Path::new("analysis").join(source.id.as_str()).join("audio_events.json");
    store.write_json(&rel, &ev)?;
    log_event(ctx, store, "analyze.audio", json!({"source_id": source.id, "peaks": ev.peaks.len(), "silences": ev.silences.len()}))?;
    let out = AudioOut {
        source_id: source.id.clone(),
        duration: source.probe.duration,
        buckets: ev.curve.len(),
        peaks: top_peaks(&ev, 10),
        peak_count: ev.peaks.len(),
        silences_total: ev.silences.len(),
        silences_preview: ev.silences.iter().take(20).map(|s| [s.start, s.end]).collect(),
        path: rel,
        next: vec![format!("rustler analyze transcribe {}", source.id)],
    };
    Ok((out, ev))
}

pub async fn audio(ctx: Ctx, input: AudioIn) -> rustler_core::Result<Outcome<AudioOut>> {
    let store = ctx.store()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if ctx.dry_run {
        return Ok(Outcome::dry_run(json!({"analyze_audio": source.id})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    Ok(Outcome::Done(audio_source(&ctx, &store, &paths, &source).await?.0))
}

// ---------------------------------------------------------------------------------------
// analyze.quick
// ---------------------------------------------------------------------------------------

/// When `analyze.quick` runs whisper.
#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranscribeMode {
    /// Reuse captions fetched with `source fetch-transcript` for a URL source when they exist, else run whisper.
    #[default]
    Auto,
    /// Always run whisper (also accepts `true`).
    Always,
    /// Never transcribe (also accepts `false`): audio + scenes only; run `analyze transcribe` later if needed.
    Never,
}

/// Accepts `auto|always|never` and the booleans `true|false` (string or JSON bool).
fn de_transcribe_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<TranscribeMode>, D::Error> {
    use serde::de::Error;
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Bool(true)) => Some(TranscribeMode::Always),
        Some(serde_json::Value::Bool(false)) => Some(TranscribeMode::Never),
        Some(serde_json::Value::String(s)) => Some(match s.trim().to_lowercase().as_str() {
            "auto" => TranscribeMode::Auto,
            "always" | "true" | "yes" | "on" | "1" => TranscribeMode::Always,
            "never" | "false" | "no" | "off" | "0" => TranscribeMode::Never,
            other => return Err(D::Error::custom(format!("transcribe must be auto, always or never, got `{other}`"))),
        }),
        Some(other) => return Err(D::Error::custom(format!("transcribe must be auto, always or never, got {other}"))),
    })
}

#[derive(Deserialize, JsonSchema, Debug)]
pub struct QuickIn {
    pub source_id: SourceId,
    pub model: Option<String>,
    pub language: Option<String>,
    pub device: Option<String>,
    /// Re-transcribe from scratch (delete saved chunk parts).
    #[serde(default)]
    pub force: bool,
    /// `auto` (default): for a URL source whose captions were fetched (`source fetch-transcript`) reuse them and
    /// skip whisper; otherwise run whisper. `always` (or `true`): run whisper. `never` (or `false`): no transcript.
    #[serde(default, deserialize_with = "de_transcribe_mode")]
    #[schemars(with = "Option<TranscribeMode>")]
    pub transcribe: Option<TranscribeMode>,
}

#[derive(Serialize, JsonSchema, Debug, Clone)]
pub struct HighlightSignal {
    pub t: f64,
    pub mmss: String,
    pub lufs: f64,
    /// Unit being spoken at the loud moment, if any.
    pub unit_id: Option<u32>,
    pub text: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct QuickOut {
    pub source_id: SourceId,
    pub duration: f64,
    pub complete: bool,
    pub language: Option<String>,
    pub words: usize,
    pub units: usize,
    /// Where the transcript came from: `whisper`, `captions` (fetched YouTube captions, approximate word timing) or `none`.
    pub transcript_source: String,
    pub scenes: Option<usize>,
    pub peaks: Vec<Peak>,
    pub silences_total: usize,
    /// Loudest moments joined with what was said then (candidates for clips, not decisions).
    pub highlight_signals: Vec<HighlightSignal>,
    pub resume: Option<String>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// Joins loudness peaks (top `n`) with the unit spoken at that second.
pub fn highlight_signals(ev: &AudioEvents, units: &[Unit], n: usize) -> Vec<HighlightSignal> {
    top_peaks(ev, n)
        .into_iter()
        .map(|p| {
            let u = unit_at(units, p.t, p.t + 1.0, 2.0);
            HighlightSignal {
                t: p.t,
                mmss: p.mmss,
                lufs: p.lufs,
                unit_id: u.map(|u| u.id),
                text: u.map(|u| if u.text.chars().count() > 90 { format!("{}…", u.text.chars().take(90).collect::<String>()) } else { u.text.clone() }),
            }
        })
        .collect()
}

/// Builds this source's transcript from captions fetched for its URL (`source fetch-transcript`), cut to the
/// imported section and shifted to the source's own timeline. `None` when the source is not a URL source, no
/// captions were fetched, or the source already has a whisper transcript.
fn reuse_fetched_captions(
    ctx: &Ctx,
    store: &ProjectStore,
    source: &Source,
) -> rustler_core::Result<Option<(Transcript, Vec<Unit>)>> {
    let SourceOrigin::Url { url, section, original_offset } = &source.origin else {
        return Ok(None);
    };
    let id = source.id.to_string();
    let own = std::path::Path::new("analysis").join(&id).join("transcript.json");
    if let Ok(t) = store.read_json::<Transcript>(&own) {
        if !t.engine.starts_with("youtube:") {
            return Ok(None); // a whisper transcript exists: keep it
        }
    }
    let fetched = std::path::Path::new("analysis").join(crate::source_cmds::pending_id(url)).join("transcript.json");
    let Ok(t) = store.read_json::<Transcript>(&fetched) else {
        return Ok(None);
    };
    if !t.engine.starts_with("youtube:") {
        return Ok(None);
    }
    let (lo, hi) = section.map_or((0.0, f64::INFINITY), |s| (s.start, s.end));
    let duration = source.probe.duration.max(0.0);
    let words: Vec<Word> = t
        .words
        .iter()
        .filter(|w| w.end > lo && w.start < hi)
        .map(|w| Word {
            i: 0,
            text: w.text.clone(),
            start: (w.start - original_offset).max(0.0),
            end: (w.end - original_offset).max(0.0).min(if duration > 0.0 { duration } else { f64::INFINITY }),
            conf: w.conf,
            speaker: None,
        })
        .enumerate()
        .map(|(i, mut w)| {
            w.i = i as u32;
            w
        })
        .collect();
    let own_t = Transcript {
        schema: t.schema.clone(),
        source_id: source.id.clone(),
        language: t.language.clone(),
        engine: t.engine.clone(),
        script: t.script,
        words,
        word_timing: WordTiming::Approximate,
    };
    let units = save_transcript(store, &id, &own_t)?;
    log_event(ctx, store, "analyze.transcribed", json!({"source_id": id, "words": own_t.words.len(), "units": units.len(), "model": t.engine, "reused_captions": true}))?;
    Ok(Some((own_t, units)))
}

pub async fn quick(ctx: Ctx, input: QuickIn) -> rustler_core::Result<Outcome<QuickOut>> {
    let store = ctx.store()?;
    let project = store.load_project()?;
    let source = load_source_checked(&store, &input.source_id)?;
    if ctx.dry_run {
        let mode = input.transcribe.unwrap_or_default();
        let steps: Vec<&str> = if mode == TranscribeMode::Never { vec!["audio", "scenes"] } else { vec!["audio", "scenes", "transcribe"] };
        return Ok(Outcome::dry_run(json!({"analyze_quick": source.id, "steps": steps, "transcribe": format!("{mode:?}").to_lowercase(), "force": input.force})));
    }
    let paths = ToolPaths::from_home(&ctx.home);
    let id = source.id.to_string();
    let mut warnings = Vec::new();

    progress(&ctx, "quick", 0.0, "audio loudness and silences");
    let (events, silences_total) = if source.probe.has_audio {
        let (_, ev) = audio_source(&ctx, &store, &paths, &source).await?;
        let n = ev.silences.len();
        (ev, n)
    } else {
        warnings.push("no audio track: skipped loudness and transcript".to_string());
        (AudioEvents::default(), 0)
    };

    let mut scene_count = None;
    if source.probe.has_video {
        progress(&ctx, "quick", 0.2, "scene detection");
        match scenes_source(&ctx, &store, &paths, &source).await {
            Ok(s) => scene_count = Some(s.scene_count),
            Err(e) if e.code == ErrorCode::DependencyMissing => warnings.push(format!("scenes skipped: {} ({})", e.message, e.hint.clone().unwrap_or_default())),
            Err(e) => return Err(e),
        }
    }

    let mut complete = true;
    let mut resume = None;
    let mut language = None;
    let (mut words, mut unit_list): (usize, Vec<Unit>) = (0, vec![]);
    let mode = input.transcribe.unwrap_or_default();
    let mut transcript_source = "none";
    let reused = if source.probe.has_audio && mode == TranscribeMode::Auto && !input.force {
        reuse_fetched_captions(&ctx, &store, &source)?
    } else {
        None
    };
    if let Some((t, units)) = reused {
        // Captions were already fetched for this URL: no whisper, no engine.
        progress(&ctx, "quick", 0.4, "reusing fetched captions (no whisper)");
        transcript_source = "captions";
        language = Some(t.language.clone());
        words = t.words.len();
        unit_list = units;
        warnings.push(format!(
            "transcript reused from fetched captions ({}): word timing is approximate; run `rustler analyze transcribe {id}` for exact whisper timing",
            t.engine
        ));
    } else if source.probe.has_audio && mode == TranscribeMode::Never {
        warnings.push("transcript skipped (--transcribe never); run `rustler analyze transcribe` when you need words, or look with `rustler see tiles`".to_string());
    } else if source.probe.has_audio {
        progress(&ctx, "quick", 0.4, "transcription");
        let tin = TranscribeIn { source_id: source.id.clone(), model: input.model, language: input.language, device: input.device, force: input.force };
        match transcribe_source(&ctx, &store, &project, &paths, &source, &tin).await {
            Ok((t, units)) => {
                complete = t.complete;
                resume = t.resume;
                language = t.language;
                words = t.words;
                warnings.extend(t.warnings);
                unit_list = units;
                transcript_source = "whisper";
            }
            Err(e) if e.code == ErrorCode::DependencyMissing => {
                complete = false;
                warnings.push(format!("transcript skipped: {} ({})", e.message, e.hint.clone().unwrap_or_default()));
            }
            Err(e) => return Err(e),
        }
    }
    let mut next = Vec::new();
    if !complete && resume.is_some() {
        next.push(format!("rustler analyze quick {id}   (rerun to continue transcription)"));
    } else if !unit_list.is_empty() {
        next.push(format!("rustler transcript get {id}   (read; pick unit ids)"));
        next.push(format!("rustler see tiles {id} --around \"<phrase>\"   (look at a moment)"));
    } else {
        next.push(format!("rustler see tiles {id}"));
    }
    if mode == TranscribeMode::Never {
        next.push(format!("rustler analyze transcribe {id}   (only if you need words)"));
    }
    Ok(Outcome::Done(QuickOut {
        source_id: source.id.clone(),
        duration: source.probe.duration,
        complete,
        language,
        words,
        units: unit_list.len(),
        transcript_source: transcript_source.to_string(),
        scenes: scene_count,
        peaks: top_peaks(&events, 10),
        silences_total,
        highlight_signals: highlight_signals(&events, &unit_list, 10),
        resume,
        warnings,
        next,
    }))
}

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let long = Annotations { long_running: true, ..Default::default() };
    reg.register(typed(
        "analyze.transcribe",
        "Transcribe a source with faster-whisper (resumable; --max-seconds budget; --force restarts); writes transcript + units",
        &["source_id"],
        long,
        vec![
            Example { description: "Default model", cli: "rustler analyze transcribe src_01k..." },
            Example { description: "Redo in Hindi", cli: "rustler analyze transcribe src_01k... --language hi --force" },
        ],
        transcribe,
    ));
    reg.register(typed(
        "analyze.scenes",
        "Detect scene cuts on the proxy video (writes scenes.json)",
        &["source_id"],
        long,
        vec![],
        scenes,
    ));
    reg.register(typed(
        "analyze.audio",
        "Loudness curve, loudness peaks and silences (writes audio_events.json)",
        &["source_id"],
        long,
        vec![],
        audio,
    ));
    reg.register(typed(
        "analyze.quick",
        "One call: audio loudness + scene cuts (on the proxy) + transcript, with loud-moment-plus-words highlight signals. Light by design: --transcribe auto (default) reuses captions fetched for a URL source instead of running whisper; never skips the transcript; always forces whisper. No OCR/events/faces/speakers (those are separate, optional commands).",
        &["source_id"],
        long,
        vec![
            Example { description: "First look at a new source", cli: "rustler analyze quick src_01k... --max-seconds 240" },
            Example { description: "Audio + scenes only; look at frames yourself", cli: "rustler analyze quick src_01k... --transcribe never" },
        ],
        quick,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_core::{LoudnessPoint, UnitKind};

    fn unit(id: u32, start: f64, end: f64, text: &str) -> Unit {
        Unit { id, kind: UnitKind::Sentence, word_start: 0, word_end: 0, start, end, text: text.into(), boundary_strength: 1.0 }
    }

    #[test]
    fn devanagari_detection() {
        assert!(has_devanagari(["bro", "क्या"]));
        assert!(!has_devanagari(["bro", "kya"]));
    }

    #[test]
    fn highlight_join_and_ranking() {
        let ev = AudioEvents {
            curve: vec![],
            peaks: vec![LoudnessPoint { t: 3.0, lufs: -20.0 }, LoudnessPoint { t: 12.0, lufs: -9.0 }],
            silences: vec![],
        };
        let units = vec![unit(0, 0.0, 5.0, "quiet start"), unit(1, 11.5, 14.0, "this is the loud part")];
        let h = highlight_signals(&ev, &units, 10);
        assert_eq!(h[0].t, 12.0);
        assert_eq!(h[0].unit_id, Some(1));
        assert_eq!(h[1].unit_id, Some(0));
        assert_eq!(h[0].mmss, "00:12");
    }
}
