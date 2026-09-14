//! chiave-tui: the full-screen ratatui front end.
//!
//! The application state ([`App`]) is deliberately free of terminal I/O: it is
//! driven by [`App::handle_key`] and [`App::tick`] and rendered by
//! [`App::draw`], so the whole UI can be exercised against
//! `ratatui::backend::TestBackend` without a tty. [`run`] is the thin shell
//! that owns the terminal and the event loop.

pub mod app;
pub mod dialog;
pub mod form;
pub mod search;
pub mod terminal;
pub mod tree;
pub mod ui;

use std::time::Duration;

pub use app::{App, Focus, Mode};
pub use dialog::{Dialog, DialogAction};
pub use form::{FormAction, FormState};
pub use terminal::run;

use chiave_core::SecretString;

/// Runtime knobs for the TUI.
#[derive(Debug, Clone, Default)]
pub struct TuiOptions {
    /// How long a copied secret stays on the clipboard. `None` means "no auto-clear".
    pub clip_timeout: Option<Duration>,
    /// Lock the vault after this much time without a keypress. `None` disables it.
    pub idle_lock: Option<Duration>,
    /// Refuse every mutation, whatever the vault itself allows.
    pub read_only: bool,
}

/// A way to ask for the master password outside the TUI.
///
/// The TUI renders its own unlock screen, so this is only a fallback: it is used
/// when the user asks for it explicitly (`Ctrl-p` on the unlock screen), which
/// leaves the alternate screen, prompts on the real terminal and comes back.
pub trait PasswordPrompt {
    fn prompt(&self, msg: &str) -> std::io::Result<SecretString>;
}

/// A [`PasswordPrompt`] that always fails; useful in tests and when there is no tty.
pub struct NoPrompt;

impl PasswordPrompt for NoPrompt {
    fn prompt(&self, _msg: &str) -> std::io::Result<SecretString> {
        Err(std::io::Error::other("no password prompt available"))
    }
}
