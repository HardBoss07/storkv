/// Consistency level requested for data operations.
///
/// The level selects which node serves a read and how much replication lag is
/// acceptable. A primary always serves strong reads. A replica serves reads
/// according to the configured level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Consistency {
    /// Reads and writes run on the primary. A replica rejects the request.
    #[default]
    Strong,
    /// A replica serves reads when its lag does not exceed the bound.
    BoundedStaleness {
        /// Maximum accepted replication lag in milliseconds.
        max_lag_ms: u64,
    },
    /// A replica serves reads from its local state at any lag.
    Eventual,
}

impl Consistency {
    /// Encode the level as a mode tag and a lag bound.
    #[must_use]
    pub fn to_wire(self) -> (u8, u64) {
        match self {
            Consistency::Strong => (0, 0),
            Consistency::BoundedStaleness { max_lag_ms } => (1, max_lag_ms),
            Consistency::Eventual => (2, 0),
        }
    }

    /// Decode a mode tag and a lag bound.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvWireError::Protocol`] when the mode tag is
    /// unknown.
    pub fn from_wire(mode: u8, max_lag_ms: u64) -> Result<Self, crate::StorkvWireError> {
        match mode {
            0 => Ok(Consistency::Strong),
            1 => Ok(Consistency::BoundedStaleness { max_lag_ms }),
            2 => Ok(Consistency::Eventual),
            other => Err(crate::StorkvWireError::Protocol(format!(
                "Unknown consistency mode: {other}"
            ))),
        }
    }
}
