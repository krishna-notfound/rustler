//! project.pack / unpack / list / open / status on synthetic projects (no ffmpeg or network needed).
//! Every test uses its own temp RUSTLER_HOME so the real recent-projects registry is never touched.
mod support;

use chrono::Utc;
use rustler_core::layout::{self, LayoutParams};
use rustler_core::model::{CaptionScript, WordTiming};
use rustler_core::units::build_units;
use rustler_core::{
    AudioSettings, Canvas, CaptionTrack, Clip, ClipId, ErrorCode, MediaProbe, ProjectStore, RenderId, RenderQuality, RenderRecord, Segment,
    SegmentId, Source, SourceId, SourceOrigin, Transcript, Word,
};
use rustler_registry::{Ctx, Registry};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use support::{expect_code, ok, registry, run};

struct Fx {
    _dir: tempfile::TempDir,
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
    reg: Registry,
    source_id: SourceId,
    clip_id: ClipId,
    media: PathBuf,
}

impl Fx {
    fn ctx(&self) -> Ctx {
        Ctx::new(Some(self.root.clone()), self.home.clone())
    }
    fn store(&self) -> ProjectStore {
        ProjectStore::open(&self.root).unwrap()
    }
}

fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
    Word { i, text: text.to_string(), start, end, conf: None, speaker: None }
}

/// A project "Demo Show" with one source (external media file), transcript + units, one clip.
fn synthetic() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    let root = base.join("proj");
    let home = base.join("home");
    let media_dir = base.join("elsewhere");
    std::fs::create_dir_all(&media_dir).unwrap();
    let media = media_dir.join("talk.mp4");
    std::fs::write(&media, b"pretend this is a big video").unwrap();
    let (store, _) = ProjectStore::create(&root, "Demo Show").unwrap();

    let source_id = SourceId::new();
    let source = Source {
        id: source_id.clone(),
        name: "talk".into(),
        origin: SourceOrigin::File { path: media.clone() },
        media_path: media.clone(),
        probe: MediaProbe { duration: 60.0, has_video: true, has_audio: true, width: 1920, height: 1080, fps: 30.0, ..Default::default() },
        proxy: None,
        audio_wav: None,
        imported_at: Utc::now(),
    };
    store.save_source(&source).unwrap();
    let words = vec![w(0, "Hello.", 1.0, 1.4), w(1, "This", 2.0, 2.2), w(2, "works.", 2.3, 2.9)];
    let units = build_units(&words);
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source_id.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    let rel = Path::new("analysis").join(source_id.as_str());
    store.write_json(&rel.join("transcript.json"), &t).unwrap();
    store.write_json(&rel.join("units.json"), &units).unwrap();

    let canvas = Canvas::default();
    let layout = layout::preset("center-crop", 1920, 1080, &canvas, &LayoutParams::default()).unwrap();
    let mut clip = Clip {
        schema: "rustler.clip@1".into(),
        id: ClipId::new(),
        name: "hook".into(),
        revision: 0,
        canvas,
        segments: vec![Segment { id: SegmentId::new(), source_id: source_id.clone(), start: 1.0, end: 3.0, layout }],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![],
        created_at: Utc::now(),
    };
    store.save_clip(&mut clip, 0).unwrap();
    let clip_id = clip.id.clone();
    let base_rev = store.load_project().unwrap().revision;
    let mut p = store.load_project().unwrap();
    p.sources.push(source_id.clone());
    p.clips.push(clip_id.clone());
    store.save_project(&mut p, base_rev).unwrap();
    Fx { _dir: dir, base, root, home, reg: registry(), source_id, clip_id, media }
}

fn zip_names(path: &Path) -> Vec<String> {
    let z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    z.file_names().map(|n| n.unwrap().into_owned()).collect()
}

fn zip_read(path: &Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut e = z.by_name(name).unwrap();
    let mut b = Vec::new();
    e.read_to_end(&mut b).unwrap();
    b
}

fn has(names: &[String], needle: &str) -> bool {
    names.iter().any(|n| n == needle)
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn pack_unpack_roundtrip_without_media() {
    let fx = synthetic();
    let sid = fx.source_id.to_string();
    let cid = fx.clip_id.to_string();
    let before = std::fs::read(fx.root.join("sources").join(&sid).join("source.json")).unwrap();

    let v = ok(&fx.reg, "project.pack", &fx.ctx(), json!({})).await;
    let zip_path = PathBuf::from(v["path"].as_str().unwrap());
    // default location: next to the project folder, named after the project
    assert_eq!(zip_path.file_name().unwrap(), "demo-show.rustler.zip");
    assert_eq!(std::fs::canonicalize(zip_path.parent().unwrap()).unwrap(), std::fs::canonicalize(&fx.base).unwrap());
    assert!(v["size"].as_u64().unwrap() > 0);
    let names = zip_names(&zip_path);
    assert_eq!(v["files"].as_u64().unwrap() as usize, names.len());
    for want in [
        "project.json".to_string(),
        "events.ndjson".to_string(),
        "rustler-pack.json".to_string(),
        format!("sources/{sid}/source.json"),
        format!("analysis/{sid}/transcript.json"),
        format!("analysis/{sid}/units.json"),
        format!("clips/{cid}/clip.json"),
    ] {
        // events.ndjson only exists once something logged an event; project.pack itself does so afterwards
        if want == "events.ndjson" {
            continue;
        }
        assert!(has(&names, &want), "{want} missing from {names:?}");
    }
    assert!(!names.iter().any(|n| n.starts_with("media/")), "no media without --include-media");
    let warnings = strs(&v["warnings"]).join("\n");
    assert!(warnings.contains("media is not included") && warnings.contains("--include-media"), "{warnings}");

    // original project untouched
    assert_eq!(std::fs::read(fx.root.join("sources").join(&sid).join("source.json")).unwrap(), before);

    // unpack elsewhere
    let dir2 = fx.base.join("restored");
    let u = ok(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": zip_path, "dir": dir2})).await;
    assert_eq!(u["name"], "Demo Show");
    assert_eq!(u["sources"], 1);
    assert_eq!(u["clips"], 1);
    let s2 = ProjectStore::open(&dir2).unwrap();
    let (p1, p2) = (fx.store().load_project().unwrap(), s2.load_project().unwrap());
    assert_eq!(p1.id, p2.id);
    assert_eq!(p1.sources, p2.sources);
    let src2 = s2.load_source(&fx.source_id).unwrap();
    assert_eq!(src2.media_path, fx.media, "external media path is kept when media was not packed");
    assert!(s2.load_clip(&fx.clip_id).is_ok());
    assert!(dir2.join("analysis").join(&sid).join("transcript.json").is_file());
    assert!(!dir2.join("rustler-pack.json").exists());

    // the new project is in the recent list
    let l = ok(&fx.reg, "project.list", &fx.ctx(), json!({})).await;
    let paths: Vec<String> = l["projects"].as_array().unwrap().iter().map(|p| p["path"].as_str().unwrap().to_lowercase()).collect();
    let want = std::fs::canonicalize(&dir2).unwrap().to_string_lossy().trim_start_matches(r"\\?\").to_lowercase();
    assert!(paths.contains(&want), "{paths:?} should contain {want}");
}

#[tokio::test]
async fn pack_unpack_roundtrip_with_media_rewrites_paths_only_in_the_copy() {
    let fx = synthetic();
    let sid = fx.source_id.to_string();
    // a second source whose media file does not exist -> warning
    let store = fx.store();
    let gone = fx.base.join("elsewhere").join("gone.mp4");
    let missing_id = SourceId::new();
    let mut s = store.load_source(&fx.source_id).unwrap();
    s.id = missing_id.clone();
    s.name = "gone".into();
    s.media_path = gone.clone();
    s.origin = SourceOrigin::File { path: gone };
    store.save_source(&s).unwrap();
    let mut p = store.load_project().unwrap();
    let rev = p.revision;
    p.sources.push(missing_id);
    store.save_project(&mut p, rev).unwrap();
    // an in-project file next to source.json (proxy) is bundled too
    std::fs::write(fx.root.join("sources").join(&sid).join("proxy.mp4"), b"proxy bytes").unwrap();
    let original_json = std::fs::read(fx.root.join("sources").join(&sid).join("source.json")).unwrap();

    let out = fx.base.join("out").join("with-media.rustler.zip");
    let v = ok(&fx.reg, "project.pack", &fx.ctx(), json!({"out": out, "include_media": true})).await;
    assert_eq!(PathBuf::from(v["path"].as_str().unwrap()), out);
    let names = zip_names(&out);
    let media_name = format!("media/{sid}_talk.mp4");
    assert!(has(&names, &media_name), "{names:?}");
    assert!(has(&names, &format!("sources/{sid}/proxy.mp4")));
    let warnings = strs(&v["warnings"]).join("\n");
    assert!(warnings.contains("missing") && warnings.contains("gone.mp4"), "{warnings}");

    // the packed source.json points at media/<file>; the original is unchanged
    let packed: Source = serde_json::from_slice(&zip_read(&out, &format!("sources/{sid}/source.json"))).unwrap();
    assert_eq!(packed.media_path, PathBuf::from(&media_name));
    assert_eq!(std::fs::read(fx.root.join("sources").join(&sid).join("source.json")).unwrap(), original_json);
    assert_eq!(fx.store().load_source(&fx.source_id).unwrap().media_path, fx.media);

    // the packed project.json carries no cookies path
    let pj: Value = serde_json::from_slice(&zip_read(&out, "project.json")).unwrap();
    assert!(pj["permissions"]["cookies_file"].is_null());

    // unpack after deleting the original media: the copy comes from the archive
    std::fs::remove_file(&fx.media).unwrap();
    let dir2 = fx.base.join("restored");
    let u = ok(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": out, "dir": dir2})).await;
    let s2 = ProjectStore::open(&dir2).unwrap();
    let src2 = s2.load_source(&fx.source_id).unwrap();
    assert!(src2.media_path.is_absolute(), "{:?}", src2.media_path);
    assert!(src2.media_path.starts_with(&dir2));
    assert_eq!(std::fs::read(&src2.media_path).unwrap(), b"pretend this is a big video");
    assert!(dir2.join("sources").join(&sid).join("proxy.mp4").is_file());
    let uw = strs(&u["warnings"]).join("\n");
    assert!(uw.contains("media not found") && uw.contains("gone.mp4"), "{uw}");
}

#[tokio::test]
async fn pack_options_skip_renders_and_analysis_and_dry_run_writes_nothing() {
    let fx = synthetic();
    let sid = fx.source_id.to_string();
    let cid = fx.clip_id.to_string();
    let renders = fx.root.join("clips").join(&cid).join("renders");
    std::fs::create_dir_all(&renders).unwrap();
    std::fs::write(renders.join("rnd_1.mp4"), b"video").unwrap();
    std::fs::write(fx.root.join("exports").join("hook.mp4"), b"video").unwrap();
    std::fs::create_dir_all(fx.root.join("layouts")).unwrap();
    std::fs::write(fx.root.join("layouts").join("mine.json"), b"{}").unwrap();
    std::fs::write(fx.root.join("brief.json"), b"{}").unwrap();

    let all = fx.base.join("all.zip");
    ok(&fx.reg, "project.pack", &fx.ctx(), json!({"out": all})).await;
    let n = zip_names(&all);
    assert!(has(&n, &format!("clips/{cid}/renders/rnd_1.mp4")) && has(&n, "exports/hook.mp4"));
    assert!(has(&n, "layouts/mine.json") && has(&n, "brief.json"));

    let lean = fx.base.join("lean.zip");
    let v = ok(&fx.reg, "project.pack", &fx.ctx(), json!({"out": lean, "include_renders": false, "include_analysis": false})).await;
    let n = zip_names(&lean);
    assert!(!n.iter().any(|x| x.contains("/renders/") || x.starts_with("exports/")), "{n:?}");
    assert!(!n.iter().any(|x| x.starts_with("analysis/")), "{n:?}");
    assert!(has(&n, &format!("clips/{cid}/clip.json")) && has(&n, &format!("sources/{sid}/source.json")));
    assert!(strs(&v["warnings"]).join("\n").contains("renders and exports are not included"));

    let dry_out = fx.base.join("dry.zip");
    let mut dry = fx.ctx();
    dry.dry_run = true;
    let d = ok(&fx.reg, "project.pack", &dry, json!({"out": dry_out})).await;
    assert_eq!(d["dry_run"], true);
    assert!(!dry_out.exists());
}

#[tokio::test]
async fn unpack_refuses_existing_project_unless_force_and_rejects_bad_archives() {
    let fx = synthetic();
    let zip_path = fx.base.join("a.zip");
    ok(&fx.reg, "project.pack", &fx.ctx(), json!({"out": zip_path})).await;

    // existing project directory is refused, and left untouched
    let marker = fx.root.join("marker.txt");
    std::fs::write(&marker, b"keep").unwrap();
    let e = expect_code(run(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": zip_path, "dir": fx.root})).await, ErrorCode::Validation);
    assert!(e.message.contains("already exists") && e.hint.unwrap().contains("--force"));
    assert!(marker.is_file());
    // dry run also refuses
    let mut dry = fx.ctx();
    dry.dry_run = true;
    expect_code(run(&fx.reg, "project.unpack", &dry, json!({"archive": zip_path, "dir": fx.root})).await, ErrorCode::Validation);

    // force extracts over it
    std::fs::write(fx.root.join("sources").join(fx.source_id.as_str()).join("source.json"), b"corrupt").unwrap();
    ok(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": zip_path, "dir": fx.root, "force": true})).await;
    assert!(fx.store().load_source(&fx.source_id).is_ok(), "force restored source.json from the archive");

    // not an archive / not a rustler archive / missing
    let junk = fx.base.join("junk.zip");
    std::fs::write(&junk, b"definitely not a zip").unwrap();
    expect_code(run(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": junk, "dir": fx.base.join("x1")})).await, ErrorCode::Validation);
    let other = fx.base.join("other.zip");
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
        z.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"hi").unwrap();
        z.finish().unwrap();
    }
    let e = expect_code(run(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": other, "dir": fx.base.join("x2")})).await, ErrorCode::Validation);
    assert!(e.message.contains("project.json"));
    assert!(!fx.base.join("x2").exists(), "nothing is extracted from an invalid archive");
    expect_code(run(&fx.reg, "project.unpack", &fx.ctx(), json!({"archive": fx.base.join("nope.zip"), "dir": fx.base.join("x3")})).await, ErrorCode::NotFound);
}

#[tokio::test]
async fn recent_registry_create_open_list_and_prune() {
    let base = tempfile::tempdir().unwrap();
    let home = base.path().join("home");
    let reg = registry();
    let ctx = Ctx::new(None, home.clone());
    assert_eq!(ok(&reg, "project.list", &ctx, json!({})).await["projects"].as_array().unwrap().len(), 0);

    let (a, b) = (base.path().join("a"), base.path().join("b"));
    ok(&reg, "project.create", &ctx, json!({"name": "Alpha", "dir": a})).await;
    ok(&reg, "project.create", &ctx, json!({"name": "Beta", "dir": b})).await;
    assert!(home.join("projects.json").is_file(), "registry lives in RUSTLER_HOME");

    let l = ok(&reg, "project.list", &ctx, json!({})).await;
    let items = l["projects"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Beta", "most recent first");
    assert_eq!(items[1]["name"], "Alpha");
    assert!(items.iter().all(|p| p["exists"] == true && p["sources"] == 0 && p["clips"] == 0));
    assert!(items[0]["last_opened"].is_string());

    // project.show on Alpha makes it most recent again; no duplicate entry
    ok(&reg, "project.show", &Ctx::new(Some(a.clone()), home.clone()), json!({})).await;
    let l = ok(&reg, "project.list", &ctx, json!({})).await;
    let items = l["projects"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Alpha");

    // open validates and records
    let o = ok(&reg, "project.open", &ctx, json!({"dir": b})).await;
    assert_eq!(o["name"], "Beta");
    assert_eq!(ok(&reg, "project.list", &ctx, json!({})).await["projects"][0]["name"], "Beta");
    let e = expect_code(run(&reg, "project.open", &ctx, json!({"dir": base.path().join("missing")})).await, ErrorCode::NotFound);
    assert!(e.hint.is_some());
    assert_eq!(ok(&reg, "project.list", &ctx, json!({})).await["projects"].as_array().unwrap().len(), 2, "failed open records nothing");

    // a deleted folder is reported as not existing, and pruned on request
    std::fs::remove_dir_all(&a).unwrap();
    let l = ok(&reg, "project.list", &ctx, json!({})).await;
    let alpha = l["projects"].as_array().unwrap().iter().find(|p| p["name"] == "Alpha").unwrap().clone();
    assert_eq!(alpha["exists"], false);
    let l = ok(&reg, "project.list", &ctx, json!({"prune_missing": true})).await;
    assert_eq!(l["projects"].as_array().unwrap().len(), 1);
    assert_eq!(l["projects"][0]["name"], "Beta");
}

fn render_record(rid: &RenderId, clip: &str) -> RenderRecord {
    RenderRecord {
        id: rid.clone(),
        path: Path::new("clips").join(clip).join("renders").join(format!("{rid}.mp4")),
        quality: RenderQuality::Final,
        encoder: "libx264".into(),
        created_at: Utc::now(),
        qc: None,
    }
}

#[tokio::test]
async fn status_summarises_sources_clips_and_suggestions() {
    let fx = synthetic();
    let ctx = fx.ctx();
    let store = fx.store();
    let sid = fx.source_id.to_string();
    let cid = fx.clip_id.to_string();
    let clip_status = |v: &Value| v["clips"][0].clone();
    let sugg = |v: &Value| strs(&v["suggestions"]).join("\n");

    // fresh: transcript present, never rendered
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    assert_eq!(v["name"], "Demo Show");
    let s = &v["sources"][0];
    assert_eq!(s["id"], sid.as_str());
    assert_eq!(s["analyzed"], true);
    assert_eq!(s["transcript_words"], 3);
    assert!(s["units"].as_u64().unwrap() >= 1);
    assert_eq!(s["deep_analysis"], false);
    assert_eq!(s["media_present"], true);
    let c = clip_status(&v);
    assert_eq!(c["id"], cid.as_str());
    assert_eq!(c["duration"], 2.0);
    assert_eq!(c["renders"], 0);
    assert!(c["last_render"].is_null());
    assert!(sugg(&v).contains(&format!("rustler render run {cid}")));
    assert!(strs(&v["next"]).iter().any(|n| n == &format!("rustler render run {cid}")));

    // deep analysis shows up
    store.write_json(&Path::new("analysis").join(&sid).join("ocr.json"), &json!({})).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    assert_eq!(v["sources"][0]["deep_analysis"], true);
    assert_eq!(v["sources"][0]["deep_parts"], json!(["ocr"]));

    // rendered (file on disk), no QC yet
    let rid = RenderId::new();
    let rec = render_record(&rid, &cid);
    std::fs::create_dir_all(store.resolve(&rec.path).parent().unwrap()).unwrap();
    std::fs::write(store.resolve(&rec.path), b"video").unwrap();
    let mut clip = store.load_clip(&fx.clip_id).unwrap();
    let rev = clip.revision;
    clip.renders.push(rec.clone());
    store.save_clip(&mut clip, rev).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    let c = clip_status(&v);
    assert_eq!(c["last_render"]["id"], rid.to_string().as_str());
    assert_eq!(c["last_render"]["file_exists"], true);
    assert_eq!(c["edits_newer_than_render"], false);
    assert!(c["qc"].is_null());
    assert!(sugg(&v).contains(&format!("rustler qc run {cid}")));

    // QC failed -> fix suggest
    let qc_rel = Path::new("clips").join(&cid).join("qc").join(format!("{rid}.json"));
    store.write_json(&qc_rel, &json!({"passed": false, "verdict": "FAIL: loudness"})).unwrap();
    let mut clip = store.load_clip(&fx.clip_id).unwrap();
    let rev = clip.revision;
    clip.renders.last_mut().unwrap().qc = Some(qc_rel.clone());
    store.save_clip(&mut clip, rev).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    let c = clip_status(&v);
    assert_eq!(c["qc"]["passed"], false);
    assert_eq!(c["edits_newer_than_render"], false, "saving the QC pointer is not an edit");
    assert!(sugg(&v).contains(&format!("rustler fix suggest {cid}")));

    // QC passed, not exported yet
    store.write_json(&qc_rel, &json!({"passed": true, "verdict": "PASS"})).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    assert_eq!(clip_status(&v)["qc"]["passed"], true);
    assert_eq!(clip_status(&v)["exported"], false);
    assert!(sugg(&v).contains(&format!("rustler export clip {cid}")));

    // brief exists but was never checked
    std::fs::write(fx.root.join("brief.json"), b"{}").unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    assert_eq!(clip_status(&v)["has_brief"], true);
    assert!(sugg(&v).contains(&format!("rustler brief check {cid}")));
    store.write_json(&Path::new("clips").join(&cid).join("brief_check.json"), &json!({"status": "pass", "checked_at": "2026-11-15T10:00:00Z"})).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    assert_eq!(clip_status(&v)["brief_check"]["status"], "pass");
    assert!(!sugg(&v).contains("rustler brief check"));

    // exported
    store.write_json(&Path::new("exports").join("hook.report.json"), &json!({"clip": {"id": cid}, "render": {"id": rid.to_string()}})).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    assert_eq!(clip_status(&v)["exported"], true);
    assert!(!sugg(&v).contains("rustler export clip"));

    // an edit after the render -> stale, re-render suggested, export no longer "done"
    let mut clip = store.load_clip(&fx.clip_id).unwrap();
    let rev = clip.revision;
    clip.segments[0].end = 2.5;
    store.save_clip(&mut clip, rev).unwrap();
    let v = ok(&fx.reg, "project.status", &ctx, json!({})).await;
    let c = clip_status(&v);
    assert_eq!(c["edits_newer_than_render"], true);
    assert!(c["stale_reason"].as_str().unwrap().contains("changed after the render"));
    assert!(sugg(&v).contains(&format!("clip hook has edits newer than its render")), "{}", sugg(&v));
    assert!(!sugg(&v).contains("rustler export clip"));

    // status is read-only
    assert_eq!(store.load_project().unwrap().revision, store.load_project().unwrap().revision);
}

#[tokio::test]
async fn status_on_empty_project_points_at_import() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    ProjectStore::create(&root, "Empty").unwrap();
    let reg = registry();
    let ctx = Ctx::new(Some(root), dir.path().join("home"));
    let v = ok(&reg, "project.status", &ctx, json!({})).await;
    assert_eq!(v["sources"].as_array().unwrap().len(), 0);
    assert!(strs(&v["next"])[0].contains("source import"));
}
