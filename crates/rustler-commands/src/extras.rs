//! Shared read-modify-write access to the `clips/<id>/extras.json` sidecar: clip data the frozen
//! `Clip` model has no field for (`music`, `crop_paths`, `overlays`, `audio_fx`, `sfx`,
//! `transitions`, `effects`, `progress_bar`, ...).
//!
//! Every writer goes through [`update`], which changes exactly one top-level key and keeps all
//! others byte-for-byte (as JSON values), so command modules never clobber each other's data.
//! Writes are atomic (`ProjectStore::write_json`: temp file + rename) and serialized within the
//! process.

use rustler_core::{Clip, ClipId, ErrorCode, ProjectStore, RustlerError, Segment};
use rustler_media::facepath::CropKey;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

type Result<T> = rustler_core::Result<T>;

/// Serializes read-modify-write cycles inside this process.
static LOCK: Mutex<()> = Mutex::new(());

/// Project-relative path of a clip's sidecar.
pub fn rel(clip: &ClipId) -> PathBuf {
    Path::new("clips").join(clip.as_str()).join("extras.json")
}

fn read(store: &ProjectStore, clip: &ClipId) -> Result<Map<String, Value>> {
    let rel = rel(clip);
    if !store.resolve(&rel).is_file() {
        return Ok(Map::new());
    }
    match store.read_json::<Value>(&rel)? {
        Value::Object(m) => Ok(m),
        Value::Null => Ok(Map::new()),
        _ => Err(RustlerError::new(
            ErrorCode::Internal,
            "extras",
            format!("{} is not a JSON object", store.resolve(&rel).display()),
        )
        .hint("fix or delete the file; it holds music, crop paths and overlays of the clip")),
    }
}

/// All keys of the sidecar (empty when the file does not exist).
pub fn load(store: &ProjectStore, clip: &ClipId) -> Result<Map<String, Value>> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    read(store, clip)
}

/// One key, deserialized; `None` when the key is absent or null.
pub fn get<T: DeserializeOwned>(store: &ProjectStore, clip: &ClipId, key: &str) -> Result<Option<T>> {
    from_map(&load(store, clip)?, key)
}

/// One key of an already loaded map, deserialized; `None` when absent or null.
pub fn from_map<T: DeserializeOwned>(map: &Map<String, Value>, key: &str) -> Result<Option<T>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| {
            RustlerError::new(ErrorCode::Internal, "extras", format!("extras.json key '{key}' is invalid: {e}"))
                .hint("fix the key in clips/<clip_id>/extras.json or remove it")
        }),
    }
}

/// Replaces the whole sidecar (prefer [`update`], which keeps other keys).
pub fn save(store: &ProjectStore, clip: &ClipId, map: &Map<String, Value>) -> Result<()> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    store.write_json(&rel(clip), map)
}

/// Read-modify-write of one top-level key: `f` gets the current value (`None` when absent) and
/// returns the new one (`None` removes the key). Other keys are preserved. The write is atomic.
/// When `f` errors nothing is written.
pub fn update(
    store: &ProjectStore,
    clip: &ClipId,
    key: &str,
    f: impl FnOnce(Option<Value>) -> Result<Option<Value>>,
) -> Result<()> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut map = read(store, clip)?;
    if let Some(v) = f(map.remove(key))? {
        map.insert(key.to_string(), v);
    }
    store.write_json(&rel(clip), &map)
}

/// Keeps face-tracked crops attached after a timeline edit.
///
/// `crop_paths` is keyed by segment id and its key times are relative to the segment start, so an
/// edit that re-cuts segments (split, remove-pauses, remove-words: new ids) or moves a segment's
/// start (trim, set-range, refine, history restore) would otherwise leave the path on a dead id or
/// out of step with the picture, and the crop silently falls back to the static `focus_x` window.
///
/// For every center-crop segment of `new`:
/// - same id as a segment of `old` with a path: keys are shifted by the start change (all keys are
///   kept, so undoing the edit shifts them back without loss);
/// - new id with no path yet: the path of the `old` segment with the largest source overlap (same
///   source) is copied, shifted and cut to the piece (boundary keys are interpolated);
/// - an id that already has a path but is not in `old` (history restore) is left as it is.
///
/// Paths of ids that no longer exist are kept so `history undo` gets them back. Returns warnings
/// (e.g. a segment now reaches outside the tracked range).
pub fn carry_crop_paths(store: &ProjectStore, old: &Clip, new: &Clip) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let map = load(store, &new.id)?;
    let Some(paths) = from_map::<BTreeMap<String, Vec<CropKey>>>(&map, "crop_paths")? else {
        return Ok(warnings);
    };
    if paths.is_empty() {
        return Ok(warnings);
    }
    let mut out = paths.clone();
    let mut changed = false;
    for seg in &new.segments {
        if seg.layout.preset.as_deref() != Some("center-crop") {
            continue;
        }
        let id = seg.id.as_str();
        let same = old.segments.iter().find(|o| o.id == seg.id);
        let (parent, keep_all) = match same {
            Some(o) if paths.contains_key(id) => (o, true),
            Some(_) => continue,
            None if paths.contains_key(id) => continue,
            None => {
                let overlap = |o: &Segment| (o.end.min(seg.end) - o.start.max(seg.start)).max(0.0);
                match old
                    .segments
                    .iter()
                    .filter(|o| o.source_id == seg.source_id && paths.contains_key(o.id.as_str()) && overlap(o) > 0.0)
                    .max_by(|a, b| overlap(a).total_cmp(&overlap(b)))
                {
                    Some(o) => (o, false),
                    None => continue,
                }
            }
        };
        let shift = parent.start - seg.start;
        if keep_all && shift.abs() < 1e-9 {
            continue;
        }
        let keys = &paths[parent.id.as_str()];
        let shifted: Vec<CropKey> = keys.iter().map(|k| CropKey { t: k.t + shift, x: k.x }).collect();
        let new_keys = if keep_all { shifted } else { cut_keys(&shifted, seg.end - seg.start) };
        if new_keys.is_empty() {
            continue;
        }
        if seg.start < parent.start - 0.05 || seg.end > parent.end + 0.05 {
            warnings.push(format!(
                "segment {} now reaches outside the face-tracked range of {}; the crop holds the edge position there (re-run `rustler crop track-face {} --segment-id {}` to track it)",
                seg.id, parent.id, new.id, seg.id
            ));
        }
        out.insert(id.to_string(), new_keys);
        changed = true;
    }
    if changed {
        let value = serde_json::to_value(&out).map_err(|e| RustlerError::new(ErrorCode::Internal, "extras", e.to_string()))?;
        update(store, &new.id, "crop_paths", |_| Ok(Some(value)))?;
    }
    Ok(warnings)
}

/// Keys inside `0..=len`, with interpolated keys added at both ends.
fn cut_keys(keys: &[CropKey], len: f64) -> Vec<CropKey> {
    let mut keys: Vec<CropKey> = keys.iter().copied().filter(|k| k.t.is_finite() && k.x.is_finite()).collect();
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    if keys.is_empty() {
        return keys;
    }
    let at = |t: f64| -> f64 {
        if t <= keys[0].t {
            return keys[0].x;
        }
        if t >= keys[keys.len() - 1].t {
            return keys[keys.len() - 1].x;
        }
        let k = keys.partition_point(|k| k.t <= t);
        let (a, b) = (keys[k - 1], keys[k]);
        let span = b.t - a.t;
        if span <= 0.0 { b.x } else { a.x + (b.x - a.x) * (t - a.t) / span }
    };
    let mut out = vec![CropKey { t: 0.0, x: at(0.0) }];
    out.extend(keys.iter().copied().filter(|k| k.t > 1e-6 && k.t < len - 1e-6));
    if len > 1e-6 {
        out.push(CropKey { t: len, x: at(len) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, ProjectStore, ClipId) {
        let tmp = tempfile::tempdir().unwrap();
        let store = ProjectStore { root: tmp.path().to_path_buf() };
        (tmp, store, ClipId::new())
    }

    #[test]
    fn update_keeps_other_keys() {
        let (_t, s, c) = store();
        assert!(load(&s, &c).unwrap().is_empty());
        std::fs::create_dir_all(s.resolve(&rel(&c)).parent().unwrap()).unwrap();
        std::fs::write(
            s.resolve(&rel(&c)),
            r#"{"music":{"path":"/m.mp3","volume_db":-18,"duck":true,"fade_in":0.5,"fade_out":1},"crop_paths":{"seg_1":[{"t":0,"x":0.2}]},"future_key":[1,2]}"#,
        )
        .unwrap();
        update(&s, &c, "overlays", |cur| {
            assert!(cur.is_none());
            Ok(Some(json!([{"id": "ovl_1"}])))
        })
        .unwrap();
        let m = load(&s, &c).unwrap();
        assert_eq!(m["overlays"], json!([{"id": "ovl_1"}]));
        assert_eq!(m["music"]["path"], "/m.mp3");
        assert_eq!(m["crop_paths"]["seg_1"][0]["x"], 0.2);
        assert_eq!(m["future_key"], json!([1, 2]));

        // Removing a key keeps the rest; a failing closure writes nothing.
        update(&s, &c, "future_key", |_| Ok(None)).unwrap();
        let e = update(&s, &c, "music", |_| {
            Err(RustlerError::new(ErrorCode::Validation, "t", "no"))
        });
        assert!(e.is_err());
        let m = load(&s, &c).unwrap();
        assert!(!m.contains_key("future_key"));
        assert!(m.contains_key("music"));
        let ov: Option<Vec<Value>> = get(&s, &c, "overlays").unwrap();
        assert_eq!(ov.unwrap().len(), 1);
        let none: Option<Vec<Value>> = get(&s, &c, "sfx").unwrap();
        assert!(none.is_none());
    }
}
