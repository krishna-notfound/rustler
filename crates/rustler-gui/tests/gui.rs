use rustler_gui::{GuiOptions, RunningGui, start};
use rustler_registry::{Annotations, Ctx, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

#[derive(Deserialize, JsonSchema)]
struct In {
    a: u32,
}
#[derive(Serialize, JsonSchema)]
struct Out {
    b: u32,
    project: String,
}

struct Env {
    gui: RunningGui,
    base: String,
    _proj: tempfile::TempDir,
    _home: tempfile::TempDir,
    outside: tempfile::TempDir,
}

async fn setup() -> Env {
    let proj = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(proj.path().join("data.bin"), (0u8..=255).collect::<Vec<u8>>()).unwrap();
    std::fs::write(proj.path().join("clip.mp4"), vec![7u8; 1000]).unwrap();
    std::fs::write(
        proj.path().join("events.ndjson"),
        "{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"agent\",\"kind\":\"x\",\"project_revision\":1,\"data\":{}}\nnot json\n",
    )
    .unwrap();
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    std::fs::write(proj.path().join("frame_src_a.jpg"), b"\xFF\xD8fakejpeg").unwrap();
    std::fs::create_dir_all(home.path().join("fonts").join("Anton")).unwrap();
    std::fs::write(home.path().join("fonts").join("Anton").join("Anton-Regular.ttf"), b"ttfbytes").unwrap();
    std::fs::create_dir_all(proj.path().join("clips").join("clp_1")).unwrap();
    std::fs::write(
        proj.path().join("clips").join("clp_1").join("clip.json"),
        r#"{"id":"clp_1","revision":7}"#,
    )
    .unwrap();

    let mut reg = Registry::new();
    // see.frame stand-in: writes nothing, returns the jpg for source "src_a" only.
    reg.register(typed(
        "see.frame",
        "fake frame",
        &[],
        Annotations::default(),
        vec![],
        |ctx: Ctx, i: Value| async move {
            if i["source_id"] != "src_a" || !i["t"].is_number() || i["width"] != 320 {
                return Err(rustler_core::RustlerError::new(
                    rustler_core::ErrorCode::NotFound,
                    "see.frame",
                    format!("bad input {i}"),
                ));
            }
            let p = ctx.project_dir.unwrap().join("frame_src_a.jpg");
            Ok(json!({"image": p}))
        },
    ));
    // A slow command that reports progress and honours cancellation.
    reg.register(typed(
        "fake.slow",
        "slow",
        &[],
        Annotations::default(),
        vec![],
        |ctx: Ctx, i: Value| async move {
            let ms = i["ms"].as_u64().unwrap_or(300);
            ctx.progress.emit(rustler_registry::ProgressEvent {
                stage: "work".into(),
                pct: Some(0.5),
                message: Some("half".into()),
            });
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(json!({"done": true})),
                _ = ctx.cancel.cancelled() => Err(rustler_core::RustlerError::new(
                    rustler_core::ErrorCode::Cancelled, "fake.slow", "cancelled")),
            }
        },
    ));
    reg.register(typed(
        "fake.double",
        "Doubles a",
        &[],
        Annotations::default(),
        vec![],
        |ctx: Ctx, i: In| async move {
            Ok(Out {
                b: i.a * 2,
                project: ctx
                    .project_dir
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            })
        },
    ));
    let ctx = Ctx::new(Some(proj.path().to_path_buf()), home.path().to_path_buf());
    let gui = start(
        Arc::new(reg),
        ctx,
        &GuiOptions {
            port: Some(0),
            open_browser: false,
        },
    )
    .await
    .unwrap();
    let base = format!("http://127.0.0.1:{}", gui.port);
    Env {
        gui,
        base,
        _proj: proj,
        _home: home,
        outside,
    }
}

#[tokio::test]
async fn token_required() {
    let e = setup().await;
    let c = reqwest::Client::new();
    for path in ["/", "/api/commands", "/api/events", "/app.js"] {
        let r = c.get(format!("{}{}", e.base, path)).send().await.unwrap();
        assert_eq!(r.status(), 401, "{path}");
    }
    let r = c
        .get(format!("{}/api/commands?t=wrong", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    // header token works
    let r = c
        .get(format!("{}/api/commands", e.base))
        .header("x-rustler-token", &e.gui.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // bad host header is rejected
    let r = c
        .get(format!("{}/api/commands?t={}", e.base, e.gui.token))
        .header("host", "evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn page_references_js_modules() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    let html = c
        .get(format!("{}/?t={t}", e.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("style.css"));
    assert!(!html.contains("__TOKEN__"));
    assert!(!html.contains("__SCRIPTS__"));
    let mods = rustler_gui::js_modules();
    assert!(mods.len() >= 10, "{mods:?}");
    // every module is referenced in load order and served as JS
    let mut last = 0;
    for m in &mods {
        let at = html.find(&format!("/js/{m}?t=")).unwrap_or_else(|| panic!("{m} not in page"));
        assert!(at > last, "{m} out of order");
        last = at;
        let r = c.get(format!("{}/js/{m}?t={t}", e.base)).send().await.unwrap();
        assert_eq!(r.status(), 200, "{m}");
        assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/javascript"));
        assert!(!r.text().await.unwrap().trim().is_empty(), "{m} empty");
    }
    assert_eq!(mods[0], "core.js");
    assert_eq!(*mods.last().unwrap(), "boot.js");
    let core = c.get(format!("{}/js/core.js?t={t}", e.base)).send().await.unwrap().text().await.unwrap();
    assert!(core.contains("/api/run") && core.contains("/api/jobs"));
    // unknown module -> 404; modules need the token too
    let r = c.get(format!("{}/js/nope.js?t={t}", e.base)).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let r = c.get(format!("{}/js/core.js", e.base)).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

async fn jobs(c: &reqwest::Client, e: &Env) -> Vec<Value> {
    let v: Value = c
        .get(format!("{}/api/jobs?t={}", e.base, e.gui.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["jobs"].as_array().unwrap().clone()
}

async fn wait_job(c: &reqwest::Client, e: &Env, id: &str) -> Value {
    for _ in 0..200 {
        let js = jobs(c, e).await;
        let j = js.iter().find(|j| j["id"] == id).unwrap().clone();
        if j["status"] != "running" {
            return j;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("job {id} did not finish");
}

#[tokio::test]
async fn jobs_run_in_background_with_progress_and_cancel() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    // start -> returns at once with a job id; progress visible while running
    let env: Value = c
        .post(format!("{}/api/jobs?t={t}", e.base))
        .json(&json!({"command": "fake.slow", "input": {"ms": 400}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "{env}");
    let id = env["data"]["job_id"].as_str().unwrap().to_string();
    let mut saw_progress = false;
    for _ in 0..50 {
        let js = jobs(&c, &e).await;
        let j = js.iter().find(|j| j["id"] == id.as_str()).unwrap();
        if j["status"] == "running" && j["progress"]["stage"] == "work" {
            assert_eq!(j["command"], "fake.slow");
            assert!(j["result"].is_null());
            saw_progress = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(saw_progress, "progress event never showed up on the running job");
    let done = wait_job(&c, &e, &id).await;
    assert_eq!(done["status"], "ok");
    assert_eq!(done["result"]["ok"], true);
    assert_eq!(done["result"]["data"]["done"], true);
    assert!(done["finished_ms"].as_u64().unwrap() >= done["started_ms"].as_u64().unwrap());

    // cancel a long job
    let env: Value = c
        .post(format!("{}/api/jobs?t={t}", e.base))
        .json(&json!({"command": "fake.slow", "input": {"ms": 60000}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id2 = env["data"]["job_id"].as_str().unwrap().to_string();
    let r: Value = c
        .post(format!("{}/api/jobs/cancel?t={t}", e.base))
        .json(&json!({"job_id": id2}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["ok"], true);
    let j = wait_job(&c, &e, &id2).await;
    assert_eq!(j["status"], "cancelled");
    assert_eq!(j["result"]["error"]["code"], "cancelled");
    // cancelling a finished job -> error envelope
    let r: Value = c
        .post(format!("{}/api/jobs/cancel?t={t}", e.base))
        .json(&json!({"job_id": id2}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["ok"], false);

    // unknown command -> error envelope, no job
    let env: Value = c
        .post(format!("{}/api/jobs?t={t}", e.base))
        .json(&json!({"command": "nope.nope"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(jobs(&c, &e).await.len(), 2);
    // a failing command -> status error with the error envelope
    let env: Value = c
        .post(format!("{}/api/jobs?t={t}", e.base))
        .json(&json!({"command": "fake.double", "input": {"a": "x"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let j = wait_job(&c, &e, env["data"]["job_id"].as_str().unwrap()).await;
    assert_eq!(j["status"], "error");
    assert_eq!(j["result"]["error"]["code"], "validation");
    // token required
    let r = c.get(format!("{}/api/jobs", e.base)).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn frame_route_serves_see_frame_jpeg() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{}/api/frame?source_id=src_a&at=1.5&w=320&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"\xFF\xD8fakejpeg");
    // command error -> 422; bad ids / times -> 400
    let r = c
        .get(format!("{}/api/frame?source_id=src_b&at=1&w=320&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    for q in ["source_id=..%2Fx&at=1", "source_id=src_a&at=-1", "source_id=src_a&at=abc", "at=1"] {
        let r = c.get(format!("{}/api/frame?{q}&t={t}", e.base)).send().await.unwrap();
        assert_eq!(r.status(), 400, "{q}");
    }
}

#[tokio::test]
async fn font_route_serves_only_allowlisted_installed_fonts() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    assert!(rustler_gui::font_names().contains(&"anton"));
    let r = c.get(format!("{}/api/font/anton?t={t}", e.base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "font/ttf");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"ttfbytes");
    // allowlisted but not installed -> 404; unknown name / traversal -> 404
    for n in ["poppins-extrabold", "nope", "..%2F..%2Fsecret"] {
        let r = c.get(format!("{}/api/font/{n}?t={t}", e.base)).send().await.unwrap();
        assert_eq!(r.status(), 404, "{n}");
    }
}

#[tokio::test]
async fn clip_state_reports_revision() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    let v: Value = c
        .get(format!("{}/api/clip_state?clip_id=clp_1&t={t}", e.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["clip_id"], "clp_1");
    assert_eq!(v["revision"], 7);
    assert!(v["extras_mtime_ms"].is_null());
    let r = c
        .get(format!("{}/api/clip_state?clip_id=clp_missing&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .get(format!("{}/api/clip_state?clip_id=..%5Cx&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn commands_and_run() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    let list: Value = c
        .get(format!("{}/api/commands?t={t}", e.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.as_array().unwrap().iter().any(|x| x["name"] == "fake.double"));

    let env: Value = c
        .post(format!("{}/api/run?t={t}", e.base))
        .json(&json!({"command": "fake.double", "input": {"a": 21}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    assert_eq!(env["schema"], "rustler.fake.double@1");
    assert_eq!(env["data"]["b"], 42);

    // validation error and unknown command come back as error envelopes
    let env: Value = c
        .post(format!("{}/api/run?t={t}", e.base))
        .json(&json!({"command": "fake.double", "input": {"a": "x"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "validation");
    let env: Value = c
        .post(format!("{}/api/run?t={t}", e.base))
        .json(&json!({"command": "nope.nope"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(env["error"]["code"], "not_found");
}

#[tokio::test]
async fn file_serving_and_events() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    let enc = |p: PathBuf| {
        reqwest::Url::parse_with_params("http://x/?", [("path", p.to_str().unwrap())])
            .unwrap()
            .query()
            .unwrap()
            .to_string()
    };

    // outside allowed roots -> 403
    let q = enc(e.outside.path().join("secret.txt"));
    let r = c
        .get(format!("{}/api/file?{q}&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // traversal out of the project via .. -> 403
    let r = c
        .get(format!("{}/api/file?path=..%2F..%2F..&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert!(r.status() == 403 || r.status() == 404);

    // inside, with Range -> 206
    let r = c
        .get(format!("{}/api/file?path=data.bin&t={t}", e.base))
        .header("range", "bytes=10-19")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(
        r.headers().get("content-range").unwrap().to_str().unwrap(),
        "bytes 10-19/256"
    );
    assert_eq!(r.bytes().await.unwrap().to_vec(), (10u8..=19).collect::<Vec<_>>());

    // open-ended + suffix ranges, content type, full body
    let r = c
        .get(format!("{}/api/file?path=clip.mp4&t={t}", e.base))
        .header("range", "bytes=990-")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.headers().get("content-type").unwrap(), "video/mp4");
    assert_eq!(r.bytes().await.unwrap().len(), 10);
    let r = c
        .get(format!("{}/api/file?path=clip.mp4&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers().get("accept-ranges").unwrap(), "bytes");
    assert_eq!(r.bytes().await.unwrap().len(), 1000);
    let r = c
        .get(format!("{}/api/file?path=clip.mp4&t={t}", e.base))
        .header("range", "bytes=5000-")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 416);

    // optional=1: missing -> 204 (not 404); present -> normal; outside roots still 403
    let r = c
        .get(format!("{}/api/file?path=analysis%2Fnope.json&optional=1&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = c
        .get(format!("{}/api/file?path=clip.mp4&optional=1&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let q = enc(e.outside.path().join("secret.txt"));
    let r = c
        .get(format!("{}/api/file?{q}&optional=1&t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // events: bad lines skipped
    let ev: Value = c
        .get(format!("{}/api/events?t={t}", e.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ev.as_array().unwrap().len(), 1);
    assert_eq!(ev[0]["kind"], "x");
}

/// Library audio (sfx .ogg, music) under RUSTLER_HOME/assets is playable in an <audio> element on
/// the Assets page: served from the home root with an audio content type and Range support.
#[tokio::test]
async fn library_audio_served_with_audio_types() {
    let e = setup().await;
    let t = &e.gui.token;
    let c = reqwest::Client::new();
    let dir = e._home.path().join("assets").join("sfx");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, ct) in [("hit.ogg", "audio/ogg"), ("bed.flac", "audio/flac"), ("bed.mp3", "audio/mpeg"), ("f.ttf", "font/ttf")] {
        let p = dir.join(name);
        std::fs::write(&p, vec![1u8; 64]).unwrap();
        let q = reqwest::Url::parse_with_params("http://x/?", [("path", p.to_str().unwrap())])
            .unwrap()
            .query()
            .unwrap()
            .to_string();
        let r = c
            .get(format!("{}/api/file?{q}&t={t}", e.base))
            .header("range", "bytes=0-9")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 206, "{name}");
        assert_eq!(r.headers().get("content-type").unwrap(), ct, "{name}");
        assert_eq!(r.bytes().await.unwrap().len(), 10);
    }
}
