//! Stages `<repo>/engine` (minus `.venv`, `tests`, `__pycache__`, ...) into `$OUT_DIR/engine`
//! so `include_dir!` embeds only what `setup` extracts, never a developer's local virtualenv.
//! A missing `engine/` directory is fine: the staged copy is simply empty.

use std::fs;
use std::path::{Path, PathBuf};

const EXCLUDED: &[&str] = &[
    ".venv",
    "venv",
    "tests",
    "__pycache__",
    ".pytest_cache",
    ".pytest_tmp",
    ".mypy_cache",
    ".ruff_cache",
    ".keep",
    ".git",
];

fn excluded(name: &str) -> bool {
    EXCLUDED.contains(&name) || name.ends_with(".pyc")
}

fn stage(src: &Path, dst: &Path) {
    let Ok(rd) = fs::read_dir(src) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if excluded(&name) {
            continue;
        }
        let path = entry.path();
        let target = dst.join(&name);
        if path.is_dir() {
            fs::create_dir_all(&target).expect("create staged dir");
            stage(&path, &target);
        } else if path.is_file() {
            fs::copy(&path, &target).expect("stage engine file");
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("engine");
    let src = manifest.join("../../engine");
    // Cargo scans a directory path recursively (it would walk a local `.venv`), so only
    // watch the directory itself while it does not exist yet; afterwards each staged file
    // is watched individually (touch build.rs to pick up newly added files).
    if !src.exists() {
        println!("cargo:rerun-if-changed={}", src.display());
    }

    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("create OUT_DIR/engine");
    stage(&src, &out);
}
