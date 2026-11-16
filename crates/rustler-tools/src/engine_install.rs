//! Extract the embedded Python engine into `home/engine`, then `uv python install 3.11`
//! and `uv sync --project home/engine`.
//!
//! The engine is embedded from `$OUT_DIR/engine`, a copy of the repo's `engine/` staged by
//! `build.rs` (it leaves out `.venv`, `tests`, `__pycache__`, ...). Extraction applies the
//! same exclusions again. Behind cargo feature `embed-engine` (default on).

use crate::paths::ToolPaths;
use crate::run::run_streaming_with_env;
use crate::setup::Component;
use include_dir::{Dir, DirEntry, File};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Component as PathComponent, Path};
use tokio_util::sync::CancellationToken;

pub const PYTHON_VERSION: &str = "3.11";

#[cfg(feature = "embed-engine")]
static ENGINE: Dir<'static> = include_dir::include_dir!("$OUT_DIR/engine");

/// The embedded engine tree, or `None` when built without `embed-engine`.
pub fn embedded() -> Option<&'static Dir<'static>> {
    #[cfg(feature = "embed-engine")]
    {
        Some(&ENGINE)
    }
    #[cfg(not(feature = "embed-engine"))]
    {
        None
    }
}

/// True when this build embeds a real engine package (`pyproject.toml` present).
pub fn has_embedded_project() -> bool {
    embedded().is_some_and(|d| d.get_file("pyproject.toml").is_some())
}

/// Paths with any component named like this are never extracted.
const EXCLUDED: &[&str] = &[
    ".venv",
    "tests",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".keep",
];

/// Whether a path relative to the engine root is left out of extraction.
pub fn is_excluded(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        PathComponent::Normal(n) => {
            let n = n.to_string_lossy();
            EXCLUDED.contains(&n.as_ref()) || n.ends_with(".pyc")
        }
        _ => false,
    })
}

fn walk<'a>(dir: &'a Dir<'a>, f: &mut impl FnMut(&'a File<'a>)) {
    for e in dir.entries() {
        match e {
            DirEntry::Dir(d) => {
                if !is_excluded(d.path()) {
                    walk(d, f);
                }
            }
            DirEntry::File(file) => {
                if !is_excluded(file.path()) {
                    f(file);
                }
            }
        }
    }
}

/// sha256 over (path, contents) of every extractable file, in tree order.
pub fn tree_hash(dir: &Dir<'_>) -> String {
    let mut files: Vec<(String, &[u8])> = Vec::new();
    walk(dir, &mut |f| {
        files.push((f.path().to_string_lossy().replace('\\', "/"), f.contents()))
    });
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (p, c) in files {
        h.update(p.as_bytes());
        h.update([0u8]);
        h.update((c.len() as u64).to_le_bytes());
        h.update(c);
    }
    crate::download::hex(&h.finalize())
}

/// Writes the extractable files of `dir` under `dest`, replacing changed files only.
/// Returns the number of files written.
pub fn extract_dir(dir: &Dir<'_>, dest: &Path) -> rustler_core::Result<usize> {
    let mut written = 0usize;
    let mut failure: Option<RustlerError> = None;
    walk(dir, &mut |f| {
        if failure.is_some() {
            return;
        }
        let target = dest.join(f.path());
        let res = (|| -> std::io::Result<bool> {
            if let Ok(existing) = std::fs::read(&target) {
                if existing == f.contents() {
                    return Ok(false);
                }
            }
            if let Some(p) = target.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&target, f.contents())?;
            Ok(true)
        })();
        match res {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => {
                failure = Some(RustlerError::new(
                    ErrorCode::Internal,
                    "setup.engine",
                    format!("write {}: {e}", target.display()),
                ));
            }
        }
    });
    match failure {
        Some(e) => Err(e),
        None => Ok(written),
    }
}

/// Environment for every `uv` call: Python and cache live under the Rustler home.
pub fn uv_env(paths: &ToolPaths) -> Vec<(OsString, OsString)> {
    vec![
        (
            "UV_PYTHON_INSTALL_DIR".into(),
            paths.home.join("python").into_os_string(),
        ),
        (
            "UV_CACHE_DIR".into(),
            paths.home.join("uv-cache").into_os_string(),
        ),
    ]
}

/// Version string recorded in the manifest (`<engine hash>+base` or `<hash>+ocr+speakers`); a
/// change (new engine code, an extra added) triggers a re-sync.
pub fn expected_version(extras: &[String]) -> Option<String> {
    let dir = embedded()?;
    let suffix = if extras.is_empty() {
        "+base".to_string()
    } else {
        extras.iter().map(|e| format!("+{e}")).collect::<String>()
    };
    Some(format!("{}{suffix}", &tree_hash(dir)[..12]))
}

fn emit(progress: &ProgressSink, message: impl Into<String>) {
    progress.emit(ProgressEvent {
        stage: "setup.engine".into(),
        pct: None,
        message: Some(message.into()),
        eta_s: None,
    });
}

async fn uv_step(
    paths: &ToolPaths,
    args: Vec<OsString>,
    what: &str,
    progress: &ProgressSink,
) -> rustler_core::Result<()> {
    let uv = paths.require(crate::paths::Tool::Uv)?;
    let env = uv_env(paths);
    let mut tail: Vec<String> = Vec::new();
    let status = run_streaming_with_env(
        &uv,
        &args,
        Some(&paths.home),
        &env,
        &CancellationToken::new(),
        |line| {
            emit(progress, format!("{what}: {line}"));
            tail.push(line.to_string());
            if tail.len() > 20 {
                tail.remove(0);
            }
        },
    )
    .await?;
    if status.success() {
        Ok(())
    } else {
        Err(RustlerError::new(
            ErrorCode::DependencyMissing,
            "setup.engine",
            format!("`uv {what}` failed ({status}): {}", tail.join("\n")),
        )
        .hint("re-run `rustler setup`; completed steps are skipped")
        .retryable())
    }
}

/// Extracts the engine, installs Python 3.11 and syncs the venv with the base dependencies plus
/// `extras` (`ocr`, `audio-events`, `speakers`, `cuda`). The sync is `--inexact`: nothing already
/// installed is ever removed, so running a lighter profile over a fuller install is harmless.
pub async fn install(
    paths: &ToolPaths,
    extras: &[String],
    progress: &ProgressSink,
) -> rustler_core::Result<Component> {
    if !has_embedded_project() {
        return Err(RustlerError::new(
            ErrorCode::DependencyMissing,
            "setup.engine",
            "this build has no embedded Python engine (engine/pyproject.toml was absent at build time)",
        )
        .hint("rebuild rustler with the `engine/` directory present"));
    }
    let dir = embedded().expect("checked above");
    let version = expected_version(extras).expect("embedded");
    let hash = tree_hash(dir);

    emit(progress, "extracting engine");
    let dest = paths.engine_dir.clone();
    let n = tokio::task::spawn_blocking(move || {
        let dir = embedded().expect("embedded");
        extract_dir(dir, &dest)
    })
    .await
    .map_err(|e| RustlerError::new(ErrorCode::Internal, "setup.engine", format!("join: {e}")))??;
    tracing::debug!(files = n, "engine extracted");

    emit(progress, format!("installing Python {PYTHON_VERSION}"));
    uv_step(
        paths,
        vec!["python".into(), "install".into(), PYTHON_VERSION.into()],
        "python install",
        progress,
    )
    .await?;

    emit(
        progress,
        if extras.is_empty() {
            "syncing the base engine dependencies (lightweight; add heavy analyzers later with `setup run --with ocr|speakers|audio-events|cuda`)".to_string()
        } else {
            format!("syncing engine dependencies with extras: {} (this can take several minutes)", extras.join(", "))
        },
    );
    let mut args: Vec<OsString> = vec![
        "sync".into(),
        "--project".into(),
        paths.engine_dir.clone().into_os_string(),
        "--python".into(),
        PYTHON_VERSION.into(),
        "--no-dev".into(),
        // never uninstall packages that an earlier (fuller) setup added
        "--inexact".into(),
    ];
    for e in extras {
        args.push("--extra".into());
        args.push(e.into());
    }
    uv_step(paths, args, "sync", progress).await?;

    if !paths.venv_python.is_file() {
        return Err(RustlerError::new(
            ErrorCode::DependencyMissing,
            "setup.engine",
            format!(
                "uv sync finished but {} does not exist",
                paths.venv_python.display()
            ),
        ));
    }
    Ok(Component {
        name: "engine".into(),
        version,
        path: paths.engine_dir.clone(),
        sha256: Some(hash),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "embed-engine")]
    #[test]
    fn version_names_the_extras() {
        let v = |e: &[&str]| {
            expected_version(&e.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        if let (Some(base), Some(ocr)) = (v(&[]), v(&["ocr", "speakers"])) {
            assert!(base.ends_with("+base"));
            assert!(ocr.ends_with("+ocr+speakers"));
            assert_ne!(base, ocr);
        }
    }

    #[test]
    fn exclusions() {
        assert!(is_excluded(Path::new(".venv/Lib/x.py")));
        assert!(is_excluded(Path::new("tests/test_a.py")));
        assert!(is_excluded(Path::new("rustler_engine/__pycache__/a.pyc")));
        assert!(is_excluded(Path::new("rustler_engine/a.pyc")));
        assert!(is_excluded(Path::new(".keep")));
        assert!(!is_excluded(Path::new("rustler_engine/rpc.py")));
        assert!(!is_excluded(Path::new("pyproject.toml")));
    }

    #[cfg(feature = "embed-engine")]
    #[test]
    fn extract_sample_tree() {
        static SAMPLE: Dir<'static> =
            include_dir::include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/engine_sample");
        let tmp = tempfile::tempdir().unwrap();
        let n = extract_dir(&SAMPLE, tmp.path()).unwrap();
        assert_eq!(n, 2);
        assert!(tmp.path().join("pyproject.toml").is_file());
        assert!(tmp.path().join("pkg/mod.py").is_file());
        assert!(!tmp.path().join("tests").exists());
        // second run: nothing changed, nothing rewritten
        assert_eq!(extract_dir(&SAMPLE, tmp.path()).unwrap(), 0);
        assert_eq!(tree_hash(&SAMPLE), tree_hash(&SAMPLE));
    }
}
