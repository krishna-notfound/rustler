//! Shared helpers for the command integration tests. Tool-dependent tests skip (with a printed
//! reason) when the tools are not installed under `RUSTLER_HOME` / `~/.rustler`.
#![allow(dead_code)]

use rustler_core::{ErrorCode, ProjectStore, RustlerError};
use rustler_registry::{Ctx, Registry};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME") {
        return PathBuf::from(h);
    }
    let user = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    PathBuf::from(user.unwrap_or_default()).join(".rustler")
}

fn exe(name: &str) -> PathBuf {
    home().join("bin").join(format!("{name}{}", if cfg!(windows) { ".exe" } else { "" }))
}

pub fn ffmpeg() -> PathBuf {
    exe("ffmpeg")
}

/// True if ffmpeg/ffprobe are installed; prints a skip reason otherwise.
pub fn have_media_tools() -> bool {
    let ok = ffmpeg().is_file() && exe("ffprobe").is_file();
    if !ok {
        eprintln!("SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)", home().join("bin").display());
    }
    ok
}

/// True if yt-dlp is installed.
pub fn have_ytdlp() -> bool {
    let ok = exe("yt-dlp").is_file();
    if !ok {
        eprintln!("SKIP: yt-dlp not found (run `rustler setup`)");
    }
    ok
}

/// True if uv and the engine sources are available; points the commands at the repo's engine.
pub fn have_engine() -> bool {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../engine");
    if !exe("uv").is_file() || !engine.join("rustler_engine").is_dir() {
        eprintln!("SKIP: uv or engine sources not available");
        return false;
    }
    // SAFETY: tests only ever set these to the same values.
    unsafe {
        std::env::set_var("RUSTLER_ENGINE_PROJECT", std::fs::canonicalize(&engine).unwrap());
        if std::env::var_os("HF_HOME").is_none() {
            std::env::set_var("HF_HOME", home().join("models").join("hf"));
        }
    }
    true
}

pub fn jfk() -> Option<PathBuf> {
    let p = home().join("test-media").join("jfk.wav");
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: {} missing", p.display());
        None
    }
}

pub fn registry() -> Registry {
    let mut r = Registry::new();
    rustler_commands::register_all(&mut r);
    r
}

/// A fresh project in a temp dir plus a context for it.
pub fn project() -> (tempfile::TempDir, Ctx) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    ProjectStore::create(&root, "test").unwrap();
    let ctx = Ctx::new(Some(root), home());
    (dir, ctx)
}

pub async fn run(reg: &Registry, name: &str, ctx: &Ctx, input: Value) -> Result<Value, RustlerError> {
    reg.dispatch(name, input, ctx.clone()).await
}

pub async fn ok(reg: &Registry, name: &str, ctx: &Ctx, input: Value) -> Value {
    match run(reg, name, ctx, input).await {
        Ok(v) => v,
        Err(e) => panic!("{name} failed: {e} hint={:?}", e.hint),
    }
}

pub fn expect_code(r: Result<Value, RustlerError>, code: ErrorCode) -> RustlerError {
    match r {
        Err(e) if e.code == code => e,
        Err(e) => panic!("expected {code:?}, got {:?}: {}", e.code, e.message),
        Ok(v) => panic!("expected {code:?}, got Ok({v})"),
    }
}

fn ff(args: &[&str]) {
    let out = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg fixture failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// 320x240 test video of `secs` seconds; optional audio = 440 Hz tone (2 s on / 2 s off).
pub fn make_video(path: &Path, secs: u32, audio: bool) {
    let dur = secs.to_string();
    let src = format!("testsrc2=s=320x240:r=25:d={secs}");
    let tone = format!("sine=frequency=440:duration={secs}");
    let mut a: Vec<&str> = vec!["-f", "lavfi", "-i", &src];
    if audio {
        a.extend(["-f", "lavfi", "-i", &tone, "-af", "volume='if(lt(mod(t,4),2),1,0)':eval=frame"]);
    }
    a.extend(["-t", &dur, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
    if audio {
        a.extend(["-c:a", "aac"]);
    }
    a.push(path.to_str().unwrap());
    ff(&a);
}

/// Small video whose audio is the given wav (JFK sample, ~11 s).
pub fn make_video_with_wav(path: &Path, wav: &Path) {
    ff(&[
        "-f", "lavfi", "-i", "testsrc2=s=320x240:r=25:d=11",
        "-i", wav.to_str().unwrap(),
        "-t", "11", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
        path.to_str().unwrap(),
    ]);
}

/// Audio-only m4a (tone) of `secs` seconds.
pub fn make_audio_only(path: &Path, secs: u32) {
    let src = format!("sine=frequency=440:duration={secs}");
    ff(&["-f", "lavfi", "-i", &src, "-c:a", "aac", path.to_str().unwrap()]);
}
