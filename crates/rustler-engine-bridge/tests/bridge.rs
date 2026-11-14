//! Bridge tests against the real Python engine (`uv run --project <repo>/engine ...`).
//! They skip (with a printed reason) when `uv` or the engine sources are not available.

use rustler_core::ErrorCode;
use rustler_engine_bridge::EngineClient;
use rustler_registry::{ProgressEvent, ProgressSink};
use rustler_tools::ToolPaths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn home() -> PathBuf {
    if let Ok(h) = std::env::var("RUSTLER_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default();
    base.join(".rustler")
}

fn engine_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../engine")
}

fn uv_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "uv.exe" } else { "uv" };
    let p = home().join("bin").join(name);
    p.is_file().then_some(p)
}

/// Real engine via `uv run`. `None` (and a printed reason) when it cannot be started.
async fn real_engine() -> Option<EngineClient> {
    let Some(uv) = uv_path() else {
        eprintln!("SKIP: uv not found under {}", home().join("bin").display());
        return None;
    };
    let engine = engine_dir();
    if !engine.join("rustler_engine").is_dir() {
        eprintln!("SKIP: engine sources missing at {}", engine.display());
        return None;
    }
    let args: Vec<String> = [
        "run",
        "--project",
        engine.to_str().unwrap(),
        "python",
        "-m",
        "rustler_engine",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut env = vec![];
    if let Ok(hf) = std::env::var("HF_HOME") {
        env.push(("HF_HOME".to_string(), hf));
    }
    Some(
        EngineClient::spawn_command(&uv, &args, &engine, &env)
            .await
            .expect("spawn engine"),
    )
}

fn noop() -> ProgressSink {
    ProgressSink::noop()
}

#[tokio::test]
async fn ping_roundtrip() {
    let Some(c) = real_engine().await else { return };
    let r: Value = c
        .call("engine.ping", json!({}), &noop(), &CancellationToken::new())
        .await
        .expect("ping");
    assert!(r["version"].is_string(), "{r}");
    assert!(r["python"].as_str().unwrap().starts_with("3."), "{r}");
    c.shutdown().await;
}

#[tokio::test]
async fn unknown_method_is_validation() {
    let Some(c) = real_engine().await else { return };
    let err = c
        .call::<_, Value>("nope", json!({}), &noop(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation, "{err:?}");
    // The engine survives a protocol-level error.
    let r: Value = c
        .call("engine.ping", json!({}), &noop(), &CancellationToken::new())
        .await
        .unwrap();
    assert!(r["version"].is_string());
    c.shutdown().await;
}

#[tokio::test]
async fn translit_to_latin() {
    let Some(c) = real_engine().await else { return };
    let r: Value = c
        .call(
            "translit.to_latin",
            json!({"texts": ["क्या scene है"]}),
            &noop(),
            &CancellationToken::new(),
        )
        .await
        .expect("translit");
    assert_eq!(r["texts"][0], "kya scene hai", "{r}");
    c.shutdown().await;
}

#[tokio::test]
async fn concurrent_calls_are_routed_by_id() {
    let Some(c) = real_engine().await else { return };
    let c = Arc::new(c);
    let mut tasks = vec![];
    for i in 0..6 {
        let c = c.clone();
        tasks.push(tokio::spawn(async move {
            let text = ["है", "नहीं", "भाई", "क्या", "मैं", "plain"][i];
            let r: Value = c
                .call(
                    "translit.to_latin",
                    json!({"texts": [text]}),
                    &ProgressSink::noop(),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            (text, r["texts"][0].as_str().unwrap().to_string())
        }));
    }
    let want = ["hai", "nahi", "bhai", "kya", "main", "plain"];
    for (i, t) in tasks.into_iter().enumerate() {
        let (text, got) = t.await.unwrap();
        assert_eq!(got, want[i], "text {text}");
    }
}

fn jfk() -> Option<PathBuf> {
    let p = home().join("test-media").join("jfk.wav");
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: {} missing", p.display());
        None
    }
}

#[tokio::test]
async fn transcribe_reports_progress() {
    let Some(wav) = jfk() else { return };
    let Some(c) = real_engine().await else { return };
    let parts = tempfile::tempdir().unwrap();
    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::default();
    let ev = events.clone();
    let sink = ProgressSink::new(move |e| ev.lock().unwrap().push(e));
    let r: Value = c
        .call(
            "transcribe.run",
            json!({
                "audio_path": wav,
                "model": "tiny",
                "language": "en",
                "device": "cpu",
                "parts_dir": parts.path(),
            }),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .expect("transcribe");
    assert_eq!(r["complete"], true, "{r}");
    let words = r["words"].as_array().unwrap();
    assert!(
        words
            .iter()
            .any(|w| w["text"].as_str().unwrap_or("").contains("Americans")),
        "{r}"
    );
    {
        let ev = events.lock().unwrap();
        assert!(!ev.is_empty(), "no progress events received");
        assert!(ev.iter().all(|e| e.stage == "transcribe"), "{ev:?}");
        let last = ev.last().unwrap().pct.expect("pct");
        assert!((last - 1.0).abs() < 1e-6, "pct is 0..1, last = {last}");
    }
    c.shutdown().await;
}

#[tokio::test]
async fn cancel_mid_transcribe_kills_engine() {
    let Some(wav) = jfk() else { return };
    let Some(c) = real_engine().await else { return };
    let parts = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        c2.cancel();
    });
    let err = c
        .call::<_, Value>(
            "transcribe.run",
            json!({"audio_path": wav, "model": "tiny", "device": "cpu", "parts_dir": parts.path()}),
            &noop(),
            &cancel,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err:?}");
    // The child is gone shortly after.
    let mut dead = false;
    for _ in 0..50 {
        if !c.is_alive().await {
            dead = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(dead, "engine process still alive after cancel");
    // And the dead client fails fast, retryable.
    let err = c
        .call::<_, Value>("engine.ping", json!({}), &noop(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal);
    assert!(err.retryable);
}

#[tokio::test]
async fn killing_child_mid_call_is_internal_retryable() {
    let Some(wav) = jfk() else { return };
    let Some(c) = real_engine().await else { return };
    let c = Arc::new(c);
    let parts = tempfile::tempdir().unwrap();
    let killer = c.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        killer.kill().await;
    });
    let err = c
        .call::<_, Value>(
            "transcribe.run",
            json!({"audio_path": wav, "model": "tiny", "device": "cpu", "parts_dir": parts.path()}),
            &noop(),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal, "{err:?}");
    assert!(err.retryable, "{err:?}");
    assert!(err.message.contains("engine"), "{err:?}");
}

/// A fake "engine" that writes 25 lines to stderr and exits: the crash message keeps the last 20.
#[tokio::test]
async fn crash_message_has_last_20_stderr_lines() {
    #[cfg(windows)]
    let (prog, args) = (
        PathBuf::from("powershell"),
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "1..25 | ForEach-Object { [Console]::Error.WriteLine('line' + $_) }; exit 3"
                .to_string(),
        ],
    );
    #[cfg(unix)]
    let (prog, args) = (
        PathBuf::from("sh"),
        vec![
            "-c".to_string(),
            "i=1; while [ $i -le 25 ]; do echo line$i >&2; i=$((i+1)); done; exit 3".to_string(),
        ],
    );
    let cwd = std::env::temp_dir();
    let c = EngineClient::spawn_command(&prog, &args, &cwd, &[])
        .await
        .expect("spawn fake");
    let err = c
        .call::<_, Value>("engine.ping", json!({}), &noop(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal, "{err:?}");
    assert!(err.retryable);
    assert!(err.message.contains("engine crashed"), "{}", err.message);
    assert!(err.message.contains("line25"), "{}", err.message);
    let n = err
        .message
        .lines()
        .filter(|l| l.starts_with("line"))
        .count();
    assert_eq!(n, 20, "{}", err.message);
    assert!(
        !err.message.lines().any(|l| l.trim() == "line5"),
        "{}",
        err.message
    );
    assert_eq!(c.stderr_tail().len(), 20);
}

#[tokio::test]
async fn spawn_without_venv_is_dependency_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().to_path_buf();
    let paths = ToolPaths {
        home: h.clone(),
        bin: h.join("bin"),
        ffmpeg: h.join("bin/ffmpeg"),
        ffprobe: h.join("bin/ffprobe"),
        ytdlp: h.join("bin/yt-dlp"),
        uv: h.join("bin/uv"),
        engine_dir: h.join("engine"),
        venv_python: h.join("engine/.venv/Scripts/python.exe"),
        models: h.join("models"),
        fonts: h.join("fonts"),
    };
    let err = EngineClient::spawn(&paths).await.err().expect("must fail");
    assert_eq!(err.code, ErrorCode::DependencyMissing);
    assert!(err.hint.unwrap_or_default().contains("rustler setup"));
}
