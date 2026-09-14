//! The detached clipboard helper process.
//!
//! On Wayland, `wl-clipboard-rs` serves paste requests from a thread inside the
//! copying process: when `chiave xp` exits, the offer dies with it. The helper
//! solves that (and auto-clear that outlives the caller) the same way `wl-copy`
//! does, with a background process that owns the selection:
//!
//! ```text
//! chiave xp  --(fork/exec, setsid)-->  chiave __clip-serve --clear-after 10
//!            --(secret on stdin)------------>
//!            <--("ok" on stdout once the selection is taken)--
//!   exits                                    sleeps, then clears if unchanged
//! ```
//!
//! The secret only ever travels over the child's stdin: never argv (world-readable
//! in `/proc`), never the environment.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::clear::{clear_if_unchanged, ClearOutcome};
use crate::offers::build_offers;
use crate::raw::RawBackend;
use crate::session::{choose_backend, BackendKind, EnvSnapshot};
use crate::ClipError;

/// The hidden subcommand the chiave binary forwards to [`crate::helper_main`].
pub const HELPER_SUBCOMMAND: &str = "__clip-serve";

/// Line the helper prints on stdout once it owns the clipboard.
const READY: &str = "ok";

/// What the helper process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperArgs {
    /// Clear the clipboard after this long. `None` means "hold it until another
    /// application takes the selection".
    pub clear_after: Option<Duration>,
    /// Force a backend. `None` means detect from the environment.
    pub backend: Option<BackendKind>,
    /// Stop serving after the first paste (`wl-copy --paste-once`).
    pub paste_once: bool,
    /// Offer `x-kde-passwordManagerHint`. False for plain text (`--text`).
    pub sensitive: bool,
}

impl Default for HelperArgs {
    fn default() -> Self {
        HelperArgs {
            clear_after: None,
            backend: None,
            paste_once: false,
            sensitive: true,
        }
    }
}

impl HelperArgs {
    /// Render back to a command line (excluding the `__clip-serve` token).
    pub fn to_argv(&self) -> Vec<String> {
        let mut argv = Vec::new();
        if let Some(d) = self.clear_after {
            argv.push("--clear-after".to_string());
            argv.push(d.as_secs().to_string());
        }
        if let Some(backend) = self.backend {
            argv.push("--backend".to_string());
            argv.push(backend.as_str().to_string());
        }
        if self.paste_once {
            argv.push("--paste-once".to_string());
        }
        if !self.sensitive {
            argv.push("--text".to_string());
        }
        argv
    }
}

fn take_value<'a, I: Iterator<Item = &'a String>>(
    flag: &str,
    inline: Option<&'a str>,
    rest: &mut I,
) -> Result<String, String> {
    match inline {
        Some(v) => Ok(v.to_string()),
        None => rest
            .next()
            .map(|v| v.to_string())
            .ok_or_else(|| format!("{flag} needs a value")),
    }
}

/// Parse the helper's arguments.
///
/// Accepts `--flag value` and `--flag=value`. Returns a human-readable message on
/// error rather than a typed error, since the only consumer prints it and exits.
pub fn parse_helper_args(args: &[String]) -> Result<HelperArgs, String> {
    let mut parsed = HelperArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (arg.as_str(), None),
        };
        match flag {
            "--clear-after" => {
                let raw = take_value("--clear-after", inline, &mut it)?;
                let secs: u64 = raw
                    .parse()
                    .map_err(|_| format!("--clear-after: not a number of seconds: {raw:?}"))?;
                parsed.clear_after = if secs == 0 {
                    None
                } else {
                    Some(Duration::from_secs(secs))
                };
            }
            "--backend" => {
                let raw = take_value("--backend", inline, &mut it)?;
                parsed.backend = Some(
                    BackendKind::parse(&raw)
                        .ok_or_else(|| format!("--backend: unknown backend {raw:?}"))?,
                );
            }
            "--paste-once" => parsed.paste_once = true,
            "--text" => parsed.sensitive = false,
            "--secret" => parsed.sensitive = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(parsed)
}

/// Point stdin/stdout/stderr at `/dev/null` so the helper holds nothing of the
/// caller's terminal open once it has reported readiness.
fn detach_stdio() {
    let Ok(devnull) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    else {
        return;
    };
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(&devnull);
    for target in 0..=2 {
        // SAFETY: `fd` is a valid open file descriptor for the duration of this
        // loop, and 0/1/2 are valid descriptor numbers to overwrite.
        unsafe {
            libc::dup2(fd, target);
        }
    }
}

/// Body of the `__clip-serve` subcommand.
pub(crate) fn helper_main(args: Vec<String>) -> ExitCode {
    let parsed = match parse_helper_args(&args) {
        Ok(parsed) => parsed,
        Err(msg) => {
            println!("err {msg}");
            eprintln!("chiave __clip-serve: {msg}");
            return ExitCode::FAILURE;
        }
    };

    let mut secret = Zeroizing::new(Vec::new());
    if let Err(err) = std::io::stdin().read_to_end(&mut secret) {
        println!("err reading secret from stdin: {err}");
        return ExitCode::FAILURE;
    }

    match run_helper(&parsed, &secret) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            println!("err {err}");
            let _ = std::io::stdout().flush();
            ExitCode::FAILURE
        }
    }
}

fn run_helper(args: &HelperArgs, secret: &[u8]) -> Result<(), ClipError> {
    let kind = args
        .backend
        .unwrap_or_else(|| choose_backend(&EnvSnapshot::from_env()));
    let backend: Box<dyn RawBackend> = crate::raw_backend(kind)?;
    let offers = build_offers(args.sensitive);

    // Take the clipboard first: only then can we honestly report readiness.
    let serving = backend.copy(&offers, secret, args.paste_once)?;

    println!("{READY}");
    let _ = std::io::stdout().flush();
    detach_stdio();

    match args.clear_after {
        Some(delay) => {
            thread::sleep(delay);
            let outcome = clear_if_unchanged(backend.as_ref(), secret)?;
            debug_assert!(matches!(
                outcome,
                ClearOutcome::Cleared
                    | ClearOutcome::AlreadyEmpty
                    | ClearOutcome::Changed
                    | ClearOutcome::ClearedUnverified
            ));
        }
        // Hold the selection until somebody else takes it (or forever, for the
        // command backends, whose own daemon owns it -- this returns at once).
        None => serving.wait(),
    }
    Ok(())
}

/// Spawn a detached `<exe> __clip-serve ...` and hand it `payload` over stdin.
///
/// Returns once the helper reports that it owns the clipboard, so a failure to
/// copy is still reported synchronously to the caller.
pub(crate) fn spawn_detached(
    exe: &Path,
    args: &HelperArgs,
    payload: &[u8],
) -> Result<(), ClipError> {
    let mut cmd = Command::new(exe);
    cmd.arg(HELPER_SUBCOMMAND)
        .args(args.to_argv())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    // SAFETY: `setsid` is async-signal-safe and is all this hook does. It puts the
    // helper in its own session so it survives the caller's process group being
    // signalled (Ctrl-C in the shell, terminal close).
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|err| ClipError::Backend(format!("clipboard helper {exe:?}: {err}")))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClipError::Backend("clipboard helper: no stdin".into()))?;
        stdin.write_all(payload)?;
        stdin.flush()?;
        // Dropping closes the pipe, which is the helper's signal to stop reading.
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ClipError::Backend("clipboard helper: no stdout".into()))?;

    let mut line = String::new();
    let read = BufReader::new(stdout).read_line(&mut line);

    // Reap the helper without blocking this thread. In a one-shot invocation the
    // process exits before this ever completes and the helper is reparented to
    // init; in the long-running shell it keeps zombies from piling up.
    thread::spawn(move || {
        let _ = child.wait();
    });

    match read {
        Ok(0) => Err(ClipError::Backend(
            "clipboard helper exited without taking the clipboard".into(),
        )),
        Ok(_) => {
            let line = line.trim();
            if line == READY {
                Ok(())
            } else {
                Err(ClipError::Backend(format!(
                    "clipboard helper: {}",
                    line.strip_prefix("err ").unwrap_or(line)
                )))
            }
        }
        Err(err) => Err(ClipError::Backend(format!("clipboard helper: {err}"))),
    }
}

/// In-process auto-clear, used when no helper executable is available.
///
/// The timer lives in a detached thread, so it **dies with this process**: a
/// one-shot invocation that exits before the delay elapses will leave the secret
/// on the clipboard. Long-running modes (shell, TUI) are unaffected.
pub(crate) fn spawn_inprocess_clear(
    backend: std::sync::Arc<dyn RawBackend>,
    secret: Zeroizing<Vec<u8>>,
    delay: Duration,
) {
    thread::Builder::new()
        .name("chiave-clip-clear".into())
        .spawn(move || {
            thread::sleep(delay);
            let _ = clear_if_unchanged(backend.as_ref(), &secret);
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_are_sensitive_and_persistent() {
        let parsed = parse_helper_args(&[]).unwrap();
        assert_eq!(parsed, HelperArgs::default());
        assert!(parsed.sensitive);
        assert_eq!(parsed.clear_after, None);
        assert_eq!(parsed.backend, None);
        assert!(!parsed.paste_once);
    }

    #[test]
    fn parses_the_full_command_line() {
        let parsed = parse_helper_args(&argv(&[
            "--clear-after",
            "10",
            "--backend",
            "wayland",
            "--paste-once",
        ]))
        .unwrap();
        assert_eq!(parsed.clear_after, Some(Duration::from_secs(10)));
        assert_eq!(parsed.backend, Some(BackendKind::Wayland));
        assert!(parsed.paste_once);
        assert!(parsed.sensitive);
    }

    #[test]
    fn accepts_equals_form() {
        let parsed = parse_helper_args(&argv(&["--clear-after=45", "--backend=x11"])).unwrap();
        assert_eq!(parsed.clear_after, Some(Duration::from_secs(45)));
        assert_eq!(parsed.backend, Some(BackendKind::X11));
    }

    #[test]
    fn zero_seconds_means_no_auto_clear() {
        assert_eq!(
            parse_helper_args(&argv(&["--clear-after", "0"]))
                .unwrap()
                .clear_after,
            None
        );
    }

    #[test]
    fn text_flag_drops_the_sensitive_hint() {
        assert!(!parse_helper_args(&argv(&["--text"])).unwrap().sensitive);
        assert!(
            parse_helper_args(&argv(&["--text", "--secret"]))
                .unwrap()
                .sensitive
        );
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            vec!["--clear-after"],
            vec!["--clear-after", "soon"],
            vec!["--backend"],
            vec!["--backend", "telepathy"],
            vec!["--wat"],
            vec!["secret-on-argv"],
        ] {
            assert!(
                parse_helper_args(&argv(&bad)).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn argv_round_trips() {
        for args in [
            HelperArgs::default(),
            HelperArgs {
                clear_after: Some(Duration::from_secs(10)),
                backend: Some(BackendKind::Wayland),
                paste_once: true,
                sensitive: true,
            },
            HelperArgs {
                clear_after: Some(Duration::from_secs(3)),
                backend: Some(BackendKind::X11),
                paste_once: false,
                sensitive: false,
            },
        ] {
            let rendered = args.to_argv();
            assert_eq!(parse_helper_args(&rendered).unwrap(), args, "{rendered:?}");
        }
    }

    /// A stand-in for `chiave __clip-serve` that records how it was invoked and
    /// speaks the readiness protocol. Lets the spawn path be tested with no display.
    fn fake_helper(dir: &Path, reply: &str) -> std::path::PathBuf {
        let exe = dir.join("fake-helper.sh");
        let log = dir.join("log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{log}.argv'\ncat > '{log}.stdin'\necho '{reply}'\n",
            log = log.display(),
            reply = reply
        );
        std::fs::write(&exe, script).unwrap();
        let mut perms = std::fs::metadata(&exe).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&exe, perms).unwrap();
        exe
    }

    /// Writing a script and exec'ing it from parallel tests races on ETXTBSY: a
    /// fork by one test can inherit the other's still-open write descriptor.
    /// Serialise the tests that do it.
    fn spawn_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chiave-clip-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn spawn_sends_the_secret_on_stdin_and_waits_for_readiness() {
        let _lock = spawn_lock();
        let dir = scratch("ok");
        let exe = fake_helper(&dir, "ok");
        let args = HelperArgs {
            clear_after: Some(Duration::from_secs(10)),
            backend: Some(BackendKind::Wayland),
            paste_once: false,
            sensitive: true,
        };

        spawn_detached(&exe, &args, b"hunter2").expect("helper should report readiness");

        let argv = std::fs::read_to_string(dir.join("log.argv")).unwrap();
        assert!(argv.contains(HELPER_SUBCOMMAND), "{argv}");
        assert!(argv.contains("--clear-after 10"), "{argv}");
        assert!(argv.contains("--backend wayland"), "{argv}");
        assert!(!argv.contains("hunter2"), "secret leaked onto argv: {argv}");

        let stdin = std::fs::read_to_string(dir.join("log.stdin")).unwrap();
        assert_eq!(stdin, "hunter2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spawn_reports_helper_failure() {
        let _lock = spawn_lock();
        let dir = scratch("err");
        let exe = fake_helper(&dir, "err no clipboard available");
        let err = spawn_detached(&exe, &HelperArgs::default(), b"hunter2").unwrap_err();
        assert!(err.to_string().contains("no clipboard available"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spawn_reports_a_missing_helper_executable() {
        let err = spawn_detached(
            Path::new("/nonexistent/chiave-helper"),
            &HelperArgs::default(),
            b"hunter2",
        )
        .unwrap_err();
        assert!(matches!(err, ClipError::Backend(_)), "{err}");
    }

    #[test]
    fn argv_never_contains_the_secret() {
        let args = HelperArgs {
            clear_after: Some(Duration::from_secs(10)),
            backend: Some(BackendKind::Wayland),
            paste_once: true,
            sensitive: true,
        };
        let rendered = args.to_argv().join(" ");
        assert!(rendered.starts_with("--"));
        assert!(!rendered.contains("hunter2"));
    }
}
