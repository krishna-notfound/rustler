//! Posting JSON + campaigns (v2 Task 1.7): campaign.*, clip.meta_*, QC rules and the export's post.json.
//! Tests that render/QC use real ffmpeg and skip (printing why) when it is missing.

use chrono::Utc;
use rustler_commands::register_all;
use rustler_core::layout::{self, LayoutParams};
use rustler_core::model::{CaptionScript, WordTiming};
use rustler_core::{
    AudioSettings, Canvas, CaptionTrack, Clip, ClipId, ErrorCode, MediaProbe, ProjectStore, RenderId,
    RenderQuality, RenderRecord, Segment, SegmentId, Source, SourceId, SourceOrigin, Transcript, Word,
};
use rustler_registry::{Ctx, Registry};
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    reg: Registry,
    clip_id: ClipId,
    render_id: RenderId,
}

fn tool_paths() -> Option<(PathBuf, ToolPaths)> {
    let home = ToolPaths::default_home();
    let paths = ToolPaths::from_home(&home);
    if !paths.ffmpeg.exists() || !paths.ffprobe.exists() {
        eprintln!("SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)", paths.bin.display());
        return None;
    }
    if rustler_media::qc::default_font(&paths).is_none() {
        eprintln!("SKIP: no font for the contact sheet");
        return None;
    }
    Some((home, paths))
}

fn ffmpeg(t: &ToolPaths, args: &[&str]) {
    let out = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
}

/// 6 s 1080x1920 render: `good` = testsrc2 + loudnormalised tone, else all black.
fn make_render(t: &ToolPaths, path: &Path, good: bool) {
    let v = if good {
        "testsrc2=size=1080x1920:rate=30:duration=6"
    } else {
        "color=c=black:size=1080x1920:rate=30:duration=6"
    };
    let mut args = vec!["-f", "lavfi", "-i", v, "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=6"];
    if good {
        args.extend(["-af", "loudnorm=I=-14:TP=-2:LRA=11,aresample=48000"]);
    }
    args.extend(["-t", "6", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ar", "48000"]);
    args.push(path.to_str().unwrap());
    ffmpeg(t, &args);
}

fn w(i: u32, text: &str, start: f64, end: f64) -> Word {
    Word { i, text: text.to_string(), start, end, conf: None, speaker: None }
}

/// Project with one source (+ transcript), one clip [10, 16) and one render record whose mp4 is `good` or black.
fn fixture(good: bool) -> Option<Fx> {
    let (home, paths) = tool_paths()?;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (store, _) = ProjectStore::create(&root, "t").unwrap();

    let source_id = SourceId::new();
    store
        .save_source(&Source {
            id: source_id.clone(),
            name: "talk.mp4".into(),
            origin: SourceOrigin::File { path: PathBuf::from("/x/talk.mp4") },
            media_path: PathBuf::from("/x/talk.mp4"),
            probe: MediaProbe { duration: 60.0, has_video: true, has_audio: true, width: 1920, height: 1080, fps: 30.0, ..Default::default() },
            proxy: None,
            audio_wav: None,
            imported_at: Utc::now(),
        })
        .unwrap();
    let words = vec![
        w(0, "So", 9.0, 9.2), // outside the segment
        w(1, "Hello", 10.2, 10.6),
        w(2, "there", 10.65, 11.0),
        w(3, "my", 11.05, 11.2),
        w(4, "friends.", 11.25, 11.9),
        w(5, "Bye", 15.0, 15.4),
    ];
    let transcript = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source_id.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    store
        .write_json(&Path::new("analysis").join(source_id.as_str()).join("transcript.json"), &transcript)
        .unwrap();

    let canvas = Canvas::default();
    let layout = layout::preset("center-crop", 1920, 1080, &canvas, &LayoutParams::default()).unwrap();
    let clip_id = ClipId::new();
    let render_id = RenderId::new();
    let rel = Path::new("clips").join(clip_id.as_str()).join("renders").join(format!("{render_id}.mp4"));
    std::fs::create_dir_all(root.join(&rel).parent().unwrap()).unwrap();
    make_render(&paths, &root.join(&rel), good);
    let mut clip = Clip {
        schema: "rustler.clip@1".into(),
        id: clip_id.clone(),
        name: "My hook (1) / test".into(),
        revision: 0,
        canvas,
        segments: vec![Segment { id: SegmentId::new(), source_id, start: 10.0, end: 16.0, layout }],
        captions: CaptionTrack::default(),
        audio: AudioSettings::default(),
        renders: vec![RenderRecord {
            id: render_id.clone(),
            path: rel,
            quality: RenderQuality::Final,
            encoder: "libx264".into(),
            created_at: Utc::now(),
            qc: None,
        }],
        created_at: Utc::now(),
    };
    store.save_clip(&mut clip, 0).unwrap();
    let mut reg = Registry::new();
    register_all(&mut reg);
    Some(Fx { _dir: dir, root, home, reg, clip_id, render_id })
}

impl Fx {
    fn ctx(&self) -> Ctx {
        Ctx::new(Some(self.root.clone()), self.home.clone())
    }
    fn store(&self) -> ProjectStore {
        ProjectStore::open(&self.root).unwrap()
    }
    async fn run(&self, cmd: &str, input: Value) -> rustler_core::Result<Value> {
        self.reg.dispatch(cmd, input, self.ctx()).await
    }
    fn events(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("events.ndjson"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}


// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

fn spring() -> Value {
    json!({
        "id": "spring",
        "name": "Spring promo",
        "platform": "shorts",
        "payout": {"model": "per_view", "rate": 1.5},
        "duration": {"min": 3, "max": 60},
        "hashtags_required": ["ad"],
        "mentions_required": ["brand"],
        "rules": [
            {"id": "no-sponsor", "text": "Never say the word sponsor", "kind": "machine", "check": {"type": "banned_word", "value": "sponsor"}},
            {"id": "cta", "text": "Description links the shop", "kind": "machine", "check": {"type": "description_contains", "value": "brand.shop"}},
            {"id": "title-len", "text": "Title max 60 characters", "kind": "machine", "check": {"type": "title_max_chars", "value": 60}},
            {"id": "shows-product", "text": "The product is on screen", "kind": "agent"}
        ]
    })
}

fn good_copy() -> Value {
    json!({"title": "He said hello to the whole world", "description": "Full story at brand.shop #ad @brand", "hashtags": ["ad", "#viral"], "mentions": ["brand"]})
}

fn rule<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["rules"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no rule {id} in {v}"))
}

#[tokio::test]
async fn campaign_crud_and_validation() {
    let Some(fx) = fixture(true) else { return };
    let made = fx.run("campaign.create", spring()).await.unwrap();
    assert_eq!(made["campaign"]["hashtags_required"], json!(["#ad"]));
    assert_eq!(made["campaign"]["mentions_required"], json!(["@brand"]));
    assert_eq!(made["requirements"].as_array().unwrap().len(), 4 + 3);
    assert!(fx.root.join("campaigns").join("spring.json").is_file());

    let e = fx.run("campaign.create", spring()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation, "duplicate id needs --replace");
    let mut again = spring();
    again["replace"] = json!(true);
    assert_eq!(fx.run("campaign.create", again).await.unwrap()["replaced"], true);

    for bad in [
        json!({"id": "Bad Id"}),
        json!({"id": "x", "rules": [{"id": "r", "text": "t", "kind": "machine"}]}),
        json!({"id": "x", "rules": [{"id": "r", "text": "t", "kind": "agent", "check": {"type": "hashtag", "value": "#a"}}]}),
        json!({"id": "x", "rules": [{"id": "r", "text": "t", "kind": "machine", "check": {"type": "duration_min", "value": "ten"}}]}),
        json!({"id": "x", "rules": [{"id": "duration", "text": "t", "kind": "agent"}]}),
        json!({"id": "x", "rules": [{"id": "r", "text": "t", "kind": "agent"}, {"id": "r", "text": "u", "kind": "agent"}]}),
        json!({"id": "x", "duration": {"min": 60, "max": 20}}),
        json!({"id": "x", "hashtags_required": ["two words"]}),
    ] {
        let e = fx.run("campaign.create", bad.clone()).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "{bad}");
    }
    let list = fx.run("campaign.list", json!({})).await.unwrap();
    assert_eq!(list["count"], 1);
    assert_eq!(list["campaigns"][0]["agent_rules"], 1);
    let e = fx.run("campaign.show", json!({"id": "nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.hint.unwrap().contains("spring"));
    fx.run("campaign.delete", json!({"id": "spring"})).await.unwrap();
    assert_eq!(fx.run("campaign.list", json!({})).await.unwrap()["count"], 0);
}

#[tokio::test]
async fn machine_rules_checked() {
    let Some(fx) = fixture(true) else { return };
    fx.run("campaign.create", spring()).await.unwrap();
    let id = fx.clip_id.to_string();

    // Copy that breaks every machine rule: no hashtag/mention, no shop link, banned word, long title.
    let bad = json!({"title": "A very long title about the sponsor that goes on and on and on and on forever", "description": "buy now", "hashtags": ["viral"]});
    let v = fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "spring", "platforms": {"shorts": bad}})).await.unwrap();
    let c = &v["compliance_now"];
    assert_eq!(c["passed"], false);
    for r in ["no-sponsor", "cta", "title-len", "hashtags_required", "mentions_required"] {
        assert_eq!(rule(c, r)["status"], "fail", "{r}: {c}");
    }
    assert!(rule(c, "no-sponsor")["reason"].as_str().unwrap().contains("title"));
    // The clip is ~6 s: inside 3..60.
    assert_eq!(rule(c, "duration")["status"], "pass");
    assert_eq!(rule(c, "shows-product")["status"], "pending");

    // Good copy: every machine rule passes; only the agent rule is left.
    let v = fx.run("clip.meta_set", json!({"clip_id": id, "platforms": {"shorts": good_copy()}})).await.unwrap();
    let c = &v["compliance_now"];
    for r in ["no-sponsor", "cta", "title-len", "hashtags_required", "mentions_required", "duration"] {
        assert_eq!(rule(c, r)["status"], "pass", "{r}: {c}");
    }
    assert_eq!(c["passed"], false, "agent rule still pending");

    // A duration rule that the ~6 s clip violates, and a spoken banned word ("friends" is in the transcript).
    let mut tight = spring();
    tight["id"] = json!("tight");
    tight["duration"] = json!({"min": 20, "max": 60});
    tight["rules"] = json!([{"id": "no-friends", "text": "do not say friends", "kind": "machine", "check": {"type": "banned_word", "value": "friends"}}]);
    fx.run("campaign.create", tight).await.unwrap();
    let v = fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "tight"})).await.unwrap();
    let c = &v["compliance_now"];
    assert_eq!(rule(c, "duration")["status"], "fail");
    assert!(rule(c, "duration")["reason"].as_str().unwrap().contains("below 20"));
    assert!(rule(c, "no-friends")["reason"].as_str().unwrap().contains("spoken"));

    // meta_set limits and unknown platforms.
    let long = json!({"title": "x".repeat(101), "description": "d"});
    let e = fx.run("clip.meta_set", json!({"clip_id": id, "platforms": {"shorts": long}})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let e = fx.run("clip.meta_set", json!({"clip_id": id, "platforms": {"youtube": good_copy()}})).await.unwrap_err();
    assert!(matches!(e.code, ErrorCode::Validation | ErrorCode::Usage), "unknown platform key is rejected: {e:?}");
    let e = fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "ghost"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn agent_rule_needs_reason() {
    let Some(fx) = fixture(true) else { return };
    fx.run("campaign.create", spring()).await.unwrap();
    let id = fx.clip_id.to_string();

    // Not attached to a campaign yet.
    let e = fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "shows-product", "pass": true, "reason": "ok"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "spring", "platforms": {"shorts": good_copy()}})).await.unwrap();

    for reason in ["", "   "] {
        let e = fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "shows-product", "pass": true, "reason": reason})).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "empty reason refused");
        assert!(e.message.contains("reason"));
    }
    // Machine rules and unknown rules cannot be marked.
    let e = fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "cta", "pass": true, "reason": "looked fine"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("machine"));
    let e = fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "zzz", "pass": true, "reason": "r"})).await.unwrap_err();
    assert!(e.hint.unwrap().contains("shows-product"));

    let m = fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "shows-product", "pass": false, "reason": "product never visible"})).await.unwrap();
    assert_eq!(m["pending_agent_rules"], json!([]));
    let ctxv = fx.run("clip.meta_context", json!({"clip_id": id})).await.unwrap();
    assert_eq!(rule(&ctxv["campaign"]["compliance_now"], "shows-product")["status"], "fail");
    fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "shows-product", "pass": true, "reason": "product visible 0:01-0:04 in the contact sheet"})).await.unwrap();
    let ctxv = fx.run("clip.meta_context", json!({"clip_id": id})).await.unwrap();
    assert_eq!(ctxv["campaign"]["compliance_now"]["passed"], true, "{ctxv}");

    // Re-attaching to another campaign clears the marks.
    let mut other = spring();
    other["id"] = json!("other");
    fx.run("campaign.create", other).await.unwrap();
    fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "other"})).await.unwrap();
    let ctxv = fx.run("clip.meta_context", json!({"clip_id": id})).await.unwrap();
    assert_eq!(rule(&ctxv["campaign"]["compliance_now"], "shows-product")["status"], "pending");
}

#[tokio::test]
async fn meta_context_gives_the_agent_what_it_needs() {
    let Some(fx) = fixture(true) else { return };
    fx.run("campaign.create", spring()).await.unwrap();
    let id = fx.clip_id.to_string();
    fx.run("overlay.add_text", json!({"clip_id": id, "text": "Say hello to everyone"})).await.unwrap();
    let v = fx.run("clip.meta_context", json!({"clip_id": id})).await.unwrap();
    assert!(v["transcript"].as_str().unwrap().starts_with("Hello there my friends."), "{}", v["transcript"]);
    assert_eq!(v["hook"], "Say hello to everyone");
    assert!(v["clip"]["duration"].as_f64().unwrap() > 5.0);
    assert_eq!(v["platform_limits"]["shorts"]["hashtags_max"], 15);
    assert_eq!(v["campaigns_available"][0]["id"], "spring");
    assert!(v["campaign"].is_null(), "not attached yet");
    assert!(v["next"][0].as_str().unwrap().contains("campaign"));
    fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "spring"})).await.unwrap();
    let v = fx.run("clip.meta_context", json!({"clip_id": id})).await.unwrap();
    assert_eq!(v["campaign"]["id"], "spring");
    assert_eq!(v["needs_platform"], "shorts");
    assert!(v["campaign"]["requirements"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("Never say")));
}

#[tokio::test]
async fn qc_requires_meta() {
    let Some(fx) = fixture(true) else { return };
    let id = fx.clip_id.to_string();

    // No campaign anywhere: a missing copy is only a warning.
    let v = fx.run("qc.run", json!({"clip_id": id, "allow_fail": true})).await.unwrap();
    let check = |v: &Value, n: &str| v["checks"].as_array().unwrap().iter().find(|c| c["name"] == n).cloned().unwrap_or_else(|| panic!("no check {n}"));
    assert_eq!(check(&v, "meta_present")["status"], "warn");
    assert_eq!(check(&v, "campaign_compliant")["status"], "pass");
    assert_eq!(v["passed"], true);

    // With a campaign in the project the copy is required and QC fails (exit 7) without it.
    fx.run("campaign.create", spring()).await.unwrap();
    let e = fx.run("qc.run", json!({"clip_id": id})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::QcFailed);
    let v = fx.run("qc.run", json!({"clip_id": id, "allow_fail": true})).await.unwrap();
    assert_eq!(check(&v, "meta_present")["status"], "fail");
    assert!(check(&v, "meta_present")["fix_hint"].as_str().unwrap().contains("meta-set"));
    assert_eq!(check(&v, "campaign_compliant")["status"], "warn", "not attached to a campaign");

    // Copy for the wrong platform does not satisfy a Shorts campaign.
    fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "spring", "platforms": {"tiktok": good_copy()}})).await.unwrap();
    let v = fx.run("qc.run", json!({"clip_id": id, "allow_fail": true})).await.unwrap();
    assert_eq!(check(&v, "meta_present")["status"], "fail");

    // Copy written, agent rule still pending: meta passes, compliance fails and names the rule.
    fx.run("clip.meta_set", json!({"clip_id": id, "platforms": {"shorts": good_copy()}})).await.unwrap();
    let v = fx.run("qc.run", json!({"clip_id": id, "allow_fail": true})).await.unwrap();
    assert_eq!(check(&v, "meta_present")["status"], "pass");
    assert_eq!(check(&v, "campaign_compliant")["status"], "fail");
    assert!(check(&v, "campaign_compliant")["fix_hint"].as_str().unwrap().contains("shows-product"));

    // Marked: everything passes.
    fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "shows-product", "pass": true, "reason": "visible 0:01"})).await.unwrap();
    let v = fx.run("qc.run", json!({"clip_id": id})).await.unwrap();
    assert_eq!(check(&v, "campaign_compliant")["status"], "pass");
    assert_eq!(v["passed"], true);
}

#[tokio::test]
async fn export_writes_post_json() {
    let Some(fx) = fixture(true) else { return };
    let id = fx.clip_id.to_string();
    fx.run("campaign.create", spring()).await.unwrap();
    fx.run("clip.meta_set", json!({"clip_id": id, "campaign_id": "spring", "platforms": {"shorts": good_copy(), "tiktok": {"title": "tt title", "description": "tt desc #ad @brand brand.shop"}}})).await.unwrap();
    fx.run("campaign.mark", json!({"clip_id": id, "rule_id": "shows-product", "pass": true, "reason": "visible 0:01"})).await.unwrap();

    let out = fx.run("export.clip", json!({"clip_id": id, "name": "hello"})).await.unwrap();
    let post_path = PathBuf::from(out["post"].as_str().unwrap());
    assert_eq!(post_path, fx.root.join("exports").join("hello.post.json"));
    let post: Value = serde_json::from_slice(&std::fs::read(&post_path).unwrap()).unwrap();
    assert_eq!(post["schema"], "rustler.post@1");
    assert_eq!(post["clip"], id);
    assert_eq!(post["file"], "hello.mp4");
    assert!((post["duration"].as_f64().unwrap() - 6.0).abs() < 0.2);
    assert_eq!(post["platform"], "shorts", "the campaign's platform fills the top-level fields");
    assert_eq!(post["title"], "He said hello to the whole world");
    assert_eq!(post["hashtags"], json!(["#ad", "#viral"]));
    assert_eq!(post["mentions"], json!(["@brand"]));
    assert_eq!(post["platforms"]["tiktok"]["title"], "tt title");
    assert_eq!(post["meta_present"], true);
    assert_eq!(post["campaign"]["id"], "spring");
    assert_eq!(post["campaign"]["compliance"]["passed"], true);
    assert_eq!(post["campaign"]["requirements"].as_array().unwrap().len(), 7);
    assert_eq!(post["marks"]["shows-product"]["pass"], true);
    assert!(post.get("created_at").is_some() && post.get("score").is_some() && post.get("cover").is_some());
    assert!(fx.events().iter().any(|e| e["kind"] == "export.clip" && e["data"]["post"].is_string()));
}

#[tokio::test]
async fn export_without_meta_still_writes_post_json_and_warns() {
    let Some(fx) = fixture(true) else { return };
    let out = fx.run("export.clip", json!({"clip_id": fx.clip_id})).await.unwrap();
    let post: Value = serde_json::from_slice(&std::fs::read(out["post"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(post["meta_present"], false);
    assert!(post["title"].is_null() && post["campaign"].is_null());
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("no posting copy")), "{out}");
}
