//! Fixture generator shared by the media integration tests. Uses the installed ffmpeg
//! (`RUSTLER_HOME` or `~/.rustler`, `bin/`); tests skip with a printed reason if absent.
#![allow(dead_code)]

use rustler_tools::ToolPaths;
use std::path::{Path, PathBuf};
use std::process::Command;

fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME") {
        return PathBuf::from(h);
    }
    let user = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    PathBuf::from(user.unwrap_or_default()).join(".rustler")
}

/// Tool paths if ffmpeg and ffprobe are installed, else `None` (with a printed skip reason).
pub fn tools() -> Option<ToolPaths> {
    let home = home();
    let bin = home.join("bin");
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let ffmpeg = bin.join(format!("ffmpeg{exe}"));
    let ffprobe = bin.join(format!("ffprobe{exe}"));
    if !ffmpeg.exists() || !ffprobe.exists() {
        eprintln!("SKIP: ffmpeg/ffprobe not found in {} (run `rustler setup`)", bin.display());
        return None;
    }
    Some(ToolPaths {
        ytdlp: bin.join(format!("yt-dlp{exe}")),
        uv: bin.join(format!("uv{exe}")),
        engine_dir: home.join("engine"),
        venv_python: home.join("engine").join(".venv").join("python"),
        models: home.join("models"),
        fonts: home.join("fonts"),
        bin,
        ffmpeg,
        ffprobe,
        home,
    })
}

/// A TTF font usable for tile labels, if one is present on this machine.
pub fn font() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = vec![
        PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/Library/Fonts/Arial.ttf"),
    ];
    let fonts = home().join("fonts");
    if let Ok(rd) = std::fs::read_dir(&fonts) {
        for d in rd.flatten() {
            if let Ok(inner) = std::fs::read_dir(d.path()) {
                for f in inner.flatten() {
                    if f.path().extension().is_some_and(|e| e == "ttf") {
                        cands.push(f.path());
                    }
                }
            }
        }
    }
    let found = cands.into_iter().find(|p| p.exists());
    if found.is_none() {
        eprintln!("SKIP: no TTF font found for tile labels");
    }
    found
}

fn ffmpeg(paths: &ToolPaths, args: &[&str]) {
    let out = Command::new(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("spawn ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg fixture failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Audio source: 440 Hz tone for 2 s, silence for 2 s, repeating.
pub fn tone_gap_source(secs: u32) -> String {
    format!(
        "sine=frequency=440:sample_rate=48000:duration={secs},volume='if(lt(mod(t,4),2),1,0)':eval=frame"
    )
}

/// `testsrc2` video of `secs` seconds at `w`x`h` 25 fps, optionally with the tone/silence audio.
pub fn make_video(paths: &ToolPaths, path: &Path, secs: u32, w: u32, h: u32, with_audio: bool) {
    let v = format!("testsrc2=size={w}x{h}:rate=25:duration={secs}");
    let a = tone_gap_source(secs);
    let t = secs.to_string();
    let p = path.to_str().unwrap();
    let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &v];
    if with_audio {
        args.extend(["-f", "lavfi", "-i", &a]);
    }
    args.extend(["-t", &t, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
    if with_audio {
        args.extend(["-c:a", "aac"]);
    }
    args.push(p);
    ffmpeg(paths, &args);
}

/// 1920x1080 stream carrying a 90 degree display rotation (so display dims are 1080x1920).
pub fn make_rotated(paths: &ToolPaths, path: &Path) {
    let plain = path.with_file_name("plain_for_rotation.mp4");
    make_video(paths, &plain, 2, 1920, 1080, false);
    // New ffmpeg: `-display_rotation` input option, stream-copied into the output.
    ffmpeg(
        paths,
        &[
            "-display_rotation:v:0",
            "90",
            "-i",
            plain.to_str().unwrap(),
            "-c",
            "copy",
            path.to_str().unwrap(),
        ],
    );
}

/// Audio-only file: tone/silence, `secs` seconds, AAC in m4a.
pub fn make_audio_only(paths: &ToolPaths, path: &Path, secs: u32) {
    let a = tone_gap_source(secs);
    ffmpeg(
        paths,
        &["-f", "lavfi", "-i", &a, "-c:a", "aac", path.to_str().unwrap()],
    );
}
