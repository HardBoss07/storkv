//! Language binding tests for the C ABI.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{Result, XtaskError};
use crate::test_runner;
use crate::tools;

use languages::{executable, run_c, run_cpp, run_java, run_python, run_typescript};

mod languages;

const SERVER_ADDR: &str = "127.0.0.1:7888";
const LIBRARY_NAMES: [&str; 3] = ["storkv_ffi.dll", "libstorkv_ffi.so", "libstorkv_ffi.dylib"];

/// Shared settings for one binding test run.
pub(super) struct Context {
    pub(super) root: PathBuf,
    pub(super) library: PathBuf,
    pub(super) uri: Option<String>,
}

impl Context {
    pub(super) fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.root)
            .env("STORKV_FFI_LIB", &self.library);
        if let Some(uri) = &self.uri {
            command.env("STORKV_TEST_URI", uri);
        }
        command
    }
}

/// Run every binding test that the host can build.
///
/// # Errors
///
/// Returns an error when the FFI library or one available test fails.
pub fn run(root: &Path, live_uri: Option<&str>, with_server: bool) -> Result<()> {
    test_runner::cargo(
        root,
        &["build", "-p", "storkv-ffi"],
        "cargo build -p storkv-ffi",
    )?;
    let library = library_path(root)?;
    let server = if live_uri.is_none() && with_server {
        Some(ServerGuard::start(root)?)
    } else {
        None
    };
    let context = Context {
        root: root.to_path_buf(),
        library,
        uri: live_uri
            .map(str::to_string)
            .or_else(|| server.as_ref().map(ServerGuard::uri)),
    };
    run_c(&context)?;
    run_cpp(&context)?;
    run_python(&context)?;
    run_java(&context)?;
    run_typescript(&context)?;
    drop(server);
    Ok(())
}

fn library_path(root: &Path) -> Result<PathBuf> {
    let directory = root.join("target").join("debug");
    for name in LIBRARY_NAMES {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(XtaskError::Layout(format!(
        "no FFI library in {}",
        directory.display()
    )))
}

/// Temporary `storkvd` process for the `--with-server` option.
struct ServerGuard {
    child: Child,
}

impl ServerGuard {
    fn start(root: &Path) -> Result<Self> {
        test_runner::cargo(
            root,
            &["build", "-p", "storkv-server", "--bin", "storkvd"],
            "cargo build storkvd",
        )?;
        let data_dir = root.join("target").join("sandbox").join("bindings-server");
        tools::reset_dir(&data_dir)?;
        let binary = root
            .join("target")
            .join("debug")
            .join(executable("storkvd"));
        let child = Command::new(binary)
            .arg("--data-dir")
            .arg(&data_dir)
            .arg("--listen")
            .arg(SERVER_ADDR)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut guard = Self { child };
        guard.wait_ready()?;
        Ok(guard)
    }

    fn uri(&self) -> String {
        format!("storkv://{SERVER_ADDR}")
    }

    fn wait_ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                return Err(XtaskError::Layout(
                    "storkvd exited before it was ready".into(),
                ));
            }
            if TcpStream::connect(SERVER_ADDR).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(XtaskError::Layout(format!(
            "storkvd did not listen on {SERVER_ADDR}"
        )))
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
