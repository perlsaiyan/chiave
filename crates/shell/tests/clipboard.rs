//! xu / xw / xp / xo / xpx / xx.

mod common;

use std::time::Duration;

use chiave_shell::ShellOptions;

#[test]
fn xp_copies_the_password_as_a_secret_with_the_clear_timeout() {
    let mut h = common::harness();
    let out = h.ok("xp '/Sample Entry'");
    assert!(
        out.contains("Password copied to clipboard (clears in 10s)"),
        "{out}"
    );
    assert!(!out.contains("s3cret"), "{out}");
    let state = h.clip.state();
    assert_eq!(state.content.as_deref(), Some("s3cret"));
    assert!(state.sensitive);
    assert_eq!(state.clear_after, Some(Duration::from_secs(10)));
}

#[test]
fn a_zero_clip_timeout_never_clears() {
    let mut h = common::harness_with(ShellOptions {
        clip_timeout: None,
        ..ShellOptions::default()
    });
    let out = h.ok("xp '/Sample Entry'");
    assert!(out.contains("Password copied to clipboard."), "{out}");
    assert_eq!(h.clip.state().clear_after, None);
}

#[test]
fn xu_copies_the_username_as_plain_text() {
    let mut h = common::harness();
    let out = h.ok("xu '/Sample Entry'");
    assert!(out.contains("Username copied to clipboard"), "{out}");
    let state = h.clip.state();
    assert_eq!(state.content.as_deref(), Some("alice"));
    assert!(!state.sensitive);
    assert_eq!(state.clear_after, None);
}

#[test]
fn xw_copies_the_url_as_plain_text() {
    let mut h = common::harness();
    h.ok("xw '/Sample Entry'");
    let state = h.clip.state();
    assert_eq!(state.content.as_deref(), Some("https://example.com"));
    assert!(!state.sensitive);
}

#[test]
fn xo_copies_the_one_time_code() {
    let mut h = common::harness();
    h.ok("ls /Internet");
    let out = h.ok("xo 2");
    assert!(
        out.contains("One-time code copied to clipboard (clears in 10s)"),
        "{out}"
    );
    let state = h.clip.state();
    let code = state.content.expect("a code on the clipboard");
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
    assert!(state.sensitive);
}

#[test]
fn xx_clears_the_clipboard() {
    let mut h = common::harness();
    h.ok("xp '/Sample Entry'");
    let out = h.ok("xx");
    assert!(out.contains("Clipboard cleared"), "{out}");
    let state = h.clip.state();
    assert_eq!(state.content, None);
    assert_eq!(state.clears, 1);
}

#[test]
fn xpx_holds_the_password_then_clears_it() {
    let mut h = common::harness_with(ShellOptions {
        xpx_secs: 0,
        ..ShellOptions::default()
    });
    let out = h.ok("xpx '/Sample Entry'");
    assert!(out.contains("clearing in 0s"), "{out}");
    assert!(out.contains("Clipboard cleared"), "{out}");
    assert!(!out.contains("s3cret"), "{out}");
    let state = h.clip.state();
    assert_eq!(state.content, None);
    assert_eq!(state.clears, 1);
}

#[test]
fn copying_a_missing_field_is_an_error() {
    let mut h = common::harness();
    let out = h.run("xw /Internet/2");
    assert!(out.contains("error:"), "{out}");
}
