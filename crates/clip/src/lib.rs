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
//! # Backends
//!
//! | name | when | sensitive hint | survives caller exit |
//! |---|---|---|---|
//! | `wayland` | `WAYLAND_DISPLAY` set | yes | only via the helper process |
//! | `x11` | `DISPLAY` set | no (X11 selections cannot carry it) | yes (`xclip` daemonises) |
//! | `wl-copy` | native Wayland unavailable, `wl-copy` installed | no (one MIME type per invocation) | yes |
//! | `none` | no display | n/a | n/a |
//!
//! # Wayland lifetime, and why there is a helper process
//!
//! `wl-clipboard-rs` 0.9 serves paste requests from a **thread inside the copying
//! process** (older versions forked). When `chiave xp` exits the offer is dropped
//! and the clipboard goes empty. So whenever `DetectOptions::helper_exe` is set,
//! the Wayland backend copies through a detached `chiave __clip-serve` process
//! instead. See [`helper`] for the protocol.
//!
//! See `crates/clip/README.md` for the full design and verification notes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use zeroize::Zeroizing;

mod clear;
mod command;
mod helper;
mod offers;
mod raw;
mod session;
mod wayland;
mod wlcopy;
mod x11;

pub use clear::{clear_if_unchanged, ClearOutcome, ReadClear};
pub use helper::{parse_helper_args, HelperArgs, HELPER_SUBCOMMAND};
pub use offers::{
    build_offers, primary_text_mime, MimeOffer, OfferPayload, SENSITIVE_MIME, SENSITIVE_VALUE,
    TEXT_MIME_TYPES,
};
pub use session::{choose_backend, BackendKind, EnvSnapshot, OVERRIDE_VAR};

use raw::RawBackend;

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

    /// Whether `copy_secret` can actually mark the content sensitive.
    ///
    /// False for the X11 and `wl-copy` backends, where clipboard managers *will*
    /// record the secret and only the auto-clear timer protects it. Callers may
    /// want to warn the user.
    fn supports_sensitive_hint(&self) -> bool {
        true
    }

    /// Current clipboard content, for diagnostics and round-trip tests.
    fn read_back(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        Err(ClipError::Backend(
            "read-back is not supported by this backend".into(),
        ))
    }
}

/// How `detect()` should behave.
#[derive(Debug, Default, Clone)]
pub struct DetectOptions {
    /// Path to the chiave executable, used to spawn the detached `__clip-serve` helper
    /// for auto-clear that outlives the caller. `None` falls back to an in-process timer.
    pub helper_exe: Option<PathBuf>,
    /// Serve only the first paste request, then drop the offer (`wl-copy --paste-once`).
    ///
    /// Applies to `copy_secret` only. Wayland honours it natively, `xclip` via
    /// `-loops 1`; `xsel` has no equivalent and ignores it. Note that some
    /// clients (XWayland ones in particular) have trouble pasting a one-shot
    /// offer, so this is off by default.
    pub paste_once: bool,
}

/// Pick a backend for this session (Wayland, X11, or none).
pub fn detect(opts: DetectOptions) -> Box<dyn Clipboard> {
    let kind = choose_backend(&EnvSnapshot::from_env());
    match raw_backend(kind) {
        Ok(backend) => Box::new(BackendClipboard {
            kind,
            backend: Arc::from(backend),
            helper_exe: opts.helper_exe,
            paste_once: opts.paste_once,
        }),
        Err(_) => Box::new(NullClipboard),
    }
}

/// Entry point for the hidden `chiave __clip-serve` subcommand. `args` excludes the
/// program name and the `__clip-serve` token. Reads the secret from stdin.
pub fn helper_main(args: Vec<String>) -> std::process::ExitCode {
    helper::helper_main(args)
}

pub(crate) fn raw_backend(kind: BackendKind) -> Result<Box<dyn RawBackend>, ClipError> {
    match kind {
        BackendKind::Wayland => Ok(Box::new(WaylandWithFallback::new())),
        BackendKind::X11 => Ok(Box::new(x11::X11Raw::new())),
        BackendKind::WlCopy => Ok(Box::new(wlcopy::WlCopyRaw)),
        BackendKind::None => Err(ClipError::Unavailable),
    }
}

/// Native Wayland, degrading to the `wl-copy` binary if the compositor turns out
/// not to implement `ext-data-control`/`wlr-data-control`.
///
/// The fallback is decided at the first failing operation rather than at
/// detection time, because there is no way to know without connecting.
struct WaylandWithFallback {
    native: wayland::WaylandRaw,
    fallback: Option<wlcopy::WlCopyRaw>,
    using_fallback: AtomicBool,
}

impl WaylandWithFallback {
    fn new() -> Self {
        WaylandWithFallback {
            native: wayland::WaylandRaw,
            fallback: wlcopy::available().then_some(wlcopy::WlCopyRaw),
            using_fallback: AtomicBool::new(false),
        }
    }

    fn degraded(&self) -> bool {
        self.using_fallback.load(Ordering::Relaxed)
    }

    /// Map a native failure onto the fallback when it is both possible and right.
    fn fallback_for(&self, err: &ClipError) -> Option<&wlcopy::WlCopyRaw> {
        if wayland::is_protocol_missing(err) {
            self.fallback.as_ref()
        } else {
            None
        }
    }
}

impl RawBackend for WaylandWithFallback {
    fn copy(
        &self,
        offers: &[MimeOffer],
        content: &[u8],
        paste_once: bool,
    ) -> Result<raw::Serving, ClipError> {
        if self.degraded() {
            if let Some(fallback) = self.fallback.as_ref() {
                return fallback.copy(offers, content, paste_once);
            }
        }
        match self.native.copy(offers, content, paste_once) {
            Ok(serving) => Ok(serving),
            Err(err) => match self.fallback_for(&err) {
                Some(fallback) => {
                    self.using_fallback.store(true, Ordering::Relaxed);
                    fallback.copy(offers, content, paste_once)
                }
                None => Err(err),
            },
        }
    }

    fn display_name(&self) -> &'static str {
        if self.degraded() {
            "wayland (wl-copy fallback)"
        } else {
            "wayland"
        }
    }

    fn supports_sensitive_hint(&self) -> bool {
        !self.degraded()
    }
}

impl ReadClear for WaylandWithFallback {
    fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        if self.degraded() {
            if let Some(fallback) = self.fallback.as_ref() {
                return fallback.read_current();
            }
        }
        match self.native.read_current() {
            Ok(value) => Ok(value),
            Err(err) => match self.fallback_for(&err) {
                Some(fallback) => fallback.read_current(),
                None => Err(err),
            },
        }
    }

    fn clear_now(&self) -> Result<(), ClipError> {
        if self.degraded() {
            if let Some(fallback) = self.fallback.as_ref() {
                return fallback.clear_now();
            }
        }
        match self.native.clear_now() {
            Ok(()) => Ok(()),
            Err(err) => match self.fallback_for(&err) {
                Some(fallback) => fallback.clear_now(),
                None => Err(err),
            },
        }
    }
}

/// A real clipboard: a backend plus the policy for making the copy outlive us.
struct BackendClipboard {
    kind: BackendKind,
    backend: Arc<dyn RawBackend>,
    helper_exe: Option<PathBuf>,
    paste_once: bool,
}

impl BackendClipboard {
    /// The Wayland offer is served from a thread in this process, so a one-shot
    /// caller must hand it to the helper or the clipboard empties on exit. The
    /// command backends fork their own daemon and need the helper only for the
    /// auto-clear timer.
    fn needs_helper(&self, clear_after: Option<Duration>) -> bool {
        clear_after.is_some() || matches!(self.kind, BackendKind::Wayland)
    }

    fn put(
        &self,
        content: &Zeroizing<Vec<u8>>,
        sensitive: bool,
        clear_after: Option<Duration>,
    ) -> Result<(), ClipError> {
        let paste_once = sensitive && self.paste_once;

        let mut helper_err = None;
        if let Some(exe) = self.helper_exe.as_deref() {
            if self.needs_helper(clear_after) {
                let args = HelperArgs {
                    clear_after,
                    backend: Some(self.kind),
                    paste_once,
                    sensitive,
                };
                match helper::spawn_detached(exe, &args, content) {
                    Ok(()) => return Ok(()),
                    // A missing or broken helper must not stop us from copying;
                    // degrade to the in-process path (which dies with us).
                    Err(err) => helper_err = Some(err),
                }
            }
        }

        let offers = build_offers(sensitive);
        // Dropping the `Serving` handle detaches the serving thread; it keeps
        // running for as long as this process lives.
        let _serving = self
            .backend
            .copy(&offers, content, paste_once)
            .map_err(|err| match helper_err {
                Some(helper_err) => ClipError::Backend(format!("{err} (and {helper_err})")),
                None => err,
            })?;

        if let Some(delay) = clear_after {
            helper::spawn_inprocess_clear(Arc::clone(&self.backend), content.clone(), delay);
        }
        Ok(())
    }
}

impl Clipboard for BackendClipboard {
    fn name(&self) -> &'static str {
        self.backend.display_name()
    }

    fn copy_secret(
        &self,
        secret: &SecretString,
        clear_after: Option<Duration>,
    ) -> Result<(), ClipError> {
        let content = Zeroizing::new(secret.expose_secret().as_bytes().to_vec());
        self.put(&content, true, clear_after)
    }

    fn copy_text(&self, text: &str) -> Result<(), ClipError> {
        let content = Zeroizing::new(text.as_bytes().to_vec());
        self.put(&content, false, None)
    }

    fn clear(&self) -> Result<(), ClipError> {
        self.backend.clear_now()
    }

    fn supports_sensitive_hint(&self) -> bool {
        self.backend.supports_sensitive_hint()
    }

    fn read_back(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        self.backend.read_current()
    }
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
    fn supports_sensitive_hint(&self) -> bool {
        false
    }
    fn read_back(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
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
    fn read_back(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        let s = self.state.lock().unwrap();
        Ok(s.content
            .as_ref()
            .map(|c| Zeroizing::new(c.as_bytes().to_vec())))
    }
}

impl ReadClear for MemoryClipboard {
    fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        self.read_back()
    }
    fn clear_now(&self) -> Result<(), ClipError> {
        self.clear()
    }
}
