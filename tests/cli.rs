//! The one-shot CLI: `chiave --kdb v.kdbx <command>`. A mutating command has no
//! later `save` to rely on, so it writes the database itself unless `--no-save`
//! says otherwise.

use std::path::Path;
use std::process::Command;

use chiave_core::{testdb, Credentials, Vault};
use tempfile::TempDir;

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
}

/// Run the real binary against `db`, with the master password in the
/// environment and HOME pointed at the temporary directory so the developer's
/// own configuration file cannot influence the result.
fn chiave(dir: &TempDir, db: &Path, args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_chiave"))
        .arg("--kdb")
        .arg(db)
        .arg("--no-clip")
        .args(args)
        .env("CHIAVE_PASSWORD", testdb::PASSWORD)
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env("XDG_STATE_HOME", dir.path())
        .env_remove("CHIAVE_KDB")
        .env_remove("CHIAVE_KEYFILE")
        .output()
        .expect("spawn chiave");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

fn sample() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, _creds) = testdb::sample_file(dir.path());
    (dir, path)
}

fn reopen(path: &Path) -> Vault {
    Vault::open(path, &Credentials::password(testdb::PASSWORD), true).expect("reopen")
}

#[test]
fn a_read_only_command_prints_and_saves_nothing() {
    let (dir, db) = sample();
    let r = chiave(&dir, &db, &["show", "/Sample Entry"]);
    assert!(r.ok, "{}", r.stderr);
    assert!(r.stdout.contains("Uname: alice"), "{}", r.stdout);
    assert!(!r.stdout.contains("Saved"), "{}", r.stdout);
    assert!(!db.with_extension("kdbx.bak").exists());
}

#[test]
fn a_mutating_command_saves_before_exiting() {
    let (dir, db) = sample();
    let r = chiave(&dir, &db, &["mkdir", "/Internet/Forums"]);
    assert!(r.ok, "{}", r.stderr);
    assert!(
        r.stdout.contains("Created /Internet/Forums"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("Saved"), "{}", r.stdout);
    assert!(r.stdout.contains("Backup:"), "{}", r.stdout);
    assert!(reopen(&db).resolve_group("/Internet/Forums").is_ok());
}

#[test]
fn new_with_flags_works_without_a_terminal() {
    let (dir, db) = sample();
    let r = chiave(
        &dir,
        &db,
        &[
            "new",
            "--title",
            "Lobsters",
            "--user",
            "tom",
            "--generate",
            "--length",
            "24",
            "/Internet/Lobsters",
        ],
    );
    assert!(r.ok, "{}", r.stderr);
    assert!(r.stdout.contains("Generated 24 characters"), "{}", r.stdout);
    assert!(r.stdout.contains("Saved"), "{}", r.stdout);

    let v = reopen(&db);
    let id = v
        .resolve_entry("/Internet/Lobsters")
        .expect("the new entry");
    let view = v.entry(id).expect("view");
    assert_eq!(view.username.as_deref(), Some("tom"));
    let password = view.password.expect("a generated password");
    use chiave_core::ExposeSecret;
    assert_eq!(password.expose_secret().chars().count(), 24);
    assert!(
        !r.stdout.contains(password.expose_secret()),
        "the generated password leaked:\n{}",
        r.stdout
    );
}

#[test]
fn no_save_leaves_the_file_alone() {
    let (dir, db) = sample();
    let r = chiave(&dir, &db, &["--no-save", "mkdir", "/Ephemeral"]);
    assert!(r.ok, "{}", r.stderr);
    assert!(r.stdout.contains("Created /Ephemeral"), "{}", r.stdout);
    assert!(!r.stdout.contains("Saved"), "{}", r.stdout);
    assert!(reopen(&db).resolve_group("/Ephemeral").is_err());
}

#[test]
fn a_failing_command_reports_on_stderr_and_exits_non_zero() {
    let (dir, db) = sample();
    let r = chiave(&dir, &db, &["mkdir", "/Internet"]);
    assert!(!r.ok);
    assert!(r.stderr.contains("already exists"), "{}", r.stderr);
    assert!(!db.with_extension("kdbx.bak").exists(), "nothing was saved");
}
