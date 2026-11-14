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
