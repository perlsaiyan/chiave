//! Smoke tests against the KeePassXC fixture corpus in tests/fixtures/keepassxc.

use std::path::PathBuf;

use chiave_core::{open, walk, Credentials};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/keepassxc")
        .join(name)
}

#[test]
fn opens_kdbx4_format400() {
    let db = open(&fixture("Format400.kdbx"), &Credentials::password("t")).expect("open");
    let root = db.root();
    assert_eq!(root.name, "Format400");
    let entry = root.entry_by_name("Format400").expect("entry");
    assert_eq!(entry.get_username(), Some("Format400"));
    assert_eq!(entry.get("Format400"), Some("Format400"));
    let att = entry.attachment_by_name("Format400").expect("attachment");
    assert_eq!(att.data.get().as_slice(), b"Format400\n");
}

#[test]
fn walks_format400() {
    let db = open(&fixture("Format400.kdbx"), &Credentials::password("t")).expect("open");
    let items = walk(&db);
    let paths: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"/Format400"), "{paths:?}");
    assert!(paths.contains(&"/Format400/Format400"), "{paths:?}");
}

#[test]
fn wrong_password_is_an_error() {
    assert!(open(&fixture("Format400.kdbx"), &Credentials::password("wrong")).is_err());
}

#[test]
fn no_credentials_is_an_error() {
    assert!(open(&fixture("Format400.kdbx"), &Credentials::default()).is_err());
}

#[test]
fn opens_kdbx3_format300() {
    let db = open(&fixture("Format300.kdbx"), &Credentials::password("a")).expect("open");
    assert_eq!(db.root().name, "Format300");
}

#[test]
fn opens_kdbx3_protected_strings() {
    let db = open(
        &fixture("ProtectedStrings.kdbx"),
        &Credentials::password("masterpw"),
    )
    .expect("open");
    let n = walk(&db).iter().filter(|i| !i.is_group).count();
    assert!(n > 0);
}

#[test]
fn opens_kdbx3_compressed_with_empty_password() {
    let db = open(&fixture("Compressed.kdbx"), &Credentials::password("")).expect("open");
    assert!(!walk(&db).is_empty());
}

#[test]
fn opens_kdbx3_non_ascii_password() {
    let db = open(
        &fixture("NonAscii.kdbx"),
        &Credentials::password("\u{394}\u{f6}\u{636}"),
    )
    .expect("open");
    assert!(!walk(&db).is_empty());
}

// keepass-rs 0.13 does not verify the KDBX3 header hash stored in Meta/HeaderHash,
// so a tampered header opens without error. Tracked as a chiave-side check to add.
#[test]
#[ignore = "upstream gap: keepass-rs does not verify the KDBX3 header hash"]
fn broken_header_hash_is_rejected() {
    assert!(open(
        &fixture("BrokenHeaderHash.kdbx"),
        &Credentials::password("")
    )
    .is_err());
}

fn keyfile_only(db: &str, key: &str) {
    let creds = Credentials::default().with_keyfile(fixture(key));
    let db = open(&fixture(db), &creds).unwrap_or_else(|e| panic!("{db}: {e}"));
    assert!(!walk(&db).is_empty());
}

#[test]
#[ignore = "fixture is a KDBX 2.x (pre-3.1) file, which keepass-rs cannot open"]
fn opens_with_xml_keyfile() {
    keyfile_only("FileKeyXml.kdbx", "FileKeyXml.key");
}

#[test]
fn opens_with_xml_v2_keyfile() {
    keyfile_only("FileKeyXmlV2.kdbx", "FileKeyXmlV2.keyx");
}

#[test]
#[ignore = "fixture is a KDBX 2.x (pre-3.1) file, which keepass-rs cannot open"]
fn opens_with_binary_keyfile() {
    keyfile_only("FileKeyBinary.kdbx", "FileKeyBinary.key");
}

#[test]
#[ignore = "fixture is a KDBX 2.x (pre-3.1) file, which keepass-rs cannot open"]
fn opens_with_hex_keyfile() {
    keyfile_only("FileKeyHex.kdbx", "FileKeyHex.key");
}

#[test]
#[ignore = "fixture is a KDBX 2.x (pre-3.1) file, which keepass-rs cannot open"]
fn opens_with_hashed_keyfile() {
    keyfile_only("FileKeyHashed.kdbx", "FileKeyHashed.key");
}
