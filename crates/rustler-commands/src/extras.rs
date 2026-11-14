//! Shared read-modify-write access to the `clips/<id>/extras.json` sidecar: clip data the frozen
//! `Clip` model has no field for (`music`, `crop_paths`, `overlays`, `audio_fx`, `sfx`,
//! `transitions`, `effects`, `progress_bar`, ...).
//!
//! Every writer goes through [`update`], which changes exactly one top-level key and keeps all
//! others byte-for-byte (as JSON values), so command modules never clobber each other's data.
//! Writes are atomic (`ProjectStore::write_json`: temp file + rename) and serialized within the
//! process.

use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
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
