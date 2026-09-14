//! Navigation, listing, search and show against the sample vault.

use chiave_core::testdb;
use chiave_core::{ExposeSecret, FindOptions, NodeId, ResolveError, Vault};

fn open() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let (path, creds) = testdb::sample_file(dir.path());
    let v = Vault::open(&path, &creds, false).expect("open sample");
    (dir, v)
}

#[test]
fn root_is_slash_and_kdbx4_is_saveable() {
    let (_d, v) = open();
    assert_eq!(v.cwd_path(), "/");
    assert!(v.can_save());
    assert_eq!(v.version().to_string(), "KDBX4.1");
}

#[test]
fn ls_sorts_and_numbers_entries() {
    let (_d, mut v) = open();
    let l = v.list(None).unwrap();
    let names: Vec<&str> = l.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Empty", "Internet", "Recycle Bin", "Work"]);
    assert_eq!(l.entries.len(), 1);
    assert_eq!(l.entries[0].number, 1);
    assert_eq!(l.entries[0].title, "Sample Entry");

    v.cd("Internet").unwrap();
    assert_eq!(v.cwd_path(), "/Internet");
    let l = v.list(None).unwrap();
    let titles: Vec<(usize, &str)> = l
        .entries
        .iter()
        .map(|e| (e.number, e.title.as_str()))
        .collect();
    assert_eq!(
        titles,
        [(1, "Comcast/Xfinity"), (2, "GitHub"), (3, "GitHub")]
    );
    assert!(l.entries[1].has_otp || l.entries[2].has_otp);
}

#[test]
fn numbers_refer_to_last_listing() {
    let (_d, mut v) = open();
    v.cd("/Internet").unwrap();
    v.list(None).unwrap();
    let e = v.resolve_entry("1").unwrap();
    assert_eq!(v.entry_path(e), r"/Internet/Comcast\/Xfinity");
    assert!(matches!(
        v.resolve_entry("9"),
        Err(ResolveError::BadNumber(9))
    ));
}

#[test]
fn duplicate_titles_are_ambiguous_by_path_but_fine_by_number() {
    let (_d, mut v) = open();
    assert!(matches!(
        v.resolve_entry("/Internet/GitHub"),
        Err(ResolveError::Ambiguous(_, 2))
    ));
    v.list(Some("/Internet")).unwrap();
    let a = v.resolve_entry("2").unwrap();
    let b = v.resolve_entry("3").unwrap();
    assert_ne!(a, b);
}

#[test]
fn escaped_slash_and_relative_paths_resolve() {
    let (_d, mut v) = open();
    v.cd("Work/Servers").unwrap();
    assert_eq!(v.cwd_path(), "/Work/Servers");
    let e = v.resolve_entry(r"../../Internet/Comcast\/Xfinity").unwrap();
    assert_eq!(v.entry_path(e), r"/Internet/Comcast\/Xfinity");
    v.cd("..").unwrap();
    assert_eq!(v.cwd_path(), "/Work");
    v.cd("../..").unwrap();
    assert_eq!(v.cwd_path(), "/");
    v.cd("/").unwrap();
    assert_eq!(v.cwd_path(), "/");
}

#[test]
fn case_insensitive_fallback_and_errors() {
    let (_d, mut v) = open();
    v.cd("internet").unwrap();
    assert_eq!(v.cwd_path(), "/Internet");
    assert!(matches!(v.cd("nope"), Err(ResolveError::NotFound(_))));
    assert!(matches!(
        v.cd("/Sample Entry"),
        Err(ResolveError::NotAGroup(_))
    ));
    assert!(matches!(
        v.resolve_entry("/Work"),
        Err(ResolveError::NotAnEntry(_))
    ));
    assert!(matches!(v.resolve_one("/Work"), Ok(NodeId::Group(_))));
    assert!(matches!(
        v.resolve_one("/Sample Entry"),
        Ok(NodeId::Entry(_))
    ));
}

#[test]
fn find_title_and_all_fields() {
    let (_d, mut v) = open();
    let hits = v.find("git", FindOptions::default());
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].number, 1);
    assert!(hits.iter().all(|h| h.path.starts_with("/Internet/GitHub")));
    let e = v.resolve_entry("2").unwrap();
    assert!(hits.iter().any(|h| h.id == e));

    let by_user = v.find("perlsaiyan", FindOptions::default());
    assert!(by_user.is_empty());
    let by_user = v.find(
        "perlsaiyan",
        FindOptions {
            all_fields: true,
            ..Default::default()
        },
    );
    assert_eq!(by_user.len(), 1);

    let old = v.find("old", FindOptions::default());
    assert_eq!(old.len(), 1);
    assert!(old[0].in_recycle_bin);

    let expired = v.find(
        "",
        FindOptions {
            expired_only: true,
            ..Default::default()
        },
    );
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].title, "db01");
}

#[test]
fn find_never_matches_on_password() {
    let (_d, mut v) = open();
    let hits = v.find(
        "s3cret",
        FindOptions {
            all_fields: true,
            ..Default::default()
        },
    );
    assert!(hits.is_empty());
}

#[test]
fn entry_view_keeps_secrets_wrapped() {
    let (_d, v) = open();
    let id = v.resolve_entry("/Sample Entry").unwrap();
    let e = v.entry(id).unwrap();
    assert_eq!(e.title, "Sample Entry");
    assert_eq!(e.username.as_deref(), Some("alice"));
    assert_eq!(e.password.as_ref().unwrap().expose_secret(), "s3cret");
    assert_eq!(e.url.as_deref(), Some("https://example.com"));
    assert_eq!(e.notes.as_deref(), Some("some notes\nsecond line"));
    assert_eq!(e.tags, ["demo"]);
    assert!(!e.has_otp);
    let names: Vec<&str> = e.custom.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["PIN", "Plain custom"]);
    assert!(matches!(
        e.custom[0].1,
        chiave_core::FieldValue::Protected(_)
    ));
    assert!(matches!(e.custom[1].1, chiave_core::FieldValue::Plain(_)));
    assert!(e.created.is_some());
}

#[test]
fn expired_entry_is_flagged() {
    let (_d, v) = open();
    let id = v.resolve_entry("/Work/Servers/db01").unwrap();
    let e = v.entry(id).unwrap();
    assert!(e.expired);
    assert!(e.expires.is_some());
    let id = v.resolve_entry("/Work/Servers/web01").unwrap();
    assert!(!v.entry(id).unwrap().expired);
}

#[test]
fn totp_generates_six_digits() {
    let (_d, mut v) = open();
    v.list(Some("/Internet")).unwrap();
    let with_otp = v
        .numbered()
        .iter()
        .copied()
        .find(|id| v.entry(*id).unwrap().has_otp)
        .unwrap();
    let code = v.totp(with_otp).unwrap();
    assert_eq!(code.code.len(), 6);
    assert!(code.code.bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(code.period_secs, 30);
    assert!(code.valid_for_secs <= 30);
    let none = v.resolve_entry("/Sample Entry").unwrap();
    assert!(matches!(v.totp(none), Err(chiave_core::VaultError::NoOtp)));
}

#[test]
fn stats_count_everything() {
    let (_d, v) = open();
    let s = v.stats();
    assert_eq!(s.groups, 6);
    assert_eq!(s.entries, 7);
    assert_eq!(s.expired, 1);
    assert_eq!(s.with_otp, 1);
    assert!(s.recycle_bin_enabled);
    assert_eq!(s.name.as_deref(), Some("Sample"));
    assert!(s.kdf.contains("Argon2"), "{}", s.kdf);
}

#[test]
fn lock_and_unlock() {
    let (_d, mut v) = open();
    v.cd("/Work").unwrap();
    let locked = v.lock();
    assert!(locked.unlock(Some("wrong".to_string().into())).is_err());
    let v = locked.unlock(Some("test".to_string().into())).unwrap();
    assert_eq!(v.cwd_path(), "/Work", "cwd survives lock/unlock");
    let mut v = v;
    let internet = v.resolve_group("/Internet").unwrap();
    v.set_cwd(internet).unwrap();
    assert_eq!(v.cwd_path(), "/Internet");
}
