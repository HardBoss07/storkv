use std::time::Duration;

use storkv_core::{CompactionStyle, Durability, Retention, StorageMode};
use storkv_server::{ConfigError, FileConfig};

use super::helpers::{
    cleanup, daemon_command, free_local_address, http_get, http_put, spawn_with_config, stop,
    temp_root, toml_path, wait_for_listener, write_config,
};

#[test]
fn parses_full_scope_entry() {
    let config = FileConfig::parse(
        r#"
        [[scopes]]
        name = "sessions"
        durability = "buffered"
        retention = "ttl"
        default_ttl_ms = 3600000
        min_ttl_ms = 1000
        max_ttl_ms = 86400000
        allow_permanent = false
        active_ttl_tracking = true
        storage_mode = "in_memory_persistent"
        prefix_bloom_len = 1024
    "#,
    )
    .expect("parse scope config");
    let options = config.scope_options().expect("scope options");
    assert_eq!(options.len(), 1);
    let (name, scope) = &options[0];
    assert_eq!(name, "sessions");
    assert_eq!(
        scope.durability,
        Durability::Buffered {
            sync_interval: Duration::from_millis(1000)
        }
    );
    assert_eq!(
        scope.retention,
        Retention::Ttl {
            default_ttl: Duration::from_millis(3_600_000),
            compaction_style: CompactionStyle::Leveled
        }
    );
    assert_eq!(scope.min_ttl, Some(Duration::from_millis(1000)));
    assert_eq!(scope.max_ttl, Some(Duration::from_millis(86_400_000)));
    assert!(!scope.allow_permanent);
    assert!(scope.active_ttl_tracking);
    assert_eq!(scope.storage_mode, StorageMode::InMemoryPersistent);
    assert_eq!(scope.prefix_bloom_len, Some(1024));
}

#[test]
fn scope_entry_defaults_to_strict_permanent_disk() {
    let config = FileConfig::parse("[[scopes]]\nname = \"users\"\n").expect("parse scope");
    let options = config.scope_options().expect("scope options");
    let (name, scope) = &options[0];
    assert_eq!(name, "users");
    assert_eq!(scope.durability, Durability::StrictFsync);
    assert_eq!(scope.retention, Retention::Permanent);
    assert_eq!(scope.storage_mode, StorageMode::Disk);
    assert!(scope.allow_permanent);
    assert!(!scope.active_ttl_tracking);
}

#[test]
fn rejects_invalid_scope_entries() {
    let cases = [
        "[[scopes]]\nname = \"\"\n",
        "[[scopes]]\nname = \"x\"\ndurability = \"fast\"\n",
        "[[scopes]]\nname = \"x\"\nretention = \"forever\"\n",
        "[[scopes]]\nname = \"x\"\nretention = \"ttl\"\n",
        "[[scopes]]\nname = \"x\"\ndefault_ttl_ms = 1000\n",
        "[[scopes]]\nname = \"x\"\nretention = \"ttl\"\ndefault_ttl_ms = 10\nmin_ttl_ms = 100\n",
        "[[scopes]]\nname = \"x\"\nretention = \"ttl\"\ndefault_ttl_ms = 100\nmax_ttl_ms = 10\n",
        "[[scopes]]\nname = \"x\"\nmin_ttl_ms = 100\nmax_ttl_ms = 10\n",
        "[[scopes]]\nname = \"x\"\nstorage_mode = \"tape\"\n",
        "[[scopes]]\nname = \"x\"\nunknown = true\n",
        "[[scopes]]\nname = \"x\"\nmin_ttl_ms = 0\n",
        "[[scopes]]\nname = \"x\"\nmax_ttl_ms = 0\n",
        "[[scopes]]\nname = \"x\"\nprefix_bloom_len = 0\n",
    ];
    for source in cases {
        let config = match FileConfig::parse(source) {
            Ok(config) => config,
            Err(error @ ConfigError::Parse(_)) => {
                assert!(error.to_string().contains("cannot parse"), "{error}");
                continue;
            }
            Err(error) => panic!("unexpected parse error for {source}: {error}"),
        };
        let error = config.scope_options().expect_err("invalid scope");
        assert!(
            matches!(error, ConfigError::Scope { .. }),
            "unexpected error for {source}: {error}"
        );
    }
}

#[test]
fn rejects_duplicate_scope_names() {
    let config = FileConfig::parse("[[scopes]]\nname = \"a\"\n\n[[scopes]]\nname = \" a \"\n")
        .expect("parse scope config");
    let error = config.scope_options().expect_err("duplicate scope");
    assert!(matches!(error, ConfigError::Scope { .. }));
}

#[test]
fn daemon_provisions_scopes_before_traffic() {
    let root = temp_root("scope_provision");
    let data_dir = root.join("data");
    let address = free_local_address();
    let config = write_config(
        &root,
        &format!(
            "data_dir = \"{}\"\nhttp_listen = \"{address}\"\n\
             [[scopes]]\nname = \"guarded\"\nmin_ttl_ms = 1000\n\
             max_ttl_ms = 60000\nallow_permanent = false\n",
            toml_path(&data_dir)
        ),
    );

    let child = spawn_with_config(&config);
    wait_for_listener(&address);
    let missing_ttl = http_put(&address, "/v1/scopes/guarded/keys/k", "v");
    let too_short = http_put(&address, "/v1/scopes/guarded/keys/k?ttl_ms=10", "v");
    let accepted = http_put(&address, "/v1/scopes/guarded/keys/k?ttl_ms=5000", "v");
    stop(child);

    assert_eq!(missing_ttl, 400, "permanent writes must be rejected");
    assert_eq!(too_short, 400, "short TTLs must be rejected");
    assert_eq!(accepted, 200, "valid TTL writes must succeed");
    cleanup(&root);
}

#[test]
fn daemon_restart_keeps_data_and_declared_guardrails() {
    let root = temp_root("scope_guardrails_restart");
    let data_dir = root.join("data");
    let address = free_local_address();
    let config = write_config(
        &root,
        &format!(
            "data_dir = \"{}\"\nhttp_listen = \"{address}\"\n\
             [[scopes]]\nname = \"guarded\"\nmin_ttl_ms = 1000\n\
             max_ttl_ms = 60000\nallow_permanent = false\n",
            toml_path(&data_dir)
        ),
    );

    let first = spawn_with_config(&config);
    wait_for_listener(&address);
    let written = http_put(&address, "/v1/scopes/guarded/keys/k?ttl_ms=5000", "v");
    stop(first);

    let second = spawn_with_config(&config);
    wait_for_listener(&address);
    let existing = http_get(&address, "/v1/scopes/guarded/keys/k");
    let permanent = http_put(&address, "/v1/scopes/guarded/keys/permanent", "v");
    let too_short = http_put(&address, "/v1/scopes/guarded/keys/short?ttl_ms=10", "v");
    let accepted = http_put(&address, "/v1/scopes/guarded/keys/second?ttl_ms=5000", "v");
    stop(second);

    assert_eq!(
        written, 200,
        "the first daemon must accept a valid TTL write"
    );
    assert_eq!(existing.0, 200, "existing data must stay readable");
    assert_eq!(existing.1, "v", "existing data must keep its value");
    assert_eq!(permanent, 400, "permanent writes must stay rejected");
    assert_eq!(too_short, 400, "short TTLs must stay rejected");
    assert_eq!(accepted, 200, "valid TTL writes must still succeed");
    cleanup(&root);
}

#[test]
fn daemon_startup_fails_on_invalid_scope_entry() {
    let root = temp_root("scope_invalid_startup");
    let data_dir = root.join("data");
    let config = write_config(
        &root,
        &format!(
            "data_dir = \"{}\"\nlisten = \"127.0.0.1:0\"\n\
             [[scopes]]\nname = \"broken\"\nmin_ttl_ms = 0\n",
            toml_path(&data_dir)
        ),
    );

    let output = daemon_command()
        .arg("--config")
        .arg(&config)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run storkvd");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "startup must fail: {stderr}");
    assert!(
        stderr.contains("Scope {") && stderr.contains("broken"),
        "startup must report the invalid scope: {stderr}"
    );
    assert!(
        stderr.contains("min_ttl_ms must be greater than zero"),
        "startup must report the zero bound: {stderr}"
    );
    cleanup(&root);
}
