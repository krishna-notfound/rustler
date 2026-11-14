//! Project folder store: paths, atomic JSON io, revision checks, event log.
//!
//! Layout under `root`: `project.json`, `events.ndjson`, `sources/<id>/source.json`,
//! `analysis/<id>/…`, `clips/<id>/clip.json`, `exports/`, `history/`.
//!
//! Signatures frozen in Task 2; bodies implemented in Task 3.

use crate::error::Result;
use crate::events::Event;
use crate::ids::{ClipId, SourceId};
use crate::model::{Clip, Project, Source};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ProjectStore {
    pub root: PathBuf,
}

impl ProjectStore {
    /// Makes dirs `sources/ analysis/ clips/ exports/ history/` and `project.json` (revision 0).
    /// Fails `Validation` if `project.json` already exists.
    pub fn create(root: &Path, name: &str) -> Result<(Self, Project)> {
        let _ = (root, name);
        todo!("Task 3: ProjectStore::create")
    }

    /// `NotFound` if there is no `project.json` in `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let _ = root;
        todo!("Task 3: ProjectStore::open")
    }

    pub fn load_project(&self) -> Result<Project> {
        todo!("Task 3: ProjectStore::load_project")
    }

    /// `RevisionConflict` if the on-disk revision != `base_revision`; increments `p.revision`.
    pub fn save_project(&self, p: &mut Project, base_revision: u64) -> Result<()> {
        let _ = (p, base_revision);
        todo!("Task 3: ProjectStore::save_project")
    }

    pub fn load_source(&self, id: &SourceId) -> Result<Source> {
        let _ = id;
        todo!("Task 3: ProjectStore::load_source")
    }

    pub fn save_source(&self, s: &Source) -> Result<()> {
        let _ = s;
        todo!("Task 3: ProjectStore::save_source")
    }

    pub fn load_clip(&self, id: &ClipId) -> Result<Clip> {
        let _ = id;
        todo!("Task 3: ProjectStore::load_clip")
    }

    /// `RevisionConflict` if the on-disk revision != `base_revision`; increments `c.revision`.
    pub fn save_clip(&self, c: &mut Clip, base_revision: u64) -> Result<()> {
        let _ = (c, base_revision);
        todo!("Task 3: ProjectStore::save_clip")
    }

    pub fn source_dir(&self, id: &SourceId) -> PathBuf {
        let _ = id;
        todo!("Task 3: ProjectStore::source_dir")
    }

    pub fn analysis_dir(&self, id: &SourceId) -> PathBuf {
        let _ = id;
        todo!("Task 3: ProjectStore::analysis_dir")
    }

    pub fn clip_dir(&self, id: &ClipId) -> PathBuf {
        let _ = id;
        todo!("Task 3: ProjectStore::clip_dir")
    }

    pub fn exports_dir(&self) -> PathBuf {
        todo!("Task 3: ProjectStore::exports_dir")
    }

    /// Read `root/rel` as JSON.
    pub fn read_json<T: DeserializeOwned>(&self, rel: &Path) -> Result<T> {
        let _ = rel;
        todo!("Task 3: ProjectStore::read_json")
    }

    /// Atomic: write a temp file in the same dir, fsync, rename over `root/rel`.
    pub fn write_json<T: Serialize>(&self, rel: &Path, v: &T) -> Result<()> {
        let _ = (rel, v);
        todo!("Task 3: ProjectStore::write_json")
    }

    /// Appends one JSON line to `events.ndjson`.
    pub fn append_event(&self, e: &Event) -> Result<()> {
        let _ = e;
        todo!("Task 3: ProjectStore::append_event")
    }

    /// Project-relative paths are joined to `root`; absolute paths are returned unchanged.
    pub fn resolve(&self, p: &Path) -> PathBuf {
        let _ = p;
        todo!("Task 3: ProjectStore::resolve")
    }

    /// Paths inside `root` become project-relative; others are returned unchanged.
    pub fn relativize(&self, p: &Path) -> PathBuf {
        let _ = p;
        todo!("Task 3: ProjectStore::relativize")
    }
}
