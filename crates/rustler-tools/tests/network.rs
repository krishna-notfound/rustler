//! Network self-fixes: JS runtime detection, TLS-behind-a-proxy classification, yt-dlp config.
//! Unit-level (no network) except the `#[ignore]`d live test.

use rustler_registry::ProgressSink;
use rustler_tools::{ToolPaths, deno_install, ytdlp_install};
use std::ffi::OsString;

const VERBOSE_DENO: &str = "[debug] Command-line config: ['-v']\n\
[debug] yt-dlp version stable@2026.10.26 from yt-dlp/yt-dlp [c91f8de0b] (win_exe)\n\
[debug] Optional libraries: Cryptodome-3.24.1, yt_dlp_ejs-0.8.0\n\
[debug] JS runtimes: deno-2.9.2\n\
[debug] Proxy map: {}\n";

const VERBOSE_NONE: &str = "[debug] Command-line config: ['-v']\n[debug] JS runtimes: none\n[debug] Proxy map: {}\n";

#[test]
fn js_runtime_detected_from_verbose_output() {
    assert_eq!(ytdlp_install::js_runtime_detected(VERBOSE_DENO), Some(true));
    assert_eq!(ytdlp_install::js_runtime_name(VERBOSE_DENO).as_deref(), Some("deno-2.9.2"));
    assert_eq!(ytdlp_install::js_runtime_detected(VERBOSE_NONE), Some(false));
    assert_eq!(ytdlp_install::js_runtime_name(VERBOSE_NONE).as_deref(), Some("none"));
    // several runtimes, and an older yt-dlp that prints no such line
    assert_eq!(ytdlp_install::js_runtime_detected("[debug] JS runtimes: deno-2.9.2, node-22.1.0\n"), Some(true));
    assert_eq!(ytdlp_install::js_runtime_detected("[debug] Proxy map: {}\n"), None);
}

#[test]
fn classify_tls_error_hint() {
    let tls = "ERROR: [youtube] jNQXAC9IVRw: Unable to download webpage: [SSL: CERTIFICATE_VERIFY_FAILED] \
               certificate verify failed: self-signed certificate in certificate chain (_ssl.c:1006)";
    assert_eq!(ytdlp_install::classify_tls_error_hint(tls), Some("rustler setup run --insecure-tls"));
    let issue = ytdlp_install::classify_probe_failure(tls).unwrap();
    assert_eq!(issue.kind, "tls");
    assert!(issue.hint.contains("rustler setup run --insecure-tls"));
    // plain network trouble is not a TLS problem
    assert_eq!(ytdlp_install::classify_tls_error_hint("ERROR: Unable to download webpage: timed out"), None);
    // JS runtime trouble points at plain setup
    let js = "WARNING: [youtube] n challenge solving failed: Ensure you have a supported JavaScript runtime";
    let issue = ytdlp_install::classify_probe_failure(js).unwrap();
    assert_eq!(issue.kind, "js_runtime");
    assert!(issue.hint.contains("rustler setup run") && !issue.hint.contains("insecure"));
    assert!(ytdlp_install::classify_probe_failure("ERROR: Video unavailable").is_none());
}

#[test]
fn config_flag_added_when_conf_exists() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(ytdlp_install::config_args(tmp.path()).is_empty(), "no conf, no flag");
    let conf = ytdlp_install::write_insecure_tls_conf(tmp.path()).unwrap();
    assert_eq!(conf, tmp.path().join("yt-dlp.conf"));
    assert!(std::fs::read_to_string(&conf).unwrap().contains("--no-check-certificates"));
    let args = ytdlp_install::config_args(tmp.path());
    assert_eq!(args, vec![OsString::from("--config-locations"), conf.into_os_string()]);
}

#[test]
fn path_gets_the_rustler_bin_first() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    let joined = ytdlp_install::path_with_bin(&bin);
    let first = std::env::split_paths(&joined).next().unwrap();
    assert_eq!(first, bin);
    // the rest of the inherited PATH is kept
    if let Some(orig) = std::env::var_os("PATH") {
        assert_eq!(std::env::split_paths(&joined).count(), std::env::split_paths(&orig).count() + 1);
    }
}

#[test]
fn deno_asset_and_checksum_parsing() {
    assert_eq!(deno_install::asset_name_for("windows"), Some("deno-x86_64-pc-windows-msvc.zip"));
    assert_eq!(deno_install::asset_name_for("linux"), Some("deno-x86_64-unknown-linux-gnu.zip"));
    assert_eq!(deno_install::asset_name_for("macos"), None);
    // Linux release: sha256sum style
    let linux = "e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609  deno-x86_64-unknown-linux-gnu.zip\n";
    assert_eq!(
        deno_install::parse_checksum(linux).as_deref(),
        Some("e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609")
    );
    // Windows release: PowerShell Get-FileHash table
    let win = "\nAlgorithm : SHA256\nHash      : CBA84B26E0DF5AE7F74FC66B86A95B866872A86292EBD84FA1E959927EDD93BF\n\
               Path      : C:\\a\\deno\\deno\\target\\release\\deno-x86_64-pc-windows-msvc.zip\n\n";
    assert_eq!(
        deno_install::parse_checksum(win).as_deref(),
        Some("cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf")
    );
    assert_eq!(deno_install::parse_checksum("not a checksum"), None);
}

/// Real download into a temp home (network): `cargo test -p rustler-tools --test network -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn deno_install_live() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ToolPaths::from_home(tmp.path());
    let progress = ProgressSink::new(|e| eprintln!("[progress] {} {:?} {:?}", e.stage, e.pct, e.message));
    let c = deno_install::install_current(&paths, &progress).await.unwrap();
    eprintln!("installed: {c:?}");
    assert!(c.path.is_file());
    assert!(c.version.chars().next().unwrap().is_ascii_digit(), "version: {}", c.version);

    // yt-dlp finds that Deno when only `<home>/bin` is on PATH (copy of the machine's yt-dlp; skipped without one)
    let real = ToolPaths::from_home(&ToolPaths::default_home());
    if !real.ytdlp.is_file() {
        eprintln!("SKIP yt-dlp check: no yt-dlp in {}", real.bin.display());
        return;
    }
    std::fs::copy(&real.ytdlp, &paths.ytdlp).unwrap();
    let out = std::process::Command::new(&paths.ytdlp)
        .arg("-v")
        .env("PATH", &paths.bin)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    let name = ytdlp_install::js_runtime_name(&text);
    eprintln!("yt-dlp reports JS runtimes: {name:?}");
    assert_eq!(ytdlp_install::js_runtime_detected(&text), Some(true), "{text}");
}
