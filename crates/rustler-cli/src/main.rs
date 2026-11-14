//! `rustler` binary. Wave 0 stub: Task 8 replaces this with the generic CLI front.

use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{Envelope, Registry};

fn main() {
    let mut reg = Registry::new();
    rustler_commands::register_all(&mut reg);
    let env = Envelope::err(
        RustlerError::new(
            ErrorCode::Internal,
            "cli",
            "the rustler CLI front is not implemented yet",
        )
        .hint("Slice 1 Task 8 implements argument parsing"),
    );
    println!("{}", env.to_json());
    std::process::exit(env.exit_code());
}
