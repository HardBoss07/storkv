#[path = "config/daemon.rs"]
mod daemon;
#[path = "config/helpers.rs"]
mod helpers;
#[path = "config/scopes.rs"]
mod scopes;
#[path = "config/startup.rs"]
mod startup;

use std::time::Duration;

use storkv_core::{CompactionStyle, Retention};
use storkv_server::{ConfigError, FileConfig, validate_listen_address};

const EXAMPLE_CONFIG: &str = include_str!("../../storkv.example.toml");

const EXAMPLE_KEYS: [&str; 26] = [
    "data_dir",
    "listen",
    "admin_sock",
    "http_listen",
    "ws_listen",
    "grpc_listen",
    "redis_listen",
    "memcached_listen",
    "role",
    "primary_uri",
    "admin_token",
    "admin_token_file",
    "redis_password",
    "strict_admin",
    "http_admin",
    "ws_admin",
    "grpc_admin",
    "backup_dir",
    "backup_pipe_cmd",
    "backup_pipe_args",
    "backup_compression",
    "backup_interval_secs",
    "backup_retention",
    "backup_ship_wal",
    "tls_cert",
    "tls_key",
];

fn backup_command() -> clap::Command {
    clap::Command::new("storkvd")
        .arg(
            clap::Arg::new("role")
                .long("role")
                .default_value("standalone"),
        )
        .arg(
            clap::Arg::new("strict_admin")
                .long("strict_admin")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("backup_interval_secs")
                .long("backup_interval_secs")
                .default_value("3600")
                .value_parser(clap::value_parser!(u64)),
        )
        .arg(
            clap::Arg::new("backup_retention")
                .long("backup_retention")
                .default_value("7")
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(
            clap::Arg::new("backup_pipe_args")
                .long("backup_pipe_arg")
                .value_delimiter(',')
                .action(clap::ArgAction::Append),
        )
}

fn example_settings() -> Vec<(String, String)> {
    EXAMPLE_CONFIG
        .lines()
        .filter_map(|line| {
            let setting = line.strip_prefix("# ")?;
            let (key, value) = setting.split_once(" = ")?;
            let valid_key = key
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '_');
            (valid_key && !key.is_empty()).then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

#[test]
fn parses_supported_keys_and_empty_file() {
    let config = FileConfig::parse(
        r#"
        data_dir = "/var/lib/storkv"
        listen = "0.0.0.0:7878"
        role = "primary"
        strict_admin = true
        backup_pipe_args = ["--bucket", "prod"]
    "#,
    )
    .expect("parse config");
    assert_eq!(
        config.data_dir,
        Some(std::path::PathBuf::from("/var/lib/storkv"))
    );
    assert_eq!(config.listen.as_deref(), Some("0.0.0.0:7878"));
    assert_eq!(config.role.as_deref(), Some("primary"));
    assert_eq!(config.strict_admin, Some(true));
    assert_eq!(
        config.backup_pipe_args,
        Some(vec!["--bucket".to_owned(), "prod".to_owned()])
    );

    let empty = FileConfig::parse("").expect("parse empty config");
    assert!(empty.data_dir.is_none());
}

#[test]
fn rejects_unknown_keys() {
    let error = FileConfig::parse("data_dirs = \"/var/lib/storkv\"").expect_err("unknown key");
    assert!(matches!(error, ConfigError::Parse(_)));
}

#[test]
fn validates_listen_addresses() {
    validate_listen_address("--listen", "127.0.0.1:7878").expect("tcp address");
    validate_listen_address("--admin-sock", "unix:///run/storkv.sock").expect("unix address");
    assert!(validate_listen_address("--listen", "not an address").is_err());
    assert!(validate_listen_address("--admin-sock", "unix:").is_err());
}

#[test]
fn example_template_matches_config_schema() {
    let commented = FileConfig::parse(EXAMPLE_CONFIG).expect("parse commented template");
    assert!(commented.data_dir.is_none());
    assert!(
        EXAMPLE_CONFIG.contains("# [[scopes]]"),
        "template must document scope provisioning"
    );

    let settings = example_settings();
    let mut keys: Vec<&str> = settings.iter().map(|(key, _)| key.as_str()).collect();
    keys.sort_unstable();
    let mut expected = EXAMPLE_KEYS;
    expected.sort_unstable();
    assert_eq!(
        keys, expected,
        "template must list every FileConfig key once"
    );

    let uncommented = settings
        .iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let config = FileConfig::parse(&uncommented).expect("parse uncommented template");
    assert_eq!(config.strict_admin, Some(false));
    assert_eq!(config.backup_interval_secs, Some(3600));
    assert_eq!(config.backup_retention, Some(7));
    assert_eq!(
        config.backup_pipe_args,
        Some(vec!["--bucket".to_owned(), "prod-storkv".to_owned()])
    );

    let scopes = FileConfig::parse(&helpers::uncommented_scopes(EXAMPLE_CONFIG))
        .expect("parse uncommented scope template");
    let options = scopes.scope_options().expect("scope options");
    assert_eq!(options.len(), 1);
    let (name, scope) = &options[0];
    assert_eq!(name, "sessions");
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
    assert_eq!(scope.prefix_bloom_len, Some(1024));
}

#[test]
fn file_values_lose_to_command_line() {
    let file = FileConfig::parse("listen = \"0.0.0.0:9000\"\nstrict_admin = true\n")
        .expect("parse config");
    let command = clap::Command::new("storkvd")
        .arg(
            clap::Arg::new("listen")
                .long("listen")
                .default_value("127.0.0.1:7878"),
        )
        .arg(
            clap::Arg::new("strict_admin")
                .long("strict_admin")
                .action(clap::ArgAction::SetTrue),
        );

    let defaults = file.apply_to(command.clone()).get_matches_from(["storkvd"]);
    assert_eq!(
        defaults.get_one::<String>("listen").map(String::as_str),
        Some("0.0.0.0:9000")
    );
    assert!(defaults.get_flag("strict_admin"));

    let cli = file
        .apply_to(command)
        .get_matches_from(["storkvd", "--listen", "127.0.0.1:9999"]);
    assert_eq!(
        cli.get_one::<String>("listen").map(String::as_str),
        Some("127.0.0.1:9999")
    );
}

#[test]
fn file_values_override_built_in_defaults() {
    let file = FileConfig::parse(
        r#"
        role = "primary"
        strict_admin = true
        backup_interval_secs = 42
        backup_retention = 3
        backup_pipe_args = ["--bucket", "prod"]
    "#,
    )
    .expect("parse config");
    let matches = file
        .apply_to(backup_command())
        .get_matches_from(["storkvd"]);

    assert_eq!(
        matches.get_one::<String>("role").map(String::as_str),
        Some("primary")
    );
    assert!(matches.get_flag("strict_admin"));
    assert_eq!(matches.get_one::<u64>("backup_interval_secs"), Some(&42));
    assert_eq!(matches.get_one::<usize>("backup_retention"), Some(&3));
    let pipe_args: Vec<&String> = matches
        .get_many::<String>("backup_pipe_args")
        .expect("pipe arguments")
        .collect();
    assert_eq!(pipe_args, ["--bucket", "prod"]);
}

#[test]
fn empty_pipe_arguments_stay_empty() {
    let file = FileConfig::parse("backup_pipe_args = []").expect("parse config");
    let matches = file
        .apply_to(backup_command())
        .get_matches_from(["storkvd"]);
    assert!(
        matches.get_many::<String>("backup_pipe_args").is_none(),
        "an empty array must not create an empty argument"
    );
}
