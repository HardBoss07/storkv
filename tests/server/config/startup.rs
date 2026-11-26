use super::helpers::{
    LOOPBACK_LISTEN, cleanup, daemon_command, spawn_with_config, stop, temp_root, toml_path,
    wait_for_path, write_config,
};

#[test]
fn missing_config_file_fails_fast() {
    let root = temp_root("missing_config");
    let path = root.join("absent.toml");
    let output = daemon_command()
        .arg("--config")
        .arg(&path)
        .arg("--listen")
        .arg(LOOPBACK_LISTEN)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run storkvd");
    cleanup(&root);

    assert!(!output.status.success(), "startup must fail fast");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Read {"), "unexpected error: {stderr}");
}

#[test]
fn malformed_config_file_fails_fast() {
    let root = temp_root("malformed_config");
    let config = write_config(&root, "data_dir = [\n");
    let output = daemon_command()
        .arg("--config")
        .arg(&config)
        .arg("--listen")
        .arg(LOOPBACK_LISTEN)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run storkvd");
    cleanup(&root);

    assert!(!output.status.success(), "startup must fail fast");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Parse("), "unexpected error: {stderr}");
}

#[test]
fn invalid_native_listen_address_fails_fast() {
    let root = temp_root("invalid_native_listen");
    let config = write_config(&root, "listen = \"not-an-address\"\n");
    let output = daemon_command()
        .arg("--config")
        .arg(&config)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run storkvd");
    cleanup(&root);

    assert!(!output.status.success(), "startup must fail fast");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Listen {") && stderr.contains("not-an-address"),
        "unexpected error: {stderr}"
    );
}

#[test]
fn invalid_http_listen_address_fails_fast() {
    let root = temp_root("invalid_http_listen");
    let config = write_config(&root, "http_listen = \"bad:addr\"\n");
    let output = daemon_command()
        .arg("--config")
        .arg(&config)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run storkvd");
    cleanup(&root);

    assert!(!output.status.success(), "startup must fail fast");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Listen {") && stderr.contains("bad:addr"),
        "unexpected error: {stderr}"
    );
}

#[test]
fn command_line_rescues_invalid_file_listen_address() {
    let root = temp_root("rescued_listen");
    let data_dir = root.join("data");
    let config = write_config(
        &root,
        &format!(
            "data_dir = \"{}\"\nlisten = \"not-an-address\"\n",
            toml_path(&data_dir)
        ),
    );

    let child = spawn_with_config(&config);
    let started = wait_for_path(&data_dir);
    stop(child);
    cleanup(&root);

    assert!(
        started,
        "a higher-precedence listen address must rescue the file value"
    );
}
