//! Headless tests for the public contract. These must pass in CI with no display.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use chiave_clip::{
    build_offers, detect, ClipError, Clipboard, DetectOptions, MemoryClipboard, NullClipboard,
    OfferPayload, SENSITIVE_MIME,
};
use secrecy::SecretString;

// ---------------------------------------------------------------- MemoryClipboard

#[test]
fn memory_records_secrets_with_the_clear_delay() {
    let clip = MemoryClipboard::new();
    clip.copy_secret(
        &SecretString::from("hunter2"),
        Some(Duration::from_secs(10)),
    )
    .unwrap();

    let state = clip.state();
    assert_eq!(state.content.as_deref(), Some("hunter2"));
    assert!(state.sensitive);
    assert_eq!(state.clear_after, Some(Duration::from_secs(10)));
    assert_eq!(state.clears, 0);
    assert_eq!(clip.name(), "memory");
}

#[test]
fn memory_text_is_not_sensitive_and_never_auto_clears() {
    let clip = MemoryClipboard::new();
    clip.copy_secret(&SecretString::from("hunter2"), Some(Duration::from_secs(5)))
        .unwrap();
    clip.copy_text("tom").unwrap();

    let state = clip.state();
    assert_eq!(state.content.as_deref(), Some("tom"));
    assert!(!state.sensitive);
    assert_eq!(state.clear_after, None);
}

#[test]
fn memory_clear_empties_and_counts() {
    let clip = MemoryClipboard::new();
    clip.copy_text("tom").unwrap();
    clip.clear().unwrap();
    clip.clear().unwrap();

    let state = clip.state();
    assert!(state.content.is_none());
    assert_eq!(state.clears, 2);
    assert!(clip.read_back().unwrap().is_none());
}

#[test]
fn memory_read_back_returns_what_was_copied() {
    let clip = MemoryClipboard::new();
    clip.copy_secret(&SecretString::from("hunter2"), None)
        .unwrap();
    let value = clip.read_back().unwrap().expect("content");
    assert_eq!(&value[..], b"hunter2");
}

// ---------------------------------------------------------------- NullClipboard

#[test]
fn null_fails_every_operation() {
    let clip = NullClipboard;
    assert_eq!(clip.name(), "none");
    assert!(matches!(
        clip.copy_secret(&SecretString::from("x"), None),
        Err(ClipError::Unavailable)
    ));
    assert!(matches!(clip.copy_text("x"), Err(ClipError::Unavailable)));
    assert!(matches!(clip.clear(), Err(ClipError::Unavailable)));
    assert!(!clip.supports_sensitive_hint());
}

// ---------------------------------------------------------------- offers

#[test]
fn secret_offers_include_the_kde_hint() {
    let offers = build_offers(true);
    let hint = offers
        .iter()
        .find(|o| o.mime == SENSITIVE_MIME)
        .expect("hint offer");
    assert_eq!(hint.payload, OfferPayload::Literal("secret"));
    assert!(!build_offers(false).iter().any(|o| o.mime == SENSITIVE_MIME));
}

// ---------------------------------------------------------------- detect()

/// `detect()` reads the process environment, which is global. Serialise the tests
/// that poke at it.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, Option<&str>)]) -> EnvGuard {
        let lock = env_lock();
        let keys = ["CHIAVE_CLIPBOARD", "WAYLAND_DISPLAY", "DISPLAY"];
        let saved = keys
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect::<Vec<_>>();
        for key in keys {
            std::env::remove_var(key);
        }
        for (key, value) in vars {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        EnvGuard { saved, _lock: lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn detect_name(vars: &[(&'static str, Option<&str>)]) -> &'static str {
    let _guard = EnvGuard::set(vars);
    detect(DetectOptions::default()).name()
}

#[test]
fn detect_without_a_display_is_none() {
    assert_eq!(detect_name(&[]), "none");
}

#[test]
fn detect_prefers_wayland() {
    assert_eq!(
        detect_name(&[
            ("WAYLAND_DISPLAY", Some("wayland-1")),
            ("DISPLAY", Some(":0"))
        ]),
        "wayland"
    );
}

#[test]
fn detect_uses_x11_when_only_display_is_set() {
    assert_eq!(detect_name(&[("DISPLAY", Some(":0"))]), "x11");
}

#[test]
fn detect_honours_the_override() {
    assert_eq!(
        detect_name(&[
            ("CHIAVE_CLIPBOARD", Some("x11")),
            ("WAYLAND_DISPLAY", Some("wayland-1")),
        ]),
        "x11"
    );
    assert_eq!(
        detect_name(&[
            ("CHIAVE_CLIPBOARD", Some("wl-copy")),
            ("WAYLAND_DISPLAY", Some("wayland-1")),
        ]),
        "wl-copy"
    );
    assert_eq!(
        detect_name(&[
            ("CHIAVE_CLIPBOARD", Some("none")),
            ("WAYLAND_DISPLAY", Some("wayland-1")),
            ("DISPLAY", Some(":0")),
        ]),
        "none"
    );
}

#[test]
fn detect_returns_a_send_sync_boxed_trait_object() {
    fn assert_send_sync<T: Send + Sync + ?Sized>(_: &T) {}
    let _guard = EnvGuard::set(&[]);
    let clip = detect(DetectOptions {
        helper_exe: None,
        paste_once: true,
    });
    assert_send_sync(clip.as_ref());
}
