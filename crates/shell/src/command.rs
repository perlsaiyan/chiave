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
    Close {
        /// Discard unsaved changes
        #[arg(short = 'f', long = "force")]
        force: bool,
    },
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
        spec: Option<String>,
        /// Move a kpcli-style "2FA-TOTP:" seed from the notes into the otp field
        #[arg(long)]
        migrate: bool,
        /// With --migrate: process every entry in the database
        #[arg(long, requires = "migrate")]
        all: bool,
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
    /// Create a group
    Mkdir {
        /// New group path; the last component is the new name
        path: String,
    },
    /// Remove a group (to the recycle bin unless --permanent)
    Rmdir {
        /// Remove the group even when it still holds entries or groups
        #[arg(short = 'r', long = "recursive")]
        recursive: bool,
        /// Delete outright instead of moving to the recycle bin
        #[arg(long = "permanent")]
        permanent: bool,
        /// Group to remove
        path: String,
    },
    /// Rename a group
    Rename {
        /// Group to rename
        path: String,
        /// New name (not a path)
        new_name: String,
    },
    /// Create an entry, asking for each field
    New {
        /// Title, skipping the Title prompt
        #[arg(long)]
        title: Option<String>,
        /// Username, skipping the Username prompt
        #[arg(long = "user", alias = "username", value_name = "NAME")]
        user: Option<String>,
        /// URL, skipping the URL prompt
        #[arg(long)]
        url: Option<String>,
        /// Notes, skipping the Notes prompt (`\n` becomes a newline)
        #[arg(long)]
        notes: Option<String>,
        /// Read the password from the first line of stdin
        #[arg(long = "password-from-stdin", conflicts_with = "generate")]
        password_from_stdin: bool,
        /// Generate the password instead of asking for one
        #[arg(long)]
        generate: bool,
        /// Length of the generated password
        #[arg(long, value_name = "N")]
        length: Option<usize>,
        /// Leave special characters out of the generated password
        #[arg(long = "no-special")]
        no_special: bool,
        /// Where to put the entry: a path whose last component is the title
        path: Option<String>,
    },
    /// Edit an entry, field by field
    Edit {
        /// Entry path, title or number from the last listing
        spec: String,
    },
    /// Set one field of an entry
    Set {
        /// Entry path, title or number from the last listing
        spec: String,
        /// Field name, `expires`, or the name of a custom field
        field: String,
        /// New value; omitted means "ask for it"
        value: Option<String>,
        /// Remove the field instead of setting it
        #[arg(long = "delete", conflicts_with = "value")]
        delete: bool,
    },
    /// Remove an entry (to the recycle bin unless --permanent)
    Rm {
        /// Delete outright instead of moving to the recycle bin
        #[arg(long = "permanent")]
        permanent: bool,
        /// Do not ask for confirmation
        #[arg(short = 'f', long = "force")]
        force: bool,
        /// Entry path, title or number from the last listing
        spec: String,
    },
    /// Move an entry or group into another group
    Mv {
        /// Entry or group to move
        spec: String,
        /// Destination group
        dest: String,
    },
    /// Copy an entry
    #[command(alias = "copy")]
    Cp {
        /// Entry to copy
        spec: String,
        /// Destination group, or group/NewTitle
        dest: String,
    },
    /// Copy an entry and edit the copy
    Clone {
        /// Entry to copy
        spec: String,
        /// Destination group, or group/NewTitle
        dest: String,
    },
    /// List, add, export or remove an entry's attachments
    Attach {
        /// Entry path, title or number from the last listing
        spec: String,
        /// Attach this file
        #[arg(long = "add", value_name = "FILE")]
        add: Option<PathBuf>,
        /// Name to store the added file under (default: its file name)
        #[arg(long = "name", value_name = "NAME", requires = "add")]
        name: Option<String>,
        /// Write an attachment out to a file
        #[arg(long = "export", num_args = 2, value_names = ["NAME", "FILE"])]
        export: Option<Vec<String>>,
        /// Remove an attachment
        #[arg(long = "rm", value_name = "NAME")]
        rm: Option<String>,
    },
    /// Write the database back to its file
    Save {
        /// Save even if the file changed on disk since it was opened
        #[arg(short = 'f', long = "force")]
        force: bool,
    },
    /// Write the database to a new file, which becomes the current one
    Saveas {
        /// Destination file
        file: PathBuf,
    },
    /// Change the master password (takes effect on the next save)
    Passwd,
    /// Create a new database and switch to it
    Newdb {
        /// File to create
        file: PathBuf,
    },
    /// Convert a KDBX3 database to KDBX4 (takes effect on the next save)
    Upgrade {
        /// Write the converted database here instead of the default
        /// (a `.kdb` source becomes `<name>.kdbx` next to it; a `.kdbx` source is saved in place)
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Generate passwords and print them
    Pwgen {
        /// Password length
        #[arg(long, value_name = "N")]
        length: Option<usize>,
        /// Generate a passphrase of this many words instead
        #[arg(long, value_name = "N")]
        words: Option<usize>,
        /// Leave special characters out
        #[arg(long = "no-special")]
        no_special: bool,
        /// How many to print
        #[arg(long, value_name = "N", default_value_t = 1)]
        count: usize,
    },
    /// Show help for all commands or one command
    Help {
        /// Command to describe
        cmd: Option<String>,
    },
    /// Leave the shell
    #[command(alias = "exit")]
    Quit {
        /// Discard unsaved changes
        #[arg(short = 'f', long = "force")]
        force: bool,
    },
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
            | Command::Quit { .. }
            | Command::Cls
            | Command::History { .. }
            | Command::Lock
            | Command::Close { .. }
            | Command::Open { .. }
            | Command::Pwgen { .. }
            | Command::Newdb { .. }
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
            "-force" => "--force".to_string(),
            "-permanent" => "--permanent".to_string(),
            "-delete" => "--delete".to_string(),
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
        assert_eq!(
            parse_line("exit").unwrap(),
            Some(Command::Quit { force: false })
        );
        assert_eq!(
            parse_line("quit --force").unwrap(),
            Some(Command::Quit { force: true })
        );
        assert_eq!(
            parse_line("rmdir -r --permanent /Old").unwrap(),
            Some(Command::Rmdir {
                recursive: true,
                permanent: true,
                path: "/Old".into()
            })
        );
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
        for n in [
            "mkdir", "rmdir", "rename", "new", "edit", "set", "rm", "mv", "cp", "copy", "clone",
            "attach", "save", "saveas", "passwd", "newdb", "upgrade", "pwgen",
        ] {
            assert!(names.contains(&n.to_string()), "missing {n}");
        }
    }
}
