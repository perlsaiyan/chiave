use chiave_core::testdb;
use chiave_core::{AgeField, EntryPatch, PurgeOptions, Vault};

fn open() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let (path, creds) = testdb::sample_file(dir.path());
    (dir, Vault::open(&path, &creds, false).unwrap())
}

#[test]
fn pwck_scores_reuse_and_empties() {
    let (_d, mut v) = open();
    let a = v.resolve_entry("/Sample Entry").unwrap();
    let b = v.resolve_entry("/Work/Servers/web01").unwrap();
    v.edit_entry(
        a,
        EntryPatch::default().set_secret("password", "password123"),
    )
    .unwrap();
    v.edit_entry(
        b,
        EntryPatch::default().set_secret("password", "password123"),
    )
    .unwrap();
    let c = v.new_entry("/Work/blank", Default::default()).unwrap();

    let rows = v.pwck(v.root(), true);
    let row = |p: &str| rows.iter().find(|r| r.path == p).unwrap();
    let weak = row("/Sample Entry");
    assert!(weak.score.unwrap() <= 1, "{weak:?}");
    assert!(weak.is_weak());
    assert_eq!(weak.reused_with, ["/Work/Servers/web01"]);
    assert!(weak.warning.is_some() || !weak.suggestions.is_empty());
    let blank = rows.iter().find(|r| r.id == c).unwrap();
    assert!(blank.empty && blank.is_weak() && blank.score.is_none());
    let expired = row("/Work/Servers/db01");
    assert!(expired.expired);

    // scoped, non-recursive
    let work = v.resolve_group("/Work").unwrap();
    let scoped = v.pwck(work, false);
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].path, "/Work/blank");
}

#[test]
fn purge_by_expiry_and_modified() {
    let (_d, mut v) = open();
    let root = v.root();
    let opts = PurgeOptions {
        field: AgeField::Expiry,
        older_than_days: 7,
        recursive: true,
        permanent: false,
    };
    let cands = v.purge_candidates(root, opts);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].path, "/Work/Servers/db01");
    let removed = v.purge(root, opts).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(v.entry_path(removed[0].id), "/Recycle Bin/db01");

    // nothing was modified more than a day ago in the fresh sample
    let none = v.purge_candidates(
        root,
        PurgeOptions {
            field: AgeField::Modified,
            older_than_days: 1,
            recursive: true,
            permanent: true,
        },
    );
    assert!(none.is_empty());
    // everything is "older than -1 days", i.e. cutoff in the future, non-recursive only sees root entries
    let all_root = v.purge_candidates(
        root,
        PurgeOptions {
            field: AgeField::Created,
            older_than_days: -1,
            recursive: false,
            permanent: true,
        },
    );
    assert_eq!(all_root.len(), 1);
    assert_eq!(all_root[0].path, "/Sample Entry");
}
