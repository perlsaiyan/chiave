//! Shared plumbing for backends that shell out to a clipboard tool.

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

use zeroize::Zeroizing;

use crate::ClipError;

/// Is `tool` on `PATH`?
pub(crate) fn have(tool: &str) -> bool {
    which::which(tool).is_ok()
}

/// Run `tool` with `args`, feeding `input` on stdin, and wait for it to exit.
///
/// `xclip`, `xsel` and `wl-copy` all fork a background process that owns the
/// selection and then exit in the foreground, so waiting here does not block for
/// the lifetime of the clipboard content.
pub(crate) fn run_with_stdin<S: AsRef<OsStr>>(
    tool: &str,
    args: &[S],
    input: &[u8],
) -> Result<(), ClipError> {
    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| ClipError::Backend(format!("{tool}: {err}")))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClipError::Backend(format!("{tool}: no stdin")))?;
        stdin.write_all(input)?;
        stdin.flush()?;
    }

    let output = child
        .wait_with_output()
        .map_err(|err| ClipError::Backend(format!("{tool}: {err}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        Err(ClipError::Backend(if detail.is_empty() {
            format!("{tool} exited with {}", output.status)
        } else {
            format!("{tool}: {detail}")
        }))
    }
}

/// Run `tool` with `args` and capture stdout into a zeroizing buffer.
pub(crate) fn capture<S: AsRef<OsStr>>(
    tool: &str,
    args: &[S],
) -> Result<Zeroizing<Vec<u8>>, ClipError> {
    let output = Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| ClipError::Backend(format!("{tool}: {err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(ClipError::Backend(if detail.is_empty() {
            format!("{tool} exited with {}", output.status)
        } else {
            format!("{tool}: {detail}")
        }));
    }
    Ok(Zeroizing::new(output.stdout))
}
