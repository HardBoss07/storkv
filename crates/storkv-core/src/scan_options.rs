//! Bounded scan options for range and prefix reads.

/// Options for [`crate::Db::scan`] and [`crate::Db::scan_prefix`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// Stop after this many live entries. `None` means no limit.
    pub limit: Option<usize>,
    /// Read keys from high to low when true.
    pub reverse: bool,
    /// Omit values when true. Returned values are empty bytes.
    pub keys_only: bool,
}
