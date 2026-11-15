//! `rustler` binary: a thin shell around [`rustler_cli::run_cli_with`].

use rustler_cli::{CliOptions, init_logging, output, run_cli_with};
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{Envelope, Registry};
use std::io::IsTerminal;
use tokio_util::sync::CancellationToken;

fn main() {
    // A panic must still honour the contract: one JSON envelope on stdout, exit 10.
    std::panic::set_hook(Box::new(|info| {
        let env = Envelope::err(
            RustlerError::new(ErrorCode::Internal, "cli", format!("internal error: {info}"))
                .hint("this is a bug in rustler; re-run with RUSTLER_LOG=debug and report it"),
        );
        output::print_stdout(&env.to_json());
        std::process::exit(10);
    }));
    init_logging();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let env = Envelope::err(RustlerError::new(ErrorCode::Internal, "cli", format!("cannot start runtime: {e}")));
            output::print_stdout(&env.to_json());
            std::process::exit(10);
        }
    };

    // `rustler desktop`: the native window owns the main thread (event loop), so it runs outside
    // `block_on`; its GUI server runs on this runtime's worker threads.
    if is_desktop_builtin(&args) {
        std::process::exit(run_desktop(&args, &runtime));
    }

    let cancel = CancellationToken::new();
    let (out, code) = runtime.block_on(async {
        let c = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                c.cancel();
            }
        });
        let mut reg = Registry::new();
        rustler_commands::register_all(&mut reg);
        if is_server_builtin(&args) {
            return (String::new(), serve_builtin(&args, reg).await);
        }
        let opts = CliOptions {
            tty: std::io::stdout().is_terminal(),
            cancel,
            ..CliOptions::default()
        };
        run_cli_with(args, reg, opts).await
    });
    if !out.is_empty() {
        output::print_stdout(&out);
    }
    std::process::exit(code);
}

/// `rustler mcp serve` and `rustler gui` are long-running servers over the same registry.
fn is_server_builtin(args: &[String]) -> bool {
    // `--help`/`--version` always answer immediately instead of starting a server.
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h" | "--version" | "-V")) {
        return false;
    }
    let words = positional_words(args);
    (words.first() == Some(&"mcp") && words.get(1) == Some(&"serve")) || words.first() == Some(&"gui")
}

/// `rustler desktop [--project <dir>] [--view <route>]` (not with `--help`).
fn is_desktop_builtin(args: &[String]) -> bool {
    !args.iter().any(|a| matches!(a.as_str(), "--help" | "-h" | "--version" | "-V"))
        && positional_words(args).first() == Some(&"desktop")
}

/// Runs the desktop app in-process over the same registry; returns the exit code.
fn run_desktop(args: &[String], runtime: &tokio::runtime::Runtime) -> i32 {
    let mut reg = Registry::new();
    rustler_commands::register_all(&mut reg);
    let opts = rustler_desktop::DesktopOptions {
        project: resolve_project(args),
        home: resolve_home(),
        view: flag_value(args, "--view"),
        open_after_load: rustler_desktop::open_env(),
    };
    match rustler_desktop::run(reg, opts, runtime.handle()) {
        Ok(code) => code,
        Err(e) => {
            let env = Envelope::err(
                RustlerError::new(ErrorCode::Internal, "desktop", format!("{e:#}"))
                    .hint("the desktop app needs the WebView2 runtime (Windows) or WebKitGTK (Linux); `rustler gui` serves the same editor in a browser"),
            );
            output::print_stdout(&env.to_json());
            10
        }
    }
}

/// Non-flag words, skipping the values of global flags that take one (`--project <dir>` etc.).
fn positional_words(args: &[String]) -> Vec<&str> {
    const VALUE_FLAGS: &[&str] = &["--project", "--port", "--actor", "--max-seconds", "--input-json", "--view"];
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if VALUE_FLAGS.contains(&a.as_str()) {
            skip = true;
        } else if !a.starts_with('-') {
            out.push(a.as_str());
        }
    }
    out
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    args.iter().enumerate().find_map(|(i, a)| {
        if a == name {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&eq).map(str::to_string)
        }
    })
}

fn resolve_home() -> std::path::PathBuf {
    if let Ok(h) = std::env::var("RUSTLER_HOME") {
        return h.into();
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join(".rustler")
}

fn resolve_project(args: &[String]) -> Option<std::path::PathBuf> {
    if let Some(p) = flag_value(args, "--project") {
        return Some(p.into());
    }
    if let Ok(p) = std::env::var("RUSTLER_PROJECT") {
        return Some(p.into());
    }
    let cwd = std::env::current_dir().ok()?;
    cwd.join("project.json").exists().then_some(cwd)
}

/// Runs `rustler mcp serve` or `rustler gui` until the server stops; returns the exit code.
async fn serve_builtin(args: &[String], reg: Registry) -> i32 {
    let is_mcp = positional_words(args).first() == Some(&"mcp");
    let ctx = rustler_registry::Ctx::new(resolve_project(args), resolve_home());
    let reg = std::sync::Arc::new(reg);
    let res = if is_mcp {
        rustler_mcp::serve_stdio(reg, ctx).await
    } else {
        let port = flag_value(args, "--port").and_then(|p| p.parse().ok());
        let open_browser = !args.iter().any(|a| a == "--no-open");
        rustler_gui::serve(reg, ctx, rustler_gui::GuiOptions { port, open_browser }).await
    };
    match res {
        Ok(()) => 0,
        Err(e) => {
            let env = Envelope::err(RustlerError::new(ErrorCode::Internal, if is_mcp { "mcp" } else { "gui" }, e.to_string()));
            output::print_stdout(&env.to_json());
            10
        }
    }
}
