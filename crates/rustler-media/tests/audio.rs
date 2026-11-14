mod support;

use rustler_media::analyze_audio;
use rustler_registry::ProgressSink;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn audio_events_find_silences() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("pod.m4a");
    support::make_audio_only(&t, &f, 20);
    let ev = analyze_audio(&t, &f, &CancellationToken::new(), &ProgressSink::noop())
        .await
        .unwrap();
    assert!(ev.silences.len() >= 4, "{:?}", ev.silences);
    // silences at ~[2,4], [6,8], ... each ~2 s
    for s in ev.silences.iter().take(4) {
        assert!(((s.end - s.start) - 2.0).abs() < 0.3, "{s:?}");
    }
    assert!((ev.silences[0].start - 2.0).abs() < 0.3, "{:?}", ev.silences[0]);
    assert!((19..=21).contains(&ev.curve.len()), "curve len {}", ev.curve.len());
    assert!(ev.curve.windows(2).all(|w| w[0].t < w[1].t));
    // tone seconds are loud, silent seconds are at the floor
    assert!(ev.curve[0].lufs > -40.0, "{:?}", ev.curve[0]);
    assert!(ev.curve[3].lufs < -60.0, "{:?}", ev.curve[3]);
}

#[tokio::test]
async fn audio_events_video_without_audio_is_empty() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("mute.mp4");
    support::make_video(&t, &f, 2, 320, 240, false);
    let ev = analyze_audio(&t, &f, &CancellationToken::new(), &ProgressSink::noop())
        .await
        .unwrap();
    assert!(ev.curve.is_empty() && ev.silences.is_empty() && ev.peaks.is_empty());
}

#[tokio::test]
async fn audio_events_peaks_on_loud_burst() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("burst.wav");
    // quiet tone (about -24 dBFS, above the silence threshold) with a 2 s burst 18 dB louder at t=10
    let src = "sine=frequency=300:sample_rate=48000:duration=20,volume='if(between(t,10,12),4,0.5)':eval=frame";
    let out = std::process::Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", src])
        .arg(&f)
        .output()
        .unwrap();
    assert!(out.status.success());
    let ev = analyze_audio(&t, &f, &CancellationToken::new(), &ProgressSink::noop())
        .await
        .unwrap();
    assert!(ev.peaks.iter().any(|p| p.t >= 10.0 && p.t < 13.0), "{:?}", ev.peaks);
    assert!(ev.silences.is_empty(), "{:?}", ev.silences);
}
