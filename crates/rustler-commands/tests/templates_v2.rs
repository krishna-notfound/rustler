//! Templates, the spec §5.3 fields: layout_rules, captions keyword_highlight / emoji, cutaway_rules,
//! audio.music by tag / mood, brand (logo, CTA, outro), export, post_defaults. Each is captured
//! from a clip, applied through timeline.apply (tagged, re-apply is a no-op), validated, and the
//! referenced assets travel with export / import.
mod common;

use common::*;
use rustler_core::{ClipId, ErrorCode, ProjectStore};
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
    for f in ["logo.png", "cta.png", "outro.png", "broll.png", "calm.mp3", "loud.mp3"] {
        std::fs::write(files.join(f), format!("bytes of {f}")).unwrap();
    }
    Fx { _tmp: tmp, store, ctx, reg: registry(), src, files }
}

impl Fx {
    async fn call(&self, name: &str, v: Value) -> Value {
        call(&self.reg, &self.ctx, name, v).await.unwrap_or_else(|e| panic!("{name}: {} ({:?})", e.message, e.hint))
    }
    async fn err(&self, name: &str, v: Value) -> rustler_core::RustlerError {
        call(&self.reg, &self.ctx, name, v).await.expect_err(name)
    }
    /// One segment, source 1.0..9.5 (8.5 s, no cuts).
    async fn clip(&self, name: &str) -> String {
        let c = self.call("clip.create", json!({"name": name, "source_id": self.src.id, "start": 1.0, "end": 9.5})).await;
        c["id"].as_str().unwrap().to_string()
    }
    async fn doc(&self, clip: &str) -> Value {
        self.call("timeline.show", json!({"clip_id": clip})).await["document"].clone()
    }
    async fn asset(&self, file: &str, kind: &str, tags: &[&str]) -> String {
        let a = self.call("asset.add", json!({"path": self.files.join(file), "kind": kind, "license": "CC0-1.0", "tags": tags})).await;
        a["asset"]["id"].as_str().unwrap().to_string()
    }
    async fn create(&self, name: &str, t: Value) -> Value {
        self.call("template.create", json!({"name": name, "template": t})).await
    }
    async fn apply(&self, name: &str, clip: &str) -> Value {
        self.call("template.apply", json!({"name": name, "clip_ids": [clip]})).await["clips"][0].clone()
    }
    fn faces_json(&self, samples: Vec<Value>) {
        let dir = self.store.root.join("analysis").join(self.src.id.as_str());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("faces.json"), serde_json::to_vec(&json!({"width": 1920, "height": 1080, "fps": 1.0, "samples": samples})).unwrap()).unwrap();
    }
}

fn warnings(c: &Value) -> String {
    c["warnings"].to_string()
}

/// Validation errors name the JSON path.
async fn invalid(f: &Fx, name: &str, t: Value, path: &str) {
    let e = f.err("template.create", json!({"name": name, "template": t})).await;
    assert_eq!(e.code, ErrorCode::Validation, "{}", e.message);
    assert!(e.message.contains(path), "{path} not in: {}", e.message);
}

#[tokio::test]
async fn layout_rules_apply_per_shot_capture_and_validate() {
    let f = fx();
    // One face (left of centre) for source 1..5 s, nobody after.
    f.faces_json((1..10).map(|t| json!({"t": t as f64, "faces": if t < 5 { json!([{"x": 0.3, "y": 0.2, "w": 0.08, "h": 0.15, "score": 0.9}]) } else { json!([]) }})).collect());
    let c = f.call("clip.create", json!({"name": "a", "source_id": f.src.id, "start": 1.0, "end": 4.8})).await;
    let id = c["id"].as_str().unwrap().to_string();
    f.call("clip.add_segment", json!({"clip_id": id, "source_id": f.src.id, "start": 5.2, "end": 9.5})).await;
    // Both segments blur-fit first (a segment already on the rule's preset keeps its framing).
    f.call("clip.set_layout", json!({"clip_id": id, "preset": "blur-fit"})).await;
    f.create(
        "rules",
        json!({"layout_rules": [
            {"when": "faces=1", "layout": "center-crop", "track": "face", "look_room": false},
            {"when": "no_face", "layout": "blur-fit", "background": "blur_fill"}
        ]}),
    )
    .await;
    let r = f.apply("rules", &id).await;
    let w = warnings(&r);
    assert!(w.contains("crop track-face") && w.contains("--look-room false"), "the tracking command is given: {w}");
    let doc = f.doc(&id).await;
    let v = doc["tracks"]["video"].as_array().unwrap();
    assert_eq!(v[0]["layout"]["preset"], "center-crop");
    // Framed on the face (source x 0.34): the crop window starts left of the centre one.
    let src_x = v[0]["layout"]["regions"][0]["src"]["x"].as_f64().unwrap();
    assert!(src_x < 0.3, "crop follows the face: {src_x}");
    assert_eq!(v[1]["layout"]["preset"], "blur-fit");
    // Re-apply: nothing changes.
    assert_eq!(f.apply("rules", &id).await["ops"], 0);

    // Capture: one rule per shot class, with the segment's layout and (no crop path) a static crop.
    let t = f.call("template.create", json!({"name": "rules-cap", "from_clip": id})).await["template"].clone();
    let rules = t["layout_rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2, "{rules:?}");
    assert_eq!(rules[0]["when"], "faces=1");
    assert_eq!(rules[0]["layout"], "center-crop");
    assert_eq!(rules[0]["track"], "none");
    assert_eq!(rules[1]["when"], "no_face");
    assert_eq!(rules[1]["layout"], "blur-fit");

    // A source without any shot data: warning, segment unchanged.
    let media = f.store.root.join("w.mp4");
    std::fs::write(&media, b"x").unwrap();
    let other = add_source(&f.store, &media, probe_1080p(12.0, true, true));
    let c = f.call("clip.create", json!({"name": "b", "source_id": other.id, "start": 1.0, "end": 4.0})).await;
    let b = c["id"].as_str().unwrap().to_string();
    let before = f.doc(&b).await["tracks"]["video"][0]["layout"].clone();
    let r = f.apply("rules", &b).await;
    assert!(warnings(&r).contains("no face / shot data"), "{r}");
    assert_eq!(f.doc(&b).await["tracks"]["video"][0]["layout"], before);

    // Validation.
    invalid(&f, "bad1", json!({"layout_rules": [{"when": "faces=1", "layout": "blur-fit", "track": "face"}]}), "/layout_rules/0/track").await;
    invalid(&f, "bad2", json!({"layout_rules": [{"when": "faces=1", "layout": "blur-fit"}, {"when": "faces=1", "layout": "center-crop"}]}), "/layout_rules/1/when").await;
    invalid(&f, "bad3", json!({"layout_rules": [{"when": "faces=1"}]}), "/layout_rules/0").await;
    let e = f.err("template.create", json!({"name": "bad4", "template": {"layout_rules": [{"when": "faces=3", "layout": "blur-fit"}]}})).await;
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn keyword_highlight_and_emoji() {
    let f = fx();
    let id = f.clip("k").await;
    // The agent put an emoji on "Hello" (word 0) and coloured "country" itself.
    f.call("caption.word", json!({"clip_id": id, "source_id": f.src.id, "word_i": 0, "emoji": "\u{1F525}"})).await;
    f.call("caption.word", json!({"clip_id": id, "source_id": f.src.id, "word_i": 9, "color": "#00FF00"})).await;
    f.create("kw", json!({"captions": {"max_words_per_line": 3, "keyword_highlight": {"color": "#FF00FF", "max_per_line": 1}, "emoji": "off"}})).await;
    let r = f.apply("kw", &id).await;
    assert!(warnings(&r).contains("emoji off"), "{r}");
    let ov = f.doc(&id).await["tracks"]["captions"]["word_overrides"].as_array().unwrap().clone();
    let marked: Vec<&Value> = ov.iter().filter(|o| o["by_template"] == "kw@1").collect();
    assert!(!marked.is_empty(), "{ov:?}");
    assert!(marked.iter().all(|o| o["color"] == "#FF00FF" && o["template_role"] == "keyword"));
    // "Americans" (a name inside a sentence) is a keyword; the agent's own colour is kept.
    assert!(marked.iter().any(|o| o["word_anchor"]["word_i"] == 4), "{marked:?}");
    assert!(ov.iter().any(|o| o["word_anchor"]["word_i"] == 9 && o["color"] == "#00FF00" && o.get("by_template").is_none()));
    assert!(ov.iter().all(|o| o.get("emoji").is_none()), "emoji off removes them: {ov:?}");
    // At most one per caption line of 3 words.
    let mut lines = std::collections::BTreeSet::new();
    for o in &marked {
        assert!(lines.insert(o["word_anchor"]["word_i"].as_u64().unwrap() / 3), "two keywords on a line: {marked:?}");
    }
    assert_eq!(f.apply("kw", &id).await["ops"], 0, "re-apply is a no-op");

    // Capture: the colour and the per-line count.
    let t = f.call("template.create", json!({"name": "kw-cap", "from_clip": id})).await["template"].clone();
    assert_eq!(t["captions"]["keyword_highlight"]["color"], "#FF00FF");
    assert_eq!(t["captions"]["keyword_highlight"]["max_per_line"], 1);
    // emoji: on is captured from a clip with emoji, and applying "on" keeps them.
    let b = f.clip("e").await;
    f.call("caption.word", json!({"clip_id": b, "source_id": f.src.id, "word_i": 1, "emoji": "\u{1F525}"})).await;
    let t = f.call("template.create", json!({"name": "emo", "from_clip": b})).await["template"].clone();
    assert_eq!(t["captions"]["emoji"], "on");
    f.apply("emo", &b).await;
    assert!(f.doc(&b).await["tracks"]["captions"]["word_overrides"].to_string().contains('\u{1F525}'));

    invalid(&f, "bad", json!({"captions": {"keyword_highlight": {"color": "pink", "max_per_line": 9}}}), "/captions/keyword_highlight/color").await;
    invalid(&f, "bad2", json!({"captions": {"keyword_highlight": {"max_per_line": 9}}}), "/captions/keyword_highlight/max_per_line").await;
}

#[tokio::test]
async fn cutaway_rules_fill_weak_visual_stretches() {
    let f = fx();
    let id = f.clip("c").await;
    // No tagged asset yet: warning, nothing added.
    f.create("cut", json!({"cutaway_rules": {"on_weak_visual_after_s": 3.0, "kind": "asset_tag", "tag": "broll", "duration": 1.5}})).await;
    let r = f.apply("cut", &id).await;
    assert!(warnings(&r).contains("no image asset in the library has tag 'broll'"), "{r}");
    assert!(f.doc(&id).await["tracks"]["cutaways"].as_array().is_none_or(|c| c.is_empty()));

    let broll = f.asset("broll.png", "image", &["broll"]).await;
    let r = f.apply("cut", &id).await;
    assert!(r["ops"].as_u64().unwrap() > 0, "{r}");
    let cuts = f.doc(&id).await["tracks"]["cutaways"].as_array().unwrap().clone();
    assert_eq!(cuts.len(), 1, "one 8.5 s static stretch: {cuts:?}");
    assert_eq!(cuts[0]["media"]["asset"], broll.as_str());
    assert_eq!(cuts[0]["by_template"], "cut@1");
    let (s, e) = (cuts[0]["out_start"].as_f64().unwrap(), cuts[0]["out_end"].as_f64().unwrap());
    assert!((e - s - 1.5).abs() < 0.02 && s > 3.0 && e < 5.5, "centred in the stretch: {s}..{e}");
    assert_eq!(f.apply("cut", &id).await["ops"], 0, "re-apply is a no-op");

    // Capture: library cutaways with a tag become an asset_tag rule.
    let t = f.call("template.create", json!({"name": "cut-cap", "from_clip": id})).await["template"].clone();
    assert_eq!(t["cutaway_rules"]["kind"], "asset_tag");
    assert_eq!(t["cutaway_rules"]["tag"], "broll");

    // punch_in (only when a template sets it): planner punch-ins inside the weak stretch, tagged.
    let b = f.clip("p").await;
    f.create("weak-punch", json!({"cutaway_rules": {"on_weak_visual_after_s": 2.0, "kind": "punch_in"}})).await;
    f.apply("weak-punch", &b).await;
    let fx = f.doc(&b).await["tracks"]["effects"].as_array().cloned().unwrap_or_default();
    assert!(fx.iter().all(|e| e["kind"] == "punch-in" && e["template_role"] == "weak_punch"), "{fx:?}");
    assert_eq!(f.apply("weak-punch", &b).await["ops"], 0);

    invalid(&f, "bad", json!({"cutaway_rules": {"on_weak_visual_after_s": 3.0, "kind": "asset_tag"}}), "/cutaway_rules/tag").await;
    invalid(&f, "bad2", json!({"cutaway_rules": {"on_weak_visual_after_s": 0.2, "kind": "punch_in"}}), "/cutaway_rules/on_weak_visual_after_s").await;
}

#[tokio::test]
async fn music_by_tag_or_mood() {
    let f = fx();
    let id = f.clip("m").await;
    f.create("calm", json!({"audio": {"music": {"tag": "calm", "volume_db": -20.0}}})).await;
    let r = f.apply("calm", &id).await;
    assert!(warnings(&r).contains("no music asset in the library with tag 'calm'"), "{r}");
    assert!(f.doc(&id).await["tracks"]["audio"]["music"].is_null());

    f.asset("calm.mp3", "music", &["calm", "mood:chill"]).await;
    f.asset("loud.mp3", "music", &["hype"]).await;
    f.apply("calm", &id).await;
    let m = f.doc(&id).await["tracks"]["audio"]["music"].clone();
    assert!(m["path"].as_str().unwrap().ends_with("calm.mp3"), "{m}");
    assert_eq!(m["volume_db"], -20.0);
    assert_eq!(f.apply("calm", &id).await["ops"], 0);
    // mood matches a `mood:<x>` tag.
    f.create("chill", json!({"audio": {"music": {"mood": "chill"}}})).await;
    f.create("silent", json!({"audio": {"music": {"none": true}}})).await;
    f.apply("silent", &id).await;
    assert!(f.doc(&id).await["tracks"]["audio"]["music"].is_null());
    f.apply("chill", &id).await;
    assert!(f.doc(&id).await["tracks"]["audio"]["music"]["path"].as_str().unwrap().ends_with("calm.mp3"));
    // Not a fixed reference: show lists no music ref.
    let s = f.call("template.show", json!({"name": "chill"})).await;
    assert!(s["refs"].as_array().unwrap().is_empty());

    invalid(&f, "bad", json!({"audio": {"music": {"tag": "calm", "asset": "music-x"}}}), "/audio/music").await;
    invalid(&f, "bad2", json!({"audio": {"music": {"tag": " "}}}), "/audio/music").await;
}

#[tokio::test]
async fn brand_logo_cta_outro_capture_missing_and_bundle() {
    let f = fx();
    let logo = f.asset("logo.png", "image", &["logo"]).await;
    let cta = f.asset("cta.png", "image", &["cta"]).await;
    let outro = f.asset("outro.png", "image", &["outro"]).await;
    let music = f.asset("calm.mp3", "music", &["calm"]).await;
    let id = f.clip("b").await;
    let t = json!({
        "brand": {
            "logo_asset": logo, "rect": {"x": 0.8, "y": 0.02, "w": 0.15, "h": 0.07}, "opacity": 0.8,
            "cta_card": {"asset": cta, "duration": 2.0},
            "outro_card": {"asset": outro, "duration": 2.5}
        },
        "audio": {"music": {"asset": music}}
    });
    f.create("brand", t).await;
    let s = f.call("template.show", json!({"name": "brand"})).await;
    assert_eq!(s["refs"].as_array().unwrap().len(), 4, "{}", s["refs"]);
    f.apply("brand", &id).await;
    let ims = f.doc(&id).await["tracks"]["images"].as_array().unwrap().clone();
    assert_eq!(ims.len(), 3, "{ims:?}");
    let role = |r: &str| ims.iter().find(|x| x["template_role"] == r).unwrap_or_else(|| panic!("{r} in {ims:?}")).clone();
    let l = role("brand_logo");
    assert_eq!(l["asset"], logo.as_str());
    assert_eq!(l["opacity"], 0.8);
    assert!(l.get("out_start").is_none() && l.get("out_end").is_none(), "the logo stays the whole clip: {l}");
    let c = role("brand_cta");
    assert_eq!(c["out_start"], 3.4, "CTA 40 % in by default");
    assert_eq!(c["out_end"], 5.4);
    let o = role("brand_outro");
    assert_eq!(o["out_start"], 6.0);
    assert_eq!(f.apply("brand", &id).await["ops"], 0, "re-apply is a no-op");

    // Capture: the brand kit comes back from the clip.
    let cap = f.call("template.create", json!({"name": "brand-cap", "from_clip": id})).await["template"].clone();
    assert_eq!(cap["brand"]["logo_asset"], logo.as_str());
    assert_eq!(cap["brand"]["cta_card"]["asset"], cta.as_str());
    assert_eq!(cap["brand"]["cta_card"]["at"], 3.4);
    assert_eq!(cap["brand"]["outro_card"]["asset"], outro.as_str());
    assert_eq!(cap["brand"]["outro_card"]["duration"], 2.5);
    assert!(cap["images"].as_array().is_none_or(|i| i.is_empty()), "brand images are not repeated in images: {}", cap["images"]);

    // Missing logo asset -> not_found naming it.
    f.create("no-logo", json!({"brand": {"logo_asset": "image-gone-logo"}})).await;
    let e = f.err("template.apply", json!({"name": "no-logo", "clip_ids": [id]})).await;
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("image-gone-logo") && e.message.contains("/brand/logo_asset"), "{}", e.message);
    invalid(&f, "bad", json!({"brand": {"opacity": 2.0}}), "/brand/opacity").await;
    invalid(&f, "bad2", json!({"brand": {"outro_card": {"asset": "x", "at": 3.0}}}), "/brand/outro_card/at").await;

    // Export / import round trip with the logo, cards and music.
    let zip = f.files.join("out").join("brand.zip");
    let x = f.call("template.export", json!({"name": "brand", "out": zip})).await;
    let mut assets: Vec<String> = x["assets"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assets.sort();
    let mut want = vec![logo.clone(), cta.clone(), outro.clone(), music.clone()];
    want.sort();
    assert_eq!(assets, want);
    let g = fx();
    let gid = g.clip("other").await;
    let i = g.call("template.import", json!({"file": zip})).await;
    assert_eq!(i["assets_added"].as_array().unwrap().len(), 4, "{i}");
    assert!(g.call("template.show", json!({"name": "brand"})).await.get("missing").is_none());
    g.apply("brand", &gid).await;
    let doc = g.doc(&gid).await;
    let ims = doc["tracks"]["images"].as_array().unwrap();
    assert_eq!(ims.len(), 3);
    assert!(ims.iter().all(|x| x["file"].as_str().unwrap().starts_with(g.store.root.to_str().unwrap())), "{ims:?}");
    assert!(doc["tracks"]["audio"]["music"]["path"].as_str().unwrap().starts_with(g.store.root.to_str().unwrap()));
}

#[tokio::test]
async fn export_settings_are_stored_and_length_warned() {
    let f = fx();
    let id = f.clip("x").await;
    f.create("short", json!({"export": {"quality": "draft", "fps": 25.0, "max_len": 5.0, "min_len": 2.0}})).await;
    let r = f.apply("short", &id).await;
    assert!(warnings(&r).contains("longer than this template's max_len 5s"), "{r}");
    let doc = f.doc(&id).await;
    assert_eq!(doc["canvas"]["fps"], 25.0);
    assert_eq!(doc["tracks"]["extra"]["export"]["quality"], "draft");
    let x = rustler_commands::template_cmds::clip_export(&f.store, &serde_json::from_value::<ClipId>(json!(id)).unwrap()).unwrap();
    assert_eq!(x.max_len, Some(5.0));
    assert_eq!(serde_json::to_value(x.quality).unwrap(), "draft");
    assert_eq!(f.apply("short", &id).await["ops"], 0);
    // Captured back.
    let t = f.call("template.create", json!({"name": "short-cap", "from_clip": id})).await["template"].clone();
    assert_eq!(t["export"], json!({"quality": "draft", "fps": 25.0, "max_len": 5.0, "min_len": 2.0}));
    f.create("long", json!({"export": {"min_len": 20.0}})).await;
    assert!(warnings(&f.apply("long", &id).await).contains("shorter than this template's min_len 20s"));
    invalid(&f, "bad", json!({"export": {"min_len": 30.0, "max_len": 10.0}}), "/export").await;
    invalid(&f, "bad2", json!({"export": {"fps": 0.0}}), "/export/fps").await;
}

#[tokio::test]
async fn post_defaults_merge_without_overwriting_copy() {
    let f = fx();
    let id = f.clip("p").await;
    f.call(
        "clip.meta_set",
        json!({"clip_id": id, "platforms": {"shorts": {"title": "He quit", "description": "The story.", "hashtags": ["#a"], "mentions": []},
                                            "tiktok": {"title": "He quit", "description": "Watch.", "hashtags": ["#a", "#brand"]}}}),
    )
    .await;
    f.create("post", json!({"post_defaults": {"hashtags_always": ["brand", "#A"], "mentions": ["@me"], "cta_line": "Follow for part 2"}})).await;
    f.apply("post", &id).await;
    let doc = f.doc(&id).await;
    let s = &doc["meta"]["post"]["platforms"]["shorts"];
    assert_eq!(s["title"], "He quit");
    assert_eq!(s["hashtags"], json!(["#a", "#brand"]));
    assert_eq!(s["mentions"], json!(["@me"]));
    assert_eq!(s["description"], "The story.\n\nFollow for part 2");
    let t = &doc["meta"]["post"]["platforms"]["tiktok"];
    assert_eq!(t["hashtags"], json!(["#a", "#brand"]));
    assert_eq!(doc["tracks"]["extra"]["post_defaults"]["cta_line"], "Follow for part 2");
    assert_eq!(f.apply("post", &id).await["ops"], 0, "re-apply adds nothing twice");
    let cap = f.call("template.create", json!({"name": "post-cap", "from_clip": id})).await["template"].clone();
    assert_eq!(cap["post_defaults"]["hashtags_always"], json!(["brand", "#A"]));
    invalid(&f, "bad", json!({"post_defaults": {"hashtags_always": ["two words"]}}), "/post_defaults/hashtags_always/0").await;

    // The merge itself (export.clip uses it on the post JSON too).
    let d: rustler_commands::template_cmds::TplPost = serde_json::from_value(json!({"hashtags_always": ["x"], "cta_line": "Follow"})).unwrap();
    let mut body = json!({"title": null, "description": null, "hashtags": [], "mentions": []});
    assert!(rustler_commands::template_cmds::merge_post_defaults(&mut body, &d));
    assert_eq!(body["hashtags"], json!(["#x"]));
    assert!(body["description"].is_null(), "no copy written: the CTA is not invented into it");
}

#[tokio::test]
async fn old_template_files_still_load() {
    let f = fx();
    // A v0.2.1 template file (no new fields) still parses, validates and applies.
    let dir = f.store.root.join("templates");
    std::fs::create_dir_all(&dir).unwrap();
    let old = json!({"schema": "rustler.template@1", "name": "old", "version": 1, "captions": {"preset": "hormozi-pop"},
                     "audio": {"music": {"none": true}}, "effects": [{"kind": "punch-in", "at": "start", "duration": 1.0, "amount": 1.1}]});
    std::fs::write(dir.join("old@1.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    let id = f.clip("o").await;
    let r = f.apply("old", &id).await;
    assert!(r["ops"].as_u64().unwrap() > 0);
    assert_eq!(f.doc(&id).await["tracks"]["effects"].as_array().unwrap().len(), 1);
}
