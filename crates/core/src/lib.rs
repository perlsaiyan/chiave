//! chiave-core: the vault model shared by the shell, CLI and TUI front ends.

pub mod fields;
pub mod fingerprint;
pub mod open;
pub mod path;
#[cfg(any(test, feature = "test-support"))]
pub mod testdb;
pub mod vault;
pub mod write;

pub use fingerprint::Fingerprint;
pub use keepass::config::DatabaseVersion;
pub use keepass::db::{EntryId, GroupId};
pub use keepass::Database;
pub use open::{open, open_with_key, Credentials, OpenError};
pub use secrecy::{ExposeSecret, SecretString};
pub use vault::{
    is_expired, DiskState, EntryRow, EntryView, FieldValue, FindHit, FindOptions, GroupRow,
    Listing, LockedVault, NodeId, OtpCode, ResolveError, Stats, Vault, VaultError,
};
pub use write::{EntryPatch, NewEntry, SaveError, SaveOptions, SaveReport, WriteError};

/// One row of a recursive listing: a slash-separated path and whether it is a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkItem {
    pub path: String,
    pub is_group: bool,
}

/// Depth-first walk of the whole tree, groups before entries, root shown as "/".
pub fn walk(db: &Database) -> Vec<WalkItem> {
    let mut out = Vec::new();
    walk_group(db.root(), "", &mut out);
    out
}

fn walk_group(group: keepass::db::GroupRef<'_>, prefix: &str, out: &mut Vec<WalkItem>) {
    let here = format!("{prefix}/{}", group.name);
    out.push(WalkItem {
        path: here.clone(),
        is_group: true,
    });
    for g in group.groups() {
        walk_group(g, &here, out);
    }
    for e in group.entries() {
        out.push(WalkItem {
            path: format!("{here}/{}", e.get_title().unwrap_or("(untitled)")),
            is_group: false,
        });
    }
}
