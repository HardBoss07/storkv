use clap::Parser;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use storkv_core::backup::{
    BackupCompression, BackupConfig, BackupRetention, CommandPipeSinkConfig,
};
use storkv_server::{ReplicationRole, Server, TransportOptions, transports};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Parser, Debug)]
#[command(name = "storkvd", about = "StorKV daemon")]
struct Args {
    /// Ship sealed WAL segments continuously (point-in-time recovery).
    #[arg(long)]
    backup_ship_wal: bool,
    /// Restrict admin-only requests to admin listeners or token holders.
    #[arg(long)]
    strict_admin: bool,
    /// Treat every HTTP connection as admin (dedicated admin listener).
    #[arg(long)]
    http_admin: bool,
    /// Treat every WebSocket connection as admin.
    #[arg(long)]
    ws_admin: bool,
    /// Treat every gRPC connection as admin.
    #[arg(long)]
    grpc_admin: bool,
    /// Seconds between automatic full backups.
    #[arg(long, default_value_t = 3600)]
    backup_interval_secs: u64,
    /// Number of full generations to retain.
    #[arg(long, default_value_t = 7)]
    backup_retention: usize,
    #[arg(long, default_value = "./data/storkv")]
    data_dir: PathBuf,
    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: String,
    #[arg(long)]
    admin_sock: Option<String>,
    /// Enable autonomous continuous backup into this directory.
    #[arg(long)]
    backup_dir: Option<PathBuf>,
    /// Stream backup archives into this program instead of writing locally.
    #[arg(long)]
    backup_pipe_cmd: Option<String>,
    /// Repeatable argument for --backup-pipe-cmd.
    #[arg(long = "backup-pipe-arg")]
    backup_pipe_args: Vec<String>,
    /// Archive compression for the pipe sink: none, zstd, lz4.
    #[arg(long, default_value = "none")]
    backup_compression: String,
    /// Shared secret granting admin privileges on auxiliary transports.
    #[arg(long, env = "STORKV_ADMIN_TOKEN")]
    admin_token: Option<String>,
    /// Read the admin token from this file. The file must contain one line.
    #[arg(long, value_name = "PATH")]
    admin_token_file: Option<PathBuf>,
    /// Serve the HTTP/1.1 + HTTP/2 REST API on this address.
    #[arg(long)]
    http_listen: Option<String>,
    /// Serve the WebSocket multiplexed API on this address.
    #[arg(long)]
    ws_listen: Option<String>,
    /// Serve the gRPC API on this address.
    #[arg(long)]
    grpc_listen: Option<String>,
    /// Serve the Redis RESP2/RESP3 API on this address.
    #[arg(long)]
    redis_listen: Option<String>,
    /// Serve the Memcached ASCII API on this address.
    #[arg(long)]
    memcached_listen: Option<String>,
    /// PEM server certificate that enables TLS on the native listener.
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// PEM private key that enables TLS on the native listener.
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,
    /// Replication role: standalone, primary, or replica.
    #[arg(long, default_value = "standalone")]
    role: String,
    /// Primary URI that a replica streams WAL frames from.
    #[arg(long)]
    primary_uri: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let admin_token = resolve_admin_token(&args)?;
    let role = ReplicationRole::parse(&args.role)
        .map_err(|detail| std::io::Error::new(std::io::ErrorKind::InvalidInput, detail))?;
    let server = Arc::new(
        Server::new(args.data_dir)?
            .with_role(role)
            .with_primary_uri(args.primary_uri.clone())
            .with_strict_admin(args.strict_admin),
    );

    let backup_handle = if let Some(backup_dir) = &args.backup_dir {
        let compression = BackupCompression::parse(&args.backup_compression)?;
        let mut config = match &args.backup_pipe_cmd {
            Some(program) => {
                let mut pipe = CommandPipeSinkConfig::new(program.clone());
                pipe.args = args.backup_pipe_args.clone();
                pipe.compression = compression;
                BackupConfig::command(backup_dir.clone(), pipe)?
            }
            None => BackupConfig::local(backup_dir.clone()),
        };
        config = config
            .with_full_backup_interval(Duration::from_secs(args.backup_interval_secs))
            .with_retention(BackupRetention {
                max_full_backups: args.backup_retention,
                ..BackupRetention::default()
            })
            .with_wal_shipping(args.backup_ship_wal)
            .with_compression(compression);
        println!("storkvd backups enabled at {}", backup_dir.display());
        Some(server.db().start_auto_backup(config)?)
    } else {
        None
    };

    let tls_acceptor = match (&args.tls_cert, &args.tls_key) {
        (Some(certificate), Some(key)) => {
            Some(storkv_server::tls::load_acceptor(certificate, key)?)
        }
        (None, None) => None,
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TLS requires both --tls-cert and --tls-key",
            )
            .into());
        }
    };

    let client_listener = TcpListener::bind(&args.listen).await?;
    let server_client = server.clone();
    let client_task = if let Some(acceptor) = tls_acceptor {
        println!("storkvd listening on {} (TLS)", args.listen);
        tokio::spawn(async move {
            let _ = server_client
                .run_tls_listener(client_listener, acceptor, false)
                .await;
        })
    } else {
        println!("storkvd listening on {}", args.listen);
        tokio::spawn(async move {
            let _ = server_client.run_tcp_listener(client_listener, false).await;
        })
    };

    let admin_task = if let Some(admin_addr) = args.admin_sock {
        let server_admin = server.clone();
        #[cfg(unix)]
        if storkv_proto::is_unix_socket_addr(&admin_addr) {
            let path = storkv_proto::unix_socket_path(&admin_addr);
            if let Some(parent) = std::path::Path::new(path).parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
            let _ = std::fs::remove_file(path);
            let admin_listener = tokio::net::UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            println!("storkvd admin listening on unix:{}", path);
            Some(tokio::spawn(async move {
                let _ = server_admin.run_uds_listener(admin_listener, true).await;
            }))
        } else {
            let admin_listener = TcpListener::bind(&admin_addr).await?;
            println!("storkvd admin listening on {}", admin_addr);
            Some(tokio::spawn(async move {
                let _ = server_admin.run_tcp_listener(admin_listener, true).await;
            }))
        }
        #[cfg(not(unix))]
        {
            let admin_listener = TcpListener::bind(&admin_addr).await?;
            println!("storkvd admin listening on {}", admin_addr);
            Some(tokio::spawn(async move {
                let _ = server_admin.run_tcp_listener(admin_listener, true).await;
            }))
        }
    } else {
        None
    };

    let mut transport_tasks: Vec<JoinHandle<()>> = Vec::new();
    let replica_task = server.spawn_replica();
    if let Some(uri) = &args.primary_uri {
        println!("storkvd role {:?}, primary {uri}", role);
    }

    if let Some(address) = &args.http_listen {
        let listener = TcpListener::bind(address).await?;
        let options = TransportOptions::new(args.http_admin, admin_token.clone());
        let task_server = server.clone();
        println!("storkvd http listening on {address}");
        transport_tasks.push(tokio::spawn(async move {
            let _ = transports::serve_http_tcp(task_server, listener, options).await;
        }));
    }

    if let Some(address) = &args.ws_listen {
        let listener = TcpListener::bind(address).await?;
        let options = TransportOptions::new(args.ws_admin, admin_token.clone());
        let task_server = server.clone();
        println!("storkvd websocket listening on {address}");
        transport_tasks.push(tokio::spawn(async move {
            let _ = transports::serve_ws_tcp(task_server, listener, options).await;
        }));
    }

    if let Some(address) = &args.grpc_listen {
        let listener = TcpListener::bind(address).await?;
        let options = TransportOptions::new(args.grpc_admin, admin_token.clone());
        let task_server = server.clone();
        println!("storkvd grpc listening on {address}");
        transport_tasks.push(tokio::spawn(async move {
            if let Err(error) = transports::serve_grpc_tcp(task_server, listener, options).await {
                eprintln!("storkvd grpc listener stopped: {error}");
            }
        }));
    }

    if let Some(address) = &args.redis_listen {
        let listener = TcpListener::bind(address).await?;
        let task_server = server.clone();
        println!("storkvd redis listening on {address}");
        transport_tasks.push(tokio::spawn(async move {
            let _ = storkv_server::compat::serve_redis_tcp(task_server, listener).await;
        }));
    }

    if let Some(address) = &args.memcached_listen {
        let listener = TcpListener::bind(address).await?;
        let task_server = server.clone();
        println!("storkvd memcached listening on {address}");
        transport_tasks.push(tokio::spawn(async move {
            let _ = storkv_server::compat::serve_memcached_tcp(task_server, listener).await;
        }));
    }

    tokio::signal::ctrl_c().await?;
    println!("Shutting down storkvd...");
    if let Some(handle) = &backup_handle {
        handle.shutdown()?;
    }
    let _ = server.shutdown_handle().send(());

    let _ = client_task.await;
    if let Some(task) = admin_task {
        let _ = task.await;
    }
    for task in transport_tasks {
        let _ = task.await;
    }
    if let Some(task) = replica_task {
        task.abort();
    }

    Ok(())
}

fn resolve_admin_token(args: &Args) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(path) = &args.admin_token_file else {
        return Ok(args.admin_token.clone());
    };
    if args.admin_token.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "use --admin-token or --admin-token-file, not both",
        )
        .into());
    }
    let contents = std::fs::read_to_string(path)?;
    let token = contents.trim();
    if token.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the admin token file is empty",
        )
        .into());
    }
    Ok(Some(token.to_string()))
}
