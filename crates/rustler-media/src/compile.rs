//! Pure filtergraph compiler: Clip + sources -> one ffmpeg invocation. (Task 9)

use crate::encoders::EncoderChoice;
use rustler_core::{Clip, MediaProbe, SourceId};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct SourceMedia {
    pub id: SourceId,
    pub path: PathBuf,
    pub probe: MediaProbe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledRender {
    /// Full ffmpeg argv (without the program).
    pub args: Vec<OsString>,
    /// Contents of `workdir/graph.txt`.
    pub filter_script: String,
    pub workdir: PathBuf,
}

pub fn compile(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    ass_file: Option<&Path>,
    fonts_dir: &Path,
    enc: &EncoderChoice,
    out: &Path,
    workdir: &Path,
) -> rustler_core::Result<CompiledRender> {
    let _ = (clip, sources, ass_file, fonts_dir, enc, out, workdir);
    todo!("Task 9: compile")
}
