//! Fallback Wayland backend that shells out to the `wl-copy`/`wl-paste` binaries.
//!
//! Used when the compositor does not implement `ext-data-control`/`wlr-data-control`
//! (so the native path cannot work) but `wl-copy` is installed, and when
//! `CHIAVE_CLIPBOARD=wl-copy` is set.
//!
//! **Reduced protection.** `wl-copy` offers exactly one MIME type per invocation
//! (`--type`), and a second invocation replaces the selection rather than adding to
//! it, so `x-kde-passwordManagerHint` cannot be offered next to the text types from
//! a single process. This backend therefore copies as `text/plain;charset=utf-8`
//! only: clipboard managers *will* record the secret. `supports_sensitive_hint()`
//! returns false so callers can warn, and auto-clear still applies.

use zeroize::Zeroizing;

use crate::clear::ReadClear;
use crate::command::{capture, have, run_with_stdin};
use crate::offers::{primary_text_mime, MimeOffer};
use crate::raw::{non_empty, RawBackend, Serving};
use crate::ClipError;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WlCopyRaw;

pub(crate) fn available() -> bool {
    have("wl-copy")
}

fn copy_args(paste_once: bool) -> Vec<String> {
    let mut args = vec!["--type".to_string(), primary_text_mime().to_string()];
    if paste_once {
        args.push("--paste-once".to_string());
    }
    args
}

impl RawBackend for WlCopyRaw {
    fn copy(
        &self,
        _offers: &[MimeOffer],
        content: &[u8],
        paste_once: bool,
    ) -> Result<Serving, ClipError> {
        run_with_stdin("wl-copy", &copy_args(paste_once), content)?;
        // wl-copy forks a background process to serve paste requests.
        Ok(Serving::Detached)
    }

    fn display_name(&self) -> &'static str {
        "wl-copy"
    }

    /// Only one MIME type per `wl-copy` invocation: the hint cannot be offered.
    fn supports_sensitive_hint(&self) -> bool {
        false
    }
}

impl ReadClear for WlCopyRaw {
    fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        if !have("wl-paste") {
            return Err(ClipError::Backend(
                "wl-copy backend: wl-paste is not installed, cannot verify the clipboard".into(),
            ));
        }
        match capture("wl-paste", &["--no-newline"]) {
            Ok(mut bytes) => Ok(non_empty(std::mem::take(&mut bytes))),
            // wl-paste exits 1 with this message on an empty clipboard.
            Err(ClipError::Backend(msg))
                if msg.contains("clipboard is empty") || msg.contains("No suitable type") =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    fn clear_now(&self) -> Result<(), ClipError> {
        run_with_stdin("wl-copy", &["--clear".to_string()], b"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_as_utf8_plain_text() {
        assert_eq!(copy_args(false), vec!["--type", "text/plain;charset=utf-8"]);
    }

    #[test]
    fn paste_once_is_forwarded() {
        assert_eq!(
            copy_args(true),
            vec!["--type", "text/plain;charset=utf-8", "--paste-once"]
        );
    }
}
