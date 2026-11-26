use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(super) const LOOPBACK_LISTEN: &str = "127.0.0.1:0";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const WAIT_STEP: Duration = Duration::from_millis(25);

pub(super) fn temp_root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("storkv_config_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create temp root");
    path
}

pub(super) fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

pub(super) fn write_config(root: &Path, contents: &str) -> PathBuf {
    let path = root.join("storkv.toml");
    std::fs::write(&path, contents).expect("write config file");
    path
}

pub(super) fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

pub(super) fn daemon_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_storkvd"));
    command.stdout(Stdio::null()).stderr(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("STORKV_") {
            command.env_remove(key);
        }
    }
    command
}

pub(super) fn spawn_with_config(config: &Path) -> Child {
    let mut command = daemon_command();
    command
        .arg("--config")
        .arg(config)
        .arg("--listen")
        .arg(LOOPBACK_LISTEN);
    command.spawn().expect("spawn storkvd")
}

pub(super) fn wait_for_path(path: &Path) -> bool {
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(WAIT_STEP);
    }
    false
}

pub(super) fn wait_for_listener(address: &str) {
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(address).is_ok() {
            return;
        }
        std::thread::sleep(WAIT_STEP);
    }
    panic!("listener {address} did not start");
}

pub(super) fn free_local_address() -> String {
    let listener = std::net::TcpListener::bind(LOOPBACK_LISTEN).expect("bind free port");
    listener.local_addr().expect("free address").to_string()
}

pub(super) fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub(super) fn uncommented_scopes(template: &str) -> String {
    let mut block = String::new();
    let mut in_scopes = false;
    for line in template.lines() {
        if line.trim() == "# [[scopes]]" {
            in_scopes = true;
        }
        if in_scopes {
            block.push_str(line.trim_start_matches('#').trim_start());
            block.push('\n');
        }
    }
    block
}

pub(super) fn http_request(
    method: &str,
    address: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, String) {
    let mut stream = std::net::TcpStream::connect(address).expect("connect http listener");
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .expect("write http request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read http response");
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("unexpected HTTP response: {response}"));
    let payload = response
        .split_once("\r\n\r\n")
        .map(|(_, payload)| payload.to_owned())
        .unwrap_or_default();
    (status, payload)
}

pub(super) fn http_put(address: &str, path: &str, value: &str) -> u16 {
    http_request("PUT", address, path, Some(value)).0
}

pub(super) fn http_get(address: &str, path: &str) -> (u16, String) {
    http_request("GET", address, path, None)
}
