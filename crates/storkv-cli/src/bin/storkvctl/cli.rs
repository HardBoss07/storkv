//! Command line argument definitions for `storkvctl`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "storkvctl", about = "StorKV operational management CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
    /// Check health of running storkvd daemon
    Health {
        #[arg(long, default_value = "127.0.0.1:7878")]
        sock: String,
    },
    /// Inspect offline data directory integrity (SSTables, MANIFEST)
    Doctor {
        #[arg(long, default_value = "./data/storkv")]
        data_dir: PathBuf,
    },
    /// Create a hot zero-copy MVCC checkpoint backup
    Backup {
        #[arg(long, default_value = "127.0.0.1:7878")]
        sock: String,
        #[arg(long)]
        out: String,
    },
    /// Show live autonomous backup status for a running daemon
    BackupStatus {
        #[arg(long, default_value = "127.0.0.1:7878")]
        sock: String,
    },
    /// Verify archived backup generations reachable from a running daemon
    BackupVerify {
        #[arg(long, default_value = "127.0.0.1:7878")]
        sock: String,
        #[arg(long)]
        target_seq: Option<u64>,
    },
    /// Run compaction, optionally only over a key range in one scope
    Compact {
        #[arg(long, default_value = "127.0.0.1:7878")]
        sock: String,
        /// Scope to compact. Omit to compact every scope.
        #[arg(long)]
        scope: Option<String>,
        /// Inclusive start key as UTF-8 text.
        #[arg(long)]
        start: Option<String>,
        /// Exclusive end key as UTF-8 text.
        #[arg(long)]
        end: Option<String>,
    },
    /// Restore a backup tree into a fresh data directory (offline)
    Restore {
        #[arg(long, default_value = "./backups")]
        backup_dir: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        target_seq: Option<u64>,
    },
    /// Export or import keys as JSONL, or copy a Redis database
    Migrate {
        /// Running daemon endpoint (any supported URI).
        #[arg(long)]
        sock: Option<String>,
        /// Offline data directory used for imports.
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// Write every key in --scope to this JSONL file.
        #[arg(long)]
        export_jsonl: Option<PathBuf>,
        /// Import records from this JSONL file.
        #[arg(long)]
        import_jsonl: Option<PathBuf>,
        /// Source scope for exports / destination scope for imports.
        #[arg(long, default_value = "default")]
        scope: String,
        /// Destination scope for imports.
        #[arg(long)]
        into_scope: Option<String>,
        /// Copy all keys from a Redis-compatible server (redis://host:port).
        #[arg(long)]
        from_redis: Option<String>,
    },
    /// Run YCSB benchmark workload
    Ycsb {
        #[arg(long, default_value = "127.0.0.1:7878")]
        sock: String,
        #[arg(long, default_value = "a")]
        workload: String,
        #[arg(long, default_value_t = 100)]
        records: usize,
        #[arg(long, default_value_t = 100)]
        ops: usize,
    },
}
