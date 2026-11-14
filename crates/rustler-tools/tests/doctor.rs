use rustler_tools::{ToolPaths, doctor};

#[tokio::test]
async fn doctor_on_empty_home_not_ready() {
    let tmp = tempfile::tempdir().unwrap();
    let r = doctor(&ToolPaths::from_home(tmp.path())).await;
    assert!(!r.ready);
    for m in ["ffmpeg", "ffprobe", "yt-dlp", "uv", "engine", "fonts"] {
        assert!(r.missing.iter().any(|x| x == m), "missing {m}: {:?}", r.missing);
    }
    assert!(r.ffmpeg.is_none() && r.encoders.is_empty() && !r.libass);
    assert!(r.fonts.is_empty());
    assert!(!r.os.is_empty() && !r.arch.is_empty());
}

/// Uses the machine's real tools (read-only) when present; skips otherwise.
#[tokio::test]
async fn doctor_reports_real_tools_when_installed() {
    let real = ToolPaths::from_home(&ToolPaths::default_home());
    if !real.ffmpeg.is_file() {
        eprintln!("SKIP: no ffmpeg in {}", real.bin.display());
        return;
    }
    let r = doctor(&real).await;
    let ff = r.ffmpeg.expect("ffmpeg info");
    assert!(!ff.version.is_empty());
    assert!(r.libass, "libass expected in BtbN build");
    assert!(r.encoders.iter().any(|e| e.name == "libx264" && e.ok), "{:?}", r.encoders);
    assert_eq!(r.encoders.len(), 6);
    eprintln!("nvidia: {:?}; encoders: {:?}", r.nvidia_gpu, r.encoders);
}
