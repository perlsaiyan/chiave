//! Interop oracle: KeePassXC must be able to read what chiave (via keepass-rs) writes.
//! Runs only when CHIAVE_KPXC_CLI points at a keepassxc-cli binary.

use std::io::Write;
use std::process::{Command, Stdio};

use chiave_core::testdb;

fn kpxc() -> Option<String> {
    std::env::var("CHIAVE_KPXC_CLI")
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
}

fn run(cli: &str, args: &[&str], password: &str) -> String {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn keepassxc-cli");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{password}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "keepassxc-cli {args:?} failed:\n{stdout}{stderr}"
    );
    stdout
}

#[test]
fn keepassxc_reads_chiave_written_vault() {
    let Some(cli) = kpxc() else {
        eprintln!("CHIAVE_KPXC_CLI not set; skipping interop test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = testdb::sample_file(dir.path());
    let p = path.to_str().unwrap();

    let ls = run(&cli, &["ls", "-R", "-f", p], testdb::PASSWORD);
    for expected in [
        "Sample Entry",
        "Internet/",
        "GitHub",
        "Comcast/Xfinity",
        "Work/",
        "Servers/",
        "db01",
        "web01",
        "Recycle Bin/",
        "Old thing",
    ] {
        assert!(ls.contains(expected), "missing {expected:?} in:\n{ls}");
    }

    let show = run(
        &cli,
        &["show", "-s", "-a", "Password", p, "/Sample Entry"],
        testdb::PASSWORD,
    );
    assert_eq!(show.trim(), "s3cret");

    let totp = run(
        &cli,
        &["show", "-t", p, "/Internet/GitHub"],
        testdb::PASSWORD,
    );
    let code = totp.trim();
    assert_eq!(code.len(), 6, "totp output: {totp}");

    let info = run(&cli, &["db-info", p], testdb::PASSWORD);
    assert!(info.contains("Recycle bin is enabled"), "{info}");
}
