use clap::{Parser, Subcommand};
use std::path::PathBuf;
use storkv_cli::CliRunner;

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
