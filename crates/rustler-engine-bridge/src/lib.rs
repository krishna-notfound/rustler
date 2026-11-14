//! Rust client for the Python engine (`rustler_engine`) over line-delimited
//! JSON-RPC 2.0 on stdio. Wire protocol: `../PROTOCOL.md` (frozen; `engine/PROTOCOL.md`
//! is the source of truth).

pub mod client;
pub mod protocol;

pub use client::EngineClient;
