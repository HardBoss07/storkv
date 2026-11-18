use clap::{Parser, Subcommand};
use std::path::PathBuf;
use storkv_cli::{CliRunner, MigrateTarget};

#[derive(Parser, Debug)]
#[command(name = "storkvctl", about = "StorKV operational management CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Health { sock } => {
            CliRunner::health(&sock).await?;
            println!("OK - storkvd is healthy");
        }
        Commands::Doctor { data_dir } => {
            let report = CliRunner::doctor(&data_dir);
            println!(
                "Doctor Report: {} SSTables verified, {} Manifests verified.",
                report.sstables_checked, report.manifests_checked
            );
            if report.errors.is_empty() {
                println!("All integrity checks PASSED.");
            } else {
                eprintln!("Integrity errors detected ({}):", report.errors.len());
                for err in report.errors {
                    eprintln!(" - {}", err);
                }
                std::process::exit(1);
            }
        }
        Commands::Backup { sock, out } => {
            CliRunner::backup(&sock, &out).await?;
            println!("Backup successfully created at {}", out);
        }
        Commands::BackupStatus { sock } => {
            let status = CliRunner::backup_status(&sock).await?;
            println!("Backup enabled: {}", status.enabled);
            println!(
                "Backup root: {}",
                status.backup_root.as_deref().unwrap_or("<none>")
            );
            println!("Sink: {}", status.sink);
            println!("Completed backups: {}", status.backups_completed_total);
            println!("Failed backups: {}", status.backups_failed_total);
            println!("Last backup sequence: {}", status.last_backup_seq);
            println!(
                "Last backup timestamp (ms): {}",
                status.last_backup_timestamp_ms
            );
            println!(
                "Shipped WAL segments: {}",
                status.wal_segments_shipped_total
            );
            println!(
                "Next full backup in (ms): {}",
                status.next_full_backup_in_ms
            );
            if let Some(error) = status.last_error {
                println!("Last error: {error}");
            }
        }
        Commands::BackupVerify { sock, target_seq } => {
            let report = CliRunner::backup_verify(&sock, target_seq).await?;
            println!(
                "Checked {} generations, {} files, {} bytes, {} WAL segments.",
                report.generations_checked,
                report.files_checked,
                report.bytes_checked,
                report.wal_segments_checked
            );
            if report.is_ok() {
                println!("All backup checks PASSED.");
            } else {
                eprintln!(
                    "Backup verification failed ({} errors):",
                    report.errors.len()
                );
                for error in report.errors {
                    eprintln!(" - {}: {}", error.path, error.detail);
                }
                std::process::exit(1);
            }
        }
        Commands::Restore {
            backup_dir,
            out_dir,
            target_seq,
        } => {
            let report = CliRunner::restore(&backup_dir, &out_dir, target_seq)?;
            println!(
                "Restored generation {} to {} (sequence {}, {} WAL segments replayed, {} bytes).",
                report.generation_id,
                out_dir.display(),
                report.restored_seq,
                report.wal_segments_replayed,
                report.bytes_restored
            );
            for warning in report.warnings {
                eprintln!("warning: {warning}");
            }
        }
        Commands::Migrate {
            sock,
            data_dir,
            export_jsonl,
            import_jsonl,
            scope,
            into_scope,
            from_redis,
        } => {
            let target = match (&data_dir, &sock) {
                (Some(directory), _) => Some(MigrateTarget::DataDir(directory)),
                (None, Some(socket)) => Some(MigrateTarget::Daemon(socket)),
                (None, None) => None,
            };
            if let Some(redis) = from_redis {
                let Some(target) = target else {
                    return Err("migrate --from-redis requires --sock or --data-dir".into());
                };
                let destination = into_scope.as_deref().unwrap_or(&scope);
                let imported = CliRunner::migrate_from_redis(&redis, target, destination).await?;
                println!("Imported {imported} keys from {redis} into scope {destination}");
            } else if let Some(output) = export_jsonl {
                let Some(socket) = sock else {
                    return Err("migrate --export-jsonl requires --sock".into());
                };
                let exported = CliRunner::migrate_export(&socket, &scope, &output).await?;
                println!(
                    "Exported {exported} keys from scope {scope} to {}",
                    output.display()
                );
            } else if let Some(input) = import_jsonl {
                let Some(target) = target else {
                    return Err("migrate --import-jsonl requires --sock or --data-dir".into());
                };
                let destination = into_scope.as_deref().unwrap_or(&scope);
                let imported = CliRunner::migrate_import(target, destination, &input).await?;
                println!(
                    "Imported {imported} keys from {} into scope {destination}",
                    input.display()
                );
            } else {
                return Err(
                    "migrate requires --export-jsonl, --import-jsonl, or --from-redis".into(),
                );
            }
        }
        Commands::Ycsb {
            sock,
            workload,
            records,
            ops,
        } => {
            let summary = CliRunner::run_ycsb(&sock, &workload, records, ops).await?;
            println!(
                "YCSB Workload `{}` finished: {} ops in {} ms ({:.2} ops/sec)",
                summary.workload, summary.total_ops, summary.elapsed_ms, summary.ops_per_sec
            );
        }
    }

    Ok(())
}
