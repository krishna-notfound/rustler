use rustler_core::ErrorCode;
use rustler_tools::{Tool, ToolPaths, run};
use std::ffi::OsString;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn exe(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

#[test]
fn paths_respect_rustler_home() {
    let tmp = tempfile::tempdir().unwrap();
    // Single test touching the env var in this binary (no races within this file).
    unsafe { std::env::set_var("RUSTLER_HOME", tmp.path()) };
    assert_eq!(ToolPaths::default_home(), tmp.path());
    let p = ToolPaths::from_home(&ToolPaths::default_home());
    assert_eq!(p.ffmpeg, tmp.path().join("bin").join(exe("ffmpeg")));
    assert_eq!(p.ffprobe, tmp.path().join("bin").join(exe("ffprobe")));
    assert_eq!(p.ytdlp, tmp.path().join("bin").join(exe("yt-dlp")));
    assert_eq!(p.uv, tmp.path().join("bin").join(exe("uv")));
    assert_eq!(p.engine_dir, tmp.path().join("engine"));
    assert!(p.venv_python.starts_with(tmp.path().join("engine").join(".venv")));
    assert_eq!(p.models, tmp.path().join("models"));
    assert_eq!(p.fonts, tmp.path().join("fonts"));
    unsafe { std::env::remove_var("RUSTLER_HOME") };
    assert!(ToolPaths::default_home().ends_with(".rustler"));
}

#[test]
fn require_missing_is_dependency_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let p = ToolPaths::from_home(tmp.path());
    for t in [Tool::Ffmpeg, Tool::Ffprobe, Tool::Ytdlp, Tool::Uv, Tool::EnginePython] {
        let e = p.require(t).unwrap_err();
        assert_eq!(e.code, ErrorCode::DependencyMissing);
        assert!(e.hint.unwrap().contains("rustler setup"));
    }
}

#[test]
fn require_existing_returns_path() {
    let tmp = tempfile::tempdir().unwrap();
    let p = ToolPaths::from_home(tmp.path());
    std::fs::create_dir_all(&p.bin).unwrap();
    std::fs::write(&p.ffmpeg, b"x").unwrap();
    assert_eq!(p.require(Tool::Ffmpeg).unwrap(), p.ffmpeg);
}

#[tokio::test]
async fn run_captures_stderr_tail_on_failure() {
    let cancel = CancellationToken::new();
    let args = vec![OsString::from("--definitely-bad-flag")];
    let e = run(Path::new("rustc"), &args, None, &cancel).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Media);
    assert!(e.message.contains("error"), "message: {}", e.message);
}

#[tokio::test]
async fn run_missing_command_is_dependency_missing() {
    let cancel = CancellationToken::new();
    let e = run(Path::new("definitely-not-a-real-binary-xyz"), &[], None, &cancel)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::DependencyMissing);
}

#[tokio::test]
async fn run_success_and_cancel() {
    let cancel = CancellationToken::new();
    let out = run(Path::new("rustc"), &[OsString::from("--version")], None, &cancel)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("rustc"));
    cancel.cancel();
    let e = run(Path::new("rustc"), &[OsString::from("--version")], None, &cancel)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Cancelled);
}

#[tokio::test]
async fn run_streaming_delivers_lines() {
    let cancel = CancellationToken::new();
    let mut lines: Vec<String> = Vec::new();
    let st = rustler_tools::run_streaming(
        Path::new("rustc"),
        &[OsString::from("--definitely-bad-flag")],
        None,
        &cancel,
        |l| lines.push(l.to_string()),
    )
    .await
    .unwrap();
    assert!(!st.success());
    assert!(lines.iter().any(|l| l.contains("error")), "{lines:?}");
}
