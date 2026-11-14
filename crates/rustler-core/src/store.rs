//! Project folder store: paths, atomic JSON io, revision checks, event log.
//!
//! Layout under `root`: `project.json`, `events.ndjson`, `sources/<id>/source.json`,
//! `analysis/<id>/…`, `clips/<id>/clip.json`, `clips/<id>/history/<rev>.clip.json`,
//! `exports/`, `history/`.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::error::{ErrorCode, Result, RustlerError};
use crate::events::Event;
use crate::ids::{ClipId, ProjectId, SourceId};
use crate::model::{Clip, Permissions, Project, ProjectSettings, Source};
use chrono::Utc;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};

const STAGE: &str = "store";
const PROJECT_FILE: &str = "project.json";
const EVENTS_FILE: &str = "events.ndjson";

#[derive(Clone, Debug)]
pub struct ProjectStore {
    pub root: PathBuf,
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> RustlerError {
    let code = if e.kind() == std::io::ErrorKind::NotFound { ErrorCode::NotFound } else { ErrorCode::Internal };
    RustlerError::new(code, STAGE, format!("{what} {}: {e}", path.display()))
}

impl ProjectStore {
    /// Makes dirs `sources/ analysis/ clips/ exports/ history/` and `project.json` (revision 0).
    /// Fails `Validation` if `project.json` already exists.
    pub fn create(root: &Path, name: &str) -> Result<(Self, Project)> {
        if root.join(PROJECT_FILE).exists() {
            return Err(RustlerError::new(
                ErrorCode::Validation,
                "project.create",
                format!("a project already exists in {}", root.display()),
            )
            .hint("pick another directory or open the existing project with --project"));
        }
        for d in ["sources", "analysis", "clips", "exports", "history"] {
            let p = root.join(d);
            std::fs::create_dir_all(&p).map_err(|e| io_err("create dir", &p, e))?;
        }
        let store = Self { root: root.to_path_buf() };
        let project = Project {
            schema: "rustler.project@1".to_string(),
            id: ProjectId::new(),
            name: name.to_string(),
            revision: 0,
            created_at: Utc::now(),
            sources: Vec::new(),
            clips: Vec::new(),
            permissions: Permissions::default(),
            settings: ProjectSettings::default(),
        };
        store.write_json(Path::new(PROJECT_FILE), &project)?;
        Ok((store, project))
    }

    /// `NotFound` if there is no `project.json` in `root`.
    pub fn open(root: &Path) -> Result<Self> {
        if !root.join(PROJECT_FILE).is_file() {
            return Err(RustlerError::new(
                ErrorCode::NotFound,
                "project.open",
                format!("no project.json in {}", root.display()),
            )
            .hint("run `rustler project create` or pass --project"));
        }
        Ok(Self { root: root.to_path_buf() })
    }

    pub fn load_project(&self) -> Result<Project> {
        self.read_json(Path::new(PROJECT_FILE))
    }

    /// `RevisionConflict` if the on-disk revision != `base_revision`; increments `p.revision`.
    pub fn save_project(&self, p: &mut Project, base_revision: u64) -> Result<()> {
        let on_disk = self.load_project()?.revision;
        if on_disk != base_revision {
            return Err(conflict("project", on_disk, base_revision));
        }
        p.revision = base_revision + 1;
        self.write_json(Path::new(PROJECT_FILE), p)
    }

    pub fn load_source(&self, id: &SourceId) -> Result<Source> {
        self.read_json(&Path::new("sources").join(id.as_str()).join("source.json"))
    }

    pub fn save_source(&self, s: &Source) -> Result<()> {
        self.write_json(&Path::new("sources").join(s.id.as_str()).join("source.json"), s)
    }

    pub fn load_clip(&self, id: &ClipId) -> Result<Clip> {
        self.read_json(&clip_rel(id))
    }

    /// `RevisionConflict` if the on-disk revision != `base_revision`; increments `c.revision`.
    /// A clip not yet on disk counts as revision 0. The previous on-disk clip is first
    /// copied to `clips/<id>/history/<old_revision>.clip.json` (undo snapshot).
    pub fn save_clip(&self, c: &mut Clip, base_revision: u64) -> Result<()> {
        let rel = clip_rel(&c.id);
        let previous: Option<Clip> = if self.resolve(&rel).is_file() { Some(self.read_json(&rel)?) } else { None };
        let on_disk = previous.as_ref().map_or(0, |p| p.revision);
        if on_disk != base_revision {
            return Err(conflict("clip", on_disk, base_revision));
        }
        if let Some(prev) = &previous {
            self.write_json(&snapshot_rel(&c.id, prev.revision), prev)?;
        }
        c.revision = base_revision + 1;
        self.write_json(&rel, c)
    }

    /// Additive (5-hour scope): the clip as it was at `revision`, from
    /// `clips/<id>/history/<revision>.clip.json`. `NotFound` if no such snapshot exists.
    pub fn load_clip_snapshot(&self, id: &ClipId, revision: u64) -> Result<Clip> {
        self.read_json(&snapshot_rel(id, revision)).map_err(|e| {
            if e.code == ErrorCode::NotFound {
                RustlerError::new(ErrorCode::NotFound, STAGE, format!("no snapshot of clip {id} at revision {revision}"))
                    .hint("snapshots are kept for every revision before the current one")
            } else {
                e
            }
        })
    }

    pub fn source_dir(&self, id: &SourceId) -> PathBuf {
        self.root.join("sources").join(id.as_str())
    }

    pub fn analysis_dir(&self, id: &SourceId) -> PathBuf {
        self.root.join("analysis").join(id.as_str())
    }

    pub fn clip_dir(&self, id: &ClipId) -> PathBuf {
        self.root.join("clips").join(id.as_str())
    }

    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// Read `root/rel` as JSON.
    pub fn read_json<T: DeserializeOwned>(&self, rel: &Path) -> Result<T> {
        let path = self.resolve(rel);
        let bytes = std::fs::read(&path).map_err(|e| io_err("read", &path, e))?;
        serde_json::from_slice(&bytes).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, STAGE, format!("invalid JSON in {}: {e}", path.display()))
                .hint("the file may be corrupt or from a newer rustler version")
        })
    }

    /// Atomic: write a temp file in the same dir, fsync, rename over `root/rel`.
    pub fn write_json<T: Serialize>(&self, rel: &Path, v: &T) -> Result<()> {
        let path = self.resolve(rel);
        let json = serde_json::to_vec_pretty(v)
            .map_err(|e| RustlerError::new(ErrorCode::Internal, STAGE, format!("serialize {}: {e}", path.display())))?;
        let dir = path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(dir).map_err(|e| io_err("create dir", dir, e))?;
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = dir.join(format!("{file_name}.tmp{}", ulid::Ulid::generate().to_string().to_lowercase()));
        let written = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &path)
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err("write", &path, e));
        }
        Ok(())
    }

    /// Appends one JSON line to `events.ndjson`.
    pub fn append_event(&self, e: &Event) -> Result<()> {
        let path = self.root.join(EVENTS_FILE);
        let mut line = serde_json::to_vec(e)
            .map_err(|err| RustlerError::new(ErrorCode::Internal, STAGE, format!("serialize event: {err}")))?;
        line.push(b'\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|err| io_err("open", &path, err))?;
        f.write_all(&line).map_err(|err| io_err("append", &path, err))
    }

    /// Project-relative paths are joined to `root`; absolute paths are returned unchanged.
    pub fn resolve(&self, p: &Path) -> PathBuf {
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) }
    }

    /// Paths inside `root` become project-relative; others are returned unchanged.
    pub fn relativize(&self, p: &Path) -> PathBuf {
        match p.strip_prefix(&self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.to_path_buf(),
            _ => p.to_path_buf(),
        }
    }
}

fn clip_rel(id: &ClipId) -> PathBuf {
    Path::new("clips").join(id.as_str()).join("clip.json")
}

fn snapshot_rel(id: &ClipId, revision: u64) -> PathBuf {
    Path::new("clips").join(id.as_str()).join("history").join(format!("{revision}.clip.json"))
}

fn conflict(what: &str, on_disk: u64, base: u64) -> RustlerError {
    RustlerError::new(
        ErrorCode::RevisionConflict,
        STAGE,
        format!("{what} is at revision {on_disk} but the change was based on revision {base}"),
    )
    .hint(format!("reload the {what} and re-apply the change"))
}
