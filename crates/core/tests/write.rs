//! Mutations and the atomic verified save path.

use std::io::Write;
use std::process::{Command, Stdio};

use chiave_core::testdb;
use chiave_core::{
    Credentials, EntryPatch, ExposeSecret, FieldValue, FindOptions, Fingerprint, NewEntry, NodeId,
    SaveError, SaveOptions, Vault, WriteError,
};

fn open() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let (path, creds) = testdb::sample_file(dir.path());
    let v = Vault::open(&path, &creds, false).unwrap();
    (dir, v)
}

fn reopen(v: &Vault) -> Vault {
    Vault::open(v.path(), &Credentials::password(testdb::PASSWORD), false).unwrap()
}

#[test]
fn save_round_trips_with_backup_and_verification() {
    let (_d, mut v) = open();
    assert!(!v.has_unsaved_changes());
    v.mkdir("/Internet/Forums").unwrap();
    let id = v
        .new_entry(
            "/Internet/Forums/Lobsters",
            NewEntry {
                username: Some("tom".into()),
                password: Some("hunter2".to_string().into()),
                url: Some("https://lobste.rs".into()),
                custom: vec![(
                    "PIN".into(),
                    FieldValue::Protected("0000".to_string().into()),
                )],
                tags: vec!["forum".into()],
                ..Default::default()
            },
        )
        .unwrap();
    assert!(v.has_unsaved_changes());
    let report = v.save(SaveOptions::default()).unwrap();
    assert!(report.verified);
    assert!(report.bytes > 0);
    let bak = report.backup.expect("backup path");
    assert!(bak.exists());
    assert!(bak.to_string_lossy().ends_with("sample.kdbx.bak"));
    assert!(!v.has_unsaved_changes());

    let r = reopen(&v);
    let e = r.resolve_entry("/Internet/Forums/Lobsters").unwrap();
    assert_eq!(e, id);
    let view = r.entry(e).unwrap();
    assert_eq!(view.username.as_deref(), Some("tom"));
    assert_eq!(view.password.unwrap().expose_secret(), "hunter2");
    assert_eq!(view.tags, ["forum"]);
    assert!(matches!(view.custom[0].1, FieldValue::Protected(_)));
    // the untouched attachment survived too
    let s = r.resolve_entry("/Sample Entry").unwrap();
    assert_eq!(r.entry(s).unwrap().attachments, ["note.txt"]);
    // the backup is the original
    let b = Vault::open(&bak, &Credentials::password(testdb::PASSWORD), true).unwrap();
    assert!(b.resolve_entry("/Internet/Forums/Lobsters").is_err());
}

#[test]
fn new_entry_in_cwd_by_title_only() {
    let (_d, mut v) = open();
    v.cd("/Work").unwrap();
    let id = v.new_entry("VPN", NewEntry::default()).unwrap();
    assert_eq!(v.entry_path(id), "/Work/VPN");
    let id = v.new_entry(r"Slash\/Title", NewEntry::default()).unwrap();
    assert_eq!(v.entry(id).unwrap().title, "Slash/Title");
}

#[test]
fn read_only_and_kdbx3_refuse_writes() {
    let dir = tempfile::tempdir().unwrap();
    let (path, creds) = testdb::sample_file(dir.path());
    let mut ro = Vault::open(&path, &creds, true).unwrap();
    assert!(matches!(ro.mkdir("x"), Err(WriteError::ReadOnly)));
    assert!(matches!(
        ro.save(SaveOptions::default()),
        Err(SaveError::Write(_))
    ));

    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/keepassxc/Format300.kdbx");
    let copy = dir.path().join("f300.kdbx");
    std::fs::copy(fixture, &copy).unwrap();
    let mut v3 = Vault::open(&copy, &Credentials::password("a"), false).unwrap();
    assert!(!v3.can_save());
    assert!(matches!(
        v3.mkdir("x"),
        Err(WriteError::UnsupportedVersion(_))
    ));
}

#[test]
fn detects_external_modification() {
    let (_d, mut a) = open();
    let mut b = reopen(&a);
    b.mkdir("/FromB").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    b.save(SaveOptions::default()).unwrap();
    a.mkdir("/FromA").unwrap();
    assert!(matches!(
        a.save(SaveOptions::default()),
        Err(SaveError::ChangedOnDisk(_))
    ));
    a.save(SaveOptions {
        force: true,
        ..Default::default()
    })
    .unwrap();
    let r = reopen(&a);
    assert!(r.resolve_group("/FromA").is_ok());
    assert!(
        r.resolve_group("/FromB").is_err(),
        "force overwrote B's change, as documented"
    );
}

#[test]
fn edit_records_history_only_when_something_changes() {
    let (_d, mut v) = open();
    let id = v.resolve_entry("/Sample Entry").unwrap();
    assert_eq!(v.entry(id).unwrap().history_count, 0);

    let noop = EntryPatch::default().set_plain("username", "alice");
    assert!(!v.edit_entry(id, noop).unwrap());
    assert_eq!(v.entry(id).unwrap().history_count, 0);
    assert!(!v.has_unsaved_changes());

    let patch = EntryPatch::default()
        .set_secret("password", "n3w")
        .set_plain("comments", "edited")
        .remove("Plain custom");
    assert!(v.edit_entry(id, patch).unwrap());
    let e = v.entry(id).unwrap();
    assert_eq!(e.history_count, 1);
    assert_eq!(e.password.unwrap().expose_secret(), "n3w");
    assert_eq!(e.notes.as_deref(), Some("edited"));
    assert!(e.custom.iter().all(|(k, _)| k != "Plain custom"));

    let when = testdb::days_ago(-10);
    v.edit_entry(
        id,
        EntryPatch {
            expires: Some(Some(when)),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(v.entry(id).unwrap().expires.is_some());
    assert!(!v.entry(id).unwrap().expired);
    v.edit_entry(
        id,
        EntryPatch {
            expires: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(v.entry(id).unwrap().expires.is_none());
    assert_eq!(v.entry(id).unwrap().history_count, 3);

    v.save(SaveOptions::default()).unwrap();
    assert_eq!(reopen(&v).entry(id).unwrap().history_count, 3);
}

#[test]
fn rm_moves_to_recycle_bin_then_deletes_permanently() {
    let (_d, mut v) = open();
    let id = v.resolve_entry("/Work/Servers/web01").unwrap();
    v.rm_entry(id, false).unwrap();
    assert_eq!(v.entry_path(id), "/Recycle Bin/web01");
    // second rm from inside the bin is permanent
    v.rm_entry(id, false).unwrap();
    assert!(v.entry(id).is_err());
    assert!(v.db().deleted_objects.contains_key(&id.uuid()));

    let id = v.resolve_entry("/Work/Servers/db01").unwrap();
    v.rm_entry(id, true).unwrap();
    assert!(v.entry(id).is_err());
    v.save(SaveOptions::default()).unwrap();
    assert!(reopen(&v).resolve_entry("/Work/Servers/db01").is_err());
}

#[test]
fn rm_creates_recycle_bin_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.kdbx");
    let creds = Credentials::password("pw");
    // build a vault without a recycle bin using the public create() then delete the bin's uuid
    let mut v = Vault::create(&path, &creds, "Fresh").unwrap();
    let bin = v.recycle_bin_id().unwrap();
    v.rmdir(bin, true, true).unwrap_err(); // refuses to remove the bin
    let id = v.new_entry("Thing", NewEntry::default()).unwrap();
    v.rm_entry(id, false).unwrap();
    assert_eq!(v.entry_path(id), "/Recycle Bin/Thing");
}

#[test]
fn rmdir_rules() {
    let (_d, mut v) = open();
    let work = v.resolve_group("/Work").unwrap();
    assert!(matches!(
        v.rmdir(work, false, false),
        Err(WriteError::NotEmpty(_))
    ));
    let root = v.root();
    assert!(matches!(v.rmdir(root, true, true), Err(WriteError::Root)));
    let bin = v.recycle_bin_id().unwrap();
    assert!(matches!(
        v.rmdir(bin, true, true),
        Err(WriteError::RecycleBin)
    ));

    v.cd("/Work/Servers").unwrap();
    v.rmdir(work, true, false).unwrap();
    assert_eq!(v.cwd_path(), "/", "cwd falls back to root when removed");
    assert_eq!(v.group_path(work), "/Recycle Bin/Work");
    let empty = v.resolve_group("/Empty").unwrap();
    v.rmdir(empty, false, true).unwrap();
    assert!(v.resolve_group("/Empty").is_err());
}

#[test]
fn mv_and_cycle_detection() {
    let (_d, mut v) = open();
    let e = v.resolve_entry("/Sample Entry").unwrap();
    let work = v.resolve_group("/Work").unwrap();
    v.mv(NodeId::Entry(e), work).unwrap();
    assert_eq!(v.entry_path(e), "/Work/Sample Entry");
    let servers = v.resolve_group("/Work/Servers").unwrap();
    assert!(matches!(
        v.mv(NodeId::Group(work), servers),
        Err(WriteError::Cycle)
    ));
    let internet = v.resolve_group("/Internet").unwrap();
    v.mv(NodeId::Group(servers), internet).unwrap();
    assert_eq!(v.group_path(servers), "/Internet/Servers");
}

#[test]
fn copy_and_rename() {
    let (_d, mut v) = open();
    let e = v.resolve_entry("/Sample Entry").unwrap();
    let work = v.resolve_group("/Work").unwrap();
    let c = v.copy_entry(e, work, Some("Sample Copy")).unwrap();
    let view = v.entry(c).unwrap();
    assert_eq!(view.path, "/Work/Sample Copy");
    assert_eq!(view.username.as_deref(), Some("alice"));
    assert_eq!(view.password.unwrap().expose_secret(), "s3cret");
    assert_eq!(view.attachments, ["note.txt"]);
    assert_eq!(view.history_count, 0);
    v.rename_group(work, "Job").unwrap();
    assert_eq!(v.entry_path(c), "/Job/Sample Copy");
    v.save(SaveOptions::default()).unwrap();
    assert!(reopen(&v).resolve_entry("/Job/Sample Copy").is_ok());
}

#[test]
fn change_password() {
    let (_d, mut v) = open();
    v.change_credentials(&Credentials::password("new-pw"))
        .unwrap();
    v.save(SaveOptions::default()).unwrap();
    assert!(Vault::open(v.path(), &Credentials::password(testdb::PASSWORD), true).is_err());
    assert!(Vault::open(v.path(), &Credentials::password("new-pw"), true).is_ok());
}

#[test]
fn create_uses_strong_kdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.kdbx");
    let creds = Credentials::password("pw");
    let v = Vault::create(&path, &creds, "Mine").unwrap();
    assert!(!v.has_unsaved_changes());
    let s = v.stats();
    assert!(s.kdf.contains("Argon2id"), "{}", s.kdf);
    assert!(s.kdf.contains("64 MiB"), "{}", s.kdf);
    assert!(s.recycle_bin_enabled);
    assert_eq!(s.name.as_deref(), Some("Mine"));
    assert!(Vault::create(&path, &creds, "Again").is_err());
    let r = Vault::open(&path, &creds, false).unwrap();
    assert_eq!(r.cwd_path(), "/");
    assert!(r.recycle_bin_id().is_some());
}

#[test]
fn fingerprint_detects_changes() {
    let (_d, mut v) = open();
    let before = Fingerprint::of(v.db());
    assert!(before.diff(&Fingerprint::of(v.db())).is_empty());
    let id = v.resolve_entry("/Sample Entry").unwrap();
    v.edit_entry(
        id,
        EntryPatch::default().set_plain("url", "https://changed"),
    )
    .unwrap();
    let diff = before.diff(&Fingerprint::of(v.db()));
    assert_eq!(diff.len(), 1);
    assert!(diff[0].contains("Sample Entry"), "{diff:?}");
}

// ----- keepassxc-cli oracle (only with CHIAVE_KPXC_CLI) --------------------

fn kpxc() -> Option<String> {
    std::env::var("CHIAVE_KPXC_CLI")
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
}

fn run(cli: &str, args: &[&str], stdin: &str) -> (bool, String, String) {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn keepassxc_reads_chiave_edits_and_vice_versa() {
    let Some(cli) = kpxc() else {
        eprintln!("CHIAVE_KPXC_CLI not set; skipping");
        return;
    };
    let (_d, mut v) = open();
    let id = v.resolve_entry("/Sample Entry").unwrap();
    v.edit_entry(
        id,
        EntryPatch::default().set_secret("password", "edited-by-chiave"),
    )
    .unwrap();
    v.new_entry(
        "/Work/New from chiave",
        NewEntry {
            username: Some("u".into()),
            password: Some("p".to_string().into()),
            ..Default::default()
        },
    )
    .unwrap();
    let web = v.resolve_entry("/Work/Servers/web01").unwrap();
    v.rm_entry(web, false).unwrap();
    v.save(SaveOptions::default()).unwrap();
    let p = v.path().to_str().unwrap().to_string();
    let pw = format!("{}\n", testdb::PASSWORD);

    let (ok, out, err) = run(
        &cli,
        &["show", "-s", "-a", "Password", &p, "/Sample Entry"],
        &pw,
    );
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "edited-by-chiave");
    let (ok, out, _) = run(&cli, &["ls", "-R", "-f", &p], &pw);
    assert!(ok);
    assert!(out.contains("New from chiave"), "{out}");
    assert!(
        out.contains("Recycle Bin/web01") || out.contains("web01"),
        "{out}"
    );
    let (ok, out, _) = run(
        &cli,
        &[
            "show",
            "-a",
            "Password",
            "--show-protected",
            &p,
            "/Sample Entry",
        ],
        &pw,
    );
    assert!(ok || !out.is_empty());

    // KeePassXC writes, chiave reads.
    let (ok, _, err) = run(
        &cli,
        &[
            "add",
            "-u",
            "kpxc-user",
            "-p",
            &p,
            "/Internet/Added by KeePassXC",
        ],
        &format!("{pw}kpxc-pass\n"),
    );
    assert!(ok, "{err}");
    let mut r = reopen(&v);
    let added = r.resolve_entry("/Internet/Added by KeePassXC").unwrap();
    let view = r.entry(added).unwrap();
    assert_eq!(view.username.as_deref(), Some("kpxc-user"));
    assert_eq!(view.password.unwrap().expose_secret(), "kpxc-pass");
    // and our stale handle notices the external change
    let mut stale = v;
    stale.mkdir("/late").unwrap();
    assert!(matches!(
        stale.save(SaveOptions::default()),
        Err(SaveError::ChangedOnDisk(_))
    ));
    // history written by chiave is visible to KeePassXC
    let (ok, out, _) = run(&cli, &["show", "-a", "Title", &p, "/Sample Entry"], &pw);
    assert!(ok);
    assert_eq!(out.trim(), "Sample Entry");
    let hits = r.find(
        "kpxc",
        FindOptions {
            all_fields: true,
            ..Default::default()
        },
    );
    assert_eq!(hits.len(), 1);
}

#[test]
fn upgrade_kdbx3_to_kdbx4_preserves_content() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/keepassxc/NewDatabase.kdbx");
    let copy = dir.path().join("up.kdbx");
    std::fs::copy(fixture, &copy).unwrap();
    let creds = Credentials::password("a");
    let mut v = Vault::open(&copy, &creds, false).unwrap();
    assert!(!v.can_save());
    let before = Fingerprint::of(v.db());
    assert!(v.upgrade_to_kdbx4().unwrap());
    assert!(!v.upgrade_to_kdbx4().unwrap());
    assert!(v.can_save());
    let report = v.save(SaveOptions::default()).unwrap();
    assert!(report.backup.is_some());
    let r = Vault::open(&copy, &creds, false).unwrap();
    assert_eq!(r.version().to_string(), "KDBX4.1");
    assert!(before.diff(&Fingerprint::of(r.db())).is_empty());
    assert!(r.stats().kdf.contains("Argon2id"));
    let old = Vault::open(&report.backup.unwrap(), &creds, true).unwrap();
    assert_eq!(old.version().to_string(), "KDBX3.1");
}

#[test]
fn attachments_add_read_remove_round_trip() {
    let (_d, mut v) = open();
    let id = v.resolve_entry("/Sample Entry").unwrap();
    assert_eq!(v.attachments(id).unwrap(), [("note.txt".to_string(), 5)]);
    assert_eq!(&*v.attachment_data(id, "note.txt").unwrap(), b"hello");

    v.add_attachment(id, "key.pem", b"-----BEGIN-----".to_vec())
        .unwrap();
    assert_eq!(v.entry(id).unwrap().history_count, 1);
    v.remove_attachment(id, "note.txt").unwrap();
    assert_eq!(v.entry(id).unwrap().history_count, 2);
    assert!(matches!(
        v.remove_attachment(id, "nope"),
        Err(WriteError::Resolve(chiave_core::ResolveError::NotFound(_)))
    ));
    // keepass-rs issue #360: removing an attachment must not corrupt history on save
    let report = v.save(SaveOptions::default()).unwrap();
    assert!(report.verified);
    let r = reopen(&v);
    assert_eq!(r.attachments(id).unwrap(), [("key.pem".to_string(), 15)]);
    assert_eq!(r.entry(id).unwrap().history_count, 2);

    if let Some(cli) = kpxc() {
        let p = r.path().to_str().unwrap();
        let pw = format!("{}\n", testdb::PASSWORD);
        let out_file = _d.path().join("exported.pem");
        let (ok, _, err) = run(
            &cli,
            &[
                "attachment-export",
                p,
                "/Sample Entry",
                "key.pem",
                out_file.to_str().unwrap(),
            ],
            &pw,
        );
        assert!(ok, "{err}");
        assert_eq!(std::fs::read(out_file).unwrap(), b"-----BEGIN-----");
    }
}

#[test]
fn shared_binary_survives_removal_from_one_entry() {
    let Some(cli) = kpxc() else {
        eprintln!("CHIAVE_KPXC_CLI not set; skipping");
        return;
    };
    let (_d, v) = open();
    let p = v.path().to_str().unwrap().to_string();
    let pw = format!("{}\n", testdb::PASSWORD);
    let src = _d.path().join("shared.bin");
    std::fs::write(&src, b"same bytes in two entries").unwrap();
    for entry in ["/Work/Servers/web01", "/Work/Servers/db01"] {
        let (ok, _, err) = run(
            &cli,
            &[
                "attachment-import",
                &p,
                entry,
                "shared.bin",
                src.to_str().unwrap(),
            ],
            &pw,
        );
        assert!(ok, "{err}");
    }
    let mut v = reopen(&v);
    let web = v.resolve_entry("/Work/Servers/web01").unwrap();
    let db01 = v.resolve_entry("/Work/Servers/db01").unwrap();
    assert_eq!(v.attachments(web).unwrap()[0].0, "shared.bin");
    v.remove_attachment(web, "shared.bin").unwrap();
    v.save(SaveOptions::default()).unwrap();
    let r = reopen(&v);
    assert!(r.attachments(web).unwrap().is_empty());
    assert_eq!(
        &*r.attachment_data(db01, "shared.bin").unwrap(),
        b"same bytes in two entries"
    );
    let out = _d.path().join("out.bin");
    let (ok, _, err) = run(
        &cli,
        &[
            "attachment-export",
            &p,
            "/Work/Servers/db01",
            "shared.bin",
            out.to_str().unwrap(),
        ],
        &pw,
    );
    assert!(ok, "{err}");
    assert_eq!(std::fs::read(out).unwrap(), b"same bytes in two entries");
    // history: one snapshot from KeePassXC's import, one from chiave's removal
    assert_eq!(r.entry(web).unwrap().history_count, 2);
}

#[test]
fn accessors_for_front_ends() {
    let (_d, v) = open();
    assert!(v.keyfile().is_none());
    let e = v.resolve_entry("/Work/Servers/web01").unwrap();
    let g = v.resolve_group("/Work/Servers").unwrap();
    assert_eq!(v.entry_parent(e), Some(g));
    assert_eq!(
        chiave_core::split_spec("/a/b/Name").unwrap(),
        ("/a/b/".to_string(), "Name".to_string())
    );
    assert_eq!(
        chiave_core::split_spec("Name").unwrap(),
        (String::new(), "Name".to_string())
    );
    assert_eq!(
        chiave_core::split_spec(r"x/Sl\/ash").unwrap(),
        ("x/".to_string(), "Sl/ash".to_string())
    );
}
