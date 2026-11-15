//! `rustler-desktop` binary: the Rustler GUI in a native window. Same registry and UI as
//! `rustler gui`; see `docs/desktop.md`.

// Release builds are a GUI app on Windows (no console window); debug builds keep the console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rustler_desktop::{DesktopOptions, open_env, resolve_home, resolve_project, run};
use std::path::PathBuf;

const HELP: &str = "rustler-desktop: the Rustler editor in a native window

USAGE: rustler-desktop [--project <dir>] [--view <route>]

  --project <dir>   project to open (default: RUSTLER_PROJECT, else the current folder if it holds project.json)
  --view <route>    GUI route to show first, e.g. editor/<clip_id>, sources, clips
  -h, --help        this help
  -V, --version     version

ENV: RUSTLER_HOME (default ~/.rustler), RUSTLER_DESKTOP_OPEN=<dir> (open this project after the page loads)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("rustler-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let flag = |name: &str| {
        let eq = format!("{name}=");
        args.iter().enumerate().find_map(|(i, a)| if a == name { args.get(i + 1).cloned() } else { a.strip_prefix(&eq).map(str::to_string) })
    };
    let opts = DesktopOptions {
        project: resolve_project(flag("--project").map(PathBuf::from)),
        home: resolve_home(),
        view: flag("--view"),
        open_after_load: open_env(),
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => fail(&format!("cannot start the async runtime: {e}")),
    };
    let mut reg = rustler_registry::Registry::new();
    rustler_commands::register_all(&mut reg);
    match run(reg, opts, rt.handle()) {
        Ok(code) => {
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
            std::process::exit(code)
        }
        Err(e) => fail(&format!("{e:#}")),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("rustler-desktop: {msg}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Rustler")
        .set_description(msg)
        .show();
    std::process::exit(10)
}
