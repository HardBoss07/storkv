//! Command line parsing for xtask.

use crate::error::{Result, XtaskError};
use crate::test_runner::Tier;

/// Command selected on the command line.
pub enum Action {
    /// Build the workspace and package a release.
    Dist(DistOptions),
    /// Run one test tier.
    Test(TestOptions),
    /// Start a local server and keep it in the foreground.
    Sandbox(SandboxOptions),
    /// Print the usage text.
    Help,
    /// Regenerate the C header and fail when it changes.
    VerifyHeaders,
}

/// Options for the `dist` command.
#[derive(Default)]
pub struct DistOptions {
    /// Target triple. Uses the host triple when omitted.
    pub target: Option<String>,
    /// Fail when any SDK build fails.
    pub strict_sdks: bool,
    /// Skip archive and checksum creation.
    pub skip_archive: bool,
}

/// Options for the `test` command.
pub struct TestOptions {
    /// Tier of checks to run.
    pub tier: Tier,
    /// Live server URI for the `bindings` tier.
    pub live_uri: Option<String>,
    /// Start a local server for the `bindings` tier.
    pub with_server: bool,
}

/// Options for the `sandbox` command.
#[derive(Default)]
pub struct SandboxOptions {
    /// Generate a self-signed certificate and key.
    pub tls: bool,
    /// Shared admin token for the auxiliary transports.
    pub admin_token: Option<String>,
    /// Reject admin-only requests on non-admin connections.
    pub strict_admin: bool,
}

/// Parse command line arguments into one action.
///
/// # Errors
///
/// Returns an error for an unknown command or option.
pub fn parse(arguments: impl Iterator<Item = String>) -> Result<Action> {
    let arguments: Vec<String> = arguments.collect();
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return Ok(Action::Help);
    }
    let mut arguments = arguments.into_iter();
    let command = arguments
        .next()
        .ok_or_else(|| XtaskError::InvalidArgument("missing command".into()))?;
    match command.as_str() {
        "dist" => parse_dist(arguments).map(Action::Dist),
        "test" => parse_test(arguments).map(Action::Test),
        "sandbox" => parse_sandbox(arguments).map(Action::Sandbox),
        "verify-headers" => Ok(Action::VerifyHeaders),
        other => Err(XtaskError::InvalidArgument(format!(
            "unknown command `{other}`"
        ))),
    }
}

/// Return the usage text for every command.
pub fn usage() -> &'static str {
    "usage: cargo xtask <command>\n\n\
     Commands:\n\
     \x20 dist [--target <TRIPLE>] [--strict-sdks] [--skip-archive]\n\
     \x20 test <fast|pr|full|bindings> [--live-uri <URI>] [--with-server]\n\
     \x20 sandbox [--tls] [--admin-token <TOKEN>] [--strict-admin]\n\
     \x20 verify-headers\n"
}

fn parse_dist(mut arguments: impl Iterator<Item = String>) -> Result<DistOptions> {
    let mut options = DistOptions::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--target" => options.target = Some(required_value(&mut arguments, "--target")?),
            "--strict-sdks" => options.strict_sdks = true,
            "--skip-archive" => options.skip_archive = true,
            other => {
                return Err(XtaskError::InvalidArgument(format!(
                    "unknown dist option `{other}`"
                )));
            }
        }
    }
    Ok(options)
}

fn parse_test(mut arguments: impl Iterator<Item = String>) -> Result<TestOptions> {
    let tier = parse_tier(arguments.next())?;
    let mut live_uri = None;
    let mut with_server = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--live-uri" => live_uri = Some(required_value(&mut arguments, "--live-uri")?),
            "--with-server" => with_server = true,
            other => {
                return Err(XtaskError::InvalidArgument(format!(
                    "unknown test option `{other}`"
                )));
            }
        }
    }
    if tier != Tier::Bindings && (live_uri.is_some() || with_server) {
        return Err(XtaskError::InvalidArgument(
            "--live-uri and --with-server apply to `test bindings` only".into(),
        ));
    }
    Ok(TestOptions {
        tier,
        live_uri,
        with_server,
    })
}

fn parse_tier(value: Option<String>) -> Result<Tier> {
    match value.as_deref() {
        Some("fast") => Ok(Tier::Fast),
        Some("pr") => Ok(Tier::Pr),
        Some("full") => Ok(Tier::Full),
        Some("bindings") => Ok(Tier::Bindings),
        Some(other) => Err(XtaskError::InvalidArgument(format!(
            "unknown test tier `{other}`"
        ))),
        None => Err(XtaskError::InvalidArgument(
            "missing test tier: fast, pr, full, or bindings".into(),
        )),
    }
}

fn parse_sandbox(mut arguments: impl Iterator<Item = String>) -> Result<SandboxOptions> {
    let mut options = SandboxOptions::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--tls" => options.tls = true,
            "--strict-admin" => options.strict_admin = true,
            "--admin-token" => {
                options.admin_token = Some(required_value(&mut arguments, "--admin-token")?)
            }
            other => {
                return Err(XtaskError::InvalidArgument(format!(
                    "unknown sandbox option `{other}`"
                )));
            }
        }
    }
    Ok(options)
}

fn required_value(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<String> {
    let value = arguments
        .next()
        .ok_or_else(|| XtaskError::InvalidArgument(format!("{option} requires a value")))?;
    if value.trim().is_empty() {
        return Err(XtaskError::InvalidArgument(format!("{option} is empty")));
    }
    Ok(value)
}
