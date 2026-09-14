//! Legacy kpcli "2FA-TOTP:" seeds in notes: redacted on show, live code shown, migratable.

mod common;

const SEED: &str = "JBSWY3DPEHPK3PXP";
const ENTRY: &str = "\"/Internet/Legacy 2FA\"";

fn six_digit_code(out: &str) -> Option<String> {
    out.lines()
        .find(|l| l.starts_with("OTP: "))
        .and_then(|l| l.split_whitespace().nth(1))
        .filter(|c| c.len() == 6 && c.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_string)
}

#[test]
fn show_redacts_seed_and_prints_live_code() {
    let mut h = common::harness();
    let out = h.ok(&format!("show {ENTRY}"));
    assert!(out.contains("2FA-TOTP: <redacted>"), "{out}");
    assert!(!out.contains(SEED), "{out}");
    assert!(six_digit_code(&out).is_some(), "{out}");
    assert!(out.contains("seed stored in notes"), "{out}");
    assert!(out.contains("recovery codes in the safe"), "{out}");

    // -a alone or -f alone still redacts; both reveal, like kpcli
    assert!(!h.ok(&format!("show -a {ENTRY}")).contains(SEED));
    assert!(!h.ok(&format!("show -f {ENTRY}")).contains(SEED));
    assert!(h.ok(&format!("show -a -f {ENTRY}")).contains(SEED));
}

#[test]
fn otp_get_and_copy_work_for_notes_seed() {
    let mut h = common::harness();
    let out = h.ok(&format!("otp {ENTRY}"));
    assert!(out.starts_with(|c: char| c.is_ascii_digit()), "{out}");
    assert!(out.contains("(valid"), "{out}");
    let got = h.ok(&format!("get {ENTRY} otp"));
    assert_eq!(got.trim().len(), 6, "{got}");
    // the native GitHub entry shares the seed, so the codes agree
    h.ok("ls /Internet");
    let native = h.ok("otp 2");
    assert_eq!(
        out.split_whitespace().next(),
        native.split_whitespace().next()
    );
    let copied = h.ok(&format!("xo {ENTRY}"));
    assert!(copied.contains("copied"), "{copied}");
    assert!(h.ok("ls /Internet").contains("4. Legacy 2FA [otp]"));
}

#[test]
fn migrate_moves_seed_to_native_field() {
    let mut h = common::harness();
    let before = six_digit_code(&h.ok(&format!("show {ENTRY}"))).unwrap();
    let out = h.ok(&format!("otp --migrate {ENTRY}"));
    assert!(out.contains("Migrated"), "{out}");
    let shown = h.ok(&format!("show -a -f {ENTRY}"));
    assert!(
        !shown.contains(SEED),
        "seed must be gone from notes:\n{shown}"
    );
    assert!(!shown.contains("2FA-TOTP"), "{shown}");
    assert!(!shown.contains("seed stored in notes"), "{shown}");
    assert!(shown.contains("recovery codes in the safe"), "{shown}");
    assert!(shown.contains("last line"), "{shown}");
    assert_eq!(six_digit_code(&shown).unwrap(), before);
    let view = h.entry("/Internet/Legacy 2FA");
    assert!(matches!(
        view.otp_source,
        Some(chiave_core::OtpSource::Field)
    ));
    assert_eq!(view.history_count, 1);

    let again = h.ok(&format!("otp --migrate {ENTRY}"));
    assert!(again.contains("Nothing to migrate"), "{again}");
    assert!(h.run("quit").contains("Unsaved changes"));
}

#[test]
fn migrate_all_reports_count() {
    let mut h = common::harness();
    let out = h.ok("otp --migrate --all");
    assert!(out.contains("Migrated /Internet/Legacy 2FA"), "{out}");
    assert!(out.contains("1 entry migrated"), "{out}");
    let out = h.ok("otp --migrate --all");
    assert!(out.contains("0 entries migrated"), "{out}");
    let err = h.run("otp --all");
    assert!(err.contains("error"), "{err}");
}
