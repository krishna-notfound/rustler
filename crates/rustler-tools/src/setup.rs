//! `rustler setup`: idempotent install of ffmpeg, yt-dlp, uv, the Python engine,
//! whisper models and fonts into the Rustler home.

use crate::doctor::{VersionKind, detect_nvidia, tool_version};
use crate::paths::ToolPaths;
use crate::{assets, engine_install, ffmpeg_install, fonts, uv_install, ytdlp_install};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

/// Default: Lite, Auto, `["small"]`, not forced, asset library on.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct SetupOptions {
    pub profile: Profile,
    pub gpu: GpuMode,
    pub whisper_models: Vec<String>,
    pub force: bool,
    /// Fill the global asset library (`~/.rustler/assets`): built-in styles, fonts, CC0 SFX.
    /// Skip with `--no-assets`.
    pub assets: bool,
    /// Also download the optional CC0 music pack (11 FreePD tracks, about 50 MB). Off by default and never part
    /// of the default setup: the agent adds any music it wants with `asset add`. Enable with `--music-pack`.
    pub music_pack: bool,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            profile: Profile::Lite,
            gpu: GpuMode::Auto,
            whisper_models: vec!["small".to_string()],
            force: false,
            assets: true,
            music_pack: false,
        }
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
    let cuda = match opts.gpu {
        GpuMode::Cuda => true,
        GpuMode::None => false,
        GpuMode::Auto => detect_nvidia().await.is_some(),
    };
    emit("installing the Python engine");
    let want_version = engine_install::expected_version(opts.profile, cuda);
    let engine = async {
        if !force && paths.venv_python.is_file() {
            if let (Some(c), Some(want)) = (manifest.components.get("engine"), &want_version) {
                if &c.version == want {
                    return Ok((c.clone(), false));
                }
            }
        }
        engine_install::install(paths, opts.profile, cuda, progress)
            .await
            .map(|c| (c, true))
            .map_err(|e| fail("engine", e))
    }
    .await;

    let mut report = SetupReport::default();
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
    if opts.assets {
        emit(if opts.music_pack {
            "filling the asset library (built-in styles, fonts, CC0 SFX, optional CC0 music pack)"
        } else {
            "filling the asset library (built-in styles, fonts, CC0 SFX)"
        });
        match assets::install_with(paths, force, opts.music_pack, progress).await {
            Ok(r) => {
                report.notes.extend(r.warnings.iter().cloned());
                let c = Component {
                    name: "assets".into(),
                    version: format!("{} built-in, {} fonts, {} sfx, {} music", r.builtins, r.fonts, r.sfx, r.music),
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
