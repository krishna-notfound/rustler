//! Locations of installed tools under the Rustler home (`RUSTLER_HOME` or `~/.rustler`).

use rustler_core::{ErrorCode, RustlerError};
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

/// Executable file name for `stem` on this platform (`ffmpeg` -> `ffmpeg.exe` on Windows).
pub(crate) fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

impl ToolPaths {
    pub fn from_home(home: &Path) -> Self {
        let bin = home.join("bin");
        let engine_dir = home.join("engine");
        let venv_python = if cfg!(windows) {
            engine_dir.join(".venv").join("Scripts").join("python.exe")
        } else {
            engine_dir.join(".venv").join("bin").join("python")
        };
        Self {
            home: home.to_path_buf(),
            ffmpeg: bin.join(exe_name("ffmpeg")),
            ffprobe: bin.join(exe_name("ffprobe")),
            ytdlp: bin.join(exe_name("yt-dlp")),
            uv: bin.join(exe_name("uv")),
            bin,
            engine_dir,
            venv_python,
            models: home.join("models"),
            fonts: home.join("fonts"),
        }
    }

    /// `RUSTLER_HOME` or `~/.rustler`.
    pub fn default_home() -> PathBuf {
        if let Some(h) = std::env::var_os("RUSTLER_HOME") {
            if !h.is_empty() {
                return PathBuf::from(h);
            }
        }
        let user_home = if cfg!(windows) {
            std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
        } else {
            std::env::var_os("HOME")
        };
        match user_home {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".rustler"),
            _ => PathBuf::from(".rustler"),
        }
    }

    /// Path of `which` if it exists, else `DependencyMissing` with hint "run `rustler setup`".
    pub fn require(&self, which: Tool) -> rustler_core::Result<PathBuf> {
        let (name, path) = match which {
            Tool::Ffmpeg => ("ffmpeg", &self.ffmpeg),
            Tool::Ffprobe => ("ffprobe", &self.ffprobe),
            Tool::Ytdlp => ("yt-dlp", &self.ytdlp),
            Tool::Uv => ("uv", &self.uv),
            Tool::EnginePython => ("engine python", &self.venv_python),
        };
        if path.is_file() {
            Ok(path.clone())
        } else {
            Err(RustlerError::new(
                ErrorCode::DependencyMissing,
                "tools",
                format!("{name} is not installed (expected at {})", path.display()),
            )
            .hint("run `rustler setup`"))
        }
    }
}
