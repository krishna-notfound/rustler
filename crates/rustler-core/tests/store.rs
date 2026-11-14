//! Task 3 store tests.

use chrono::Utc;
use rustler_core::*;
use std::path::Path;

fn sample_clip() -> Clip {
    Clip {
        schema: "rustler.clip@1".into(),
        id: ClipId::new(),
        name: "c".into(),
        revision: 0,
        canvas: Canvas::default(),
        segments: vec![],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![],
        created_at: Utc::now(),
    }
}

#[test]
fn create_makes_layout() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path().join("proj");
    let (store, p) = ProjectStore::create(&root, "demo").unwrap();
    for d in ["sources", "analysis", "clips", "exports", "history"] {
        assert!(root.join(d).is_dir(), "{d} missing");
    }
    assert!(root.join("project.json").is_file());
    assert_eq!(p.revision, 0);
    assert_eq!(p.name, "demo");
    assert_eq!(p.schema, "rustler.project@1");
    let loaded = store.load_project().unwrap();
    assert_eq!(loaded, p);
    let reopened = ProjectStore::open(&root).unwrap();
    assert_eq!(reopened.load_project().unwrap(), p);
}

#[test]
fn open_missing_is_not_found() {
    let td = tempfile::tempdir().unwrap();
    assert_eq!(ProjectStore::open(td.path()).unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn create_twice_fails_validation() {
    let td = tempfile::tempdir().unwrap();
    ProjectStore::create(td.path(), "a").unwrap();
    let err = ProjectStore::create(td.path(), "b").unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

#[test]
fn save_with_stale_revision_conflicts() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    let mut p1 = store.load_project().unwrap();
    let mut p2 = store.load_project().unwrap();
    store.save_project(&mut p1, 0).unwrap();
    assert_eq!(p1.revision, 1);
    assert_eq!(store.load_project().unwrap().revision, 1);
    let err = store.save_project(&mut p2, 0).unwrap_err();
    assert_eq!(err.code, ErrorCode::RevisionConflict);
    assert_eq!(store.load_project().unwrap().revision, 1);
}

#[test]
fn atomic_write_survives_partial() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    // Simulate a crash mid-write: a garbage temp file next to project.json.
    std::fs::write(td.path().join("project.json.tmp123"), b"{ \"schema\": \"rus").unwrap();
    let mut p = store.load_project().unwrap();
    assert_eq!(p.revision, 0);
    store.save_project(&mut p, 0).unwrap();
    assert_eq!(store.load_project().unwrap().revision, 1);
}

#[test]
fn events_append_lines() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    for k in 0..2 {
        store
            .append_event(&Event {
                ts: Utc::now(),
                actor: Actor::Agent,
                kind: format!("test.{k}"),
                project_revision: k,
                data: serde_json::json!({"k": k}),
            })
            .unwrap();
    }
    let text = std::fs::read_to_string(td.path().join("events.ndjson")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    for (k, l) in lines.iter().enumerate() {
        let e: Event = serde_json::from_str(l).unwrap();
        assert_eq!(e.kind, format!("test.{k}"));
    }
}

#[test]
fn relativize_inside_root() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    let inside = td.path().join("sources").join("x.mp4");
    assert_eq!(store.relativize(&inside), Path::new("sources").join("x.mp4"));
    let outside = std::env::temp_dir().join("elsewhere_zzz").join("y.mp4");
    assert_eq!(store.relativize(&outside), outside);
    // resolve is the inverse for relative paths and a no-op for absolute ones.
    assert_eq!(store.resolve(Path::new("sources/x.mp4")), td.path().join("sources/x.mp4"));
    assert_eq!(store.resolve(&outside), outside);
}

#[test]
fn clip_save_load_and_conflict() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    let mut c = sample_clip();
    store.save_clip(&mut c, 0).unwrap();
    assert_eq!(c.revision, 1);
    assert!(store.clip_dir(&c.id).join("clip.json").is_file());
    assert_eq!(store.load_clip(&c.id).unwrap(), c);
    let mut stale = c.clone();
    stale.revision = 0;
    assert_eq!(store.save_clip(&mut stale, 0).unwrap_err().code, ErrorCode::RevisionConflict);
    assert_eq!(store.load_clip(&ClipId::new()).unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn save_clip_keeps_snapshot() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    let mut c = sample_clip();
    store.save_clip(&mut c, 0).unwrap(); // on disk: rev 1, name "c"
    c.name = "second".into();
    store.save_clip(&mut c, 1).unwrap(); // on disk: rev 2; snapshot of rev 1
    c.name = "third".into();
    store.save_clip(&mut c, 2).unwrap(); // on disk: rev 3; snapshot of rev 2

    assert!(store.clip_dir(&c.id).join("history").join("1.clip.json").is_file());
    assert!(store.clip_dir(&c.id).join("history").join("2.clip.json").is_file());
    let s1 = store.load_clip_snapshot(&c.id, 1).unwrap();
    assert_eq!((s1.revision, s1.name.as_str()), (1, "c"));
    let s2 = store.load_clip_snapshot(&c.id, 2).unwrap();
    assert_eq!((s2.revision, s2.name.as_str()), (2, "second"));
    assert_eq!(store.load_clip(&c.id).unwrap().name, "third");
    assert_eq!(store.load_clip_snapshot(&c.id, 99).unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn source_dirs_and_json_helpers() {
    let td = tempfile::tempdir().unwrap();
    let (store, _) = ProjectStore::create(td.path(), "a").unwrap();
    let sid = SourceId::new();
    assert_eq!(store.source_dir(&sid), td.path().join("sources").join(sid.as_str()));
    assert_eq!(store.analysis_dir(&sid), td.path().join("analysis").join(sid.as_str()));
    assert_eq!(store.exports_dir(), td.path().join("exports"));
    // write_json creates missing parent dirs and read_json round-trips.
    let rel = Path::new("analysis").join("x").join("v.json");
    store.write_json(&rel, &serde_json::json!({"a": 1})).unwrap();
    let v: serde_json::Value = store.read_json(&rel).unwrap();
    assert_eq!(v["a"], 1);
    let missing: Result<serde_json::Value> = store.read_json(Path::new("nope.json"));
    assert_eq!(missing.unwrap_err().code, ErrorCode::NotFound);
}
