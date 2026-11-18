use std::path::Path;

use storkv_core::Db;
use storkv_core::backup::RestoreReport;

use crate::CliRunner;

impl CliRunner {
    /// Restore a backup tree into a fresh data directory. This is offline and
    /// never talks to a running daemon.
    pub fn restore(
        backup_dir: &Path,
        out_dir: &Path,
        target_seq: Option<u64>,
    ) -> storkv_core::Result<RestoreReport> {
        Db::restore_from_backup(backup_dir, out_dir, target_seq)
    }
}
