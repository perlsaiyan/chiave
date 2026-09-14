//! The kpcli-compatible command table.
//!
//! One [`Command`] enum drives both front ends: the REPL parses a typed line with
//! clap in multicall style (the command name is `argv[0]`), and the `chiave`
//! binary embeds the very same enum as its subcommand list.

use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

/// A single shell command.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Open a database (interactive shell only)
    Open {
        /// Path to the .kdbx file
        file: PathBuf,
        /// Optional key file
        keyfile: Option<PathBuf>,
    },
    /// Close the database and forget it
    Close,
    /// List groups and entries
    #[command(alias = "dir")]
    Ls {
        /// Groups to list; defaults to the current group
        paths: Vec<String>,
    },
    /// Change the current group
    #[command(alias = "chdir")]
    Cd {
        /// Group to change to; defaults to the root group
        path: Option<String>,
    },
    /// Change the current group and list it
    Cl {
        /// Group to change to
        path: String,
    },
    /// Print the current group
    Pwd,
    /// Show one entry
    Show {
        /// Reveal the password and protected fields
        #[arg(short = 'f', long = "full")]
        full: bool,
        /// Also show times, icon, history count and UUID
        #[arg(short = 'a', long = "all")]
        all: bool,
        /// Entry path, title or number from the last listing
        spec: String,
    },
    /// Print one raw field value (scripting escape hatch; prints secrets)
    Get {
        /// Entry path, title or number from the last listing
        spec: String,
        /// title, username|uname|user, password|pass, url, notes|comments, otp,
        /// or the name of a custom field
        field: String,
    },
    /// Search entries; the hits become the numbered listing
    Find {
        /// Search every field, not just the title
        #[arg(short = 'a', long = "all")]
        all_fields: bool,
        /// Only report expired entries (also spelled -expired)
        #[arg(long = "expired")]
        expired: bool,
        /// Substring to look for
        #[arg(default_value = "")]
        query: String,
    },
    /// Print the current one-time code for an entry
    Otp {
        /// Entry path, title or number from the last listing
        spec: String,
    },
    /// Copy the username to the clipboard
    Xu { spec: String },
    /// Copy the URL to the clipboard
    Xw { spec: String },
    /// Copy the password to the clipboard
    Xp { spec: String },
    /// Copy the one-time code to the clipboard
    Xo { spec: String },
    /// Copy the password, wait in the foreground, then clear the clipboard
    Xpx { spec: String },
    /// Clear the clipboard now
    Xx,
    /// Print statistics about the open database
    Stats,
    /// Print version information
    #[command(alias = "version")]
    Ver,
    /// Clear the screen
    #[command(alias = "clear")]
    Cls,
    /// Show the command history
    History {
        /// Forget the history
        #[arg(short = 'c', long = "clear")]
        clear: bool,
    },
    /// Lock the database, keeping it open for a later unlock
    Lock,
    /// Show help for all commands or one command
    Help {
        /// Command to describe
        cmd: Option<String>,
    },
    /// Leave the shell
    #[command(alias = "exit")]
    Quit,
}

/// One typed REPL line: clap in multicall mode, so `argv[0]` is the command name.
#[derive(Debug, Parser)]
#[command(
    multicall = true,
    disable_help_subcommand = true,
    disable_help_flag = true
)]
struct Line {
    #[command(subcommand)]
    command: Command,
}

/// Commands that must not be blocked by the idle lock.
pub fn is_lock_exempt(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Help { .. }
            | Command::Ver
            | Command::Quit
            | Command::Cls
            | Command::History { .. }
            | Command::Lock
            | Command::Close
            | Command::Open { .. }
    )
}

/// kpcli spells some flags with a single dash; accept both spellings.
fn normalize(words: Vec<String>) -> Vec<String> {
    words
        .into_iter()
        .map(|w| match w.as_str() {
            "-expired" => "--expired".to_string(),
            "-full" => "--full".to_string(),
            "-all" => "--all".to_string(),
            _ => w,
        })
        .collect()
}

/// Split a typed line into words the way a POSIX shell would.
pub fn split_line(line: &str) -> Result<Vec<String>, shell_words::ParseError> {
    shell_words::split(line)
}

/// Parse an already-split command line. `Ok(None)` means "blank line, do nothing".
pub fn parse_words(words: Vec<String>) -> Result<Option<Command>, clap::Error> {
    if words.is_empty() {
        return Ok(None);
    }
    Line::try_parse_from(normalize(words)).map(|l| Some(l.command))
}

/// Parse a typed REPL line. `Ok(None)` means "blank line or comment".
pub fn parse_line(line: &str) -> Result<Option<Command>, ParseLineError> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }
    let words = split_line(line).map_err(|_| ParseLineError::Quoting)?;
    parse_words(words).map_err(ParseLineError::Clap)
}

/// Why a typed line could not be turned into a [`Command`].
#[derive(Debug)]
pub enum ParseLineError {
    /// Unbalanced quoting.
    Quoting,
    /// clap rejected the line, or was asked for help.
    Clap(clap::Error),
}

impl std::fmt::Display for ParseLineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseLineError::Quoting => write!(f, "unbalanced quotes"),
            ParseLineError::Clap(e) => write!(f, "{}", e.render()),
        }
    }
}

impl std::error::Error for ParseLineError {}

/// Every command name and alias, for tab completion and `help`.
pub fn command_names() -> Vec<String> {
    let mut names = Vec::new();
    for sub in Line::command().get_subcommands() {
        names.push(sub.get_name().to_string());
        for alias in sub.get_all_aliases() {
            names.push(alias.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Rendered help for every command.
pub fn help_all() -> String {
    Line::command().render_long_help().to_string()
}

/// Rendered help for one command, by name or alias.
pub fn help_for(name: &str) -> Option<String> {
    let mut root = Line::command();
    let found = root
        .get_subcommands()
        .find(|s| s.get_name() == name || s.get_all_aliases().any(|a| a == name))
        .map(|s| s.get_name().to_string())?;
    root.find_subcommand_mut(&found)
        .map(|s| s.render_long_help().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases_and_flags() {
        assert_eq!(
            parse_line("dir /Internet").unwrap(),
            Some(Command::Ls {
                paths: vec!["/Internet".into()]
            })
        );
        assert_eq!(
            parse_line("show -f -a 2").unwrap(),
            Some(Command::Show {
                full: true,
                all: true,
                spec: "2".into()
            })
        );
        assert_eq!(
            parse_line("find -a -expired git").unwrap(),
            Some(Command::Find {
                all_fields: true,
                expired: true,
                query: "git".into()
            })
        );
        assert_eq!(parse_line("exit").unwrap(), Some(Command::Quit));
        assert_eq!(parse_line("   ").unwrap(), None);
    }

    #[test]
    fn quoted_names_stay_together() {
        assert_eq!(
            parse_line("cd 'Sample Entry'").unwrap(),
            Some(Command::Cd {
                path: Some("Sample Entry".into())
            })
        );
    }

    #[test]
    fn command_names_include_aliases() {
        let names = command_names();
        assert!(names.contains(&"ls".to_string()));
        assert!(names.contains(&"dir".to_string()));
        assert!(names.contains(&"chdir".to_string()));
        assert!(names.contains(&"version".to_string()));
    }
}
