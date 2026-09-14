//! save / saveas / passwd / newdb / upgrade, and the unsaved-changes rules that
//! guard `quit`, `close` and the idle lock.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use chiave_core::{Credentials, Vault};
use chiave_shell::{Flow, ShellOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/keepassxc")
        .join(name)
}

#[test]
fn save_writes_a_backup_and_clears_the_dirty_marker() {
    let mut h = common::harness();
    h.ok("mkdir /Fresh");
    assert!(h.shell.is_dirty());
    assert_eq!(h.shell.prompt_string(), "chiave:/*> ");

    let path = h.path();
    let out = h.ok("save");
    assert!(out.contains(&format!("Saved {}", path.display())), "{out}");
    assert!(out.contains("verified"), "{out}");
    let bak = path.with_extension("kdbx.bak");
    assert!(out.contains(&format!("Backup: {}", bak.display())), "{out}");
    assert!(bak.exists());

    assert!(!h.shell.is_dirty());
    assert_eq!(h.shell.prompt_string(), "chiave:/> ");

    let reopened = Vault::open(&path, &Credentials::password("test"), true).unwrap();
    assert!(reopened.resolve_group("/Fresh").is_ok());
    // The backup is the database as it was before the save.
    let old = Vault::open(&bak, &Credentials::password("test"), true).unwrap();
    assert!(old.resolve_group("/Fresh").is_err());
}

#[test]
fn save_explains_an_external_change_and_force_overrides_it() {
    let mut h = common::harness();
    let path = h.path();
    // Another writer gets there first.
    let mut other = Vault::open(&path, &Credentials::password("test"), false).unwrap();
    other.mkdir("/FromElsewhere").unwrap();
    std::thread::sleep(Duration::from_millis(20));
    other.save(Default::default()).unwrap();

    h.ok("mkdir /Mine");
    let out = h.run("save");
    assert!(out.contains("changed on disk"), "{out}");
    assert!(out.contains("save --force"), "{out}");
    assert!(h.shell.is_dirty());

    assert!(h.ok("save --force").contains("Saved"));
    let reopened = Vault::open(&path, &Credentials::password("test"), true).unwrap();
    assert!(reopened.resolve_group("/Mine").is_ok());
}

#[test]
fn saveas_writes_a_second_file_and_follows_it() {
    let mut h = common::harness();
    h.ok("mkdir /Copied");
    let dest = h.dir.path().join("elsewhere.kdbx");
    let out = h.ok(&format!("saveas {}", dest.display()));
    assert!(out.contains(&format!("Saved {}", dest.display())), "{out}");
    assert!(dest.exists());
    assert!(!h.shell.is_dirty());
    assert_eq!(h.path(), dest, "the vault follows its new file");

    let v = Vault::open(&dest, &Credentials::password("test"), true).unwrap();
    assert!(v.resolve_group("/Copied").is_ok());
}

#[test]
fn passwd_changes_the_master_password_on_the_next_save() {
    let mut h = common::harness();
    let path = h.path();
    let script = h.script(["new-master", "new-master"]);
    let out = h.ok("passwd");
    assert_eq!(script.remaining(), 0);
    assert!(out.contains("Run `save`"), "{out}");
    h.ok("save");

    assert!(Vault::open(&path, &Credentials::password("test"), true).is_err());
    let v = Vault::open(&path, &Credentials::password("new-master"), true).unwrap();
    assert!(v.resolve_entry("/Sample Entry").is_ok());
}

#[test]
fn passwd_refuses_mismatched_answers() {
    let mut h = common::harness();
    h.script(["one", "two"]);
    assert!(h.run("passwd").contains("do not match"));
    h.script(["", ""]);
    assert!(h.run("passwd").contains("needs a key file"));
}

#[test]
fn newdb_creates_a_database_and_switches_to_it() {
    let mut h = common::harness();
    let path = h.dir.path().join("brand-new.kdbx");
    let script = h.script(["opensesame", "opensesame"]);
    let out = h.ok(&format!("newdb {}", path.display()));
    assert_eq!(script.remaining(), 0);
    assert!(
        out.contains(&format!("Created {}", path.display())),
        "{out}"
    );
    assert!(path.exists());

    assert_eq!(h.path(), path);
    assert!(!h.shell.is_dirty());
    let stats = h.ok("stats");
    assert!(stats.contains("Name: brand-new"), "{stats}");
    assert!(stats.contains("Version: KDBX4"), "{stats}");
    assert!(stats.contains("Recycle bin: enabled"), "{stats}");

    assert!(Vault::open(&path, &Credentials::password("opensesame"), true).is_ok());
    // A second newdb over the same file refuses.
    h.script(["opensesame", "opensesame"]);
    assert!(h
        .run(&format!("newdb {}", path.display()))
        .contains("cannot write"));
}

#[test]
fn upgrade_converts_a_kdbx3_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("NewDatabase.kdbx");
    std::fs::copy(fixture("NewDatabase.kdbx"), &path).unwrap();
    let vault = Vault::open(&path, &Credentials::password("a"), false).unwrap();
    let mut h = common::harness_from(dir, vault, ShellOptions::default());

    assert!(h.ok("stats").contains("Version: KDBX3"));
    // Nothing may be written until it is converted.
    let out = h.run("mkdir /Nope");
    assert!(out.contains("cannot be saved"), "{out}");
    assert!(out.contains("upgrade"), "{out}");

    let out = h.ok("upgrade");
    assert!(out.contains("KDBX4"), "{out}");
    assert!(out.contains("save"), "{out}");
    assert!(h.shell.is_dirty());

    h.ok("mkdir /Now_It_Works");
    assert!(h.ok("save").contains("Backup:"));
    assert!(path.with_extension("kdbx.bak").exists());

    let v = Vault::open(&path, &Credentials::password("a"), true).unwrap();
    assert!(v.stats().version.contains("KDBX4"), "{}", v.stats().version);
    assert!(v.resolve_group("/Now_It_Works").is_ok());
    // The backup is still the KDBX3 original.
    let old = Vault::open(
        &path.with_extension("kdbx.bak"),
        &Credentials::password("a"),
        true,
    )
    .unwrap();
    assert!(old.resolve_group("/Now_It_Works").is_err());

    assert!(h.ok("upgrade").contains("Already KDBX4"));
}

// ----- unsaved changes -----------------------------------------------------

#[test]
fn quit_refuses_to_discard_unsaved_changes() {
    let mut h = common::harness();
    h.ok("mkdir /Unsaved");
    let (flow, out) = h.run_flow("quit");
    assert_eq!(flow, Flow::Continue);
    assert!(
        out.contains("Unsaved changes; run save, or quit --force / close --force to discard"),
        "{out}"
    );
    assert_eq!(h.run_flow("quit --force").0, Flow::Quit);
}

#[test]
fn close_refuses_to_discard_unsaved_changes() {
    let mut h = common::harness();
    h.ok("mkdir /Unsaved");
    assert!(h.run("close").contains("Unsaved changes"));
    assert!(h.shell.vault().is_some());
    assert!(h.ok("close --force").contains("Closed"));
    assert!(h.shell.vault().is_none());
}

#[test]
fn quit_and_close_are_silent_once_everything_is_saved() {
    let mut h = common::harness();
    h.ok("mkdir /Saved");
    h.ok("save");
    assert_eq!(h.run_flow("quit").0, Flow::Quit);
    assert!(h.ok("close").contains("Closed"));
}

#[test]
fn the_idle_lock_refuses_to_discard_unsaved_changes() {
    let mut h = common::harness();
    h.ok("mkdir /Unsaved");
    h.shell.options_mut().timeout = Some(Duration::ZERO);

    let out = h.ok("pwd");
    assert!(out.contains("unsaved changes"), "{out}");
    assert!(out.contains("stays unlocked"), "{out}");
    assert!(!h.shell.is_locked());
    assert_eq!(h.prompt.calls(), 0, "it must not ask for the password");

    // Saving lets the idle lock do its job again.
    h.ok("save");
    let out = h.ok("pwd");
    assert!(out.contains("Idle for too long"), "{out}");
    assert_eq!(h.prompt.calls(), 1);
}

#[test]
fn a_read_only_database_refuses_every_write() {
    let mut h = common::harness_with(ShellOptions {
        read_only: true,
        ..ShellOptions::default()
    });
    for line in [
        "mkdir /X",
        "rmdir /Empty",
        "rename /Empty Other",
        "new --title X /X",
        "set '/Sample Entry' url http://x",
        "rm -f '/Sample Entry'",
        "mv '/Sample Entry' /Work",
        "cp '/Sample Entry' /Work",
        "save",
    ] {
        let out = h.run(line);
        assert!(
            out.contains("read-only"),
            "`{line}` should refuse a read-only database:\n{out}"
        );
    }
}

#[test]
fn switching_databases_will_not_discard_unsaved_changes() {
    let mut h = common::harness();
    h.ok("mkdir /Unsaved");
    let other = h.dir.path().join("other.kdbx");
    h.script(["pw", "pw"]);
    assert!(h
        .run(&format!("newdb {}", other.display()))
        .contains("Unsaved changes"));
    assert!(!other.exists());

    let existing = h.path();
    h.script(["test"]);
    assert!(h
        .run(&format!("open {}", existing.display()))
        .contains("Unsaved changes"));
    assert!(h.shell.vault().unwrap().resolve_group("/Unsaved").is_ok());
}

#[test]
fn upgrade_converts_a_kdb1_file_to_a_new_kdbx_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/keepassxc/basic.kdb");
    let src = dir.path().join("basic.kdb");
    std::fs::copy(&fixture, &src).unwrap();
    let original = std::fs::read(&src).unwrap();
    let vault =
        chiave_core::Vault::open(&src, &chiave_core::Credentials::password("masterpw"), false)
            .unwrap();
    let mut h = common::harness_from(dir, vault, chiave_shell::shell::ShellOptions::default());
    let out = h.ok("upgrade");
    assert!(out.contains("saved to"), "{out}");
    assert!(out.contains("basic.kdbx"), "{out}");
    assert!(out.contains("was not modified"), "{out}");
    let new_path = src.with_extension("kdbx");
    assert!(new_path.exists());
    assert_eq!(
        std::fs::read(&src).unwrap(),
        original,
        "the .kdb is untouched"
    );
    assert!(!h.run("quit").contains("Unsaved"), "already saved");
    let r = chiave_core::Vault::open(
        &new_path,
        &chiave_core::Credentials::password("masterpw"),
        false,
    )
    .unwrap();
    assert_eq!(r.version().to_string(), "KDBX4.1");
    // second run on the same source refuses to clobber the new file
    let vault =
        chiave_core::Vault::open(&src, &chiave_core::Credentials::password("masterpw"), false)
            .unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let mut h2 = common::harness_from(dir2, vault, chiave_shell::shell::ShellOptions::default());
    let err = h2.run("upgrade");
    assert!(err.contains("already exists"), "{err}");
}
