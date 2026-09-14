//! Builders for a sample KDBX4 vault used in tests across the workspace.

use std::fs::File;
use std::path::{Path, PathBuf};

use chrono::{Duration, NaiveDateTime};
use keepass::db::{fields, Database, GroupMut, Times};
use keepass::DatabaseKey;

use crate::Credentials;

pub const PASSWORD: &str = "test";

fn add_entry(g: &mut GroupMut<'_>, title: &str, user: &str, pass: &str, url: Option<&str>) {
    g.add_entry().edit(|e| {
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::USERNAME, user);
        e.set_protected(fields::PASSWORD, pass);
        if let Some(u) = url {
            e.set_unprotected(fields::URL, u);
        }
    });
}

/// Layout:
/// ```text
/// /
///   Sample Entry           alice / s3cret, PIN custom field, tag "demo"
///   Empty/
///   Internet/
///     GitHub               perlsaiyan, TOTP
///     GitHub               someone-else   (duplicate title)
///     Comcast/Xfinity      slash in title
///   Work/
///     Servers/
///       db01               expired
///       web01
///   Recycle Bin/
///     Old thing
/// ```
pub fn sample() -> Database {
    let mut db = Database::new();
    db.meta.database_name = Some("Sample".into());
    {
        let mut root = db.root_mut();
        root.name = "Sample".into();
        root.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Sample Entry");
            e.set_unprotected(fields::USERNAME, "alice");
            e.set_protected(fields::PASSWORD, "s3cret");
            e.set_unprotected(fields::URL, "https://example.com");
            e.set_unprotected(fields::NOTES, "some notes\nsecond line");
            e.set_protected("PIN", "1234");
            e.set_unprotected("Plain custom", "visible");
            e.tags.push("demo".into());
        });
        root.add_group().edit(|g| g.name = "Empty".into());
        root.add_group().edit(|g| {
            g.name = "Internet".into();
            g.add_entry().edit(|e| {
                e.set_unprotected(fields::TITLE, "GitHub");
                e.set_unprotected(fields::USERNAME, "perlsaiyan");
                e.set_protected(fields::PASSWORD, "gh-pass");
                e.set_unprotected(fields::URL, "https://github.com");
                e.set_protected(
                    fields::OTP,
                    "otpauth://totp/GitHub:perlsaiyan?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&period=30&digits=6",
                );
            });
            add_entry(g, "GitHub", "someone-else", "other-pass", None);
            add_entry(g, "Comcast/Xfinity", "tom", "cable", Some("https://xfinity.com"));
        });
        root.add_group().edit(|g| {
            g.name = "Work".into();
            g.add_group().edit(|s| {
                s.name = "Servers".into();
                s.add_entry().edit(|e| {
                    e.set_unprotected(fields::TITLE, "db01");
                    e.set_unprotected(fields::USERNAME, "root");
                    e.set_protected(fields::PASSWORD, "expired-pass");
                    e.times.expires = Some(true);
                    e.times.expiry = Some(Times::now() - Duration::days(30));
                });
                add_entry(s, "web01", "deploy", "web-pass", None);
            });
        });
        let bin_id = root
            .add_group()
            .edit(|g| {
                g.name = "Recycle Bin".into();
                add_entry(g, "Old thing", "nobody", "gone", None);
            })
            .id();
        let _ = root;
        db.meta.recyclebin_enabled = Some(true);
        db.meta.recyclebin_uuid = Some(bin_id.uuid());
    }
    db
}

/// Save the sample vault into `dir` and return its path plus credentials.
pub fn sample_file(dir: &Path) -> (PathBuf, Credentials) {
    let db = sample();
    let path = dir.join("sample.kdbx");
    let key = DatabaseKey::new().with_password(PASSWORD);
    let mut f = File::create(&path).expect("create sample.kdbx");
    db.save(&mut f, key).expect("save sample.kdbx");
    (path, Credentials::password(PASSWORD))
}

/// A timestamp helper for tests.
pub fn days_ago(n: i64) -> NaiveDateTime {
    Times::now() - Duration::days(n)
}
