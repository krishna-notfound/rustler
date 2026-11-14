//! Locations of installed tools under the Rustler home (`RUSTLER_HOME` or `~/.rustler`).

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolPaths {
    pub home: PathBuf,
    pub bin: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub ytdlp: PathBuf,
    pub uv: PathBuf,
    /// `home/engine`
    pub engine_dir: PathBuf,
    pub venv_python: PathBuf,
    pub models: PathBuf,
    pub fonts: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
    Ytdlp,
    Uv,
    EnginePython,
}

impl ToolPaths {
    pub fn from_home(home: &Path) -> Self {
        let _ = home;
        todo!("Task 4: ToolPaths::from_home")
    }

    /// `RUSTLER_HOME` or `~/.rustler`.
    pub fn default_home() -> PathBuf {
        todo!("Task 4: ToolPaths::default_home")
    }

    /// Path of `which` if it exists, else `DependencyMissing` with hint "run `rustler setup`".
    pub fn require(&self, which: Tool) -> rustler_core::Result<PathBuf> {
        let _ = which;
        todo!("Task 4: ToolPaths::require")
    }
}
