//! chiave-tui: the full-screen ratatui front end.
//!
//! The application state ([`App`]) is deliberately free of terminal I/O: it is
//! driven by [`App::handle_key`], [`App::handle_mouse`] and [`App::tick`] and
//! rendered by [`App::draw`], so the whole UI can be exercised against
//! `ratatui::backend::TestBackend` without a tty. [`run`] is the thin shell
//! that owns the terminal and the event loop.
//!
//! Mouse events are hit-tested against [`Panes`], the same pure layout the
//! renderer uses ([`ui::layout`]), which [`App::draw`] leaves behind on the app.

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
pub use ui::Panes;

use chiave_core::SecretString;

/// Runtime knobs for the TUI.
#[derive(Debug, Clone)]
pub struct TuiOptions {
    /// How long a copied secret stays on the clipboard. `None` means "no auto-clear".
    pub clip_timeout: Option<Duration>,
    /// Lock the vault after this much time without a keypress. `None` disables it.
    pub idle_lock: Option<Duration>,
    /// Refuse every mutation, whatever the vault itself allows.
    pub read_only: bool,
    /// Capture the mouse: click to select, double-click to open, wheel to scroll.
    ///
    /// Defaults to `true`. While the mouse is captured the terminal no longer
    /// gets the drag events it uses for selecting text, so copying with the
    /// mouse needs `Shift` held down; turn this off to get the plain behaviour
    /// back.
    pub mouse: bool,
}

impl Default for TuiOptions {
    fn default() -> Self {
        TuiOptions {
            clip_timeout: None,
            idle_lock: None,
            read_only: false,
            mouse: true,
        }
    }
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
