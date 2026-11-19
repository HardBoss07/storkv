//! Child-process execution for a command-pipe backup.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::{Result, StorkvError};

use super::CommandPipeSink;

pub(super) fn run_child(sink: &CommandPipeSink, archive: &[u8]) -> Result<()> {
    let mut child = Command::new(&sink.config.program)
        .args(&sink.config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| StorkvError::BackupSink {
            sink: "command-pipe".into(),
            detail: format!("failed to spawn `{}`: {error}", sink.config.program),
        })?;

    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| StorkvError::BackupSink {
            sink: "command-pipe".into(),
            detail: "child stdin is unavailable".into(),
        })
        .and_then(|mut stdin| {
            stdin
                .write_all(archive)
                .map_err(|error| StorkvError::BackupSink {
                    sink: "command-pipe".into(),
                    detail: format!("write to child stdin failed: {error}"),
                })
        });

    let output = child
        .wait_with_output()
        .map_err(|error| StorkvError::BackupSink {
            sink: "command-pipe".into(),
            detail: format!("failed to wait for child: {error}"),
        })?;
    write_result?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(StorkvError::BackupSink {
            sink: "command-pipe".into(),
            detail: format!("command exited with {}: {}", output.status, stderr.trim()),
        });
    }
    Ok(())
}
