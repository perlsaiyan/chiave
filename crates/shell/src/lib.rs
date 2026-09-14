//! chiave-shell: the kpcli-compatible command layer.
//!
//! The same [`Command`] table drives the interactive REPL ([`repl`]), the
//! `--command` batch mode ([`run_commands`]) and the one-shot subcommands of the
//! `chiave` binary. [`Shell::run_line`] is the single entry point; it writes
//! everything, errors included, to a caller-supplied sink so tests can drive it
//! with a `Vec<u8>`.

pub mod command;
pub mod complete;
pub mod format;
pub mod prompt;
pub mod repl;
pub mod shell;

pub use command::{command_names, help_all, help_for, parse_line, Command, ParseLineError};
pub use complete::complete;
pub use prompt::{FixedPrompt, PasswordPrompt, RpasswordPrompt};
pub use repl::repl;
pub use shell::{default_histfile, run_commands, Flow, Shell, ShellError, ShellOptions};
