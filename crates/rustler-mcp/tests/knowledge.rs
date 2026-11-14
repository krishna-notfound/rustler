//! MCP prompts/* and resources/* (knowledge book). Assertions are content-agnostic
//! except for the guaranteed `start-here` guide, so they hold for the real knowledge/.

use rustler_mcp::serve;
use rustler_registry::{Annotations, Ctx, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
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
}

fn registry() -> Registry {
    let mut reg = Registry::new();
    reg.register(typed(
        "test.echo",
        "Echo the text back",
        &["text"],
        Annotations::default(),
        vec![],
        |_ctx: Ctx, i: EchoIn| async move { Ok(EchoOut { echoed: i.text }) },
    ));
    reg
}

struct Client {
    w: WriteHalf<DuplexStream>,
    r: Lines<BufReader<ReadHalf<DuplexStream>>>,
}

impl Client {
    fn start(project: Option<PathBuf>) -> Client {
        let (c, s) = tokio::io::duplex(1 << 20);
        let (sr, sw) = tokio::io::split(s);
        let (cr, cw) = tokio::io::split(c);
        let ctx = Ctx::new(project, std::env::temp_dir());
        tokio::spawn(async move {
            serve(Arc::new(registry()), ctx, BufReader::new(sr), sw).await.unwrap();
        });
        Client {
            w: cw,
            r: BufReader::new(cr).lines(),
        }
    }
    async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        let v = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        loop {
            let l = tokio::time::timeout(std::time::Duration::from_secs(20), self.r.next_line())
                .await
                .expect("timeout")
                .unwrap()
                .expect("eof");
            let m: Value = serde_json::from_str(&l).unwrap();
            if m.get("id") == Some(&json!(id)) {
                return m;
            }
        }
    }
    async fn init(&mut self) -> Value {
        self.request(
            1,
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
        )
        .await
    }
}

#[tokio::test]
async fn initialize_advertises_prompts_resources_and_instructions() {
    let mut c = Client::start(None);
    let r = c.init().await;
    let caps = &r["result"]["capabilities"];
    assert!(caps["tools"].is_object());
    assert!(caps["prompts"].is_object());
    assert!(caps["resources"].is_object());
    let ins = r["result"]["instructions"].as_str().unwrap();
    assert!(ins.contains("rustler://guides/start-here"));
    assert!(ins.contains("rustler_help"));
    assert!(ins.contains("prompts"));
}

#[tokio::test]
async fn prompts_list_and_get() {
    let mut c = Client::start(None);
    c.init().await;
    let r = c.request(2, "prompts/list", json!({})).await;
    let prompts = r["result"]["prompts"].as_array().unwrap();
    assert!(!prompts.is_empty());
    let p = &prompts[0];
    assert!(p["name"].is_string() && p["description"].is_string());
    let name = p["name"].as_str().unwrap().to_string();
    let mut args = Map::new();
    let mut has_required = false;
    for a in p["arguments"].as_array().unwrap() {
        assert!(a["name"].is_string() && a["required"].is_boolean());
        has_required |= a["required"] == true;
        args.insert(a["name"].as_str().unwrap().into(), json!("MARKER-VALUE"));
    }

    let r = c
        .request(3, "prompts/get", json!({"name": name, "arguments": args}))
        .await;
    let msgs = r["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"]["type"], "text");
    assert!(!msgs[0]["content"]["text"].as_str().unwrap().is_empty());

    // unknown prompt, missing required argument, missing name -> invalid params
    let r = c.request(4, "prompts/get", json!({"name": "no-such-prompt"})).await;
    assert_eq!(r["error"]["code"], -32602);
    if has_required {
        let r = c.request(5, "prompts/get", json!({"name": name})).await;
        assert_eq!(r["error"]["code"], -32602);
        assert!(r["error"]["message"].as_str().unwrap().contains("required"));
    }
    let r = c.request(6, "prompts/get", json!({})).await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn resources_list_read_and_templates() {
    let mut c = Client::start(None);
    c.init().await;

    let r = c.request(2, "resources/list", json!({})).await;
    let res = r["result"]["resources"].as_array().unwrap();
    let start = res
        .iter()
        .find(|x| x["uri"] == "rustler://guides/start-here")
        .expect("start-here guide resource");
    assert_eq!(start["mimeType"], "text/markdown");
    assert!(start["name"].is_string() && start["title"].is_string() && start["description"].is_string());
    let cmds = res.iter().find(|x| x["uri"] == "rustler://commands").expect("commands resource");
    assert_eq!(cmds["mimeType"], "application/json");
    assert!(!res.iter().any(|x| x["uri"] == "rustler://project/state"));

    let r = c
        .request(3, "resources/read", json!({"uri": "rustler://guides/start-here"}))
        .await;
    let content = &r["result"]["contents"][0];
    assert_eq!(content["uri"], "rustler://guides/start-here");
    assert_eq!(content["mimeType"], "text/markdown");
    assert!(!content["text"].as_str().unwrap().is_empty());

    let r = c.request(4, "resources/read", json!({"uri": "rustler://commands"})).await;
    let content = &r["result"]["contents"][0];
    assert_eq!(content["mimeType"], "application/json");
    let specs: Value = serde_json::from_str(content["text"].as_str().unwrap()).unwrap();
    let echo = specs
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "test.echo")
        .expect("test.echo spec");
    assert!(echo["input_schema"]["properties"]["text"].is_object());

    for (id, uri) in [
        (5, "rustler://guides/does-not-exist"),
        (6, "rustler://nothing"),
        (7, "rustler://project/state"),
    ] {
        let r = c.request(id, "resources/read", json!({"uri": uri})).await;
        assert_eq!(r["error"]["code"], -32002, "{uri}");
    }
    let r = c.request(8, "resources/read", json!({})).await;
    assert_eq!(r["error"]["code"], -32602);

    let r = c.request(9, "resources/templates/list", json!({})).await;
    let t = &r["result"]["resourceTemplates"][0];
    assert_eq!(t["uriTemplate"], "rustler://guides/{id}");
    assert_eq!(t["mimeType"], "text/markdown");
}

#[tokio::test]
async fn project_state_resource_when_project_dir_set() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("proj");
    std::fs::create_dir_all(proj.join("clips").join("clip_a")).unwrap();
    std::fs::write(
        proj.join("project.json"),
        json!({"schema":"rustler.project@1","id":"prj_1","name":"demo","revision":3}).to_string(),
    )
    .unwrap();
    std::fs::write(
        proj.join("clips/clip_a/clip.json"),
        json!({"id":"clip_a","name":"first","revision":2,"segments":[{}, {}],"renders":[{"id":"rnd_1"}]})
            .to_string(),
    )
    .unwrap();

    let mut c = Client::start(Some(proj));
    c.init().await;

    let r = c.request(2, "resources/list", json!({})).await;
    let res = r["result"]["resources"].as_array().unwrap();
    let ps = res
        .iter()
        .find(|x| x["uri"] == "rustler://project/state")
        .expect("project state resource");
    assert_eq!(ps["mimeType"], "application/json");

    let r = c
        .request(3, "resources/read", json!({"uri": "rustler://project/state"}))
        .await;
    let state: Value =
        serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(state["project"]["name"], "demo");
    assert_eq!(state["project"]["revision"], 3);
    let clips = state["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["id"], "clip_a");
    assert_eq!(clips[0]["segments"], 2);
    assert_eq!(clips[0]["renders"], 1);
}

#[tokio::test]
async fn help_tool_lists_guides_and_prompts() {
    let mut c = Client::start(None);
    c.init().await;
    let r = c
        .request(2, "tools/call", json!({"name": "rustler_help"}))
        .await;
    let out = &r["result"]["structuredContent"];
    assert_eq!(out["read_first"], "rustler://guides/start-here");
    let guides = out["guides"].as_array().unwrap();
    assert!(guides.iter().any(|g| g["id"] == "start-here"));
    assert!(!out["prompts"].as_array().unwrap().is_empty());
    assert!(out["prompts"][0]["name"].is_string());
}
