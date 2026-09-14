//! The `chiave` binary: mode dispatch for the interactive shell, one-shot
//! subcommands and the hidden clipboard helper.

mod config;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use chiave_clip::{Clipboard, DetectOptions, NullClipboard};
use chiave_core::{Credentials, ExposeSecret, SecretString, Vault};
use chiave_shell::{PasswordPrompt, RpasswordPrompt, Shell, ShellOptions};
use clap::{Parser, Subcommand};

/// Environment variable holding the master password. Convenient for scripts,
/// unsafe on a shared machine: it is visible to anything that can read the
/// process environment.
const PASSWORD_ENV: &str = "CHIAVE_PASSWORD";

#[derive(Debug, Parser)]
#[command(
    name = "chiave",
    version,
    about = "KeePass KDBX shell for the terminal",
    long_about = None,
    disable_help_subcommand = true,
)]
struct Cli {
    /// Database to open
    #[arg(long, value_name = "PATH", env = "CHIAVE_KDB")]
    kdb: Option<PathBuf>,

    /// Key file that unlocks the database
    #[arg(long = "key", value_name = "FILE", env = "CHIAVE_KEYFILE")]
    key: Option<PathBuf>,

    /// Read the master password from the first line of this file
    #[arg(long, value_name = "FILE")]
    pwfile: Option<PathBuf>,

    /// Never write to the database
    #[arg(long)]
    readonly: bool,

    /// Lock the vault after this many idle seconds (0 disables)
    #[arg(long, value_name = "SECS")]
    timeout: Option<u64>,

    /// Shell history file (/dev/null disables it)
    #[arg(long, value_name = "FILE")]
    histfile: Option<PathBuf>,

    /// Run a command and exit; may be repeated
    #[arg(long = "command", value_name = "CMD")]
    command: Vec<String>,

    /// Seconds a copied secret stays on the clipboard (0 never clears)
    #[arg(long = "clip-timeout", value_name = "SECS")]
    clip_timeout: Option<u64>,

    /// Do not touch the clipboard at all
    #[arg(long = "no-clip")]
    no_clip: bool,

    /// Word list for passphrase generation (`w` at a password prompt, `pwgen --words`)
    #[arg(long = "pwwords", value_name = "FILE")]
    pwwords: Option<PathBuf>,

    /// Do not save automatically after a one-shot command that changed the database
    #[arg(long = "no-save")]
    no_save: bool,

    /// Disable mouse support in the TUI (mouse capture makes text selection need Shift+drag)
    #[arg(long = "no-mouse")]
    no_mouse: bool,

    /// Database to open, kpcli style
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// Full-screen browser (the default when a database is configured)
    Tui,
    /// kpcli-style interactive shell
    Shell,
    #[command(flatten)]
    Run(chiave_shell::Command),
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("__clip-serve") {
        return chiave_clip::helper_main(std::env::args().skip(2).collect());
    }
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("chiave: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    let cfg = config::load()?;

    let database = cli
        .kdb
        .clone()
        .or_else(|| cli.file.clone())
        .or_else(|| cfg.database.clone());
    let keyfile = cli.key.clone().or_else(|| cfg.keyfile.clone());
    let opts = shell_options(&cli, &cfg);
    let interactive = matches!(cli.action, None | Some(Action::Shell));

    let Some(database) = database else {
        if interactive {
            return start(Shell::new(clipboard(&cli), opts), &cli, None);
        }
        hint_no_database();
        return Ok(ExitCode::from(2));
    };

    let creds = credentials(&cli, &database, keyfile.clone())?;
    let vault = Vault::open(&database, &creds, cli.readonly)
        .with_context(|| format!("opening {}", database.display()))?;
    if wants_tui(&cli) {
        let mouse = !cli.no_mouse && cfg.mouse.unwrap_or(true);
        return run_tui(vault, &cli, &opts, mouse);
    }
    let mut shell = Shell::with_vault(vault, clipboard(&cli), opts);
    shell.set_keyfile(keyfile);
    start(shell, &cli, Some(&database))
}

/// `chiave tui`, or bare `chiave` on a terminal with no --command batch.
fn wants_tui(cli: &Cli) -> bool {
    match cli.action {
        Some(Action::Tui) => true,
        None => {
            cli.command.is_empty()
                && std::io::stdin().is_terminal()
                && std::io::stdout().is_terminal()
        }
        _ => false,
    }
}

fn run_tui(vault: Vault, cli: &Cli, opts: &ShellOptions, mouse: bool) -> anyhow::Result<ExitCode> {
    let tui_opts = chiave_tui::TuiOptions {
        clip_timeout: opts.clip_timeout,
        idle_lock: opts.timeout,
        read_only: cli.readonly,
        mouse,
    };
    chiave_tui::run(vault, clipboard(cli), tui_opts, Box::new(TtyPrompt))?;
    Ok(ExitCode::SUCCESS)
}

/// Adapts the shell's rpassword prompt to the TUI's prompt trait.
struct TtyPrompt;

impl chiave_tui::PasswordPrompt for TtyPrompt {
    fn prompt(&self, msg: &str) -> std::io::Result<SecretString> {
        chiave_shell::prompt::PasswordPrompt::prompt(&RpasswordPrompt, msg)
    }
}

/// Run whatever mode the flags asked for.
fn start(mut shell: Shell, cli: &Cli, database: Option<&Path>) -> anyhow::Result<ExitCode> {
    shell.set_prompt(Box::new(RpasswordPrompt));

    if !cli.command.is_empty() {
        let mut out = std::io::stdout().lock();
        chiave_shell::run_commands(&mut shell, &cli.command, &mut out)?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    match &cli.action {
        None | Some(Action::Shell) | Some(Action::Tui) => {
            if database.is_none() {
                println!("No database configured. Use `open <file.kdbx>` to open one.");
            }
            chiave_shell::repl(&mut shell)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Action::Run(command)) => {
            if matches!(command, chiave_shell::Command::Open { .. }) {
                anyhow::bail!(chiave_shell::ShellError::OpenNotAvailable);
            }
            let mut out = std::io::stdout().lock();
            let result = shell
                .exec(command.clone(), &mut out)
                // There is no later `save` in one-shot mode, so a command that
                // changed something writes the database before we exit.
                .and_then(|flow| {
                    if cli.no_save {
                        Ok(flow)
                    } else {
                        shell.auto_save(&mut out).map(|_| flow)
                    }
                });
            out.flush()?;
            match result {
                Ok(_) => Ok(ExitCode::SUCCESS),
                Err(e) => {
                    eprintln!("chiave: {e}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
    }
}

/// Flags beat environment variables, which beat the configuration file.
fn shell_options(cli: &Cli, cfg: &config::Config) -> ShellOptions {
    let clip_timeout = cli.clip_timeout.or(cfg.clip_timeout).unwrap_or(10);
    let timeout = cli.timeout.or(cfg.timeout).unwrap_or(0);
    ShellOptions {
        clip_timeout: (clip_timeout > 0).then(|| Duration::from_secs(clip_timeout)),
        timeout: (timeout > 0).then(|| Duration::from_secs(timeout)),
        histfile: cli.histfile.clone().or_else(|| cfg.histfile.clone()),
        xpx_secs: if clip_timeout > 0 { clip_timeout } else { 10 },
        read_only: cli.readonly,
        pwwords: cli.pwwords.clone().or_else(|| cfg.pwwords.clone()),
    }
}

fn clipboard(cli: &Cli) -> Box<dyn Clipboard> {
    if cli.no_clip {
        return Box::new(NullClipboard);
    }
    chiave_clip::detect(DetectOptions {
        helper_exe: std::env::current_exe().ok(),
        ..DetectOptions::default()
    })
}

/// `--pwfile`, then `$CHIAVE_PASSWORD`, then the terminal.
fn credentials(
    cli: &Cli,
    database: &Path,
    keyfile: Option<PathBuf>,
) -> anyhow::Result<Credentials> {
    let password = master_password(cli, database, keyfile.is_some())?;
    Ok(Credentials { password, keyfile })
}

fn master_password(
    cli: &Cli,
    database: &Path,
    have_keyfile: bool,
) -> anyhow::Result<Option<SecretString>> {
    if let Some(file) = &cli.pwfile {
        let text = zeroize::Zeroizing::new(
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?,
        );
        let first = text.lines().next().unwrap_or("").to_string();
        return Ok(Some(SecretString::from(first)));
    }
    if let Some(pw) = std::env::var(PASSWORD_ENV).ok().filter(|p| !p.is_empty()) {
        return Ok(Some(SecretString::from(pw)));
    }
    let msg = format!("Master password for {}: ", database.display());
    let password = RpasswordPrompt.prompt(&msg)?;
    // An empty answer means "the key file alone unlocks this database".
    if have_keyfile && password.expose_secret().is_empty() {
        return Ok(None);
    }
    Ok(Some(password))
}

fn hint_no_database() {
    let cfg = config::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/chiave/config.toml".to_string());
    eprintln!("chiave: no database given.");
    eprintln!("Try one of:");
    eprintln!("  chiave /path/to/vault.kdbx <command>");
    eprintln!("  chiave --kdb /path/to/vault.kdbx <command>");
    eprintln!("  export CHIAVE_KDB=/path/to/vault.kdbx");
    eprintln!("  echo 'database = \"/path/to/vault.kdbx\"' >> {cfg}");
    eprintln!("Or run `chiave shell` and use `open <file.kdbx>`.");
}
