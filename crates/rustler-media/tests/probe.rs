mod support;

use rustler_core::ErrorCode;
use rustler_media::{extract_audio_wav, make_proxy, probe};
use rustler_registry::ProgressSink;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn probe_basic() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("basic.mp4");
    support::make_video(&t, &f, 10, 1280, 720, true);
    let p = probe(&t, &f).await.unwrap();
    assert!((p.duration - 10.0).abs() < 0.1, "duration {}", p.duration);
    assert!(p.has_video && p.has_audio);
    assert_eq!((p.width, p.height), (1280, 720));
    assert!((p.fps - 25.0).abs() < 0.1, "fps {}", p.fps);
    assert_eq!(p.video_codec.as_deref(), Some("h264"));
    assert_eq!(p.audio_codec.as_deref(), Some("aac"));
    assert_eq!(p.sample_rate, Some(48000));
    assert_eq!(p.channels, Some(1));
    assert_eq!(p.rotation, 0);
}

#[tokio::test]
async fn probe_reports_display_dims_for_rotated() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("rot.mp4");
    support::make_rotated(&t, &f);
    let p = probe(&t, &f).await.unwrap();
    assert_eq!((p.width, p.height), (1080, 1920), "{p:?}");
    assert_eq!(p.rotation.abs(), 90);
}

#[tokio::test]
async fn probe_audio_only() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("pod.m4a");
    support::make_audio_only(&t, &f, 6);
    let p = probe(&t, &f).await.unwrap();
    assert!(!p.has_video && p.has_audio);
    assert_eq!((p.width, p.height), (0, 0));
    assert!((p.duration - 6.0).abs() < 0.2);
}

#[tokio::test]
async fn probe_video_without_audio() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("mute.mp4");
    support::make_video(&t, &f, 2, 640, 360, false);
    let p = probe(&t, &f).await.unwrap();
    assert!(p.has_video && !p.has_audio);
    assert_eq!(p.sample_rate, None);
}

#[tokio::test]
async fn probe_awkward_filename_and_garbage() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("my clip (1) – टेस्ट.mp4");
    support::make_video(&t, &f, 2, 320, 240, true);
    assert_eq!(probe(&t, &f).await.unwrap().width, 320);

    let bad = dir.path().join("bad.mp4");
    std::fs::write(&bad, b"not media").unwrap();
    assert_eq!(probe(&t, &bad).await.unwrap_err().code, ErrorCode::Media);
    let missing = dir.path().join("nope.mp4");
    assert_eq!(probe(&t, &missing).await.unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn proxy_and_wav() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("src.mp4");
    support::make_video(&t, &f, 4, 1920, 1080, true);
    let cancel = CancellationToken::new();

    let proxy = dir.path().join("out").join("proxy.mp4");
    make_proxy(&t, &f, &proxy, &cancel, &ProgressSink::noop()).await.unwrap();
    let p = probe(&t, &proxy).await.unwrap();
    assert_eq!((p.width, p.height), (960, 540));
    assert!(p.has_audio);
    // second call is skipped (output newer than source): mtime unchanged
    let m1 = std::fs::metadata(&proxy).unwrap().modified().unwrap();
    make_proxy(&t, &f, &proxy, &cancel, &ProgressSink::noop()).await.unwrap();
    assert_eq!(m1, std::fs::metadata(&proxy).unwrap().modified().unwrap());

    let wav = dir.path().join("a").join("audio.wav");
    extract_audio_wav(&t, &f, &wav, &cancel).await.unwrap();
    let w = probe(&t, &wav).await.unwrap();
    assert_eq!((w.sample_rate, w.channels), (Some(16000), Some(1)));
    assert!((w.duration - 4.0).abs() < 0.2);

    // no audio stream -> Validation
    let mute = dir.path().join("mute.mp4");
    support::make_video(&t, &mute, 2, 320, 240, false);
    let e = extract_audio_wav(&t, &mute, &dir.path().join("m.wav"), &cancel).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}
