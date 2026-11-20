//! Foreground sandbox server for manual testing.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::cli::SandboxOptions;
use crate::error::{Result, XtaskError};
use crate::test_runner;
use crate::tools;

const PORTS: [(&str, u16); 6] = [
    ("--listen", 7878),
    ("--http-listen", 7879),
    ("--ws-listen", 7880),
    ("--grpc-listen", 7881),
    ("--redis-listen", 7882),
    ("--memcached-listen", 7883),
];

/// Start `storkvd` in the foreground and stop it when the user is done.
///
/// # Errors
///
/// Returns an error when the build, certificate, or server start fails.
pub fn run(root: &Path, options: &SandboxOptions) -> Result<()> {
    let sandbox = root.join("target").join("sandbox");
    tools::reset_dir(&sandbox)?;
    test_runner::cargo(
        root,
        &["build", "-p", "storkv-server", "--bin", "storkvd"],
        "cargo build storkvd",
    )?;

    let certificate = if options.tls {
        Some(generate_certificate(&sandbox)?)
    } else {
        None
    };
    let mut command = Command::new(server_binary(root));
    command
        .arg("--data-dir")
        .arg(sandbox.join("data"))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (flag, port) in PORTS {
        command.arg(flag).arg(format!("127.0.0.1:{port}"));
    }
    if options.strict_admin {
        command.arg("--strict-admin");
    }
    let token = options
        .admin_token
        .clone()
        .or_else(|| options.strict_admin.then(generated_token));
    if let Some(token) = &token {
        command.arg("--admin-token").arg(token);
    }
    if let Some(certificate) = &certificate {
        command.arg("--tls-cert").arg(&certificate.certificate);
        command.arg("--tls-key").arg(&certificate.key);
    }

    let child = command.spawn()?;
    let mut guard = SandboxGuard {
        child: Some(child),
        directory: sandbox.clone(),
    };
    print_summary(&sandbox, token.as_deref(), certificate.as_ref());
    wait_for_shutdown(&mut guard)?;
    Ok(())
}

fn server_binary(root: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "storkvd.exe"
    } else {
        "storkvd"
    };
    root.join("target").join("debug").join(name)
}

struct Certificate {
    certificate: PathBuf,
    key: PathBuf,
}

fn generate_certificate(directory: &Path) -> Result<Certificate> {
    let tls = directory.join("tls");
    std::fs::create_dir_all(&tls)?;
    let generated =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .map_err(|error| {
                XtaskError::Layout(format!("certificate generation failed: {error}"))
            })?;
    let certificate = tls.join("server.pem");
    let key = tls.join("server.key");
    std::fs::write(&certificate, generated.cert.pem())?;
    std::fs::write(&key, generated.key_pair.serialize_pem())?;
    Ok(Certificate { certificate, key })
}

fn generated_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("sandbox-{nanos:x}")
}

fn print_summary(directory: &Path, token: Option<&str>, certificate: Option<&Certificate>) {
    println!("sandbox data: {}", directory.display());
    println!();
    for (flag, port) in PORTS {
        let kind = flag.trim_start_matches("--");
        println!("{kind:<18} 127.0.0.1:{port}");
    }
    if let Some(token) = token {
        println!("\nadmin token: {token}");
    }
    if let Some(certificate) = certificate {
        println!("\ncertificate: {}", certificate.certificate.display());
        println!("private key: {}", certificate.key.display());
        println!("native listener TLS enabled (self-signed; trust this certificate in clients).");
    }
    println!("\nPress Ctrl+C to stop the sandbox.");
}

fn wait_for_shutdown(guard: &mut SandboxGuard) -> Result<()> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&shutdown);
    ctrlc::set_handler(move || {
        signal.store(true, Ordering::SeqCst);
    })
    .map_err(|error| XtaskError::Layout(format!("failed to install Ctrl-C handler: {error}")))?;

    while !shutdown.load(Ordering::SeqCst) {
        if let Some(child) = guard.child.as_mut()
            && child.try_wait()?.is_some()
        {
            return Err(XtaskError::Layout("storkvd stopped unexpectedly".into()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

struct SandboxGuard {
    child: Option<Child>,
    directory: PathBuf,
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
