use rustler_core::{ErrorCode, RustlerError};
use rustler_mcp::serve;
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf};

#[derive(Deserialize, JsonSchema)]
struct EchoIn {
    text: String,
}
#[derive(Serialize, JsonSchema)]
struct EchoOut {
    echoed: String,
    project: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
struct NoIn {}
#[derive(Serialize, JsonSchema)]
struct ImgOut {
    image: String,
}

fn registry(image_path: String) -> Registry {
    let mut reg = Registry::new();
    reg.register(typed(
        "test.echo",
        "Echo the text back",
        &["text"],
        Annotations {
            read_only: true,
            network: true,
            ..Default::default()
        },
        vec![Example {
            description: "say hi",
            cli: "rustler test echo hi",
        }],
        |ctx: Ctx, i: EchoIn| async move {
            Ok(EchoOut {
                echoed: i.text,
                project: ctx.project_dir.map(|p| p.display().to_string()),
            })
        },
    ));
    reg.register(typed(
        "test.image",
        "Return an image path",
        &[],
        Annotations::default(),
        vec![],
        move |_ctx: Ctx, _i: NoIn| {
            let p = image_path.clone();
            async move { Ok(ImgOut { image: p }) }
        },
    ));
    reg.register(typed(
        "test.fail",
        "Always fails",
        &[],
        Annotations::default(),
        vec![],
        |_ctx: Ctx, _i: NoIn| async move {
            Err::<EchoOut, _>(
                RustlerError::new(ErrorCode::Media, "test", "boom").hint("try again"),
            )
        },
    ));
    reg.register(typed(
        "test.progress",
        "Emits progress",
        &[],
        Annotations::default(),
        vec![],
        |ctx: Ctx, _i: NoIn| async move {
            for k in 1..=3 {
                ctx.progress.emit(ProgressEvent {
                    stage: "work".into(),
                    pct: Some(k as f32 / 3.0),
                    message: None,
                    eta_s: None,
                });
            }
            Ok(EchoOut {
                echoed: "done".into(),
                project: None,
            })
        },
    ));
    reg
}

struct Client {
    w: WriteHalf<DuplexStream>,
    r: Lines<BufReader<ReadHalf<DuplexStream>>>,
}

impl Client {
    fn start(reg: Registry) -> Client {
        let (c, s) = tokio::io::duplex(1 << 20);
        let (sr, sw) = tokio::io::split(s);
        let (cr, cw) = tokio::io::split(c);
        let ctx = Ctx::new(None, std::env::temp_dir());
        tokio::spawn(async move {
            serve(Arc::new(reg), ctx, BufReader::new(sr), sw).await.unwrap();
        });
        Client {
            w: cw,
            r: BufReader::new(cr).lines(),
        }
    }
    async fn send(&mut self, v: Value) {
        self.w
            .write_all(format!("{v}\n").as_bytes())
            .await
            .unwrap();
    }
    async fn recv(&mut self) -> Value {
        let l = tokio::time::timeout(std::time::Duration::from_secs(20), self.r.next_line())
            .await
            .expect("timeout")
            .unwrap()
            .expect("eof");
        serde_json::from_str(&l).expect("server wrote non-JSON to the protocol stream")
    }
    async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
        loop {
            let m = self.recv().await;
            if m.get("id") == Some(&json!(id)) {
                return m;
            }
        }
    }
    async fn init(&mut self) -> Value {
        let r = self
            .request(
                1,
                "initialize",
                json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
            )
            .await;
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        r
    }
}

fn write_png(dir: &std::path::Path, name: &str, w: u32, h: u32, noise: bool) -> PathBuf {
    let mut seed = 12345u32;
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        if noise {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
        } else {
            image::Rgb([(x % 255) as u8, (y % 255) as u8, 80])
        }
    });
    let p = dir.join(name);
    img.save(&p).unwrap();
    p
}

#[tokio::test]
async fn full_session() {
    let tmp = tempfile::tempdir().unwrap();
    let png = write_png(tmp.path(), "frame.png", 32, 32, false);
    let mut c = Client::start(registry(png.display().to_string()));

    // initialize
    let r = c.init().await;
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "rustler");
    assert!(r["result"]["capabilities"]["tools"].is_object());

    // unknown version negotiates to our newest
    let r = c
        .request(2, "initialize", json!({"protocolVersion":"1999-01-01"}))
        .await;
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");

    // ping
    let r = c.request(3, "ping", json!({})).await;
    assert_eq!(r["result"], json!({}));

    // tools/list
    let r = c.request(4, "tools/list", json!({})).await;
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"rustler_help"));
    assert!(names.contains(&"test_echo"));
    let echo = tools.iter().find(|t| t["name"] == "test_echo").unwrap();
    eprintln!("{}", serde_json::to_string_pretty(echo).unwrap());
    assert_eq!(echo["inputSchema"]["type"], "object");
    assert!(echo["inputSchema"]["properties"]["text"].is_object());
    assert!(echo["inputSchema"]["properties"]["project"].is_object());
    assert_eq!(echo["outputSchema"]["type"], "object");
    assert_eq!(echo["annotations"]["readOnlyHint"], true);
    assert_eq!(echo["annotations"]["destructiveHint"], false);
    assert_eq!(echo["annotations"]["openWorldHint"], true);
    assert_eq!(echo["annotations"]["idempotentHint"], false);
    let d = echo["description"].as_str().unwrap();
    assert!(d.starts_with("Echo the text back"));
    assert!(d.contains("rustler test echo hi"));

    // tools/call with structuredContent + project override
    let proj = tmp.path().join("proj");
    let r = c
        .request(
            5,
            "tools/call",
            json!({"name":"test_echo","arguments":{"text":"hello","project":proj.display().to_string()}}),
        )
        .await;
    let res = &r["result"];
    assert_eq!(res["isError"], false);
    assert_eq!(res["structuredContent"]["echoed"], "hello");
    assert_eq!(
        res["structuredContent"]["project"],
        proj.display().to_string()
    );
    assert_eq!(res["content"][0]["type"], "text");
    let txt: Value = serde_json::from_str(res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(txt["echoed"], "hello");

    // image tool
    let r = c
        .request(6, "tools/call", json!({"name":"test_image","arguments":{}}))
        .await;
    let content = r["result"]["content"].as_array().unwrap();
    let img = content.iter().find(|i| i["type"] == "image").expect("image item");
    assert_eq!(img["mimeType"], "image/png");
    assert!(img["data"].as_str().unwrap().len() > 20);
    assert_eq!(r["result"]["structuredContent"]["image"], png.display().to_string());

    // command error -> isError with RustlerError JSON
    let r = c
        .request(7, "tools/call", json!({"name":"test_fail"}))
        .await;
    assert_eq!(r["result"]["isError"], true);
    let e: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(e["code"], "media");
    assert_eq!(e["message"], "boom");
    assert_eq!(e["hint"], "try again");

    // validation error from bad arguments
    let r = c
        .request(8, "tools/call", json!({"name":"test_echo","arguments":{"nope":1}}))
        .await;
    assert_eq!(r["result"]["isError"], true);
    let e: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(e["code"], "validation");

    // unknown tool
    let r = c
        .request(9, "tools/call", json!({"name":"nope_nothing"}))
        .await;
    assert_eq!(r["result"]["isError"], true);
    let e: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(e["code"], "not_found");

    // help tool
    let r = c
        .request(10, "tools/call", json!({"name":"rustler_help"}))
        .await;
    assert_eq!(r["result"]["isError"], false);
    assert!(r["result"]["structuredContent"]["start_here"].is_array());

    // unknown method + garbage line don't kill the server
    let r = c.request(11, "bogus/method", json!({})).await;
    assert_eq!(r["error"]["code"], -32601);
    c.w.write_all(b"this is not json\n").await.unwrap();
    let m = c.recv().await;
    assert_eq!(m["error"]["code"], -32700);
    let r = c.request(12, "ping", json!({})).await;
    assert_eq!(r["result"], json!({}));

    // missing name -> protocol error
    let r = c.request(13, "tools/call", json!({})).await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn progress_notifications_only_with_token() {
    let tmp = tempfile::tempdir().unwrap();
    let png = write_png(tmp.path(), "f.png", 8, 8, false);
    let mut c = Client::start(registry(png.display().to_string()));
    c.init().await;

    c.send(json!({"jsonrpc":"2.0","id":20,"method":"tools/call",
        "params":{"name":"test_progress","_meta":{"progressToken":"tok"}}}))
        .await;
    let mut progress = Vec::new();
    loop {
        let m = c.recv().await;
        if m["method"] == "notifications/progress" {
            assert_eq!(m["params"]["progressToken"], "tok");
            progress.push(m["params"]["progress"].as_f64().unwrap());
        } else if m["id"] == 20 {
            assert_eq!(m["result"]["isError"], false);
            break;
        }
    }
    assert_eq!(progress.len(), 3);
    assert!(progress.windows(2).all(|w| w[1] > w[0]), "{progress:?}");

    // without a token: no notifications precede the response
    c.send(json!({"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"test_progress"}}))
        .await;
    let m = c.recv().await;
    assert_eq!(m["id"], 21);
}

#[tokio::test]
async fn large_images_are_downscaled() {
    let tmp = tempfile::tempdir().unwrap();
    let big = write_png(tmp.path(), "big.png", 1400, 1400, true);
    assert!(std::fs::metadata(&big).unwrap().len() > 1_500_000);
    let mut c = Client::start(registry(big.display().to_string()));
    c.init().await;
    let r = c
        .request(2, "tools/call", json!({"name":"test_image"}))
        .await;
    let img = r["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["type"] == "image")
        .expect("image item");
    assert_eq!(img["mimeType"], "image/jpeg");
    let raw = img["data"].as_str().unwrap().len() / 4 * 3;
    assert!(raw <= 1_500_000, "image too big: {raw}");
}

#[tokio::test]
async fn missing_image_file_is_ignored() {
    let mut c = Client::start(registry("/definitely/not/here.png".into()));
    c.init().await;
    let r = c
        .request(2, "tools/call", json!({"name":"test_image"}))
        .await;
    assert_eq!(r["result"]["isError"], false);
    assert!(
        r["result"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["type"] != "image")
    );
}

#[tokio::test]
async fn server_exits_on_eof() {
    let (c, s) = tokio::io::duplex(1 << 16);
    let (sr, sw) = tokio::io::split(s);
    let reg = Arc::new(registry("x.png".into()));
    let h = tokio::spawn(serve(reg, Ctx::new(None, std::env::temp_dir()), BufReader::new(sr), sw));
    drop(c);
    tokio::time::timeout(std::time::Duration::from_secs(5), h)
        .await
        .expect("server did not exit")
        .unwrap()
        .unwrap();
}
