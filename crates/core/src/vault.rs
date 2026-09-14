//! An open database plus the shell state that kpcli keeps around it:
//! a current group, and the numbered listing produced by the last `ls`/`find`.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use keepass::config::DatabaseVersion;
use keepass::db::{EntryId, EntryRef, GroupId, GroupRef, Icon, Times};
use keepass::{Database, DatabaseKey};
use secrecy::SecretString;
use thiserror::Error;
use uuid::Uuid;

use crate::open::{open_with_key, Credentials, OpenError};
use crate::path::{self, Segment};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    Group(GroupId),
    Entry(EntryId),
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("{0}: not found")]
    NotFound(String),
    #[error("{0}: ambiguous, {1} matches (use a number from ls or find)")]
    Ambiguous(String, usize),
    #[error("{0}: not a group")]
    NotAGroup(String),
    #[error("{0}: not an entry")]
    NotAnEntry(String),
    #[error("no item number {0} in the last listing")]
    BadNumber(usize),
}

#[derive(Debug, Error)]
pub enum VaultError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error("entry has no TOTP configured")]
    NoOtp,
    #[error("TOTP error: {0}")]
    Otp(#[from] keepass::db::TOTPError),
    #[error("clock error: {0}")]
    Clock(#[from] std::time::SystemTimeError),
    #[error("entry vanished from the database")]
    Gone,
}

#[derive(Debug, Clone)]
pub struct GroupRow {
    pub id: GroupId,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct EntryRow {
    /// 1-based number usable in later commands, or 0 if not numbered.
    pub number: usize,
    pub id: EntryId,
    pub title: String,
    pub username: Option<String>,
    pub url: Option<String>,
    pub expired: bool,
    pub has_otp: bool,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub group: GroupId,
    pub path: String,
    pub groups: Vec<GroupRow>,
    pub entries: Vec<EntryRow>,
}

#[derive(Debug, Clone)]
pub struct FindHit {
    pub number: usize,
    pub id: EntryId,
    pub path: String,
    pub title: String,
    pub username: Option<String>,
    pub in_recycle_bin: bool,
    pub expired: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FindOptions {
    /// Search every text field, not only the title.
    pub all_fields: bool,
    /// Return only expired entries.
    pub expired_only: bool,
}

/// A protected or plain field value.
#[derive(Clone)]
pub enum FieldValue {
    Plain(String),
    Protected(SecretString),
}

/// Everything `show` needs about one entry. Secrets stay wrapped.
pub struct EntryView {
    pub id: EntryId,
    pub uuid: Uuid,
    pub path: String,
    pub title: String,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub has_otp: bool,
    /// Custom fields in name order, standard fields excluded.
    pub custom: Vec<(String, FieldValue)>,
    pub tags: Vec<String>,
    pub attachments: Vec<String>,
    pub icon: Option<String>,
    pub history_count: usize,
    pub created: Option<NaiveDateTime>,
    pub modified: Option<NaiveDateTime>,
    pub accessed: Option<NaiveDateTime>,
    pub expires: Option<NaiveDateTime>,
    pub expired: bool,
}

#[derive(Debug, Clone)]
pub struct OtpCode {
    pub code: String,
    pub valid_for_secs: u64,
    pub period_secs: u64,
}

#[derive(Debug, Clone)]
pub struct Stats {
    pub path: PathBuf,
    pub name: Option<String>,
    pub version: String,
    pub kdf: String,
    pub cipher: String,
    pub groups: usize,
    pub entries: usize,
    pub expired: usize,
    pub with_otp: usize,
    pub recycle_bin_enabled: bool,
    pub read_only: bool,
}

/// A vault that has been locked: keeps enough to re-open it without the key file path
/// being asked again, but holds no key material.
pub struct LockedVault {
    pub path: PathBuf,
    pub keyfile: Option<PathBuf>,
    pub read_only: bool,
}

impl LockedVault {
    pub fn unlock(self, password: Option<SecretString>) -> Result<Vault, OpenError> {
        let creds = Credentials {
            password,
            keyfile: self.keyfile,
        };
        Vault::open(&self.path, &creds, self.read_only)
    }
}

pub struct Vault {
    db: Database,
    key: DatabaseKey,
    path: PathBuf,
    keyfile: Option<PathBuf>,
    read_only: bool,
    cwd: GroupId,
    listing: Vec<EntryId>,
}

impl Vault {
    pub fn open(path: &Path, creds: &Credentials, read_only: bool) -> Result<Vault, OpenError> {
        let key = creds.to_key()?;
        let db = open_with_key(path, key.clone())?;
        let cwd = db.root().id();
        Ok(Vault {
            db,
            key,
            path: path.to_path_buf(),
            keyfile: creds.keyfile.clone(),
            read_only,
            cwd,
            listing: Vec::new(),
        })
    }

    /// Drop the database and key material, keeping only what is needed to reopen.
    pub fn lock(self) -> LockedVault {
        LockedVault {
            path: self.path,
            keyfile: self.keyfile,
            read_only: self.read_only,
        }
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn key(&self) -> &DatabaseKey {
        &self.key
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> DatabaseVersion {
        self.db.config.version.clone()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// True when keepass-rs can write this database back (KDBX4 only) and it is not read-only.
    pub fn can_save(&self) -> bool {
        !self.read_only && matches!(self.version(), DatabaseVersion::KDB4(_))
    }

    // ----- navigation -------------------------------------------------------

    pub fn cwd(&self) -> GroupId {
        self.cwd
    }

    pub fn cwd_path(&self) -> String {
        self.group_path(self.cwd)
    }

    pub fn root(&self) -> GroupId {
        self.db.root().id()
    }

    /// Absolute display path of a group; the root group is "/".
    pub fn group_path(&self, id: GroupId) -> String {
        let mut names = Vec::new();
        let mut cur = self.db.group(id);
        while let Some(g) = cur {
            match g.parent() {
                Some(p) => {
                    names.push(path::escape(&g.name));
                    cur = self.db.group(p.id());
                }
                None => cur = None,
            }
        }
        names.reverse();
        path::join(&names)
    }

    pub fn entry_path(&self, id: EntryId) -> String {
        match self.db.entry(id) {
            Some(e) => {
                let parent = self.group_path(e.parent().id());
                let title = path::escape(e.get_title().unwrap_or(""));
                if parent == "/" {
                    format!("/{title}")
                } else {
                    format!("{parent}/{title}")
                }
            }
            None => String::new(),
        }
    }

    pub fn node_path(&self, id: NodeId) -> String {
        match id {
            NodeId::Group(g) => self.group_path(g),
            NodeId::Entry(e) => self.entry_path(e),
        }
    }

    pub fn is_in_recycle_bin(&self, group: GroupId) -> bool {
        let Some(bin) = self.db.recycle_bin() else {
            return false;
        };
        let bin_id = bin.id();
        let mut cur = self.db.group(group);
        while let Some(g) = cur {
            if g.id() == bin_id {
                return true;
            }
            cur = g.parent().and_then(|p| self.db.group(p.id()));
        }
        false
    }

    fn child_groups_named(&self, g: &GroupRef<'_>, name: &str) -> Vec<GroupId> {
        let exact: Vec<GroupId> = g
            .groups()
            .filter(|c| c.name == name)
            .map(|c| c.id())
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        let lower = name.to_lowercase();
        g.groups()
            .filter(|c| c.name.to_lowercase() == lower)
            .map(|c| c.id())
            .collect()
    }

    fn child_entries_titled(&self, g: &GroupRef<'_>, title: &str) -> Vec<EntryId> {
        let exact: Vec<EntryId> = g
            .entries()
            .filter(|e| e.get_title() == Some(title))
            .map(|e| e.id())
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        let lower = title.to_lowercase();
        g.entries()
            .filter(|e| e.get_title().map(|t| t.to_lowercase()) == Some(lower.clone()))
            .map(|e| e.id())
            .collect()
    }

    /// Resolve a spec to every node it could mean: groups first, then entries.
    /// A bare number refers to the numbered listing from the last `ls` or `find`.
    pub fn resolve(&self, spec: &str) -> Result<Vec<NodeId>, ResolveError> {
        let spec = spec.trim();
        if !spec.is_empty() && spec.bytes().all(|b| b.is_ascii_digit()) {
            let n: usize = spec
                .parse()
                .map_err(|_| ResolveError::NotFound(spec.to_string()))?;
            return match n.checked_sub(1).and_then(|i| self.listing.get(i)) {
                Some(id) if self.db.entry(*id).is_some() => Ok(vec![NodeId::Entry(*id)]),
                _ => Err(ResolveError::BadNumber(n)),
            };
        }
        let segs = path::parse(spec);
        let mut cur = self.cwd;
        let last = segs.len().saturating_sub(1);
        for (i, seg) in segs.iter().enumerate() {
            match seg {
                Segment::Root => cur = self.root(),
                Segment::Here => {}
                Segment::Up => {
                    cur = self
                        .db
                        .group(cur)
                        .and_then(|g| g.parent().map(|p| p.id()))
                        .unwrap_or_else(|| self.root());
                }
                Segment::Name(name) => {
                    let g = self
                        .db
                        .group(cur)
                        .ok_or_else(|| ResolveError::NotFound(spec.to_string()))?;
                    let groups = self.child_groups_named(&g, name);
                    if i == last {
                        let entries = self.child_entries_titled(&g, name);
                        let mut out: Vec<NodeId> = groups.into_iter().map(NodeId::Group).collect();
                        out.extend(entries.into_iter().map(NodeId::Entry));
                        if out.is_empty() {
                            return Err(ResolveError::NotFound(spec.to_string()));
                        }
                        return Ok(out);
                    }
                    match groups.len() {
                        0 => return Err(ResolveError::NotFound(spec.to_string())),
                        1 => cur = groups[0],
                        n => return Err(ResolveError::Ambiguous(spec.to_string(), n)),
                    }
                }
            }
        }
        Ok(vec![NodeId::Group(cur)])
    }

    pub fn resolve_group(&self, spec: &str) -> Result<GroupId, ResolveError> {
        let nodes = self.resolve(spec)?;
        let groups: Vec<GroupId> = nodes
            .iter()
            .filter_map(|n| match n {
                NodeId::Group(g) => Some(*g),
                _ => None,
            })
            .collect();
        match groups.len() {
            0 => Err(ResolveError::NotAGroup(spec.to_string())),
            1 => Ok(groups[0]),
            n => Err(ResolveError::Ambiguous(spec.to_string(), n)),
        }
    }

    pub fn resolve_entry(&self, spec: &str) -> Result<EntryId, ResolveError> {
        let nodes = self.resolve(spec)?;
        let entries: Vec<EntryId> = nodes
            .iter()
            .filter_map(|n| match n {
                NodeId::Entry(e) => Some(*e),
                _ => None,
            })
            .collect();
        match entries.len() {
            0 => Err(ResolveError::NotAnEntry(spec.to_string())),
            1 => Ok(entries[0]),
            n => Err(ResolveError::Ambiguous(spec.to_string(), n)),
        }
    }

    /// Resolve to exactly one node, preferring an entry when a group and an entry share the name.
    pub fn resolve_one(&self, spec: &str) -> Result<NodeId, ResolveError> {
        let nodes = self.resolve(spec)?;
        let entries: Vec<&NodeId> = nodes
            .iter()
            .filter(|n| matches!(n, NodeId::Entry(_)))
            .collect();
        match (entries.len(), nodes.len()) {
            (1, _) => Ok(*entries[0]),
            (0, 1) => Ok(nodes[0]),
            (_, n) => Err(ResolveError::Ambiguous(spec.to_string(), n)),
        }
    }

    pub fn cd(&mut self, spec: &str) -> Result<(), ResolveError> {
        self.cwd = self.resolve_group(spec)?;
        Ok(())
    }

    // ----- listing ----------------------------------------------------------

    fn entry_row(&self, e: &EntryRef<'_>, number: usize) -> EntryRow {
        EntryRow {
            number,
            id: e.id(),
            title: e.get_title().unwrap_or("").to_string(),
            username: e
                .get_username()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            url: e.get_url().filter(|s| !s.is_empty()).map(str::to_string),
            expired: is_expired(&e.times),
            has_otp: e.get_raw_otp_value().is_some(),
        }
    }

    /// Sorted children of a group without touching the numbered listing.
    pub fn children(&self, group: GroupId) -> Option<Listing> {
        let g = self.db.group(group)?;
        let mut groups: Vec<GroupRow> = g
            .groups()
            .map(|c| GroupRow {
                id: c.id(),
                name: c.name.clone(),
            })
            .collect();
        groups.sort_by_key(|r| r.name.to_lowercase());
        let mut entries: Vec<EntryRow> = g.entries().map(|e| self.entry_row(&e, 0)).collect();
        entries.sort_by_key(|r| r.title.to_lowercase());
        Some(Listing {
            group,
            path: self.group_path(group),
            groups,
            entries,
        })
    }

    /// `ls`: list a group and remember its entries as the numbered listing.
    pub fn list(&mut self, spec: Option<&str>) -> Result<Listing, ResolveError> {
        let group = match spec {
            Some(s) => self.resolve_group(s)?,
            None => self.cwd,
        };
        let mut listing = self
            .children(group)
            .ok_or_else(|| ResolveError::NotFound(spec.unwrap_or("").into()))?;
        self.listing.clear();
        for (i, row) in listing.entries.iter_mut().enumerate() {
            row.number = i + 1;
            self.listing.push(row.id);
        }
        Ok(listing)
    }

    /// Entries in the numbered listing from the last `ls` or `find`.
    pub fn numbered(&self) -> &[EntryId] {
        &self.listing
    }

    // ----- search -----------------------------------------------------------

    /// `find`: case-insensitive substring search from the root; results become the numbered listing.
    pub fn find(&mut self, query: &str, opts: FindOptions) -> Vec<FindHit> {
        let needle = query.to_lowercase();
        let mut hits = Vec::new();
        self.find_in(self.db.root(), &needle, opts, &mut hits);
        hits.sort_by_key(|h| h.path.to_lowercase());
        self.listing.clear();
        for (i, h) in hits.iter_mut().enumerate() {
            h.number = i + 1;
            self.listing.push(h.id);
        }
        hits
    }

    fn find_in(&self, g: GroupRef<'_>, needle: &str, opts: FindOptions, out: &mut Vec<FindHit>) {
        for e in g.entries() {
            let expired = is_expired(&e.times);
            if opts.expired_only && !expired {
                continue;
            }
            let matched = if needle.is_empty() {
                true
            } else if opts.all_fields {
                e.fields
                    .iter()
                    .filter(|(k, _)| k.as_str() != keepass::db::fields::PASSWORD)
                    .any(|(_, v)| v.get().to_lowercase().contains(needle))
                    || e.tags.iter().any(|t| t.to_lowercase().contains(needle))
            } else {
                e.get_title()
                    .map(|t| t.to_lowercase().contains(needle))
                    .unwrap_or(false)
            };
            if matched {
                out.push(FindHit {
                    number: 0,
                    id: e.id(),
                    path: self.entry_path(e.id()),
                    title: e.get_title().unwrap_or("").to_string(),
                    username: e
                        .get_username()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    in_recycle_bin: self.is_in_recycle_bin(g.id()),
                    expired,
                });
            }
        }
        for c in g.groups() {
            self.find_in(c, needle, opts, out);
        }
    }

    // ----- entries ----------------------------------------------------------

    pub fn entry(&self, id: EntryId) -> Result<EntryView, VaultError> {
        use keepass::db::fields;
        let e = self.db.entry(id).ok_or(VaultError::Gone)?;
        let mut custom: Vec<(String, FieldValue)> = e
            .fields
            .iter()
            .filter(|(k, _)| {
                !fields::KNOWN_FIELDS.contains(&k.as_str()) && k.as_str() != fields::OTP
            })
            .map(|(k, v)| {
                let val = if v.is_protected() {
                    FieldValue::Protected(SecretString::from(v.get().clone()))
                } else {
                    FieldValue::Plain(v.get().clone())
                };
                (k.clone(), val)
            })
            .collect();
        custom.sort_by_key(|(k, _)| k.to_lowercase());
        let mut attachments: Vec<String> =
            e.attachments_named().map(|(n, _)| n.to_string()).collect();
        attachments.sort();
        let icon = e.icon().map(|i| match i {
            Icon::BuiltIn(n) => format!("builtin:{n}"),
            Icon::Custom(c) => format!("custom:{}", c.uuid()),
        });
        Ok(EntryView {
            id,
            uuid: id.uuid(),
            path: self.entry_path(id),
            title: e.get_title().unwrap_or("").to_string(),
            username: e
                .get_username()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            password: e
                .get_password()
                .filter(|s| !s.is_empty())
                .map(|p| SecretString::from(p.to_string())),
            url: e.get_url().filter(|s| !s.is_empty()).map(str::to_string),
            notes: e
                .get(fields::NOTES)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            has_otp: e.get_raw_otp_value().is_some(),
            custom,
            tags: e.tags.clone(),
            attachments,
            icon,
            history_count: e
                .history
                .as_ref()
                .map(|h| h.get_entries().len())
                .unwrap_or(0),
            created: e.times.creation,
            modified: e.times.last_modification,
            accessed: e.times.last_access,
            expires: if e.times.expires == Some(true) {
                e.times.expiry
            } else {
                None
            },
            expired: is_expired(&e.times),
        })
    }

    pub fn totp(&self, id: EntryId) -> Result<OtpCode, VaultError> {
        let e = self.db.entry(id).ok_or(VaultError::Gone)?;
        if e.get_raw_otp_value().is_none() {
            return Err(VaultError::NoOtp);
        }
        let totp = e.get_otp()?;
        let code = totp.value_now()?;
        Ok(OtpCode {
            code: code.code,
            valid_for_secs: code.valid_for.as_secs(),
            period_secs: code.period.as_secs(),
        })
    }

    // ----- stats ------------------------------------------------------------

    pub fn stats(&self) -> Stats {
        let mut groups = 0;
        let mut entries = 0;
        let mut expired = 0;
        let mut with_otp = 0;
        let mut stack = vec![self.root()];
        while let Some(gid) = stack.pop() {
            let Some(g) = self.db.group(gid) else {
                continue;
            };
            groups += 1;
            for e in g.entries() {
                entries += 1;
                if is_expired(&e.times) {
                    expired += 1;
                }
                if e.get_raw_otp_value().is_some() {
                    with_otp += 1;
                }
            }
            stack.extend(g.groups().map(|c| c.id()));
        }
        let cfg = &self.db.config;
        Stats {
            path: self.path.clone(),
            name: self.db.meta.database_name.clone(),
            version: cfg.version.to_string(),
            kdf: format!("{:?}", cfg.kdf_config),
            cipher: format!("{:?}", cfg.outer_cipher_config),
            groups,
            entries,
            expired,
            with_otp,
            recycle_bin_enabled: self.db.meta.recyclebin_enabled.unwrap_or(false),
            read_only: self.read_only,
        }
    }
}

pub fn is_expired(t: &Times) -> bool {
    t.expires == Some(true) && t.expiry.map(|x| x < Times::now()).unwrap_or(false)
}
