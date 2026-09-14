//! show / get / find / otp / stats and general error handling.

mod common;

use chiave_shell::Flow;

#[test]
fn show_masks_the_password_and_protected_fields() {
    let mut h = common::harness();
    let out = h.ok("show '/Sample Entry'");
    assert!(out.contains("Path: /Sample Entry"), "{out}");
    assert!(out.contains("Title: Sample Entry"), "{out}");
    assert!(out.contains("Uname: alice"), "{out}");
    assert!(out.contains("Pass: ********"), "{out}");
    assert!(!out.contains("s3cret"), "{out}");
    assert!(out.contains("PIN: ********"), "{out}");
    assert!(!out.contains("1234"), "{out}");
    assert!(out.contains("Plain custom: visible"), "{out}");
    assert!(out.contains("Tags: demo"), "{out}");
    assert!(
        out.contains("Notes: some notes\n       second line"),
        "{out}"
    );
}

#[test]
fn show_f_reveals_secrets() {
    let mut h = common::harness();
    let out = h.ok("show -f '/Sample Entry'");
    assert!(out.contains("Pass: s3cret"), "{out}");
    assert!(out.contains("PIN: 1234"), "{out}");
    assert!(!out.contains("********"), "{out}");
}

#[test]
fn show_a_adds_metadata() {
    let mut h = common::harness();
    let out = h.ok("show -a '/Sample Entry'");
    for label in [
        "Created:",
        "Modified:",
        "Accessed:",
        "Expires:",
        "Icon:",
        "History:",
        "UUID:",
    ] {
        assert!(out.contains(label), "missing {label} in\n{out}");
    }
    assert!(out.contains("Pass: ********"), "{out}");
}

#[test]
fn show_marks_expired_entries_and_otp() {
    let mut h = common::harness();
    assert!(h
        .ok("show /Work/Servers/db01")
        .contains("Title: db01 *EXPIRED*"));
    let out = h.ok("ls /Internet");
    assert!(out.contains("[otp]"), "{out}");
    assert!(h.ok("show 2").contains("OTP: configured"));
}

#[test]
fn get_prints_raw_field_values() {
    let mut h = common::harness();
    assert_eq!(h.ok("get '/Sample Entry' password"), "s3cret\n");
    assert_eq!(h.ok("get '/Sample Entry' pass"), "s3cret\n");
    assert_eq!(h.ok("get '/Sample Entry' username"), "alice\n");
    assert_eq!(h.ok("get '/Sample Entry' uname"), "alice\n");
    assert_eq!(h.ok("get '/Sample Entry' title"), "Sample Entry\n");
    assert_eq!(h.ok("get '/Sample Entry' url"), "https://example.com\n");
    assert_eq!(h.ok("get '/Sample Entry' PIN"), "1234\n");
    assert_eq!(
        h.ok("get '/Sample Entry' comments"),
        h.ok("get '/Sample Entry' notes")
    );
    assert!(h.ok("get '/Sample Entry' notes").starts_with("some notes"));
}

#[test]
fn get_otp_prints_six_digits() {
    let mut h = common::harness();
    h.ok("ls /Internet");
    let code = h.ok("get 2 otp");
    let code = code.trim();
    assert_eq!(code.len(), 6, "{code}");
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
}

#[test]
fn get_rejects_unknown_fields() {
    let mut h = common::harness();
    let out = h.run("get '/Sample Entry' nonesuch");
    assert!(out.contains("no such field"), "{out}");
}

#[test]
fn otp_prints_a_code_and_the_seconds_left() {
    let mut h = common::harness();
    h.ok("ls /Internet");
    let out = h.ok("otp 2");
    let (code, rest) = out.trim().split_once(' ').expect("code and validity");
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
    assert!(
        rest.starts_with("(valid ") && rest.ends_with("s)"),
        "{rest}"
    );
}

#[test]
fn otp_on_an_entry_without_totp_is_an_error() {
    let mut h = common::harness();
    let out = h.run("otp '/Sample Entry'");
    assert!(out.contains("error:"), "{out}");
    assert!(out.contains("TOTP"), "{out}");
}

#[test]
fn find_numbers_hits_and_show_reuses_the_numbers() {
    let mut h = common::harness();
    let out = h.ok("find git");
    assert!(out.starts_with("1. /Internet/GitHub (perlsaiyan)"), "{out}");
    assert!(out.contains("2. /Internet/GitHub (someone-else)"), "{out}");
    let shown = h.ok("show 2");
    assert!(shown.contains("Uname: someone-else"), "{shown}");
}

#[test]
fn find_marks_recycled_and_expired_hits() {
    let mut h = common::harness();
    assert!(h.ok("find old").contains("*OLD*"));
    let out = h.ok("find -expired ''");
    assert!(out.contains("db01"), "{out}");
    assert!(out.contains("*EXPIRED*"), "{out}");
    assert_eq!(out.lines().count(), 1, "{out}");
}

#[test]
fn find_a_searches_every_field() {
    let mut h = common::harness();
    assert_eq!(h.ok("find xfinity.com"), "no matches\n");
    assert!(h.ok("find -a xfinity.com").contains("Comcast"));
}

#[test]
fn ambiguous_specs_are_refused() {
    let mut h = common::harness();
    let out = h.run("show /Internet/GitHub");
    assert!(out.contains("ambiguous"), "{out}");
}

#[test]
fn stats_reports_the_database() {
    let mut h = common::harness();
    let out = h.ok("stats");
    assert!(out.contains("Name: Sample"), "{out}");
    assert!(out.contains("Version: KDBX4"), "{out}");
    assert!(out.contains("Entries: 7"), "{out}");
    assert!(out.contains("Expired: 1"), "{out}");
    assert!(out.contains("With OTP: 1"), "{out}");
    assert!(out.contains("Mode: read-write"), "{out}");
}

#[test]
fn ver_and_help_work() {
    let mut h = common::harness();
    let out = h.ok("ver");
    assert!(out.contains("chiave "), "{out}");
    assert!(out.contains("clipboard: memory"), "{out}");
    assert_eq!(out, h.ok("version"));
    assert!(h.ok("help").contains("xpx"), "help should list commands");
    assert!(h.ok("help show").contains("Reveal the password"));
}

#[test]
fn unknown_commands_report_an_error_and_continue() {
    let mut h = common::harness();
    let (flow, out) = h.run_flow("frobnicate");
    assert_eq!(flow, Flow::Continue);
    assert!(out.contains("error:"), "{out}");
    assert!(h.ok("pwd").contains('/'));
}

#[test]
fn quit_and_exit_stop_the_loop() {
    let mut h = common::harness();
    assert_eq!(h.run_flow("quit").0, Flow::Quit);
    assert_eq!(h.run_flow("exit").0, Flow::Quit);
    assert_eq!(h.run_flow("").0, Flow::Continue);
}

#[test]
fn history_records_lines_and_can_be_cleared() {
    let mut h = common::harness();
    h.ok("pwd");
    h.ok("ls");
    let out = h.ok("history");
    assert!(out.contains("pwd"), "{out}");
    assert!(out.contains("ls"), "{out}");
    assert!(h.ok("history -c").contains("History cleared"));
    // Only the `history` line just typed remains.
    assert_eq!(h.ok("history"), "   1  history\n");
}
