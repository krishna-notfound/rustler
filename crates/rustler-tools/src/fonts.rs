//! Font download from upstream google/fonts (Anton, Poppins ExtraBold/Bold, Noto Sans Devanagari),
//! each with its `OFL.txt`, into `home/fonts/<Family>/`.

use crate::download::download;
use crate::paths::ToolPaths;
use crate::setup::Component;
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};

const BASE: &str = "https://github.com/google/fonts/raw/main/ofl";

/// (family dir name locally, upstream `ofl/<dir>`, font file names upstream)
pub(crate) const FAMILIES: &[(&str, &str, &[&str])] = &[
    ("Anton", "anton", &["Anton-Regular.ttf"]),
    // Static weights: libass on Windows cannot select weights from variable fonts.
    (
        "Poppins",
        "poppins",
        &["Poppins-ExtraBold.ttf", "Poppins-Bold.ttf"],
    ),
    (
        "NotoSansDevanagari",
        "notosansdevanagari",
        &["NotoSansDevanagari[wdth,wght].ttf"],
    ),
];

fn url_encode(name: &str) -> String {
    name.replace('[', "%5B").replace(']', "%5D").replace(',', "%2C")
}

/// Whether every font file and its OFL.txt of `family` exist.
fn family_complete(paths: &ToolPaths, local: &str, files: &[&str]) -> bool {
    let dir = paths.fonts.join(local);
    dir.join("OFL.txt").is_file() && files.iter().all(|f| dir.join(f).is_file())
}

/// Families that are fully installed (directory names under `home/fonts`).
pub fn installed(paths: &ToolPaths) -> Vec<String> {
    FAMILIES
        .iter()
        .filter(|(local, _, files)| family_complete(paths, local, files))
        .map(|(local, _, _)| local.to_string())
        .collect()
}

/// True when all families are installed.
pub fn all_installed(paths: &ToolPaths) -> bool {
    installed(paths).len() == FAMILIES.len()
}

/// Downloads every missing family; existing files are kept unless `force`.
pub async fn install(
    paths: &ToolPaths,
    force: bool,
    progress: &ProgressSink,
) -> rustler_core::Result<Component> {
    let total = FAMILIES.len();
    for (i, (local, remote, files)) in FAMILIES.iter().enumerate() {
        if !force && family_complete(paths, local, files) {
            continue;
        }
        let dir = paths.fonts.join(local);
        let mut targets: Vec<&str> = files.to_vec();
        targets.push("OFL.txt");
        for name in targets {
            let url = format!("{BASE}/{remote}/{}", url_encode(name));
            let dest = dir.join(name);
            // No upstream checksums exist for these; the file is size-checked below.
            download(&url, &dest, None, &ProgressSink::noop(), "setup.fonts").await?;
            let len = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            if len == 0 {
                let _ = std::fs::remove_file(&dest);
                return Err(RustlerError::new(
                    ErrorCode::Provider,
                    "setup.fonts",
                    format!("downloaded font file is empty: {url}"),
                )
                .retryable());
            }
        }
        progress.emit(ProgressEvent {
            stage: "setup.fonts".into(),
            pct: Some((i + 1) as f32 / total as f32),
            message: Some(format!("installed {local}")),
        });
    }
    Ok(Component {
        name: "fonts".into(),
        version: "google/fonts@main".into(),
        path: paths.fonts.clone(),
        sha256: None,
    })
}
