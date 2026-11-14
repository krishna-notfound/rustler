//! Command registry: the only way to call Rustler functionality.
//!
//! Commands are registered as `Box<dyn Command>` (usually built with [`typed`]),
//! described by a [`CommandSpec`] (JSON schemas, annotations, examples) and invoked
//! through [`Registry::dispatch`] with a [`Ctx`]. Front-ends (CLI, later MCP) wrap
//! results in an [`Envelope`].

pub mod ctx;
pub mod envelope;
pub mod progress;
pub mod registry;
pub mod spec;

pub use ctx::Ctx;
pub use envelope::Envelope;
pub use progress::{ProgressEvent, ProgressSink};
pub use registry::{Registry, typed};
pub use spec::{Annotations, Command, CommandSpec, Example};

// Re-exported so command crates can `#[async_trait]` without their own dependency.
pub use async_trait::async_trait;
