use std::io::{Read, Write};

use super::helpers::{
    LOOPBACK_LISTEN, cleanup, daemon_command, free_local_address, stop, temp_root, toml_path,
    wait_for_listener, wait_for_path, write_config,
};

fn admin_compact_status(address: &str) -> u16 {
    let mut stream = std::net::TcpStream::connect(address).expect("connect http listener");
    let request = format!(
        "POST /v1/admin/compact HTTP/1.1\r\nHost: {address}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("write admin request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read admin response");
    response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("unexpected HTTP response: {response}"))
}

#[test]
fn daemon_loads_config_from_environment() {
    let root = temp_root("env_config");
    let file_dir = root.join("file-data");
    let config = write_config(&root, &format!("data_dir = \"{}\"\n", toml_path(&file_dir)));

    let mut command = daemon_command();
    command
        .arg("--listen")
        .arg(LOOPBACK_LISTEN)
        .env("STORKV_CONFIG", &config);
    let child = command.spawn().expect("spawn storkvd");
    let started = wait_for_path(&file_dir);
    stop(child);
    cleanup(&root);

    assert!(started, "STORKV_CONFIG must load the configuration file");
}

#[test]
fn config_flag_overrides_config_environment() {
    let env_root = temp_root("config_flag_env");
    let cli_root = temp_root("config_flag_cli");
    let env_dir = env_root.join("env-data");
    let cli_dir = cli_root.join("cli-data");
    let env_config = write_config(
        &env_root,
        &format!("data_dir = \"{}\"\n", toml_path(&env_dir)),
    );
    let cli_config = write_config(
        &cli_root,
        &format!("data_dir = \"{}\"\n", toml_path(&cli_dir)),
    );

    let mut command = daemon_command();
    command
        .arg("--config")
        .arg(&cli_config)
        .arg("--listen")
        .arg(LOOPBACK_LISTEN)
        .env("STORKV_CONFIG", &env_config);
    let child = command.spawn().expect("spawn storkvd");
    let started = wait_for_path(&cli_dir);
    stop(child);

    assert!(started, "--config must win over STORKV_CONFIG");
    assert!(!env_dir.exists(), "the environment file must be ignored");
    cleanup(&env_root);
    cleanup(&cli_root);
}

#[test]
fn environment_overrides_file_data_dir() {
    let root = temp_root("env_over_file");
    let file_dir = root.join("file-data");
    let env_dir = root.join("env-data");
    let config = write_config(&root, &format!("data_dir = \"{}\"\n", toml_path(&file_dir)));

    let mut command = daemon_command();
    command
        .arg("--config")
        .arg(&config)
        .arg("--listen")
        .arg(LOOPBACK_LISTEN)
        .env("STORKV_DATA_DIR", &env_dir);
    let child = command.spawn().expect("spawn storkvd");
    let started = wait_for_path(&env_dir);
    stop(child);

    assert!(started, "the environment must win over the file");
    assert!(!file_dir.exists(), "the file value must not be used");
    cleanup(&root);
}

#[test]
fn falsey_environment_overrides_strict_admin() {
    let root = temp_root("falsey_strict_admin");
    let data_dir = root.join("data");
    let address = free_local_address();
    let config = write_config(
        &root,
        &format!(
            "data_dir = \"{}\"\nstrict_admin = true\nhttp_listen = \"{address}\"\n",
            toml_path(&data_dir)
        ),
    );

    let mut command = daemon_command();
    command
        .arg("--config")
        .arg(&config)
        .arg("--listen")
        .arg(LOOPBACK_LISTEN)
        .env("STORKV_STRICT_ADMIN", "false");
    let child = command.spawn().expect("spawn storkvd");
    wait_for_listener(&address);
    let status = admin_compact_status(&address);
    stop(child);
    cleanup(&root);

    assert_eq!(status, 200, "falsey STORKV_STRICT_ADMIN must override true");
}

#[test]
fn environment_rescues_invalid_file_listen_address() {
    let root = temp_root("rescued_env_listen");
    let data_dir = root.join("data");
    let config = write_config(
        &root,
        &format!(
            "data_dir = \"{}\"\nlisten = \"not-an-address\"\n",
            toml_path(&data_dir)
        ),
    );

    let mut command = daemon_command();
    command
        .arg("--config")
        .arg(&config)
        .env("STORKV_LISTEN", LOOPBACK_LISTEN);
    let child = command.spawn().expect("spawn storkvd");
    let started = wait_for_path(&data_dir);
    stop(child);
    cleanup(&root);

    assert!(started, "STORKV_LISTEN must rescue the invalid file value");
}
