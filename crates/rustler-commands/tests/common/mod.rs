//! Shared fixtures for the clip/see/render command tests.
#![allow(dead_code)]

use chrono::Utc;
use rustler_core::{
    CaptionScript, ErrorCode, MediaProbe, ProjectStore, Source, SourceId, SourceOrigin, Transcript, Word, WordTiming,
};
use rustler_registry::{Ctx, Registry};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub fn registry() -> Registry {
    let mut reg = Registry::new();
    rustler_commands::register_all(&mut reg);
    reg
}

pub async fn call(reg: &Registry, ctx: &Ctx, name: &str, input: Value) -> rustler_core::Result<Value> {
    reg.dispatch(name, input, ctx.clone()).await
}

pub fn code(e: &rustler_core::RustlerError) -> ErrorCode {
    e.code
}

/// Real tools home: `RUSTLER_HOME` or `~/.rustler`.
pub fn real_home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME") {
        return PathBuf::from(h);
    }
    let user = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    PathBuf::from(user.unwrap_or_default()).join(".rustler")
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn system_font() -> Option<PathBuf> {
    [
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// A throwaway tools home whose `bin/` links the real ffmpeg/ffprobe and whose `fonts/` holds one
/// system font. `None` (with a printed reason) when ffmpeg is not installed.
pub fn tools_home() -> Option<(TempDir, PathBuf)> {
    let real = real_home().join("bin");
    let (ffmpeg, ffprobe) = (real.join(exe("ffmpeg")), real.join(exe("ffprobe")));
    if !ffmpeg.is_file() || !ffprobe.is_file() {
        eprintln!("SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)", real.display());
        return None;
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for src in [&ffmpeg, &ffprobe] {
        let dst = bin.join(src.file_name().unwrap());
        if std::fs::hard_link(src, &dst).is_err() {
            std::fs::copy(src, &dst).unwrap();
        }
    }
    let fonts = home.join("fonts").join("test");
    std::fs::create_dir_all(&fonts).unwrap();
    if let Some(f) = system_font() {
        std::fs::copy(&f, fonts.join(f.file_name().unwrap())).unwrap();
    }
    Some((tmp, home))
}

pub fn ffmpeg_in(home: &Path) -> PathBuf {
    home.join("bin").join(exe("ffmpeg"))
}

/// testsrc2 (+ 440 Hz sine when `audio`) mp4.
pub fn make_video(ffmpeg: &Path, out: &Path, secs: u32, w: u32, h: u32, audio: bool) {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"]);
    cmd.arg(format!("testsrc2=size={w}x{h}:rate=30:duration={secs}"));
    if audio {
        cmd.args(["-f", "lavfi", "-i"]).arg(format!("sine=frequency=440:duration={secs}"));
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "veryfast"]);
    if audio {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    cmd.arg(out);
    let st = cmd.status().expect("run ffmpeg");
    assert!(st.success(), "fixture generation failed");
}

pub fn make_music(ffmpeg: &Path, out: &Path, secs: u32) {
    let st = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=220:duration={secs}"))
        .arg(out)
        .status()
        .unwrap();
    assert!(st.success());
}

pub fn new_project() -> (TempDir, ProjectStore, Ctx) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let (store, _) = ProjectStore::create(&dir, "test").unwrap();
    let ctx = Ctx::new(Some(dir), tmp.path().join("nohome"));
    (tmp, store, ctx)
}

pub fn probe_1080p(duration: f64, has_video: bool, has_audio: bool) -> MediaProbe {
    MediaProbe {
        duration,
        has_video,
        has_audio,
        width: if has_video { 1920 } else { 0 },
        height: if has_video { 1080 } else { 0 },
        rotation: 0,
        fps: 30.0,
        video_codec: has_video.then(|| "h264".into()),
        audio_codec: has_audio.then(|| "aac".into()),
        sample_rate: Some(48000),
        channels: Some(2),
    }
}

/// Registers a source in the project (the media file need not exist for clip-only tests).
pub fn add_source(store: &ProjectStore, media: &Path, probe: MediaProbe) -> Source {
    let source = Source {
        id: SourceId::new(),
        name: media.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        origin: SourceOrigin::File { path: media.to_path_buf() },
        media_path: media.to_path_buf(),
        probe,
        proxy: None,
        audio_wav: None,
        imported_at: Utc::now(),
    };
    store.save_source(&source).unwrap();
    let mut p = store.load_project().unwrap();
    let base = p.revision;
    p.sources.push(source.id.clone());
    store.save_project(&mut p, base).unwrap();
    source
}

pub fn word(i: u32, text: &str, start: f64, end: f64) -> Word {
    Word { i, text: text.into(), start, end, conf: None, speaker: None }
}

/// Three sentences (units 0, 1, 2) between 1 s and 7 s.
pub fn sample_words() -> Vec<Word> {
    let spec: [(&str, f64, f64); 12] = [
        ("Hello", 1.0, 1.4),
        ("there", 1.45, 1.8),
        ("friends.", 1.85, 2.3),
        ("Fellow", 3.0, 3.4),
        ("Americans", 3.45, 4.0),
        ("ask", 4.05, 4.3),
        ("not.", 4.35, 5.0),
        ("What", 5.05, 5.3),
        ("your", 5.35, 5.5),
        ("country", 5.55, 6.0),
        ("can", 6.05, 6.3),
        ("do.", 6.35, 7.0),
    ];
    spec.iter().enumerate().map(|(i, (t, s, e))| word(i as u32, t, *s, *e)).collect()
}

pub fn write_transcript(store: &ProjectStore, source: &SourceId, words: Vec<Word>) {
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id: source.clone(),
        language: "en".into(),
        engine: "test".into(),
        script: CaptionScript::Latin,
        words,
        word_timing: WordTiming::Exact,
    };
    store.write_json(&Path::new("analysis").join(source.as_str()).join("transcript.json"), &t).unwrap();
}
