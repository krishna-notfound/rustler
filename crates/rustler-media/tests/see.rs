mod support;

use rustler_core::ErrorCode;
use rustler_media::{TileSpec, frame, preview, probe, tiles};

#[tokio::test]
async fn frame_and_tiles() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path().join("clip (1) – टेस्ट.mp4");
    support::make_video(&t, &v, 10, 1280, 720, true);

    let jpg = dir.path().join("f").join("frame.jpg");
    frame(&t, &v, 5.0, 640, &jpg).await.unwrap();
    assert!(std::fs::metadata(&jpg).unwrap().len() > 1024);
    assert_eq!(image::image_dimensions(&jpg).unwrap(), (640, 360));

    let sheet = dir.path().join("sheet.jpg");
    let spec = TileSpec { start: 1.0, end: 9.0, ..TileSpec::default() };
    let labels = tiles(&t, &v, &spec, &|t| format!("words at {t:.0}"), &font, &sheet)
        .await
        .unwrap();
    assert_eq!(labels.len(), 12);
    assert!(labels.windows(2).all(|w| w[0].t < w[1].t));
    assert!(labels.iter().all(|l| l.t >= 1.0 && l.t <= 9.0));
    assert!(labels[0].text.contains("words at"), "{}", labels[0].text);
    let (w, h) = image::image_dimensions(&sheet).unwrap();
    assert_eq!(w, 1920);
    assert_eq!(h, 3 * (270 + 36));
}

#[tokio::test]
async fn tiles_rejects_bad_spec() {
    let Some(t) = support::tools() else { return };
    let Some(font) = support::font() else { return };
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path().join("v.mp4");
    support::make_video(&t, &v, 3, 320, 240, false);
    let spec = TileSpec { start: 2.0, end: 1.0, ..TileSpec::default() };
    let e = tiles(&t, &v, &spec, &|_| String::new(), &font, &dir.path().join("o.jpg"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn preview_rejects_over_60s() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path().join("v.mp4");
    support::make_video(&t, &v, 3, 320, 240, false);
    let e = preview(&t, &v, 0.0, 61.0, 360, &dir.path().join("p.mp4")).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn preview_makes_small_clip() {
    let Some(t) = support::tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path().join("v.mp4");
    support::make_video(&t, &v, 8, 1280, 720, true);
    let out = dir.path().join("pv").join("p.mp4");
    preview(&t, &v, 2.0, 5.0, 360, &out).await.unwrap();
    let p = probe(&t, &out).await.unwrap();
    assert_eq!((p.width, p.height), (640, 360));
    assert!((p.duration - 3.0).abs() < 0.3, "duration {}", p.duration);
}
