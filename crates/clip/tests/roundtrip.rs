//! Real clipboard round trip. Ignored by default: needs a live Wayland or X11
//! session (and, on X11, `xclip` or `xsel`).
//!
//! Run it on the target machine with:
//!
//! ```sh
//! cargo test -p chiave-clip -- --ignored --nocapture
//! ```
//!
//! It copies a marker string, reads it back, copies something else, checks that
//! auto-clear leaves the foreign value alone, and finally clears.

use std::time::Duration;

use chiave_clip::{detect, DetectOptions};
use secrecy::SecretString;

fn have_session() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()))
}

#[test]
#[ignore = "needs a real Wayland or X11 session"]
fn copy_read_clear_round_trip() {
    assert!(
        have_session(),
        "no WAYLAND_DISPLAY or DISPLAY; nothing to test against"
    );

    let clip = detect(DetectOptions::default());
    println!("backend: {}", clip.name());
    println!(
        "sensitive hint supported: {}",
        clip.supports_sensitive_hint()
    );

    let marker = "chiave-clip-roundtrip-1";
    clip.copy_secret(&SecretString::from(marker), None)
        .expect("copy_secret");

    let read = clip
        .read_back()
        .expect("read_back")
        .expect("clipboard empty");
    assert_eq!(
        String::from_utf8_lossy(&read).trim_end_matches('\n'),
        marker
    );

    clip.copy_text("chiave-clip-roundtrip-2")
        .expect("copy_text");
    let read = clip
        .read_back()
        .expect("read_back")
        .expect("clipboard empty");
    assert_eq!(
        String::from_utf8_lossy(&read).trim_end_matches('\n'),
        "chiave-clip-roundtrip-2"
    );

    clip.clear().expect("clear");
    let read = clip.read_back().expect("read_back");
    assert!(
        read.as_deref().is_none_or(|v| v.is_empty()),
        "clipboard should be empty after clear, got {read:?}"
    );
}

#[test]
#[ignore = "needs a real Wayland or X11 session; takes ~4s"]
fn auto_clear_fires_and_respects_a_newer_copy() {
    assert!(have_session(), "no WAYLAND_DISPLAY or DISPLAY");

    let clip = detect(DetectOptions {
        // No helper executable here: this exercises the in-process timer, which
        // is fine because the test process stays alive.
        helper_exe: None,
        paste_once: false,
    });

    clip.copy_secret(
        &SecretString::from("chiave-clip-autoclear"),
        Some(Duration::from_secs(1)),
    )
    .expect("copy_secret");
    std::thread::sleep(Duration::from_secs(2));
    let read = clip.read_back().expect("read_back");
    assert!(
        read.as_deref().is_none_or(|v| v.is_empty()),
        "auto-clear should have emptied the clipboard, got {read:?}"
    );

    clip.copy_secret(
        &SecretString::from("chiave-clip-autoclear-2"),
        Some(Duration::from_secs(1)),
    )
    .expect("copy_secret");
    clip.copy_text("the user copied something else")
        .expect("copy_text");
    std::thread::sleep(Duration::from_secs(2));
    let read = clip
        .read_back()
        .expect("read_back")
        .expect("clipboard empty");
    assert_eq!(
        String::from_utf8_lossy(&read).trim_end_matches('\n'),
        "the user copied something else",
        "auto-clear must not clobber a newer copy"
    );

    clip.clear().expect("clear");
}
