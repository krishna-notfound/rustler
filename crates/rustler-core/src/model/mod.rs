//! Serializable project data model (frozen contracts, Task 1).

pub mod analysis;
pub mod clip;
pub mod project;
pub mod qc;
pub mod source;
pub mod timeline;
pub mod transcript;

pub use analysis::*;
pub use clip::*;
pub use project::*;
pub use qc::*;
pub use source::*;
// Timeline v2 item types share names with media types (Anchor, EffectKind, ...): use them as
// `rustler_core::timeline::X`; only the document types are re-exported here.
pub use timeline::{TIMELINE_SCHEMA, Timeline, TimelineMeta, Tracks, VideoSeg};
pub use transcript::*;
