//! Rustler core: frozen data contracts (ids, errors, events, models) plus the
//! project store, time map, transcript units and layout presets.

pub mod error;
pub mod events;
pub mod ids;
pub mod layout;
pub mod model;
pub mod store;
pub mod timemap;
pub mod units;

pub use error::{ErrorCode, Result, RustlerError};
pub use events::{Actor, Event};
pub use ids::{ClipId, ProjectId, RenderId, SegmentId, SourceId};
pub use model::*;
pub use store::ProjectStore;
pub use timemap::TimeMap;
