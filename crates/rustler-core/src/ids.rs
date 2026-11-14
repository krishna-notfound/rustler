//! Prefixed ULID identifiers. Each id is a newtype over `String` that displays,
//! parses and serializes as the plain string (e.g. `src_01jd3k...`, 30 chars).

use crate::error::{ErrorCode, RustlerError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

macro_rules! prefixed_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// String prefix including the underscore.
            pub const PREFIX: &'static str = $prefix;

            /// New random id: prefix + lowercase ULID.
            pub fn new() -> Self {
                Self(format!("{}{}", $prefix, ulid::Ulid::generate().to_string().to_lowercase()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        /// Parsing checks only the prefix (so hand-written ids in tests/fixtures work).
        impl FromStr for $name {
            type Err = RustlerError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                if s.len() > $prefix.len() && s.starts_with($prefix) {
                    Ok(Self(s.to_string()))
                } else {
                    Err(RustlerError::new(
                        ErrorCode::Validation,
                        "id",
                        format!("invalid {} '{}': expected prefix '{}'", stringify!($name), s, $prefix),
                    ))
                }
            }
        }
    };
}

prefixed_id!(
    /// Project id, `prj_` prefix.
    ProjectId,
    "prj_"
);
prefixed_id!(
    /// Source id, `src_` prefix.
    SourceId,
    "src_"
);
prefixed_id!(
    /// Clip id, `clp_` prefix.
    ClipId,
    "clp_"
);
prefixed_id!(
    /// Render id, `rnd_` prefix.
    RenderId,
    "rnd_"
);
prefixed_id!(
    /// Segment id, `seg_` prefix.
    SegmentId,
    "seg_"
);
