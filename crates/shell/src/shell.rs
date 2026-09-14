//! The command runner shared by the REPL, `--command` batch mode and the
//! one-shot CLI.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chiave_clip::Clipboard;
use chiave_core::{
    Credentials, EntryId, ExposeSecret, FieldValue, FindOptions, LockedVault, SecretString, Vault,
};
use thiserror::Error;

use crate::command::{self, Command};
use crate::format;
use crate::prompt::{PasswordPrompt, RpasswordPrompt};

/// What the caller should do after a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// Errors raised by the command layer itself.
#[derive(Debug, Error)]
pub enum ShellError {
    #[error("no database is open (use `open <file>`)")]
    NoDatabase,
    #[error("`open` is only available in the interactive shell")]
    OpenNotAvailable,
    #[error("no such field: {0}")]
    NoSuchField(String),
    #[error("unknown command: {0}")]
    UnknownCommand(String),
}

/// Tunables shared by every front end.
#[derive(Debug, Clone)]
pub struct ShellOptions {
    /// How long a copied secret stays on the clipboard. `None` means "never clear".
    pub clip_timeout: Option<Duration>,
    /// Idle time after which the vault re-locks. `None` disables the idle lock.
    pub timeout: Option<Duration>,
    /// REPL history file; `None` means the XDG default.
    pub histfile: Option<PathBuf>,
    /// How long `xpx` holds the password on the clipboard while counting down.
    pub xpx_secs: u64,
    /// Open databases read-only.
    pub read_only: bool,
}

impl Default for ShellOptions {
    fn default() -> Self {
        ShellOptions {
            clip_timeout: Some(Duration::from_secs(10)),
            timeout: None,
            histfile: None,
            xpx_secs: 10,
            read_only: false,
        }
    }
}

/// The default REPL history file: `$XDG_STATE_HOME/chiave/history`, else
/// `~/.local/state/chiave/history`.
pub fn default_histfile() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("chiave").join("history"))
}

/// An open (or locked) vault plus everything the command table needs.
pub struct Shell {
    vault: Option<Vault>,
    locked: Option<LockedVault>,
    clip: Box<dyn Clipboard>,
    opts: ShellOptions,
    prompt: Box<dyn PasswordPrompt>,
    last_activity: Instant,
    history: Vec<String>,
    /// Group to return to after an idle unlock.
    resume_path: Option<String>,
}

impl Shell {
    /// A shell with no database open; `open` gives it one.
    pub fn new(clip: Box<dyn Clipboard>, opts: ShellOptions) -> Self {
        Shell {
            vault: None,
            locked: None,
            clip,
            opts,
            prompt: Box::new(RpasswordPrompt),
            last_activity: Instant::now(),
            history: Vec::new(),
            resume_path: None,
        }
    }

    /// A shell over an already-open vault.
    pub fn with_vault(vault: Vault, clip: Box<dyn Clipboard>, opts: ShellOptions) -> Self {
        let mut s = Shell::new(clip, opts);
        s.vault = Some(vault);
        s
    }

    /// Replace the master-password prompt (tests use [`crate::FixedPrompt`]).
    pub fn set_prompt(&mut self, prompt: Box<dyn PasswordPrompt>) {
        self.prompt = prompt;
    }

    pub fn options(&self) -> &ShellOptions {
        &self.opts
    }

    pub fn options_mut(&mut self) -> &mut ShellOptions {
        &mut self.opts
    }

    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    pub fn vault_mut(&mut self) -> Option<&mut Vault> {
        self.vault.as_mut()
    }

    pub fn clipboard(&self) -> &dyn Clipboard {
        self.clip.as_ref()
    }

    /// True when a database is known but currently locked.
    pub fn is_locked(&self) -> bool {
        self.vault.is_none() && self.locked.is_some()
    }

    /// Lines run so far, oldest first.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// The REPL prompt for the current state.
    pub fn prompt_string(&self) -> String {
        match (&self.vault, &self.locked) {
            (Some(v), _) => format!("chiave:{}> ", v.cwd_path()),
            (None, Some(_)) => "chiave:[locked]> ".to_string(),
            (None, None) => "chiave> ".to_string(),
        }
    }

    /// Note user activity, postponing the idle lock.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    // ----- entry points -----------------------------------------------------

    /// Parse and run one typed line. Command errors are reported on `out` and
    /// never abort the REPL.
    pub fn run_line(&mut self, line: &str, out: &mut dyn Write) -> anyhow::Result<Flow> {
        if !line.trim().is_empty() {
            self.history.push(line.trim().to_string());
        }
        match command::parse_line(line) {
            Ok(None) => Ok(Flow::Continue),
            Ok(Some(cmd)) => self.run_command(cmd, out),
            Err(command::ParseLineError::Clap(e)) => {
                write!(out, "{}", e.render())?;
                Ok(Flow::Continue)
            }
            Err(command::ParseLineError::Quoting) => {
                writeln!(out, "error: unbalanced quotes")?;
                Ok(Flow::Continue)
            }
        }
    }

    /// Run one command, reporting errors on `out` (REPL semantics).
    pub fn run_command(&mut self, cmd: Command, out: &mut dyn Write) -> anyhow::Result<Flow> {
        match self.exec(cmd, out) {
            Ok(flow) => Ok(flow),
            Err(e) => {
                writeln!(out, "error: {e}")?;
                Ok(Flow::Continue)
            }
        }
    }

    /// Run one command, propagating command errors (one-shot CLI semantics).
    pub fn exec(&mut self, cmd: Command, out: &mut dyn Write) -> anyhow::Result<Flow> {
        if !command::is_lock_exempt(&cmd) {
            self.enforce_idle_lock(out)?;
            self.unlock_if_needed(out)?;
        }
        let flow = self.dispatch(cmd, out)?;
        self.touch();
        Ok(flow)
    }

    // ----- locking ----------------------------------------------------------

    fn enforce_idle_lock(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        let Some(timeout) = self.opts.timeout else {
            return Ok(());
        };
        if self.vault.is_none() || self.last_activity.elapsed() < timeout {
            return Ok(());
        }
        self.lock_now();
        writeln!(out, "Idle for too long; the database has been locked.")?;
        Ok(())
    }

    fn lock_now(&mut self) {
        if let Some(v) = self.vault.take() {
            self.resume_path = Some(v.cwd_path());
            self.locked = Some(v.lock());
        }
    }

    fn unlock_if_needed(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        if self.vault.is_some() {
            return Ok(());
        }
        let Some(locked) = self.locked.clone() else {
            return Ok(());
        };
        let msg = format!("Master password for {}: ", locked.path.display());
        let password = self
            .prompt
            .prompt(&msg)
            .map_err(|e| anyhow::anyhow!("cannot read the password: {e}"))?;
        // On a wrong password `self.locked` is untouched, so the user can simply retry.
        let vault = locked.unlock(Some(password))?;
        self.locked = None;
        self.resume_path = None;
        self.vault = Some(vault);
        writeln!(out, "Unlocked {}.", locked.path.display())?;
        Ok(())
    }

    fn need_vault(&mut self) -> Result<&mut Vault, ShellError> {
        self.vault.as_mut().ok_or(ShellError::NoDatabase)
    }

    // ----- dispatch ---------------------------------------------------------

    fn dispatch(&mut self, cmd: Command, out: &mut dyn Write) -> anyhow::Result<Flow> {
        match cmd {
            Command::Open { file, keyfile } => self.cmd_open(&file, keyfile.as_deref(), out)?,
            Command::Close => self.cmd_close(out)?,
            Command::Ls { paths } => self.cmd_ls(&paths, out)?,
            Command::Cd { path } => self.cmd_cd(path.as_deref().unwrap_or("/"), out)?,
            Command::Cl { path } => {
                self.cmd_cd(&path, out)?;
                self.cmd_ls(&[], out)?;
            }
            Command::Pwd => {
                let p = self.need_vault()?.cwd_path();
                writeln!(out, "{p}")?;
            }
            Command::Show { full, all, spec } => self.cmd_show(&spec, full, all, out)?,
            Command::Get { spec, field } => self.cmd_get(&spec, &field, out)?,
            Command::Find {
                all_fields,
                expired,
                query,
            } => self.cmd_find(&query, all_fields, expired, out)?,
            Command::Otp { spec } => self.cmd_otp(&spec, out)?,
            Command::Xu { spec } => self.cmd_copy_text(&spec, Field::Username, out)?,
            Command::Xw { spec } => self.cmd_copy_text(&spec, Field::Url, out)?,
            Command::Xp { spec } => self.cmd_copy_secret(&spec, Field::Password, out)?,
            Command::Xo { spec } => self.cmd_copy_secret(&spec, Field::Otp, out)?,
            Command::Xpx { spec } => self.cmd_xpx(&spec, out)?,
            Command::Xx => {
                self.clip.clear()?;
                writeln!(out, "Clipboard cleared.")?;
            }
            Command::Stats => self.cmd_stats(out)?,
            Command::Ver => self.cmd_ver(out)?,
            Command::Cls => write!(out, "\x1b[2J\x1b[H")?,
            Command::History { clear } => self.cmd_history(clear, out)?,
            Command::Lock => {
                if self.vault.is_none() {
                    anyhow::bail!(ShellError::NoDatabase);
                }
                self.lock_now();
                writeln!(out, "Locked.")?;
            }
            Command::Help { cmd } => self.cmd_help(cmd.as_deref(), out)?,
            Command::Quit => return Ok(Flow::Quit),
        }
        Ok(Flow::Continue)
    }

    // ----- commands ---------------------------------------------------------

    fn cmd_open(
        &mut self,
        file: &Path,
        keyfile: Option<&Path>,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let msg = format!("Master password for {}: ", file.display());
        let password = self.prompt.prompt(&msg)?;
        let password = if password.expose_secret().is_empty() && keyfile.is_some() {
            None
        } else {
            Some(password)
        };
        let creds = Credentials {
            password,
            keyfile: keyfile.map(Path::to_path_buf),
        };
        let vault = Vault::open(file, &creds, self.opts.read_only)?;
        self.locked = None;
        self.resume_path = None;
        self.vault = Some(vault);
        writeln!(out, "Opened {}.", file.display())?;
        Ok(())
    }

    fn cmd_close(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        let had = self.vault.is_some() || self.locked.is_some();
        self.vault = None;
        self.locked = None;
        self.resume_path = None;
        if had {
            writeln!(out, "Closed.")?;
        } else {
            writeln!(out, "No database is open.")?;
        }
        Ok(())
    }

    fn cmd_ls(&mut self, paths: &[String], out: &mut dyn Write) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        if paths.is_empty() {
            let listing = vault.list(None)?;
            write!(out, "{}", format::listing(&listing))?;
            return Ok(());
        }
        let many = paths.len() > 1;
        for (i, p) in paths.iter().enumerate() {
            let listing = vault.list(Some(p))?;
            if many {
                if i > 0 {
                    writeln!(out)?;
                }
                writeln!(out, "{}:", listing.path)?;
            }
            write!(out, "{}", format::listing(&listing))?;
        }
        Ok(())
    }

    fn cmd_cd(&mut self, path: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        vault.cd(path)?;
        let _ = out;
        Ok(())
    }

    fn cmd_show(
        &mut self,
        spec: &str,
        full: bool,
        all: bool,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let id = vault.resolve_entry(spec)?;
        let view = vault.entry(id)?;
        write!(out, "{}", format::entry(&view, full, all))?;
        Ok(())
    }

    fn cmd_get(&mut self, spec: &str, field: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let id = vault.resolve_entry(spec)?;
        if matches!(field.to_lowercase().as_str(), "otp" | "totp") {
            let code = vault.totp(id)?;
            writeln!(out, "{}", code.code)?;
            return Ok(());
        }
        let view = vault.entry(id)?;
        let value = match field.to_lowercase().as_str() {
            "title" => view.title.clone(),
            "username" | "uname" | "user" => view.username.clone().unwrap_or_default(),
            "password" | "pass" => view
                .password
                .as_ref()
                .map(|p| p.expose_secret().to_string())
                .unwrap_or_default(),
            "url" => view.url.clone().unwrap_or_default(),
            "notes" | "comments" => view.notes.clone().unwrap_or_default(),
            other => {
                let found = view
                    .custom
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == other)
                    .ok_or_else(|| ShellError::NoSuchField(field.to_string()))?;
                match &found.1 {
                    FieldValue::Plain(v) => v.clone(),
                    FieldValue::Protected(v) => v.expose_secret().to_string(),
                }
            }
        };
        writeln!(out, "{value}")?;
        Ok(())
    }

    fn cmd_find(
        &mut self,
        query: &str,
        all_fields: bool,
        expired: bool,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let hits = vault.find(
            query,
            FindOptions {
                all_fields,
                expired_only: expired,
            },
        );
        write!(out, "{}", format::hits(&hits))?;
        Ok(())
    }

    fn cmd_otp(&mut self, spec: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let id = vault.resolve_entry(spec)?;
        let code = vault.totp(id)?;
        writeln!(out, "{} (valid {}s)", code.code, code.valid_for_secs)?;
        Ok(())
    }

    fn secret_of(&mut self, spec: &str, field: Field) -> anyhow::Result<SecretString> {
        let vault = self.need_vault()?;
        let id = vault.resolve_entry(spec)?;
        match field {
            Field::Password => {
                let view = vault.entry(id)?;
                view.password
                    .ok_or_else(|| ShellError::NoSuchField("password".into()).into())
            }
            Field::Otp => Ok(SecretString::from(vault.totp(id)?.code)),
            _ => unreachable!("secret_of is only for secrets"),
        }
    }

    fn text_of(&mut self, spec: &str, field: Field) -> anyhow::Result<String> {
        let vault = self.need_vault()?;
        let id: EntryId = vault.resolve_entry(spec)?;
        let view = vault.entry(id)?;
        let value = match field {
            Field::Username => view.username.clone(),
            Field::Url => view.url.clone(),
            _ => unreachable!("text_of is only for plain fields"),
        };
        value.ok_or_else(|| ShellError::NoSuchField(field.label().to_lowercase()).into())
    }

    fn cmd_copy_text(
        &mut self,
        spec: &str,
        field: Field,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let value = self.text_of(spec, field)?;
        self.clip.copy_text(&value)?;
        writeln!(out, "{} copied to clipboard.", field.label())?;
        Ok(())
    }

    fn cmd_copy_secret(
        &mut self,
        spec: &str,
        field: Field,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let secret = self.secret_of(spec, field)?;
        let timeout = self.opts.clip_timeout;
        self.clip.copy_secret(&secret, timeout)?;
        match timeout {
            Some(d) => writeln!(
                out,
                "{} copied to clipboard (clears in {}s).",
                field.label(),
                d.as_secs()
            )?,
            None => writeln!(out, "{} copied to clipboard.", field.label())?,
        }
        Ok(())
    }

    fn cmd_xpx(&mut self, spec: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        let secret = self.secret_of(spec, Field::Password)?;
        // The helper must not clear behind our back: we hold it and clear ourselves.
        self.clip.copy_secret(&secret, None)?;
        drop(secret);
        let secs = self.opts.xpx_secs;
        writeln!(
            out,
            "Password copied to clipboard; clearing in {secs}s (Ctrl-C to keep waiting)."
        )?;
        for left in (1..=secs).rev() {
            write!(out, "\r{left:>3}s ")?;
            out.flush()?;
            std::thread::sleep(Duration::from_secs(1));
        }
        if secs > 0 {
            writeln!(out, "\r     ")?;
        }
        self.clip.clear()?;
        writeln!(out, "Clipboard cleared.")?;
        Ok(())
    }

    fn cmd_stats(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        let s = self.need_vault()?.stats();
        writeln!(out, "File: {}", s.path.display())?;
        writeln!(out, "Name: {}", s.name.as_deref().unwrap_or("-"))?;
        writeln!(out, "Version: {}", s.version)?;
        writeln!(out, "Cipher: {}", s.cipher)?;
        writeln!(out, "KDF: {}", s.kdf)?;
        writeln!(out, "Groups: {}", s.groups)?;
        writeln!(out, "Entries: {}", s.entries)?;
        writeln!(out, "Expired: {}", s.expired)?;
        writeln!(out, "With OTP: {}", s.with_otp)?;
        writeln!(
            out,
            "Recycle bin: {}",
            if s.recycle_bin_enabled {
                "enabled"
            } else {
                "disabled"
            }
        )?;
        writeln!(
            out,
            "Mode: {}",
            if s.read_only {
                "read-only"
            } else {
                "read-write"
            }
        )?;
        Ok(())
    }

    fn cmd_ver(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        writeln!(out, "chiave {}", env!("CARGO_PKG_VERSION"))?;
        writeln!(out, "clipboard: {}", self.clip.name())?;
        Ok(())
    }

    fn cmd_history(&mut self, clear: bool, out: &mut dyn Write) -> anyhow::Result<()> {
        if clear {
            self.history.clear();
            writeln!(out, "History cleared.")?;
            return Ok(());
        }
        for (i, line) in self.history.iter().enumerate() {
            writeln!(out, "{:>4}  {line}", i + 1)?;
        }
        Ok(())
    }

    fn cmd_help(&mut self, cmd: Option<&str>, out: &mut dyn Write) -> anyhow::Result<()> {
        match cmd {
            None => write!(out, "{}", command::help_all())?,
            Some(name) => match command::help_for(name) {
                Some(text) => write!(out, "{text}")?,
                None => anyhow::bail!(ShellError::UnknownCommand(name.to_string())),
            },
        }
        Ok(())
    }
}

/// Which field an `x*` command copies.
#[derive(Debug, Clone, Copy)]
enum Field {
    Username,
    Url,
    Password,
    Otp,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Username => "Username",
            Field::Url => "URL",
            Field::Password => "Password",
            Field::Otp => "One-time code",
        }
    }
}

/// Run a list of commands, as `--command` does. Stops early on `quit`.
pub fn run_commands(
    shell: &mut Shell,
    commands: &[String],
    out: &mut dyn Write,
) -> anyhow::Result<Flow> {
    for line in commands {
        if shell.run_line(line, out)? == Flow::Quit {
            return Ok(Flow::Quit);
        }
    }
    Ok(Flow::Continue)
}
