//! Extract named executables out of `.zip`, `.tar.xz` and `.tar.gz` archives.

use rustler_core::{ErrorCode, RustlerError};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

fn err(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, "extract", msg)
}

fn base_name(entry_path: &str) -> &str {
    entry_path.rsplit(['/', '\\']).next().unwrap_or(entry_path)
}

fn write_exe(reader: &mut dyn Read, dest_dir: &Path, name: &str) -> io::Result<PathBuf> {
    let dest = dest_dir.join(name);
    let part = dest_dir.join(format!("{name}.part"));
    {
        let mut out = File::create(&part)?;
        io::copy(reader, &mut out)?;
        out.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

/// Extracts every file in `archive` whose base name is in `wanted` into `dest_dir`
/// (flattened). Errors if any wanted name is missing from the archive. Blocking.
pub fn extract_binaries(
    archive: &Path,
    wanted: &[&str],
    dest_dir: &Path,
) -> rustler_core::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dest_dir).map_err(|e| err(format!("create {}: {e}", dest_dir.display())))?;
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut found: Vec<PathBuf> = Vec::new();
    let io_e = |e: io::Error| err(format!("extract {}: {e}", archive.display()));

    if name.ends_with(".zip") {
        let f = File::open(archive).map_err(io_e)?;
        let mut zip = zip::ZipArchive::new(f)
            .map_err(|e| err(format!("open zip {}: {e}", archive.display())))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| err(format!("read zip entry: {e}")))?;
            if !entry.is_file() {
                continue;
            }
            let Ok(full) = entry.name().map(|n| n.into_owned()) else {
                continue;
            };
            let base = base_name(&full).to_string();
            if wanted.contains(&base.as_str()) {
                found.push(write_exe(&mut entry, dest_dir, &base).map_err(io_e)?);
            }
        }
    } else if name.ends_with(".tar.xz") || name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let f = File::open(archive).map_err(io_e)?;
        let reader: Box<dyn Read> = if name.ends_with(".tar.xz") {
            Box::new(xz2::read::XzDecoder::new(f))
        } else {
            Box::new(flate2::read::GzDecoder::new(f))
        };
        let mut tar = tar::Archive::new(reader);
        for entry in tar.entries().map_err(io_e)? {
            let mut entry = entry.map_err(io_e)?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path().map_err(io_e)?.to_string_lossy().into_owned();
            let base = base_name(&path).to_string();
            if wanted.contains(&base.as_str()) {
                found.push(write_exe(&mut entry, dest_dir, &base).map_err(io_e)?);
            }
        }
    } else {
        return Err(err(format!("unsupported archive type: {}", archive.display())));
    }

    for w in wanted {
        if !found.iter().any(|p| p.file_name().is_some_and(|n| n == *w)) {
            return Err(err(format!("`{w}` not found in {}", archive.display())));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_from_zip_flattened() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("a.zip");
        {
            let f = File::create(&zip_path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("pkg/bin/tool.exe", opts).unwrap();
            z.write_all(b"BIN").unwrap();
            z.start_file("pkg/doc/readme.txt", opts).unwrap();
            z.write_all(b"doc").unwrap();
            z.finish().unwrap();
        }
        let out = dir.path().join("out");
        let got = extract_binaries(&zip_path, &["tool.exe"], &out).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(std::fs::read(out.join("tool.exe")).unwrap(), b"BIN");
        assert!(!out.join("readme.txt").exists());
        assert!(extract_binaries(&zip_path, &["missing.exe"], &out).is_err());
    }
}
