use assert_cmd::Command;
use rustler_cli::{CliOptions, run_cli, run_cli_with};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------
// Real binary (assert_cmd)
// ---------------------------------------------------------------------------------------

fn bin() -> Command {
    Command::cargo_bin("rustler").unwrap()
}

fn stdout_json(out: &std::process::Output) -> Value {
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("stdout is not one JSON value ({e}): {s:?}"))
}

#[test]
fn commands_list_ok() {
    let out = bin().args(["commands", "list"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = stdout_json(&out);
    assert_eq!(v["ok"], true);
    assert_eq!(v["schema"], "rustler.commands.list@1");
    assert!(v["data"]["commands"].is_array());
    // Compact by default: exactly one line.
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end().lines().count(), 1);
}

#[test]
fn unknown_command_is_not_found_exit_3() {
    let out = bin().args(["nope", "run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v = stdout_json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "not_found");
    assert!(v["error"]["hint"].as_str().unwrap().contains("rustler commands list"));
    // stderr must not carry a JSON envelope.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("\"ok\""), "stderr leaked an envelope: {err}");
}

#[test]
fn version_prints_json() {
    let out = bin().arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = stdout_json(&out);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn help_has_start_text_and_pretty_flag() {
    let out = bin().arg("help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = stdout_json(&out);
    assert!(v["data"]["start"].as_str().unwrap().starts_with("rustler doctor"));

    let out = bin().args(["help", "--pretty"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.trim_end().lines().count() > 5, "--pretty should be multi-line");
    serde_json::from_str::<Value>(&s).unwrap();
}

#[test]
fn no_args_prints_help_exit_0() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout_json(&out)["ok"], true);
}

#[test]
fn reserved_builtins_are_not_built_yet() {
    for args in [vec!["mcp", "serve"], vec!["gui"]] {
        let out = bin().args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(3), "{args:?}");
        let v = stdout_json(&out);
        assert!(v["error"]["message"].as_str().unwrap().contains("not built yet"));
    }
}

#[test]
fn bad_global_flag_values_are_usage_errors() {
    let out = bin().args(["help", "--actor", "robot"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout_json(&out)["error"]["code"], "usage");
    let out = bin().args(["help", "--max-seconds", "soon"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

// ---------------------------------------------------------------------------------------
// In-process with a fake registry (run_cli)
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct EchoIn {
    name: Option<String>,
    sections: Option<Vec<String>>,
    #[serde(default)]
    loud: bool,
    start_unit: Option<u32>,
    max_seconds: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
struct CtxOut {
    project: Option<String>,
    actor: String,
    dry_run: bool,
    yes: bool,
    max_seconds: Option<u64>,
    input: Value,
}

#[derive(Deserialize, JsonSchema)]
struct IdIn {
    source_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct NoIn {}

#[derive(Serialize, JsonSchema)]
struct Empty {}

fn ro() -> Annotations {
    Annotations { read_only: true, ..Annotations::default() }
}

fn fake_registry() -> Registry {
    let mut reg = Registry::new();
    reg.register(typed(
        "test.echo",
        "Echo the input and the context",
        &["name"],
        ro(),
        vec![Example { description: "echo", cli: "rustler test echo hi --loud" }],
        |ctx: Ctx, i: EchoIn| async move {
            Ok(CtxOut {
                project: ctx.project_dir.map(|p| p.display().to_string()),
                actor: format!("{:?}", ctx.actor),
                dry_run: ctx.dry_run,
                yes: ctx.yes,
                max_seconds: ctx.max_seconds,
                input: json!({
                    "name": i.name, "sections": i.sections, "loud": i.loud,
                    "start_unit": i.start_unit, "max_seconds": i.max_seconds
                }),
            })
        },
    ));
    reg.register(typed(
        "source.import",
        "Import a source",
        &["source_id"],
        Annotations::default(),
        vec![],
        |_c: Ctx, i: IdIn| async move { Ok(json!({"imported": i.source_id, "warnings": ["careful"]})) },
    ));
    reg.register(typed("source.list", "List sources", &[], ro(), vec![], |_c: Ctx, _i: NoIn| async move {
        Ok(Empty {})
    }));
    reg.register(typed(
        "source.fetch_transcript",
        "Fetch a transcript",
        &["source_id"],
        Annotations::default(),
        vec![],
        |_c: Ctx, i: IdIn| async move { Ok(json!({"fetched": i.source_id})) },
    ));
    reg.register(typed("doctor.run", "Check the install", &[], ro(), vec![], |_c: Ctx, _i: NoIn| async move {
        Ok(json!({"healthy": true}))
    }));
    reg.register(typed("test.fail", "Always fails with a media error", &[], ro(), vec![], |_c: Ctx, _i: NoIn| async move {
        Err::<Empty, _>(RustlerError::new(ErrorCode::Media, "test", "ffmpeg exploded").hint("try again"))
    }));
    reg.register(typed("test.sleep", "Sleeps until cancelled", &[], ro(), vec![], |ctx: Ctx, _i: NoIn| async move {
        tokio::select! {
            _ = ctx.cancel.cancelled() => Err::<Empty, _>(RustlerError::new(ErrorCode::Cancelled, "test", "stopped")),
            _ = tokio::time::sleep(Duration::from_secs(30)) => Ok(Empty {}),
        }
    }));
    reg.register(typed("test.stubborn", "Ignores cancellation", &[], ro(), vec![], |_c: Ctx, _i: NoIn| async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(Empty {})
    }));
    reg
}

async fn run(args: &[&str]) -> (Value, i32, String) {
    let (out, code) = run_cli(args.iter().map(|s| s.to_string()).collect(), fake_registry()).await;
    let v = serde_json::from_str(&out).unwrap_or_else(|e| panic!("not JSON ({e}): {out:?}"));
    (v, code, out)
}

fn names(v: &Value) -> Vec<String> {
    v["data"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn run_command_end_to_end_with_warnings_moved() {
    let (v, code, out) = run(&["source", "import", "src_1"]).await;
    assert_eq!(code, 0);
    assert_eq!(v["schema"], "rustler.source.import@1");
    assert_eq!(v["data"], json!({"imported": "src_1"}));
    assert_eq!(v["warnings"], json!(["careful"]));
    assert!(!out.contains('\n'));
}

#[tokio::test]
async fn multiword_action_uses_dash() {
    let (v, code, _) = run(&["source", "fetch-transcript", "src_9"]).await;
    assert_eq!(code, 0);
    assert_eq!(v["schema"], "rustler.source.fetch_transcript@1");
    assert_eq!(v["data"]["fetched"], "src_9");
}

#[tokio::test]
async fn group_alone_and_group_help_list_the_group() {
    for args in [vec!["source"], vec!["source", "--help"], vec!["--help", "source"]] {
        let (v, code, _) = run(&args).await;
        assert_eq!(code, 0, "{args:?}");
        assert_eq!(v["schema"], "rustler.commands.list@1");
        assert_eq!(names(&v), vec!["source.fetch_transcript", "source.import", "source.list"]);
        assert_eq!(v["data"]["commands"][0]["cli"], "rustler source fetch-transcript");
    }
}

#[tokio::test]
async fn action_help_equals_describe() {
    let (a, ca, _) = run(&["test", "echo", "--help"]).await;
    let (b, cb, _) = run(&["commands", "describe", "test.echo"]).await;
    let (c, _, _) = run(&["commands", "describe", "test", "echo"]).await;
    assert_eq!((ca, cb), (0, 0));
    assert_eq!(a["data"], b["data"]);
    assert_eq!(b["data"], c["data"]);
    assert_eq!(a["data"]["name"], "test.echo");
    assert!(a["data"]["input_schema"]["properties"]["name"].is_object());
    assert_eq!(a["data"]["examples"][0]["cli"], "rustler test echo hi --loud");
    let flags = a["data"]["flags"].as_array().unwrap();
    assert!(flags.iter().any(|f| f["flag"] == "--start-unit" && f["type"] == "integer"));
    assert!(a["data"]["usage"].as_str().unwrap().starts_with("rustler test echo <name>"));
}

#[tokio::test]
async fn commands_list_with_group_filter() {
    let (v, _, _) = run(&["commands", "list", "test"]).await;
    assert!(names(&v).iter().all(|n| n.starts_with("test.")));
    let (all, _, _) = run(&["commands", "list"]).await;
    assert!(names(&all).len() > names(&v).len());
}

#[tokio::test]
async fn describe_unknown_is_not_found_with_suggestions() {
    let (v, code, _) = run(&["commands", "describe", "source.imprt"]).await;
    assert_eq!(code, 3);
    assert!(v["error"]["hint"].as_str().unwrap().contains("source.import"));
}

#[tokio::test]
async fn unknown_command_hint_lists_three_closest() {
    let (v, code, _) = run(&["source", "imprt"]).await;
    assert_eq!(code, 3);
    assert_eq!(v["error"]["code"], "not_found");
    let hint = v["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("source.import"), "{hint}");
    let listed = ["source.import", "source.list", "source.fetch_transcript", "test.echo", "doctor.run"]
        .iter()
        .filter(|n| hint.contains(*n))
        .count();
    assert!(listed >= 3, "{hint}");
    assert!(hint.contains("rustler source import"), "{hint}");

    let (v, code, _) = run(&["sourse", "list"]).await;
    assert_eq!(code, 3);
    assert!(v["error"]["hint"].as_str().unwrap().contains("source.list"));
}

#[tokio::test]
async fn sole_run_group_runs_directly() {
    let (v, code, _) = run(&["doctor"]).await;
    assert_eq!(code, 0);
    assert_eq!(v["schema"], "rustler.doctor.run@1");
    assert_eq!(v["data"]["healthy"], true);
    // ...but --help still documents it.
    let (v, _, _) = run(&["doctor", "--help"]).await;
    assert_eq!(v["schema"], "rustler.commands.list@1");
}

#[tokio::test]
async fn flags_are_typed_by_schema_and_bools_do_not_swallow_positionals() {
    let (v, code, _) = run(&[
        "test", "echo", "--loud", "123", "--start-unit", "12", "--sections", "1:00-2:00",
    ])
    .await;
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["input"],
        json!({"name":"123","sections":["1:00-2:00"],"loud":true,"start_unit":12,"max_seconds":null})
    );
}

#[tokio::test]
async fn equals_syntax_and_input_json() {
    let (v, code, _) = run(&["test", "echo", "--start-unit=4", "--input-json", r#"{"name":"x","start_unit":1}"#]).await;
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["input"]["name"], "x");
    assert_eq!(v["data"]["input"]["start_unit"], 4);
}

#[tokio::test]
async fn globals_reach_ctx_anywhere() {
    let (v, code, _) = run(&[
        "--dry-run", "test", "--actor", "human", "echo", "-q", "--project", "some/dir", "--yes", "--max-seconds", "90",
    ])
    .await;
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["project"], "some/dir");
    assert_eq!(d["actor"], "Human");
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["yes"], true);
    assert_eq!(d["max_seconds"], 90);
    // --max-seconds also feeds a same-named input field.
    assert_eq!(d["input"]["max_seconds"], 90);
}

#[tokio::test]
async fn typo_flag_is_usage_error_with_suggestion() {
    let (v, code, _) = run(&["test", "echo", "--start-unti", "12"]).await;
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
    assert!(v["error"]["hint"].as_str().unwrap().contains("--start-unit"));
}

#[tokio::test]
async fn missing_flag_value_and_extra_positional_are_usage_errors() {
    let (v, code, _) = run(&["test", "echo", "--start-unit"]).await;
    assert_eq!(code, 2);
    assert!(v["error"]["message"].as_str().unwrap().contains("needs a value"));

    let (_, code, _) = run(&["source", "import", "a", "b"]).await;
    assert_eq!(code, 2);
}

#[tokio::test]
async fn validation_and_command_errors_map_to_exit_codes() {
    let (v, code, _) = run(&["test", "echo", "--start-unit", "abc"]).await;
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "validation");

    let (v, code, _) = run(&["test", "fail"]).await;
    assert_eq!(code, 6);
    assert_eq!(v["error"]["code"], "media");
    assert_eq!(v["error"]["hint"], "try again");
}

#[tokio::test]
async fn version_and_help_in_process() {
    let (v, code, _) = run(&["--version"]).await;
    assert_eq!((code, v["version"].as_str().is_some()), (0, true));
    let (v, _, _) = run(&[]).await;
    assert!(v["data"]["commands"].as_array().unwrap().len() >= 5);
    assert!(v["data"]["exit_codes"].is_object());
}

#[tokio::test]
async fn output_formats_pretty_tty_json() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let (compact, _) = run_cli(args(&["source", "list"]), fake_registry()).await;
    assert!(!compact.contains('\n'));
    let (pretty, _) = run_cli(args(&["source", "list", "--pretty"]), fake_registry()).await;
    assert!(pretty.contains('\n'));
    assert_eq!(
        serde_json::from_str::<Value>(&pretty).unwrap(),
        serde_json::from_str::<Value>(&compact).unwrap()
    );

    let tty = CliOptions { tty: true, ..CliOptions::default() };
    let (t, _) = run_cli_with(args(&["source", "list"]), fake_registry(), tty.clone()).await;
    assert!(t.contains('\n'), "tty output should be pretty JSON");
    let (t, _) = run_cli_with(args(&["source", "list", "--json"]), fake_registry(), tty).await;
    assert!(!t.contains('\n'), "--json forces compact on a tty");
}

#[tokio::test]
async fn cancellation_maps_to_exit_8() {
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        c.cancel();
    });
    let opts = CliOptions { cancel, ..CliOptions::default() };
    let (out, code) = run_cli_with(vec!["test".into(), "sleep".into()], fake_registry(), opts).await;
    assert_eq!(code, 8);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "cancelled");
}

#[tokio::test]
async fn cancellation_of_a_command_that_ignores_the_token_still_exits_8() {
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.cancel();
    });
    let opts = CliOptions { cancel, grace: Duration::from_millis(200), ..CliOptions::default() };
    let (out, code) = run_cli_with(vec!["test".into(), "stubborn".into()], fake_registry(), opts).await;
    assert_eq!(code, 8);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "cancelled");
    assert_eq!(v["error"]["retryable"], true);
}

#[tokio::test]
async fn reserved_mcp_and_gui() {
    let (v, code, _) = run(&["mcp", "serve"]).await;
    assert_eq!(code, 3);
    assert!(v["error"]["message"].as_str().unwrap().contains("not built yet"));
    let (_, code, _) = run(&["gui"]).await;
    assert_eq!(code, 3);
}
