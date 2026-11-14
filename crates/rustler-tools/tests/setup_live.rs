//! Network test (downloads ~200 MB + Python deps):
//! `cargo test -p rustler-tools --test setup_live -- --ignored --nocapture`

use rustler_registry::ProgressSink;
use rustler_tools::{SetupOptions, ToolPaths, doctor, engine_install, setup};

#[tokio::test]
#[ignore]
async fn setup_live() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ToolPaths::from_home(tmp.path());
    let progress = ProgressSink::new(|e| {
        if e.pct.is_none() || e.pct.is_some_and(|p| ((p * 100.0) as u32) % 25 == 0) {
            eprintln!("[progress] {} {:?} {:?}", e.stage, e.pct, e.message);
        }
    });
    let report = setup(&paths, &SetupOptions::default(), &progress).await.unwrap();
    eprintln!("installed: {:#?}", report.installed);
    eprintln!("skipped: {:#?}", report.skipped);
    eprintln!("failed: {:#?}", report.failed);
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);

    // idempotent: second run skips everything
    let again = setup(&paths, &SetupOptions::default(), &progress).await.unwrap();
    assert!(again.installed.is_empty(), "second run installed {:?}", again.installed);
    assert!(again.failed.is_empty());

    let d = doctor(&paths).await;
    eprintln!("doctor: {d:#?}");
    assert!(d.libass);
    assert!(d.ready, "missing: {:?}", d.missing);
    assert!(engine_install::has_embedded_project());
}
