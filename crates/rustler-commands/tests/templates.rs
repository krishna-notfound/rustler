//! Templates (plan Task 1.5): capture a clip's look, apply it through timeline.apply, missing
//! assets, export/import bundles, built-in starters, clip.create --template.
mod common;

use common::*;
use rustler_core::{ErrorCode, ProjectStore};
use rustler_registry::{Ctx, Registry};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fx {
    _tmp: tempfile::TempDir,
    store: ProjectStore,
    ctx: Ctx,
    reg: Registry,
    src: rustler_core::Source,
    files: PathBuf,
}

fn fx() -> Fx {
    let (tmp, store, ctx) = new_project();
    let media = tmp.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(12.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let files = tmp.path().join("files");
    std::fs::create_dir_all(&files).unwrap();
    for f in ["logo.png", "boom.wav", "music.mp3", "Anton-Regular.ttf"] {
        std::fs::write(files.join(f), format!("bytes of {f}")).unwrap();
    }
    Fx { _tmp: tmp, store, ctx, reg: registry(), src, files }
}

impl Fx {
    fn file(&self, n: &str) -> String {
        self.files.join(n).to_string_lossy().into_owned()
    }
    async fn call(&self, name: &str, v: Value) -> Value {
        call(&self.reg, &self.ctx, name, v).await.unwrap_or_else(|e| panic!("{name}: {} ({:?})", e.message, e.hint))
    }
    /// Clip [1.0, 2.4] + [3.0, 5.0] + [5.05, 7.0]: edit duration 5.35 s, cuts at 1.4 and 3.4.
    async fn clip(&self, name: &str) -> String {
        let c = self.call("clip.create", json!({"name": name, "source_id": self.src.id, "start": 1.0, "end": 2.4})).await;
        let id = c["id"].as_str().unwrap().to_string();
        self.call("clip.add_segment", json!({"clip_id": id, "source_id": self.src.id, "start": 3.0, "end": 5.0})).await;
        self.call("clip.add_segment", json!({"clip_id": id, "source_id": self.src.id, "start": 5.05, "end": 7.0})).await;
        id
    }
    async fn show(&self, clip: &str) -> Value {
        self.call("timeline.show", json!({"clip_id": clip})).await
    }
    async fn asset(&self, file: &str, kind: &str) -> String {
        let a = self.call("asset.add", json!({"path": self.file(file), "kind": kind, "license": "CC0-1.0"})).await;
        a["asset"]["id"].as_str().unwrap().to_string()
    }
    /// A clip with every look feature set through the v1 commands.
    async fn styled(&self) -> (String, String, String) {
        let id = self.clip("styled").await;
        let music = self.asset("music.mp3", "music").await;
        let boom = self.asset("boom.wav", "sfx").await;
        self.call("clip.set_captions", json!({"clip_id": id, "style": "hormozi-pop", "position_y": 0.65, "max_words_per_line": 2})).await;
        self.call("clip.set_layout", json!({"clip_id": id, "preset": "blur-fit"})).await;
        self.call("overlay.add_text", json!({"clip_id": id, "text": "He quit his job", "start": 0.0, "end": 2.5, "position_y": 0.15, "font_size_factor": 0.1})).await;
        self.call("overlay.add_text", json!({"clip_id": id, "text": "Dr. Jane Doe", "style": "lower-third", "start": 2.0, "end": 4.0, "position_y": 0.82})).await;
        self.call("audio.denoise", json!({"clip_id": id, "strength": "light"})).await;
        self.call("audio.add_music", json!({"clip_id": id, "path": format!("asset:{music}"), "volume_db": -20.0})).await;
        self.call("audio.add_sfx", json!({"clip_id": id, "file": format!("asset:{boom}"), "at": 0.0})).await;
        self.call("effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 1.4, "end": 2.0, "amount": 1.15})).await;
        self.call("overlay.progress_bar", json!({"clip_id": id, "enabled": true, "color": "#FF0000"})).await;
        let segs = self.show(&id).await["document"]["tracks"]["video"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        self.call("transition.set", json!({"clip_id": id, "after_segment_id": segs[0], "kind": "dissolve", "duration": 0.25})).await;
        self.call("image.add", json!({"clip_id": id, "file": self.file("logo.png"), "rect": {"x": 0.7, "y": 0.03, "w": 0.25, "h": 0.08}})).await;
        (id, music, boom)
    }
}

#[tokio::test]
async fn create_from_clip_captures_style_layout_hook_audio() {
    let f = fx();
    let (id, music, boom) = f.styled().await;
    let out = f.call("template.create", json!({"name": "my-look", "from_clip": id, "description": "test look"})).await;
    assert_eq!(out["version"], 1);
    assert_eq!(out["scope"], "project");
    let file = PathBuf::from(out["file"].as_str().unwrap());
    assert!(file.ends_with("templates/my-look@1.json") || file.ends_with("templates\\my-look@1.json"), "{file:?}");
    assert!(file.is_file());
    let t = &out["template"];
    assert_eq!(t["schema"], "rustler.template@1");
    assert_eq!(t["description"], "test look");
    assert_eq!(t["based_on"]["clip"], id.as_str());
    // Captions
    assert_eq!(t["captions"]["preset"], "hormozi-pop");
    assert_eq!(t["captions"]["position_y"], 0.65);
    assert_eq!(t["captions"]["max_words_per_line"], 2);
    // Layout: per-source presets keep only the preset (framing is content)
    assert_eq!(t["layout"]["preset"], "blur-fit");
    assert!(t["layout"].get("regions").is_none());
    // Hook look, no text content
    assert_eq!(t["hook"]["font_size_factor"], 0.1);
    assert_eq!(t["hook"]["position_y"], 0.15);
    assert_eq!(t["hook"]["duration"], 2.5);
    assert!(t["hook"].get("text").is_none());
    assert_eq!(t["text_styles"]["lower-third"]["position_y"], 0.82);
    assert!(t.get("texts").is_none());
    // Audio
    assert_eq!(t["audio"]["denoise"], "light");
    assert_eq!(t["audio"]["music"]["asset"], music.as_str());
    assert_eq!(t["audio"]["music"]["volume_db"], -20.0);
    assert_eq!(t["audio"]["sfx"][0]["asset"], boom.as_str());
    assert_eq!(t["audio"]["sfx"][0]["at"], "start");
    assert_eq!(t["audio"]["loudnorm"], true);
    // Patterns, bar, transitions, images, canvas
    assert_eq!(t["effects"][0]["kind"], "punch-in");
    assert_eq!(t["effects"][0]["at"], "cuts");
    assert_eq!(t["effects"][0]["amount"], 1.15);
    assert_eq!(t["progress_bar"]["enabled"], true);
    assert_eq!(t["progress_bar"]["color"], "#FF0000");
    assert_eq!(t["transitions"]["kind"], "dissolve");
    assert_eq!(t["transitions"]["duration"], 0.25);
    assert!(t["images"][0]["file"].as_str().unwrap().ends_with("logo.png"));
    assert_eq!(t["canvas"]["width"], 1080);
    // refs: music + sfx assets, the logo file
    let refs = out["refs"].as_array().unwrap();
    assert_eq!(refs.len(), 3, "{refs:?}");
    assert!(out["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("include_text")));

    // include_text keeps the words; a taken name is refused; list shows it with the starters.
    let out = f.call("template.create", json!({"name": "my-look-text", "from_clip": id, "include_text": true})).await;
    assert_eq!(out["template"]["hook"]["text"], "He quit his job");
    assert_eq!(out["template"]["texts"][0]["text"], "Dr. Jane Doe");
    let e = call(&f.reg, &f.ctx, "template.create", json!({"name": "my-look", "from_clip": id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let l = f.call("template.list", json!({})).await;
    let names: Vec<(String, String)> =
        l["templates"].as_array().unwrap().iter().map(|r| (r["name"].as_str().unwrap().into(), r["scope"].as_str().unwrap().into())).collect();
    for want in [("my-look", "project"), ("podcast-viral", "built-in"), ("streamer-split", "built-in")] {
        assert!(names.contains(&(want.0.into(), want.1.into())), "{names:?}");
    }

    // update = new version (merge patch); diff shows it; show picks the latest; @1 still works.
    let u = f.call("template.update", json!({"name": "my-look", "patch": {"captions": {"position_y": 0.6}}})).await;
    assert_eq!(u["version"], 2);
    let s = f.call("template.show", json!({"name": "my-look"})).await;
    assert_eq!(s["version"], 2);
    assert_eq!(s["versions"], json!([1, 2]));
    assert!(s.get("missing").is_none());
    let d = f.call("template.diff", json!({"a": "my-look@1", "b": "my-look@2"})).await;
    assert_eq!(d["same"], false);
    assert_eq!(d["changes"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["changes"][0]["path"], "/captions/position_y");
    assert_eq!(d["changes"][0]["a"], 0.65);
    assert_eq!(d["changes"][0]["b"], 0.6);
    let del = f.call("template.delete", json!({"name": "my-look@1"})).await;
    assert_eq!(del["deleted"].as_array().unwrap().len(), 1);
    let e = call(&f.reg, &f.ctx, "template.show", json!({"name": "my-look@1"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = call(&f.reg, &f.ctx, "template.delete", json!({"name": "podcast-viral"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn apply_writes_timeline_items() {
    let f = fx();
    let (styled, _, _) = f.styled().await;
    f.call("template.create", json!({"name": "my-look", "from_clip": styled})).await;
    let b = f.clip("plain").await;
    f.call("overlay.add_text", json!({"clip_id": b, "text": "Nobody saw this coming at all today, really", "start": 0.0, "end": 1.0})).await;
    let before = f.show(&b).await;
    let rev0 = before["revision"].as_u64().unwrap();

    // Dry run: patch shown, nothing written.
    let d = f.call("template.apply", json!({"name": "my-look", "clip_ids": [b], "dry_run": true})).await;
    assert_eq!(d["dry_run"], true);
    assert!(d["clips"][0]["ops"].as_u64().unwrap() > 5);
    assert!(d["clips"][0]["patch"].as_array().unwrap().iter().all(|op| op["op"].is_string() && op["path"].is_string()));
    assert_eq!(f.show(&b).await["revision"].as_u64().unwrap(), rev0);

    let r = f.call("template.apply", json!({"name": "my-look", "clip_ids": [b]})).await;
    assert_eq!(r["version"], 1);
    let c = &r["clips"][0];
    assert_eq!(c["revision"].as_u64().unwrap(), rev0 + 1, "one revision: {c}");
    assert!(c["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("words")), "long hook warned: {c}");
    let doc = &f.show(&b).await["document"];
    let tr = &doc["tracks"];
    assert_eq!(doc["template_ref"]["id"], "my-look");
    assert_eq!(doc["template_ref"]["version"], 1);
    assert_eq!(tr["captions"]["preset"], "hormozi-pop");
    assert_eq!(tr["captions"]["position_y"], 0.65);
    assert_eq!(tr["captions"]["max_words_per_line"], 2);
    for seg in tr["video"].as_array().unwrap() {
        assert_eq!(seg["layout"]["preset"], "blur-fit");
    }
    let hook = &tr["text"][0];
    assert_eq!(hook["font_size_factor"], 0.1);
    assert_eq!(hook["position_y"], 0.15);
    assert_eq!(hook["end"], 2.5);
    assert_eq!(tr["progress_bar"]["color"], "#FF0000");
    assert_eq!(tr["audio"]["denoise"], "light");
    assert!(tr["audio"]["music"]["path"].as_str().unwrap().ends_with(".mp3"));
    assert_eq!(tr["audio"]["sfx"].as_array().unwrap().len(), 1);
    assert_eq!(tr["audio"]["sfx"][0]["at"], 0.0);
    assert_eq!(tr["audio"]["sfx"][0]["by_template"], "my-look@1");
    // punch-in at both cuts (1.4 and 3.4), tagged
    let fx: Vec<&Value> = tr["effects"].as_array().unwrap().iter().collect();
    assert_eq!(fx.len(), 2, "{fx:?}");
    assert!((fx[0]["start"].as_f64().unwrap() - 1.4).abs() < 1e-9 && (fx[1]["start"].as_f64().unwrap() - 3.4).abs() < 1e-9);
    assert!(fx.iter().all(|e| e["by_template"] == "my-look@1"));
    assert_eq!(tr["transitions"].as_array().unwrap().len(), 2);
    assert_eq!(tr["images"].as_array().unwrap().len(), 1);

    // Re-apply: nothing stacks, nothing changes.
    let r = f.call("template.apply", json!({"name": "my-look", "clip_ids": [b]})).await;
    assert_eq!(r["clips"][0]["ops"], 0, "{r}");
    assert_eq!(r["clips"][0]["revision"].as_u64().unwrap(), rev0 + 1);
    assert_eq!(f.show(&b).await["document"]["tracks"]["effects"].as_array().unwrap().len(), 2);

    // Overrides for one run.
    let r = f.call("template.apply", json!({"name": "my-look", "clip_ids": [b], "overrides": {"captions": {"position_y": 0.5}, "effects": []}})).await;
    assert_eq!(r["clips"][0]["revision"].as_u64().unwrap(), rev0 + 2);
    let tr = f.show(&b).await["document"]["tracks"].clone();
    assert_eq!(tr["captions"]["position_y"], 0.5);
    assert!(tr["effects"].as_array().unwrap().is_empty());

    // Undo goes back through history (the template run is one revision).
    f.call("history.undo", json!({"clip_id": b})).await;
    let tr = f.show(&b).await["document"]["tracks"].clone();
    assert_eq!(tr["captions"]["position_y"], 0.65);
    assert_eq!(tr["effects"].as_array().unwrap().len(), 2);

    // all = every clip in the project.
    let r = f.call("template.apply", json!({"name": "my-look", "all": true, "dry_run": true})).await;
    assert_eq!(r["clips"].as_array().unwrap().len(), 2);
    let e = call(&f.reg, &f.ctx, "template.apply", json!({"name": "my-look"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn apply_missing_asset_errors_with_name() {
    let f = fx();
    let b = f.clip("plain").await;
    let rev0 = f.show(&b).await["revision"].as_u64().unwrap();
    let t = json!({"audio": {"sfx": [{"asset": "sfx-missing-boom", "at": "start"}]}, "progress_bar": {"enabled": true}});
    f.call("template.create", json!({"name": "needs-sfx", "template": t})).await;
    let e = call(&f.reg, &f.ctx, "template.apply", json!({"name": "needs-sfx", "clip_ids": [b]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("sfx-missing-boom"), "{}", e.message);
    assert!(e.hint.unwrap().contains("template import"));
    // A missing font asset and a missing plain file name themselves too.
    f.call("template.create", json!({"name": "needs-font", "template": {"fonts": ["font-anton-regular"]}})).await;
    let e = call(&f.reg, &f.ctx, "template.apply", json!({"name": "needs-font", "clip_ids": [b]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("font-anton-regular"), "{}", e.message);
    let gone = f.files.join("gone.png");
    f.call("template.create", json!({"name": "needs-file", "template": {"images": [{"file": gone, "rect": {"x": 0.1, "y": 0.1, "w": 0.2, "h": 0.1}}]}})).await;
    let e = call(&f.reg, &f.ctx, "template.apply", json!({"name": "needs-file", "clip_ids": [b]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("gone.png"), "{}", e.message);
    // A missing custom caption style names it as well.
    f.call("template.create", json!({"name": "needs-style", "template": {"captions": {"preset": "brand-pop"}}})).await;
    let e = call(&f.reg, &f.ctx, "template.apply", json!({"name": "needs-style", "clip_ids": [b]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("brand-pop"), "{}", e.message);
    // show reports what is missing; nothing was written.
    let s = f.call("template.show", json!({"name": "needs-sfx"})).await;
    assert!(s["missing"][0].as_str().unwrap().contains("sfx-missing-boom"));
    assert_eq!(f.show(&b).await["revision"].as_u64().unwrap(), rev0);
    // clip.create with an explicit template that cannot apply refuses before creating anything.
    let n = f.store.load_project().unwrap().clips.len();
    let e = call(&f.reg, &f.ctx, "clip.create", json!({"name": "x", "source_id": f.src.id, "start": 1.0, "end": 2.0, "template": "needs-sfx"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(f.store.load_project().unwrap().clips.len(), n);
}

#[tokio::test]
async fn export_import_roundtrip_with_assets() {
    let f = fx();
    let font = f.asset("Anton-Regular.ttf", "font").await;
    let boom = f.asset("boom.wav", "sfx").await;
    f.call("caption.create_style", json!({"name": "brand-pop", "base": "hormozi-pop", "font_family": "Anton", "active_color": "#00FFAA"})).await;
    let a = f.clip("styled").await;
    f.call("clip.set_captions", json!({"clip_id": a, "style": "brand-pop"})).await;
    f.call("audio.add_sfx", json!({"clip_id": a, "file": format!("asset:{boom}"), "at": 0.0})).await;
    f.call("image.add", json!({"clip_id": a, "file": f.file("logo.png"), "rect": {"x": 0.7, "y": 0.03, "w": 0.25, "h": 0.08}})).await;
    let c = f.call("template.create", json!({"name": "brand", "from_clip": a})).await;
    assert_eq!(c["template"]["fonts"], json!([font]), "custom style font captured: {}", c["template"]);
    let zip = f.files.join("out").join("brand.zip");
    let x = f.call("template.export", json!({"name": "brand", "out": zip})).await;
    assert!(zip.is_file());
    assert!(x["bytes"].as_u64().unwrap() > 0);
    let mut assets: Vec<String> = x["assets"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assets.sort();
    let mut want = vec!["style-brand-pop".to_string(), font.clone(), boom.clone()];
    want.sort();
    assert_eq!(assets, want);
    assert_eq!(x["files"].as_array().unwrap().len(), 1);

    // A second project with its own (empty) home: nothing is there until the import.
    let g = fx();
    let bclip = g.clip("other").await;
    let e = call(&g.reg, &g.ctx, "template.show", json!({"name": "brand"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let i = g.call("template.import", json!({"file": zip})).await;
    assert_eq!(i["name"], "brand");
    assert_eq!(i["version"], 1);
    assert_eq!(i["assets_added"].as_array().unwrap().len(), 3, "{i}");
    let files = i["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    let logo = PathBuf::from(files[0].as_str().unwrap());
    assert!(logo.is_file() && logo.starts_with(&g.store.root), "{logo:?}");
    assert_eq!(std::fs::read_to_string(&logo).unwrap(), "bytes of logo.png");
    let s = g.call("template.show", json!({"name": "brand"})).await;
    assert!(s.get("missing").is_none(), "{s}");
    assert_eq!(s["template"]["images"][0]["file"], files[0]);
    let st = g.call("caption.styles", json!({})).await;
    assert!(st.to_string().contains("brand-pop"));
    let r = g.call("template.apply", json!({"name": "brand", "clip_ids": [bclip]})).await;
    assert_eq!(r["clips"][0]["revision"].as_u64().unwrap(), r["clips"][0]["from_revision"].as_u64().unwrap() + 1);
    let tr = g.show(&bclip).await["document"]["tracks"].clone();
    assert_eq!(tr["captions"]["preset"], "brand-pop");
    assert!(tr["audio"]["sfx"][0]["file"].as_str().unwrap().starts_with(g.store.root.to_str().unwrap()));
    // Importing again keeps everything (same content, same version).
    let i2 = g.call("template.import", json!({"file": zip})).await;
    assert_eq!(i2["assets_present"].as_array().unwrap().len(), 3);
    assert!(i2["assets_added"].as_array().unwrap().is_empty());
    let l = g.call("template.show", json!({"name": "brand"})).await;
    assert_eq!(l["versions"].as_array().unwrap().len(), 1, "{l}");
}

#[tokio::test]
async fn starter_templates_validate() {
    let f = fx();
    let l = f.call("template.list", json!({})).await;
    let starters: Vec<&Value> = l["templates"].as_array().unwrap().iter().filter(|r| r["scope"] == "built-in").collect();
    assert_eq!(starters.len(), 2);
    for s in ["podcast-viral", "streamer-split"] {
        let out = f.call("template.show", json!({"name": s})).await;
        assert_eq!(out["scope"], "built-in");
        assert!(out.get("missing").is_none());
        assert!(out["refs"].as_array().unwrap().is_empty());
        let t = &out["template"];
        assert_eq!(t["captions"]["preset"], "hormozi-pop");
        assert!(t["hook"]["max_words"].as_u64().unwrap() <= 7);
        assert_eq!(t["progress_bar"]["enabled"], true);
        assert_eq!(t["effects"][0]["kind"], "punch-in");
    }
    // A longer clip (8.5 s) gets the every-N-seconds punch-ins.
    let c = f.call("clip.create", json!({"name": "long", "source_id": f.src.id, "start": 1.0, "end": 9.5})).await;
    let id = c["id"].as_str().unwrap().to_string();
    f.call("overlay.add_text", json!({"clip_id": id, "text": "He quit his job"})).await;
    let r = f.call("template.apply", json!({"name": "podcast-viral", "clip_ids": [id]})).await;
    assert!(r["clips"][0]["warnings"].as_array().is_none_or(|w| w.iter().all(|w| !w.as_str().unwrap().contains("no hook"))));
    let tr = f.show(&id).await["document"]["tracks"].clone();
    assert_eq!(tr["video"][0]["layout"]["preset"], "center-crop", "podcast-viral leaves framing alone");
    assert_eq!(tr["text"][0]["font_size_factor"], 0.1);
    assert_eq!(tr["text"][0]["end"], 3.0);
    let fx = tr["effects"].as_array().unwrap();
    assert_eq!(fx.len(), 1);
    assert_eq!(fx[0]["start"], 5.0);

    // streamer-split re-lays a center-crop clip as a split with the hook on the seam.
    let r = f.call("template.apply", json!({"name": "streamer-split", "clip_ids": [id]})).await;
    assert_eq!(r["clips"][0]["changes"].as_array().unwrap().is_empty(), false);
    let doc = f.show(&id).await["document"].clone();
    let lay = &doc["tracks"]["video"][0]["layout"];
    assert_eq!(lay["preset"], "split-vertical");
    assert_eq!(lay["regions"].as_array().unwrap().len(), 2);
    assert_eq!(doc["tracks"]["text"][0]["position_y"], 0.5);
    assert_eq!(doc["tracks"]["audio"]["denoise"], "light");
    assert_eq!(doc["template_ref"]["id"], "streamer-split");
    let fx = doc["tracks"]["effects"].as_array().unwrap();
    assert_eq!(fx.len(), 1, "previous template's punch-ins replaced: {fx:?}");
    assert_eq!(fx[0]["start"], 6.0);

    // clip.create --template, and config template.default.
    let c = f.call("clip.create", json!({"name": "t", "source_id": f.src.id, "start": 1.0, "end": 7.0, "template": "podcast-viral"})).await;
    assert_eq!(c["captions"]["preset"], "hormozi-pop");
    assert_eq!(f.show(c["id"].as_str().unwrap()).await["document"]["template_ref"]["id"], "podcast-viral");
    f.call("config.set", json!({"key": "template.default", "value": "streamer-split", "scope": "project"})).await;
    let c = f.call("clip.create", json!({"name": "d", "source_id": f.src.id, "start": 1.0, "end": 7.0})).await;
    assert_eq!(c["segments"][0]["layout"]["preset"], "split-vertical");
    f.call("config.set", json!({"key": "template.default", "value": "no-such-template", "scope": "project"})).await;
    let c = f.call("clip.create", json!({"name": "e", "source_id": f.src.id, "start": 1.0, "end": 7.0})).await;
    assert!(c["warnings"].to_string().contains("no-such-template"), "{c}");
    assert_eq!(c["segments"][0]["layout"]["preset"], "center-crop");

    // Updating a starter saves the agent's own version that shadows it.
    let u = f.call("template.update", json!({"name": "podcast-viral", "patch": {"captions": {"max_words_per_line": 2}}})).await;
    assert_eq!(u["version"], 2);
    assert_eq!(u["scope"], "project");
    let s = f.call("template.show", json!({"name": "podcast-viral"})).await;
    assert_eq!(s["scope"], "project");
    let s = f.call("template.show", json!({"name": "podcast-viral@1"})).await;
    assert_eq!(s["scope"], "built-in");
}
