//! Persisted desktop state (`<RUSTLER_HOME>/desktop.json`) and the recent-projects list read from
//! `<RUSTLER_HOME>/projects.json` (written by `project.open` / `project.create`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default inner size in logical pixels.
pub const DEFAULT_SIZE: (f64, f64) = (1440.0, 900.0);
/// Minimum inner size in logical pixels.
pub const MIN_SIZE: (f64, f64) = (1280.0, 800.0);
pub const STATE_FILE: &str = "desktop.json";
pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2.0;

fn one() -> f64 {
    1.0
}

/// Window geometry and zoom remembered between runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowState {
    /// Logical inner width.
    pub width: f64,
    /// Logical inner height.
    pub height: f64,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default = "one")]
    pub zoom: f64,
}

impl Default for WindowState {
    fn default() -> Self {
        Self { width: DEFAULT_SIZE.0, height: DEFAULT_SIZE.1, maximized: false, zoom: 1.0 }
    }
}

impl WindowState {
    /// Clamps the size to the minimum and the zoom to its range; replaces non-finite values.
    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        if !self.width.is_finite() {
            self.width = d.width;
        }
        if !self.height.is_finite() {
            self.height = d.height;
        }
        if !self.zoom.is_finite() || self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
        self.width = self.width.clamp(MIN_SIZE.0, 16384.0);
        self.height = self.height.clamp(MIN_SIZE.1, 16384.0);
        self.zoom = self.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        self
    }

    /// Loads `<home>/desktop.json`; any missing or broken file gives the defaults.
    pub fn load(home: &Path) -> Self {
        std::fs::read_to_string(home.join(STATE_FILE))
            .ok()
            .and_then(|s| serde_json::from_str::<WindowState>(&s).ok())
            .unwrap_or_default()
            .sanitized()
    }

    /// Writes `<home>/desktop.json` (temp file + rename).
    pub fn save(&self, home: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(home)?;
        let tmp = home.join(format!("{STATE_FILE}.tmp"));
        let body = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, home.join(STATE_FILE))
    }
}

/// One entry of the recent-projects menu.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub name: String,
}

#[derive(Deserialize)]
struct Registry {
    #[serde(default)]
    projects: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    last_opened: String,
}

/// Most recently opened projects first, only folders that still hold a `project.json`,
/// without duplicates, at most `limit`.
pub fn recent_projects(home: &Path, limit: usize) -> Vec<RecentProject> {
    let Some(reg) = std::fs::read_to_string(home.join("projects.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Registry>(&s).ok())
    else {
        return Vec::new();
    };
    let mut entries = reg.projects;
    // RFC 3339 UTC timestamps from the same writer sort correctly as strings.
    entries.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    let mut out: Vec<RecentProject> = Vec::new();
    for e in entries {
        let path = PathBuf::from(&e.path);
        if !path.join("project.json").is_file() {
            continue;
        }
        let key = e.path.to_lowercase();
        if out.iter().any(|r| r.path.to_string_lossy().to_lowercase() == key) {
            continue;
        }
        let name = if e.name.is_empty() { folder_name(&path) } else { e.name };
        out.push(RecentProject { path, name });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Last path component (for titles and menu labels).
pub fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Whether `dir` is a Rustler project folder.
pub fn is_project(dir: &Path) -> bool {
    dir.join("project.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_defaults_when_missing_or_broken() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(WindowState::load(d.path()), WindowState::default());
        std::fs::write(d.path().join(STATE_FILE), "{not json").unwrap();
        assert_eq!(WindowState::load(d.path()), WindowState::default());
    }

    #[test]
    fn state_round_trips_and_clamps() {
        let d = tempfile::tempdir().unwrap();
        let s = WindowState { width: 1600.0, height: 1000.0, maximized: true, zoom: 1.25 };
        s.save(d.path()).unwrap();
        assert_eq!(WindowState::load(d.path()), s);
        let small = WindowState { width: 300.0, height: 200.0, maximized: false, zoom: 9.0 };
        small.save(d.path()).unwrap();
        let l = WindowState::load(d.path());
        assert_eq!((l.width, l.height, l.zoom), (MIN_SIZE.0, MIN_SIZE.1, ZOOM_MAX));
    }

    #[test]
    fn recent_projects_sorted_existing_deduped() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("project.json"), "{}").unwrap();
        std::fs::write(b.join("project.json"), "{}").unwrap();
        let gone = d.path().join("gone");
        let reg = serde_json::json!({"projects": [
            {"path": a, "name": "a", "last_opened": "2026-11-14T10:00:00Z"},
            {"path": b, "name": "", "last_opened": "2026-11-15T10:00:00Z"},
            {"path": gone, "name": "gone", "last_opened": "2026-11-16T10:00:00Z"},
            {"path": a, "name": "a-old", "last_opened": "2026-11-01T10:00:00Z"}
        ]});
        std::fs::write(d.path().join("projects.json"), reg.to_string()).unwrap();
        let r = recent_projects(d.path(), 10);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], RecentProject { path: b.clone(), name: "b".into() });
        assert_eq!(r[1].name, "a");
        assert_eq!(recent_projects(d.path(), 1).len(), 1);
        assert!(recent_projects(&d.path().join("nohome"), 10).is_empty());
    }
}
