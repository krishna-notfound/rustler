//! Library half of the `rustler` CLI (so integration tests can reach `args_to_json`).
//! The CLI is a generic, schema-driven front over the registry: no feature logic here.

pub mod argmap;
pub mod output;
