use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use storkv_server::Server;
use tokio::net::TcpListener;

#[derive(Parser, Debug)]
#[command(name = "storkvd", about = "StorKV daemon")]
struct Args {
    #[arg(long, default_value = "./data/storkv")]
    data_dir: PathBuf,

    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: String,

    #[arg(long)]
    admin_sock: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let server = Arc::new(Server::new(args.data_dir)?);

    let client_listener = TcpListener::bind(&args.listen).await?;
    println!("storkvd listening on {}", args.listen);

    let server_client = server.clone();
    let client_task = tokio::spawn(async move {
        let _ = server_client.run_tcp_listener(client_listener, false).await;
    });

    let admin_task = if let Some(admin_addr) = args.admin_sock {
        let server_admin = server.clone();
        #[cfg(unix)]
        if storkv_proto::is_unix_socket_addr(&admin_addr) {
            let path = storkv_proto::unix_socket_path(&admin_addr);
            let _ = std::fs::remove_file(path);
            let admin_listener = tokio::net::UnixListener::bind(path)?;
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

    tokio::signal::ctrl_c().await?;
    println!("Shutting down storkvd...");
    let _ = server.shutdown_handle().send(());

    let _ = client_task.await;
    if let Some(task) = admin_task {
        let _ = task.await;
    }

    Ok(())
}
