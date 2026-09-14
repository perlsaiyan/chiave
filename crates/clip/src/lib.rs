//! chiave-clip: clipboard access for secrets.
//!
//! Contract (stable, other crates code against it):
//! - `Clipboard` is the interface the shell and TUI use.
//! - `detect()` picks the best backend for the running session.
//! - `MemoryClipboard` is for tests.
//! - The binary forwards `chiave __clip-serve ...` to `helper_main()` so a detached
//!   helper process can hold the clipboard and clear it after a timeout even after
//!   a one-shot `chiave xp` invocation has exited.
//!
//! Backends are not implemented yet; `detect()` currently returns `NullClipboard`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClipError {
    #[error("no clipboard available in this session")]
    Unavailable,
    #[error("clipboard error: {0}")]
    Backend(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub trait Clipboard: Send + Sync {
    /// Human-readable backend name for `ver`/diagnostics.
    fn name(&self) -> &'static str;

    /// Copy a secret. Backends must mark it sensitive (x-kde-passwordManagerHint) so
    /// clipboard managers skip it. `clear_after = Some(d)` clears it after `d` if the
    /// clipboard still holds this value.
    fn copy_secret(
        &self,
        secret: &SecretString,
        clear_after: Option<Duration>,
    ) -> Result<(), ClipError>;

    /// Copy non-secret text (usernames, URLs). No sensitive hint, no auto-clear.
    fn copy_text(&self, text: &str) -> Result<(), ClipError>;

    /// Clear the clipboard now.
    fn clear(&self) -> Result<(), ClipError>;
}

/// How `detect()` should behave.
#[derive(Debug, Default, Clone)]
pub struct DetectOptions {
    /// Path to the chiave executable, used to spawn the detached `__clip-serve` helper
    /// for auto-clear that outlives the caller. `None` falls back to an in-process timer.
    pub helper_exe: Option<PathBuf>,
}

/// Pick a backend for this session (Wayland, X11, or none).
pub fn detect(_opts: DetectOptions) -> Box<dyn Clipboard> {
    Box::new(NullClipboard)
}

/// Entry point for the hidden `chiave __clip-serve` subcommand. `args` excludes the
/// program name and the `__clip-serve` token. Reads the secret from stdin.
pub fn helper_main(_args: Vec<String>) -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}

/// Always fails: used when no display is available.
pub struct NullClipboard;

impl Clipboard for NullClipboard {
    fn name(&self) -> &'static str {
        "none"
    }
    fn copy_secret(&self, _: &SecretString, _: Option<Duration>) -> Result<(), ClipError> {
        Err(ClipError::Unavailable)
    }
    fn copy_text(&self, _: &str) -> Result<(), ClipError> {
        Err(ClipError::Unavailable)
    }
    fn clear(&self) -> Result<(), ClipError> {
        Err(ClipError::Unavailable)
    }
}

/// In-memory clipboard for tests. Records what was copied and the requested clear delay.
#[derive(Default)]
pub struct MemoryClipboard {
    state: Mutex<MemoryState>,
}

#[derive(Default, Clone)]
pub struct MemoryState {
    pub content: Option<String>,
    pub sensitive: bool,
    pub clear_after: Option<Duration>,
    pub clears: usize,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn state(&self) -> MemoryState {
        self.state.lock().unwrap().clone()
    }
}

impl Clipboard for MemoryClipboard {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn copy_secret(
        &self,
        secret: &SecretString,
        clear_after: Option<Duration>,
    ) -> Result<(), ClipError> {
        let mut s = self.state.lock().unwrap();
        s.content = Some(secret.expose_secret().to_string());
        s.sensitive = true;
        s.clear_after = clear_after;
        Ok(())
    }
    fn copy_text(&self, text: &str) -> Result<(), ClipError> {
        let mut s = self.state.lock().unwrap();
        s.content = Some(text.to_string());
        s.sensitive = false;
        s.clear_after = None;
        Ok(())
    }
    fn clear(&self) -> Result<(), ClipError> {
        let mut s = self.state.lock().unwrap();
        s.content = None;
        s.clears += 1;
        Ok(())
    }
}
