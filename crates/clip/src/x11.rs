//! X11 backend: shells out to `xclip`, falling back to `xsel`.
//!
//! X11 selections carry no equivalent of `x-kde-passwordManagerHint` that these
//! tools can offer alongside the text targets (one `-t` per invocation), so on X11
//! the auto-clear timer is the only protection against clipboard managers.

use zeroize::Zeroizing;

use crate::clear::ReadClear;
use crate::command::{capture, have, run_with_stdin};
use crate::offers::MimeOffer;
use crate::raw::{non_empty, RawBackend, Serving};
use crate::ClipError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum X11Tool {
    Xclip,
    Xsel,
}

impl X11Tool {
    pub(crate) fn detect() -> Option<X11Tool> {
        if have("xclip") {
            Some(X11Tool::Xclip)
        } else if have("xsel") {
            Some(X11Tool::Xsel)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct X11Raw {
    tool: Option<X11Tool>,
}

impl X11Raw {
    pub(crate) fn new() -> Self {
        X11Raw {
            tool: X11Tool::detect(),
        }
    }

    fn tool(&self) -> Result<X11Tool, ClipError> {
        self.tool.ok_or_else(|| {
            ClipError::Backend("x11: neither xclip nor xsel is installed (install xclip)".into())
        })
    }
}

/// Argument vector for a copy. Pure, so it can be unit-tested.
fn copy_args(tool: X11Tool, paste_once: bool) -> Vec<String> {
    match tool {
        X11Tool::Xclip => {
            let mut args = vec![
                "-selection".to_string(),
                "clipboard".to_string(),
                "-in".to_string(),
            ];
            if paste_once {
                // xclip serves the selection `-loops` times, then exits.
                args.push("-loops".to_string());
                args.push("1".to_string());
            }
            args
        }
        // xsel has no paste-once equivalent; the flag is silently ignored.
        X11Tool::Xsel => vec!["--clipboard".to_string(), "--input".to_string()],
    }
}

fn read_args(tool: X11Tool) -> Vec<String> {
    match tool {
        X11Tool::Xclip => vec![
            "-selection".to_string(),
            "clipboard".to_string(),
            "-out".to_string(),
        ],
        X11Tool::Xsel => vec!["--clipboard".to_string(), "--output".to_string()],
    }
}

fn tool_name(tool: X11Tool) -> &'static str {
    match tool {
        X11Tool::Xclip => "xclip",
        X11Tool::Xsel => "xsel",
    }
}

impl RawBackend for X11Raw {
    fn copy(
        &self,
        _offers: &[MimeOffer],
        content: &[u8],
        paste_once: bool,
    ) -> Result<Serving, ClipError> {
        let tool = self.tool()?;
        run_with_stdin(tool_name(tool), &copy_args(tool, paste_once), content)?;
        // xclip/xsel fork their own selection owner, so the content outlives us.
        Ok(Serving::Detached)
    }

    fn display_name(&self) -> &'static str {
        "x11"
    }

    /// X11 selection targets cannot carry the KDE hint alongside the text
    /// targets through xclip/xsel, so secrets are visible to clipboard managers.
    fn supports_sensitive_hint(&self) -> bool {
        false
    }
}

impl ReadClear for X11Raw {
    fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        let tool = self.tool()?;
        // An empty X11 clipboard makes xclip exit non-zero with a "target STRING
        // not available" message; treat that as "empty" rather than an error.
        match capture(tool_name(tool), &read_args(tool)) {
            Ok(mut bytes) => Ok(non_empty(std::mem::take(&mut bytes))),
            Err(ClipError::Backend(msg)) if msg.contains("not available") => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn clear_now(&self) -> Result<(), ClipError> {
        let tool = self.tool()?;
        // Copying an empty string is how xclip/xsel "clear".
        run_with_stdin(tool_name(tool), &copy_args(tool, false), b"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xclip_copy_targets_the_clipboard_selection() {
        assert_eq!(
            copy_args(X11Tool::Xclip, false),
            vec!["-selection", "clipboard", "-in"]
        );
    }

    #[test]
    fn xclip_paste_once_uses_loops_one() {
        assert_eq!(
            copy_args(X11Tool::Xclip, true),
            vec!["-selection", "clipboard", "-in", "-loops", "1"]
        );
    }

    #[test]
    fn xsel_ignores_paste_once() {
        assert_eq!(
            copy_args(X11Tool::Xsel, true),
            copy_args(X11Tool::Xsel, false)
        );
    }

    #[test]
    fn read_args_ask_for_output() {
        assert_eq!(
            read_args(X11Tool::Xclip),
            vec!["-selection", "clipboard", "-out"]
        );
        assert_eq!(read_args(X11Tool::Xsel), vec!["--clipboard", "--output"]);
    }

    #[test]
    fn missing_tools_produce_a_helpful_error() {
        let raw = X11Raw { tool: None };
        let err = raw.clear_now().unwrap_err();
        assert!(err.to_string().contains("xclip"), "{err}");
    }
}
