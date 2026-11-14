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

    let mut reg = Registry::new();
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
async fn page_references_app_js() {
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
    assert!(html.contains("app.js"));
    assert!(html.contains("style.css"));
    assert!(!html.contains("__TOKEN__"));
    let js = c
        .get(format!("{}/app.js?t={t}", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(js.status(), 200);
    assert!(js.text().await.unwrap().contains("/api/run"));
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
