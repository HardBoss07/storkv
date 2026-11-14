//! Strongly-typed, zero-cost identifiers.
//!
//! StorKV's on-disk and wire formats deliberately use little-endian `u32`/`u64`
//! integers. Inside the engine those integers flow through many layers where a
//! scope id, a sequence number, a file number and a millisecond timestamp are
//! all interchangeable at the type level. These `#[repr(transparent)]` newtypes
//! make that distinction explicit without changing layout or serialization:
//! convert with [`From`]/[`Into`] or [`ScopeId::get`] at the format boundary.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

macro_rules! id_newtype {
    ($name:ident, $raw:ty, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        #[repr(transparent)]
        pub struct $name($raw);

        impl $name {
            /// Wrap a raw identifier value.
            pub const fn new(raw: $raw) -> Self {
                Self(raw)
            }

            /// Return the raw identifier value.
            pub const fn get(self) -> $raw {
                self.0
            }
        }

        impl From<$raw> for $name {
            fn from(raw: $raw) -> Self {
                Self(raw)
            }
        }

        impl From<$name> for $raw {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_newtype!(ScopeId, u32, "Opaque scope identifier (wire/disk: `u32`).");
id_newtype!(
    SequenceNumber,
    u64,
    "MVCC commit sequence number (wire/disk: `u64`)."
);
id_newtype!(
    FileNumber,
    u64,
    "SSTable/WAL file number (wire/disk: `u64`)."
);
id_newtype!(
    TimestampMs,
    u64,
    "Unix-epoch millisecond timestamp (wire/disk: `u64`)."
);

impl TimestampMs {
    /// Current wall-clock time as Unix-epoch milliseconds.
    ///
    /// A clock before the Unix epoch (or an out-of-range duration) yields
    /// `TimestampMs(0)` rather than panicking.
    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        Self(millis)
    }
}

// Identifiers are layout-transparent and must never grow padding.
const _: () = assert!(std::mem::size_of::<ScopeId>() == std::mem::size_of::<u32>());
const _: () = assert!(std::mem::size_of::<SequenceNumber>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::size_of::<FileNumber>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::size_of::<TimestampMs>() == std::mem::size_of::<u64>());
const _: () = assert!(std::mem::align_of::<ScopeId>() == std::mem::align_of::<u32>());
const _: () = assert!(std::mem::align_of::<SequenceNumber>() == std::mem::align_of::<u64>());
