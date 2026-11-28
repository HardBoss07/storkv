//! Range-limited compaction operation.
use crate::compaction::Compactor;
use crate::db::DbInner;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Compact only the SSTables of `scope_name` that overlap `[start, end)`.
    ///
    /// # Errors
    ///
    /// Returns `crate::StorkvError::InvalidOperation` when `start >= end`.
    /// Returns `crate::StorkvError::ScopeNotFound` when the scope is unknown.
    /// Returns an error when compaction or manifest logging fails.
    pub fn compact_range(
        &self,
        scope_name: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> crate::Result<()> {
        if let (Some(start), Some(end)) = (start, end) {
            if start >= end {
                return Err(crate::StorkvError::InvalidOperation(
                    "compact range start must be less than end".into(),
                ));
            }
        }
        let scope_id = *self
            .scope_names
            .read()
            .get(scope_name)
            .ok_or_else(|| crate::StorkvError::ScopeNotFound(scope_name.to_string()))?;
        let scope = self
            .scopes
            .read()
            .get(&scope_id)
            .cloned()
            .ok_or(crate::StorkvError::ScopeIdNotFound(scope_id))?;

        let gc_safe = SequenceNumber::new(self.gc_safe_seq());
        let edit = Compactor::compact_range(
            &scope,
            start,
            end,
            gc_safe,
            &self.next_file_num,
            TimestampMs::now(),
            None,
        )?;
        if self.apply_compaction_edit(&scope, edit)? {
            self.telemetry.record_compaction();
        }
        Ok(())
    }
}
