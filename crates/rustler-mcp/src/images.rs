//! Finding frame/contact-sheet images in command output and turning them into
//! MCP `image` content items (so vision-capable agents can SEE them).

use base64::Engine as _;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Max raw bytes of one image before it is downscaled/re-encoded.
pub const MAX_IMAGE_BYTES: usize = 1_500_000;
/// Max images attached to one tool result.
pub const MAX_IMAGES: usize = 12;

fn mime_for(path: &Path) -> Option<&'static str> {
    match path
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        _ => None,
    }
}

fn is_image_key(key: &str) -> bool {
    matches!(key, "image" | "contact_sheet" | "image_path" | "path")
}

fn candidate(s: &str, base: Option<&Path>) -> Option<PathBuf> {
    let p = PathBuf::from(s);
    mime_for(&p)?;
    if p.is_file() {
        return Some(p);
    }
    if p.is_relative() {
        if let Some(b) = base {
            let j = b.join(&p);
            if j.is_file() {
                return Some(j);
            }
        }
    }
    None
}

/// Walks `v` collecting existing image files named by `image`, `contact_sheet`,
/// `image_path` or `path` string fields (also arrays of such strings under the
/// first three). Order of appearance, de-duplicated, capped at [`MAX_IMAGES`].
pub fn collect_image_paths(v: &Value, base: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(v, base, &mut out);
    out
}

fn push(out: &mut Vec<PathBuf>, p: PathBuf) {
    if out.len() < MAX_IMAGES && !out.contains(&p) {
        out.push(p);
    }
}

fn walk(v: &Value, base: Option<&Path>, out: &mut Vec<PathBuf>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                match val {
                    Value::String(s) if is_image_key(k) => {
                        if let Some(p) = candidate(s, base) {
                            push(out, p);
                        }
                    }
                    Value::Array(items) if is_image_key(k) && k != "path" => {
                        for it in items {
                            if let Value::String(s) = it {
                                if let Some(p) = candidate(s, base) {
                                    push(out, p);
                                }
                            } else {
                                walk(it, base, out);
                            }
                        }
                    }
                    _ => walk(val, base, out),
                }
            }
        }
        Value::Array(items) => {
            for it in items {
                walk(it, base, out);
            }
        }
        _ => {}
    }
}

/// Reads the file; returns `(base64, mime)`. Files over [`MAX_IMAGE_BYTES`] are
/// decoded, downscaled and re-encoded as JPEG until they fit. `None` on any failure.
pub fn load_image(path: &Path) -> Option<(String, &'static str)> {
    let mime = mime_for(path)?;
    let bytes = std::fs::read(path).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD;
    if bytes.len() <= MAX_IMAGE_BYTES {
        return Some((b64.encode(&bytes), mime));
    }
    let img = image::load_from_memory(&bytes).ok()?;
    let mut scale = ((MAX_IMAGE_BYTES as f64 / bytes.len() as f64).sqrt() * 0.9).min(0.9);
    for _ in 0..8 {
        let w = ((img.width() as f64 * scale).round() as u32).max(16);
        let h = ((img.height() as f64 * scale).round() as u32).max(16);
        let small = img
            .resize(w, h, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let mut buf = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
        if small.write_with_encoder(enc).is_ok() && buf.len() <= MAX_IMAGE_BYTES {
            return Some((b64.encode(&buf), "image/jpeg"));
        }
        scale *= 0.75;
    }
    None
}

/// MCP image content items for every image found in `output`.
pub fn image_items(output: &Value, base: Option<&Path>) -> Vec<Value> {
    collect_image_paths(output, base)
        .iter()
        .filter_map(|p| load_image(p))
        .map(|(data, mime)| json!({"type": "image", "data": data, "mimeType": mime}))
        .collect()
}
