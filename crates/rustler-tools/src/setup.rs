//! `rustler setup`: idempotent install of ffmpeg, yt-dlp, uv, the Python engine,
//! whisper models and fonts into the Rustler home.

use crate::doctor::{VersionKind, detect_nvidia, tool_version};
use crate::paths::ToolPaths;
use crate::{assets, engine_install, ffmpeg_install, fonts, uv_install, ytdlp_install};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    #[default]
    Lite,
    Full,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpuMode {
    #[default]
    Auto,
    Cuda,
    None,
}

/// Optional Python extras of the engine (`engine/pyproject.toml`), heaviest features first.
pub const PY_EXTRAS: [&str; 3] = ["ocr", "audio-events", "speakers"];

/// Everything `--with` accepts: the Python extras, NVIDIA CUDA libraries, the Kenney SFX pack.
pub const WITH_PIECES: [&str; 5] = ["ocr", "audio-events", "speakers", "cuda", "sfx-pack"];

/// Default: Lite (base engine + whisper `base`), Auto GPU, not forced, asset library on (without
/// the SFX pack), model downloads on.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct SetupOptions {
    /// `lite` (default): base engine only, whisper `base`, no heavy-analyzer model prefetch.
    /// `full`: every optional extra + every model prefetch + the SFX pack (+ cuda when an NVIDIA GPU is found).
    /// Running `lite` on an existing full install never removes anything.
    pub profile: Profile,
    /// `auto` adds the CUDA libraries only with `--profile full` and an NVIDIA GPU; `cuda` forces them; `none` never auto-adds.
    pub gpu: GpuMode,
    /// Whisper models to download now. Empty = the profile default (`base` for lite, `base` + `small` for full).
    pub whisper_models: Vec<String>,
    pub force: bool,
    /// Fill the global asset library (`~/.rustler/assets`): built-in caption styles, layouts and fonts.
    /// Skip with `--no-assets`.
    pub assets: bool,
    /// Download whisper (and, for heavy pieces you add, their) models during setup. `--no-models` skips every download
    /// (models are then fetched on first use).
    pub models: bool,
    /// Add optional pieces: `ocr`, `audio-events`, `speakers`, `cuda`, `sfx-pack`
    /// (repeat the flag or separate with commas: `--with ocr,speakers`). Previously added pieces are kept.
    pub with: Vec<String>,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            profile: Profile::Lite,
            gpu: GpuMode::Auto,
            whisper_models: Vec::new(),
            force: false,
            assets: true,
            models: true,
            with: Vec::new(),
        }
    }
}

/// What a previous `setup` recorded in `manifest.json` (heavy pieces added so far).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
#[serde(default)]
pub struct InstallState {
    /// Python extras synced into the engine venv (`ocr`, `audio-events`, `speakers`, `cuda`), sorted.
    pub extras: Vec<String>,
    /// The Kenney CC0 SFX pack was fetched into the asset library.
    pub sfx_pack: bool,
}

/// `lite` when any of the three analyzer extras is missing, else `full`.
pub fn profile_name(state: &InstallState) -> &'static str {
    if PY_EXTRAS.iter().all(|e| state.extras.iter().any(|x| x == e)) {
        "full"
    } else {
        "lite"
    }
}

/// The concrete work one `setup` run does, resolved from options + what is already installed.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
pub struct InstallPlan {
    /// Python extras to sync (union of the previous install, `--with` and the profile), sorted.
    pub extras: Vec<String>,
    pub sfx_pack: bool,
    /// Whisper models to fetch (empty when `--no-models`).
    pub whisper_models: Vec<String>,
    /// Heavy-analyzer models to prefetch now (everything else downloads on first use).
    pub prefetch_yunet: bool,
    pub prefetch_yamnet: bool,
    pub prefetch_diarization: bool,
    pub prefetch_ocr_devanagari: bool,
}

/// Normalizes `--with` values (commas, case, `_`/`-`) and rejects unknown pieces.
pub fn parse_with(raw: &[String]) -> rustler_core::Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for item in raw.iter().flat_map(|s| s.split(',')) {
        let name = item.trim().to_lowercase().replace('_', "-");
        if name.is_empty() {
            continue;
        }
        let name = match name.as_str() {
            "sfx" | "sfxpack" => "sfx-pack".to_string(),
            "events" | "audioevents" => "audio-events".to_string(),
            "diarization" | "speaker" => "speakers".to_string(),
            _ => name,
        };
        if !WITH_PIECES.contains(&name.as_str()) {
            return Err(RustlerError::new(
                ErrorCode::Validation,
                "setup",
                format!("unknown --with piece `{item}`"),
            )
            .hint(format!("choose from: {}", WITH_PIECES.join(", "))));
        }
        out.insert(name);
    }
    Ok(out)
}

impl SetupOptions {
    /// Resolves what to install. `previous` = the manifest's recorded state (heavy pieces are only
    /// ever added, never removed); `nvidia` = an NVIDIA GPU was detected (only consulted for `full`).
    pub fn plan(&self, previous: &InstallState, nvidia: bool) -> rustler_core::Result<InstallPlan> {
        let with = parse_with(&self.with)?;
        let full = self.profile == Profile::Full;
        let mut extras: BTreeSet<String> = previous.extras.iter().cloned().collect();
        for e in PY_EXTRAS {
            if full || with.contains(e) {
                extras.insert(e.to_string());
            }
        }
        if with.contains("cuda")
            || self.gpu == GpuMode::Cuda
            || (full && self.gpu == GpuMode::Auto && nvidia)
        {
            extras.insert("cuda".to_string());
        }
        let whisper_models = if !self.models {
            Vec::new()
        } else if !self.whisper_models.is_empty() {
            self.whisper_models.clone()
        } else if full {
            vec!["base".to_string(), "small".to_string()]
        } else {
            vec!["base".to_string()]
        };
        let prefetch = |extra: &str| self.models && (full || with.contains(extra));
        Ok(InstallPlan {
            extras: extras.into_iter().collect(),
            sfx_pack: full || with.contains("sfx-pack") || previous.sfx_pack,
            whisper_models,
            prefetch_yunet: self.models && full,
            prefetch_yamnet: prefetch("audio-events"),
            prefetch_diarization: prefetch("speakers"),
            prefetch_ocr_devanagari: prefetch("ocr"),
        })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
pub struct Component {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub sha256: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
pub struct SetupReport {
    pub installed: Vec<Component>,
    pub skipped: Vec<Component>,
    /// (component, error message)
    pub failed: Vec<(String, String)>,
    /// Non-fatal notes (e.g. an asset pack that could not be fetched).
    #[serde(default)]
    pub notes: Vec<String>,
    /// What this run resolved to (extras, SFX pack, models to prefetch).
    #[serde(default)]
    pub plan: InstallPlan,
}

/// Platforms with prebuilt tool downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub(crate) fn current() -> rustler_core::Result<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Ok(Platform::Windows),
            ("linux", "x86_64") => Ok(Platform::Linux),
            (os, arch) => Err(RustlerError::new(
                ErrorCode::DependencyMissing,
                "setup",
                format!("`rustler setup` has no prebuilt tools for {os}/{arch}"),
            )
            .hint("supported: windows/x86_64, linux/x86_64; install ffmpeg, yt-dlp and uv manually into ~/.rustler/bin")),
        }
    }
}

/// `home/manifest.json`: what `setup` installed (and adopted).
#[derive(Serialize, Deserialize, Default, Debug)]
struct Manifest {
    components: BTreeMap<String, Component>,
    /// Heavy pieces added so far (extras, SFX pack); only ever grows.
    #[serde(default)]
    install: InstallState,
}

impl Manifest {
    fn path(paths: &ToolPaths) -> PathBuf {
        paths.home.join("manifest.json")
    }

    fn load(paths: &ToolPaths) -> Self {
        std::fs::read(Self::path(paths))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, paths: &ToolPaths) -> std::io::Result<()> {
        let path = Self::path(paths);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(&tmp, &path)
    }
}

/// The heavy pieces recorded by earlier `setup` runs (empty before the first run).
pub fn installed_state(paths: &ToolPaths) -> InstallState {
    Manifest::load(paths).install
}

/// Step result: the component and whether it was installed now (`false` = skipped).
type StepResult = Result<(Component, bool), (String, String)>;

fn fail(name: &str, e: RustlerError) -> (String, String) {
    let msg = match &e.hint {
        Some(h) => format!("{} (hint: {h})", e.message),
        None => e.message,
    };
    (name.to_string(), msg)
}

/// Skips when every file in `files` exists and the manifest knows the component, or (for
/// binaries set up by hand) the tool runs and reports a version; otherwise `install`s.
async fn tool_step<F, Fut>(
    name: &str,
    files: Vec<&Path>,
    kind: VersionKind,
    manifest: &Manifest,
    force: bool,
    install: F,
) -> StepResult
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = rustler_core::Result<Component>>,
{
    if !force && files.iter().all(|p| p.is_file()) {
        if let Some(c) = manifest.components.get(name) {
            return Ok((c.clone(), false));
        }
        if let Some(version) = tool_version(kind, files[0]).await {
            return Ok((
                Component {
                    name: name.to_string(),
                    version,
                    path: files[0].to_path_buf(),
                    sha256: None,
                },
                false,
            ));
        }
    }
    install().await.map(|c| (c, true)).map_err(|e| fail(name, e))
}

/// Idempotent install of ffmpeg/ffprobe, yt-dlp, uv, the Python engine (extracted +
/// `uv sync`) and fonts (steps 1-4 and 6 of the plan).
///
/// Step 5 (whisper model download) needs the running engine and `rustler-tools` cannot depend
/// on `rustler-engine-bridge` (the bridge depends on this crate); the `setup.run` command
/// performs it after this returns. A failing step is recorded in `failed`; the others still run.
pub async fn setup(
    paths: &ToolPaths,
    opts: &SetupOptions,
    progress: &ProgressSink,
) -> rustler_core::Result<SetupReport> {
    let platform = Platform::current()?;
    let io = |what: &str, e: std::io::Error| {
        RustlerError::new(ErrorCode::Internal, "setup", format!("{what}: {e}"))
    };
    std::fs::create_dir_all(&paths.bin).map_err(|e| io("create bin dir", e))?;
    let mut manifest = Manifest::load(paths);
    let force = opts.force;
    // Resolve (and validate `--with`) before anything is downloaded.
    // The GPU probe only matters for `--profile full` with `--gpu auto`.
    let nvidia = opts.profile == Profile::Full
        && opts.gpu == GpuMode::Auto
        && detect_nvidia().await.is_some();
    let plan = opts.plan(&manifest.install, nvidia)?;
    let emit = |m: &str| {
        progress.emit(ProgressEvent {
            stage: "setup".into(),
            pct: None,
            message: Some(m.to_string()),
        })
    };
    emit("installing ffmpeg, yt-dlp, uv and fonts");

    let (ff, yt, uv, fonts) = tokio::join!(
        tool_step(
            "ffmpeg",
            vec![&paths.ffmpeg, &paths.ffprobe],
            VersionKind::Ffmpeg,
            &manifest,
            force,
            || ffmpeg_install::install(paths, platform, progress),
        ),
        tool_step(
            "yt-dlp",
            vec![&paths.ytdlp],
            VersionKind::Ytdlp,
            &manifest,
            force,
            || ytdlp_install::install(paths, platform, progress),
        ),
        tool_step(
            "uv",
            vec![&paths.uv],
            VersionKind::Uv,
            &manifest,
            force,
            || uv_install::install(paths, platform, progress),
        ),
        async {
            if !force && fonts::all_installed(paths) {
                let c = Component {
                    name: "fonts".into(),
                    version: "google/fonts@main".into(),
                    path: paths.fonts.clone(),
                    sha256: None,
                };
                return Ok((c, false));
            }
            fonts::install(paths, force, progress)
                .await
                .map(|c| (c, true))
                .map_err(|e| fail("fonts", e))
        },
    );

    // The engine needs uv (and nvidia detection for the cuda extra), so it runs afterwards.
    emit(&format!(
        "installing the Python engine (base{})",
        plan.extras.iter().map(|e| format!(" + {e}")).collect::<String>()
    ));
    let want_version = engine_install::expected_version(&plan.extras);
    let engine = async {
        if !force && paths.venv_python.is_file() {
            if let (Some(c), Some(want)) = (manifest.components.get("engine"), &want_version) {
                if &c.version == want {
                    return Ok((c.clone(), false));
                }
            }
        }
        engine_install::install(paths, &plan.extras, progress)
            .await
            .map(|c| (c, true))
            .map_err(|e| fail("engine", e))
    }
    .await;

    let mut report = SetupReport {
        plan: plan.clone(),
        ..Default::default()
    };
    let engine_ok = engine.is_ok();
    for r in [ff, yt, uv, engine, fonts] {
        match r {
            Ok((c, true)) => {
                manifest.components.insert(c.name.clone(), c.clone());
                report.installed.push(c);
            }
            Ok((c, false)) => {
                manifest.components.insert(c.name.clone(), c.clone());
                report.skipped.push(c);
            }
            Err(f) => report.failed.push(f),
        }
    }
    if engine_ok {
        manifest.install.extras = plan.extras.clone();
    }
    if opts.assets {
        emit(if plan.sfx_pack {
            "filling the asset library (built-in styles, fonts, CC0 SFX pack)"
        } else {
            "filling the asset library (built-in styles, fonts; the SFX pack is opt-in: --with sfx-pack)"
        });
        match assets::install(paths, force, plan.sfx_pack, progress).await {
            Ok(r) => {
                if plan.sfx_pack && r.warnings.is_empty() {
                    manifest.install.sfx_pack = true;
                }
                report.notes.extend(r.warnings.iter().cloned());
                let c = Component {
                    name: "assets".into(),
                    version: format!("{} built-in, {} fonts, {} sfx", r.builtins, r.fonts, r.sfx),
                    path: assets::assets_root(&paths.home),
                    sha256: None,
                };
                manifest.components.insert(c.name.clone(), c.clone());
                report.installed.push(c);
            }
            Err(e) => report.failed.push(fail("assets", e)),
        }
    }
    manifest.save(paths).map_err(|e| io("write manifest.json", e))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lite_is_the_default_and_installs_no_heavy_extras() {
        let plan = SetupOptions::default().plan(&InstallState::default(), true).unwrap();
        assert!(plan.extras.is_empty(), "lite must not add extras (even with an NVIDIA GPU): {plan:?}");
        assert_eq!(plan.whisper_models, vec!["base"]);
        assert!(!plan.sfx_pack);
        assert!(!plan.prefetch_yunet && !plan.prefetch_yamnet && !plan.prefetch_diarization && !plan.prefetch_ocr_devanagari);
    }

    #[test]
    fn full_installs_everything_and_cuda_only_with_a_gpu() {
        let opts = SetupOptions { profile: Profile::Full, ..Default::default() };
        let no_gpu = opts.plan(&InstallState::default(), false).unwrap();
        assert_eq!(no_gpu.extras, vec!["audio-events", "ocr", "speakers"]);
        assert!(no_gpu.sfx_pack && no_gpu.prefetch_yunet && no_gpu.prefetch_yamnet);
        assert!(no_gpu.prefetch_diarization && no_gpu.prefetch_ocr_devanagari);
        assert_eq!(no_gpu.whisper_models, vec!["base", "small"]);
        let gpu = opts.plan(&InstallState::default(), true).unwrap();
        assert_eq!(gpu.extras, vec!["audio-events", "cuda", "ocr", "speakers"]);
        let none = SetupOptions { gpu: GpuMode::None, ..opts }.plan(&InstallState::default(), true).unwrap();
        assert!(!none.extras.contains(&"cuda".to_string()));
    }

    #[test]
    fn with_adds_only_the_named_pieces_and_their_models() {
        let opts = SetupOptions { with: with(&["ocr,speakers", "SFX_pack"]), ..Default::default() };
        let plan = opts.plan(&InstallState::default(), false).unwrap();
        assert_eq!(plan.extras, vec!["ocr", "speakers"]);
        assert!(plan.sfx_pack);
        assert!(plan.prefetch_ocr_devanagari && plan.prefetch_diarization);
        assert!(!plan.prefetch_yamnet && !plan.prefetch_yunet);
        let cuda = SetupOptions { with: with(&["cuda"]), ..Default::default() }
            .plan(&InstallState::default(), false)
            .unwrap();
        assert_eq!(cuda.extras, vec!["cuda"]);
    }

    #[test]
    fn lite_over_an_existing_full_install_removes_nothing() {
        let prev = InstallState { extras: with(&["audio-events", "cuda", "ocr", "speakers"]), sfx_pack: true };
        let plan = SetupOptions::default().plan(&prev, false).unwrap();
        assert_eq!(plan.extras, prev.extras);
        assert!(plan.sfx_pack);
        assert_eq!(profile_name(&prev), "full");
        assert_eq!(profile_name(&InstallState::default()), "lite");
    }

    #[test]
    fn no_models_skips_every_download() {
        let opts = SetupOptions { models: false, profile: Profile::Full, ..Default::default() };
        let plan = opts.plan(&InstallState::default(), false).unwrap();
        assert!(plan.whisper_models.is_empty());
        assert!(!plan.prefetch_yunet && !plan.prefetch_ocr_devanagari);
    }

    #[test]
    fn unknown_with_piece_is_a_validation_error_with_choices() {
        let opts = SetupOptions { with: with(&["gpu-magic"]), ..Default::default() };
        let e = opts.plan(&InstallState::default(), false).unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation);
        assert!(e.hint.unwrap().contains("sfx-pack"));
    }

    #[test]
    fn manifest_round_trips_and_old_manifests_load() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ToolPaths::from_home(tmp.path());
        // a manifest written before profiles existed (no `install` key) still loads
        std::fs::write(Manifest::path(&paths), br#"{"components":{}}"#).unwrap();
        assert_eq!(installed_state(&paths), InstallState::default());
        let mut m = Manifest::load(&paths);
        m.install = InstallState { extras: with(&["ocr"]), sfx_pack: true };
        m.save(&paths).unwrap();
        assert_eq!(installed_state(&paths).extras, vec!["ocr"]);
        assert!(installed_state(&paths).sfx_pack);
    }
}
