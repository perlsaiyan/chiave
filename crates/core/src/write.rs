//! Everything that changes a vault: mutations, atomic verified save, new databases.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use keepass::config::{DatabaseConfig, DatabaseVersion, KdfConfig};
use keepass::db::{fields, EntryId, GroupId, Times, Value};
use keepass::Database;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::fingerprint::Fingerprint;
use crate::open::{Credentials, OpenError};
use crate::path;
use crate::vault::{DiskState, FieldValue, NodeId, ResolveError, Vault};

pub const RECYCLE_BIN_NAME: &str = "Recycle Bin";
const RECYCLE_BIN_ICON: usize = 43;

#[derive(Debug, Error)]
pub enum WriteError {
    #[error("database is open read-only")]
    ReadOnly,
    #[error("{0} databases cannot be saved; convert to KDBX4 with `upgrade` first")]
    UnsupportedVersion(String),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error("{0} already exists")]
    AlreadyExists(String),
    #[error("{0} is not empty (use -r to remove recursively)")]
    NotEmpty(String),
    #[error("cannot remove the root group")]
    Root,
    #[error("cannot remove the recycle bin; empty it instead")]
    RecycleBin,
    #[error("{0}: invalid name")]
    BadName(String),
    #[error("cannot move a group into itself")]
    Cycle,
    #[error("item vanished from the database")]
    Gone,
    #[error(transparent)]
    Save(#[from] SaveError),
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error(transparent)]
    Write(#[from] Box<WriteError>),
    #[error("{0} changed on disk since it was opened; reopen it or save with --force")]
    ChangedOnDisk(PathBuf),
    #[error("cannot serialize database: {0}")]
    Serialize(#[from] keepass::db::DatabaseSaveError),
    #[error("re-reading the saved bytes failed: {0}")]
    Reparse(#[from] keepass::error::DatabaseOpenError),
    #[error("saved database does not match memory: {}", .0.join("; "))]
    Verify(Vec<String>),
    #[error("cannot write {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Open(#[from] OpenError),
}

#[derive(Debug, Clone, Copy)]
pub struct SaveOptions {
    /// Re-parse the written bytes and compare fingerprints before replacing the file.
    pub verify: bool,
    /// Keep the previous file as `<name>.bak`.
    pub backup: bool,
    /// Overwrite even if the file changed on disk since we opened it.
    pub force: bool,
}

impl Default for SaveOptions {
    fn default() -> Self {
        SaveOptions {
            verify: true,
            backup: true,
            force: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SaveReport {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
    pub bytes: usize,
    pub verified: bool,
}

/// Fields for a new entry. Standard fields have their own slots; anything else goes in `custom`.
#[derive(Default)]
pub struct NewEntry {
    pub title: String,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub otp: Option<SecretString>,
    pub custom: Vec<(String, FieldValue)>,
    pub expires: Option<NaiveDateTime>,
    pub tags: Vec<String>,
}

/// Changes to apply to an existing entry. `None` leaves a slot alone; for `fields`,
/// a `None` value removes that field (custom fields only).
#[derive(Default)]
pub struct EntryPatch {
    /// Field key (already canonical, see `fields::canonical`) -> new value or removal.
    pub fields: Vec<(String, Option<FieldValue>)>,
    /// `Some(None)` clears expiry, `Some(Some(t))` sets it.
    pub expires: Option<Option<NaiveDateTime>>,
    pub tags: Option<Vec<String>>,
}

impl EntryPatch {
    pub fn set(mut self, key: &str, value: FieldValue) -> Self {
        self.fields
            .push((crate::fields::canonical(key), Some(value)));
        self
    }
    pub fn set_plain(self, key: &str, value: impl Into<String>) -> Self {
        self.set(key, FieldValue::Plain(value.into()))
    }
    pub fn set_secret(self, key: &str, value: impl Into<String>) -> Self {
        self.set(key, FieldValue::Protected(SecretString::from(value.into())))
    }
    pub fn remove(mut self, key: &str) -> Self {
        self.fields.push((crate::fields::canonical(key), None));
        self
    }
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.expires.is_none() && self.tags.is_none()
    }
}

/// KeePassXC-grade defaults for databases chiave creates.
pub fn strong_config() -> DatabaseConfig {
    let mut cfg = DatabaseConfig::default();
    let version = match cfg.kdf_config {
        KdfConfig::Argon2 { version, .. } | KdfConfig::Argon2id { version, .. } => version,
        _ => unreachable!("default config is Argon2"),
    };
    cfg.kdf_config = KdfConfig::Argon2id {
        iterations: 10,
        memory: 64 * 1024 * 1024,
        parallelism: 2,
        version,
    };
    cfg
}

fn set_value(e: &mut keepass::db::Entry, key: &str, value: &FieldValue) {
    let protected = crate::fields::is_secret(key) || matches!(value, FieldValue::Protected(_));
    let text = match value {
        FieldValue::Plain(s) => s.clone(),
        FieldValue::Protected(s) => s.expose_secret().to_string(),
    };
    let v = if protected {
        Value::protected(text)
    } else {
        Value::unprotected(text)
    };
    e.fields.insert(key.to_string(), v);
}

fn value_text(v: &FieldValue) -> &str {
    match v {
        FieldValue::Plain(s) => s.as_str(),
        FieldValue::Protected(s) => s.expose_secret(),
    }
}

/// Split "a/b/Name" into the parent group spec and the unescaped final name.
/// The parent spec is empty when the name has no directory part.
pub fn split_spec(spec: &str) -> Result<(String, String), WriteError> {
    split_new(spec)
}

fn split_new(spec: &str) -> Result<(String, String), WriteError> {
    let (parent, last) = path::split_for_completion(spec.trim_end_matches('/'));
    let name = match path::parse(last).as_slice() {
        [path::Segment::Name(n)] => n.clone(),
        _ => return Err(WriteError::BadName(spec.to_string())),
    };
    Ok((parent.to_string(), name))
}

impl Vault {
    fn ensure_writable(&self) -> Result<(), WriteError> {
        if self.read_only {
            return Err(WriteError::ReadOnly);
        }
        match self.version() {
            DatabaseVersion::KDB4(_) => Ok(()),
            v => Err(WriteError::UnsupportedVersion(v.to_string())),
        }
    }

    fn touch(&mut self) {
        self.modified = true;
    }

    fn parent_for(&self, parent_spec: &str) -> Result<GroupId, ResolveError> {
        if parent_spec.is_empty() {
            Ok(self.cwd())
        } else {
            self.resolve_group(parent_spec)
        }
    }

    // ----- recycle bin ------------------------------------------------------

    pub fn recycle_bin_id(&self) -> Option<GroupId> {
        self.db.recycle_bin().map(|g| g.id())
    }

    pub fn recycle_bin_enabled(&self) -> bool {
        self.db.meta.recyclebin_enabled != Some(false)
    }

    /// Find or create the recycle bin group.
    pub fn ensure_recycle_bin(&mut self) -> Result<GroupId, WriteError> {
        self.ensure_writable()?;
        if let Some(id) = self.recycle_bin_id() {
            return Ok(id);
        }
        let id = self
            .db
            .root_mut()
            .add_group()
            .edit(|g| {
                g.name = RECYCLE_BIN_NAME.into();
                g.set_icon_builtin(RECYCLE_BIN_ICON);
                g.enable_autotype = Some(false);
                g.enable_searching = Some(false);
            })
            .id();
        self.db.meta.recyclebin_uuid = Some(id.uuid());
        self.db.meta.recyclebin_enabled = Some(true);
        self.db.meta.recyclebin_changed = Some(Times::now());
        self.touch();
        Ok(id)
    }

    // ----- groups -----------------------------------------------------------

    /// `mkdir`: create a group; the last path component is the new name.
    pub fn mkdir(&mut self, spec: &str) -> Result<GroupId, WriteError> {
        self.ensure_writable()?;
        let (parent_spec, name) = split_new(spec)?;
        let parent = self.parent_for(&parent_spec)?;
        let exists = self
            .db
            .group(parent)
            .map(|g| g.groups().any(|c| c.name == name))
            .unwrap_or(false);
        if exists {
            return Err(WriteError::AlreadyExists(spec.to_string()));
        }
        let mut pg = self.db.group_mut(parent).ok_or(WriteError::Gone)?;
        let id = pg.add_group().edit(|g| g.name = name).id();
        self.touch();
        Ok(id)
    }

    /// `rename` for groups.
    pub fn rename_group(&mut self, id: GroupId, new_name: &str) -> Result<(), WriteError> {
        self.ensure_writable()?;
        if new_name.is_empty() {
            return Err(WriteError::BadName(new_name.into()));
        }
        let mut g = self.db.group_mut(id).ok_or(WriteError::Gone)?;
        g.name = new_name.to_string();
        g.times.last_modification = Some(Times::now());
        self.touch();
        Ok(())
    }

    /// `rmdir`: move a group to the recycle bin, or delete it permanently when
    /// `permanent` is set, the bin is disabled, or it is already inside the bin.
    pub fn rmdir(
        &mut self,
        id: GroupId,
        recursive: bool,
        permanent: bool,
    ) -> Result<(), WriteError> {
        self.ensure_writable()?;
        if id == self.root() {
            return Err(WriteError::Root);
        }
        if Some(id) == self.recycle_bin_id() {
            return Err(WriteError::RecycleBin);
        }
        let (empty, name) = {
            let g = self.db.group(id).ok_or(WriteError::Gone)?;
            (
                g.groups().next().is_none() && g.entries().next().is_none(),
                self.group_path(id),
            )
        };
        if !empty && !recursive {
            return Err(WriteError::NotEmpty(name));
        }
        if self.cwd == id || self.is_descendant(self.cwd, id) {
            self.cwd = self.root();
        }
        let to_bin = !permanent && self.recycle_bin_enabled() && !self.is_in_recycle_bin(id);
        if to_bin {
            let bin = self.ensure_recycle_bin()?;
            let mut g = self.db.group_mut(id).ok_or(WriteError::Gone)?;
            g.track_changes()
                .move_to(bin)
                .map_err(|_| WriteError::Cycle)?;
        } else {
            let mut g = self.db.group_mut(id).ok_or(WriteError::Gone)?;
            g.track_changes().remove().map_err(|_| WriteError::Root)?;
        }
        self.touch();
        Ok(())
    }

    fn is_descendant(&self, node: GroupId, ancestor: GroupId) -> bool {
        let mut cur = self.db.group(node).and_then(|g| g.parent().map(|p| p.id()));
        while let Some(g) = cur {
            if g == ancestor {
                return true;
            }
            cur = self.db.group(g).and_then(|g| g.parent().map(|p| p.id()));
        }
        false
    }

    // ----- entries ----------------------------------------------------------

    /// `new`: create an entry; `spec` is the parent group path plus title, or just a title
    /// (created in the current group). Returns the id.
    pub fn new_entry(&mut self, spec: &str, e: NewEntry) -> Result<EntryId, WriteError> {
        self.ensure_writable()?;
        let (parent_spec, title_from_spec) = split_new(spec)?;
        let parent = self.parent_for(&parent_spec)?;
        let title = if e.title.is_empty() {
            title_from_spec
        } else {
            e.title.clone()
        };
        self.new_entry_in(parent, NewEntry { title, ..e })
    }

    /// Create an entry directly in `parent` using `e.title` as the title.
    pub fn new_entry_in(&mut self, parent: GroupId, e: NewEntry) -> Result<EntryId, WriteError> {
        self.ensure_writable()?;
        if e.title.is_empty() {
            return Err(WriteError::BadName(String::new()));
        }
        let title = e.title.clone();
        let mut pg = self.db.group_mut(parent).ok_or(WriteError::Gone)?;
        let id = pg
            .add_entry()
            .edit(|n| {
                n.set_unprotected(fields::TITLE, title);
                if let Some(v) = &e.username {
                    n.set_unprotected(fields::USERNAME, v.clone());
                }
                if let Some(v) = &e.password {
                    n.set_protected(fields::PASSWORD, v.expose_secret().to_string());
                }
                if let Some(v) = &e.url {
                    n.set_unprotected(fields::URL, v.clone());
                }
                if let Some(v) = &e.notes {
                    n.set_unprotected(fields::NOTES, v.clone());
                }
                if let Some(v) = &e.otp {
                    n.set_protected(fields::OTP, v.expose_secret().to_string());
                }
                for (k, v) in &e.custom {
                    set_value(n, k, v);
                }
                if let Some(t) = e.expires {
                    n.times.expires = Some(true);
                    n.times.expiry = Some(t);
                }
                n.tags = e.tags.clone();
            })
            .id();
        self.touch();
        Ok(id)
    }

    /// `edit`/`set`: apply a patch, recording the previous version in history.
    /// Returns false when the patch changed nothing (and nothing was recorded).
    pub fn edit_entry(&mut self, id: EntryId, patch: EntryPatch) -> Result<bool, WriteError> {
        self.ensure_writable()?;
        let changed = {
            let e = self.db.entry(id).ok_or(WriteError::Gone)?;
            let field_changes = patch
                .fields
                .iter()
                .any(|(k, v)| match (e.fields.get(k), v) {
                    (Some(cur), Some(new)) => cur.get() != value_text(new),
                    (None, Some(_)) => true,
                    (Some(_), None) => true,
                    (None, None) => false,
                });
            let expiry_changes = match patch.expires {
                None => false,
                Some(None) => e.times.expires == Some(true),
                Some(Some(t)) => e.times.expires != Some(true) || e.times.expiry != Some(t),
            };
            let tag_changes = patch.tags.as_ref().map(|t| *t != e.tags).unwrap_or(false);
            field_changes || expiry_changes || tag_changes
        };
        if !changed {
            return Ok(false);
        }
        let mut em = self.db.entry_mut(id).ok_or(WriteError::Gone)?;
        em.edit_tracking(|t| {
            for (k, v) in &patch.fields {
                match v {
                    Some(v) => set_value(t, k, v),
                    None => {
                        t.fields.remove(k);
                    }
                }
            }
            match patch.expires {
                None => {}
                Some(None) => t.times.expires = Some(false),
                Some(Some(when)) => {
                    t.times.expires = Some(true);
                    t.times.expiry = Some(when);
                }
            }
            if let Some(tags) = &patch.tags {
                t.tags = tags.clone();
            }
            t.times.last_modification = Some(Times::now());
        });
        self.touch();
        Ok(true)
    }

    /// `rm`: move to the recycle bin, or delete permanently.
    pub fn rm_entry(&mut self, id: EntryId, permanent: bool) -> Result<(), WriteError> {
        self.ensure_writable()?;
        let parent = self.db.entry(id).ok_or(WriteError::Gone)?.parent().id();
        let to_bin = !permanent && self.recycle_bin_enabled() && !self.is_in_recycle_bin(parent);
        if to_bin {
            let bin = self.ensure_recycle_bin()?;
            let mut em = self.db.entry_mut(id).ok_or(WriteError::Gone)?;
            em.move_to(bin).map_err(|_| WriteError::Gone)?;
            em.times.location_changed = Some(Times::now());
        } else {
            let mut em = self.db.entry_mut(id).ok_or(WriteError::Gone)?;
            em.track_changes().remove();
        }
        self.listing.retain(|e| *e != id || !permanent);
        self.touch();
        Ok(())
    }

    /// `mv`: move an entry or group into another group.
    pub fn mv(&mut self, node: NodeId, dest: GroupId) -> Result<(), WriteError> {
        self.ensure_writable()?;
        match node {
            NodeId::Entry(id) => {
                let mut em = self.db.entry_mut(id).ok_or(WriteError::Gone)?;
                em.move_to(dest).map_err(|_| WriteError::Gone)?;
                em.times.location_changed = Some(Times::now());
            }
            NodeId::Group(id) => {
                if id == self.root() {
                    return Err(WriteError::Root);
                }
                let mut g = self.db.group_mut(id).ok_or(WriteError::Gone)?;
                g.track_changes().move_to(dest).map_err(|e| match e {
                    keepass::db::MoveGroupError::WouldCreateCycle => WriteError::Cycle,
                    _ => WriteError::Gone,
                })?;
            }
        }
        self.touch();
        Ok(())
    }

    /// `cp`/`clone`: duplicate an entry into `dest` (history is not copied).
    pub fn copy_entry(
        &mut self,
        id: EntryId,
        dest: GroupId,
        new_title: Option<&str>,
    ) -> Result<EntryId, WriteError> {
        self.ensure_writable()?;
        let (fields_copy, tags, attachments, times) = {
            let e = self.db.entry(id).ok_or(WriteError::Gone)?;
            let f: Vec<(String, Value<String>)> = e
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let a: Vec<(String, Value<Vec<u8>>)> = e
                .attachments_named()
                .map(|(n, att)| (n.to_string(), att.data.clone()))
                .collect();
            (f, e.tags.clone(), a, e.times.clone())
        };
        let mut pg = self.db.group_mut(dest).ok_or(WriteError::Gone)?;
        let new_id = pg
            .add_entry()
            .edit(|n| {
                for (k, v) in fields_copy {
                    n.fields.insert(k, v);
                }
                if let Some(t) = new_title {
                    n.set_unprotected(fields::TITLE, t);
                }
                n.tags = tags;
                n.times.expires = times.expires;
                n.times.expiry = times.expiry;
                for (name, data) in attachments {
                    n.add_attachment(name, data);
                }
            })
            .id();
        self.touch();
        Ok(new_id)
    }

    // ----- otp --------------------------------------------------------------

    /// Move a kpcli-style `2FA-TOTP:` seed from the notes into the native `otp` field
    /// (as an otpauth URI) and delete that line from the notes. Returns false when the
    /// entry has no notes seed or already has an `otp` field.
    pub fn migrate_notes_otp(&mut self, id: EntryId) -> Result<bool, WriteError> {
        self.ensure_writable()?;
        let (uri, new_notes) = {
            let e = self.db.entry(id).ok_or(WriteError::Gone)?;
            let Some(crate::otp::OtpSource::Notes(n)) = crate::otp::source_of(&e) else {
                return Ok(false);
            };
            let uri =
                crate::otp::to_otpauth(&n.secret, e.get_title().unwrap_or("entry"), &n.algorithm);
            let notes = e.get(fields::NOTES).unwrap_or("");
            let kept: Vec<&str> = notes
                .lines()
                .filter(|l| l.trim_end_matches('\r') != n.line)
                .collect();
            (uri, kept.join("\n").trim_end().to_string())
        };
        let mut patch = EntryPatch::default().set_secret(fields::OTP, uri);
        patch = if new_notes.is_empty() {
            patch.remove(fields::NOTES)
        } else {
            patch.set_plain(fields::NOTES, new_notes)
        };
        self.edit_entry(id, patch)
    }

    /// Migrate every entry with a notes seed. Returns the ids that changed.
    pub fn migrate_all_notes_otp(&mut self) -> Result<Vec<EntryId>, WriteError> {
        self.ensure_writable()?;
        let mut ids = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(g) = stack.pop() {
            if let Some(group) = self.db.group(g) {
                ids.extend(group.entries().map(|e| e.id()));
                stack.extend(group.groups().map(|c| c.id()));
            }
        }
        let mut done = Vec::new();
        for id in ids {
            if self.migrate_notes_otp(id)? {
                done.push(id);
            }
        }
        Ok(done)
    }

    // ----- attachments ------------------------------------------------------

    /// Names and sizes of an entry's attachments, sorted by name.
    pub fn attachments(&self, id: EntryId) -> Result<Vec<(String, usize)>, WriteError> {
        let e = self.db.entry(id).ok_or(WriteError::Gone)?;
        let mut out: Vec<(String, usize)> = e
            .attachments_named()
            .map(|(n, a)| (n.to_string(), a.data.get().len()))
            .collect();
        out.sort();
        Ok(out)
    }

    /// Contents of one attachment.
    pub fn attachment_data(
        &self,
        id: EntryId,
        name: &str,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, WriteError> {
        let e = self.db.entry(id).ok_or(WriteError::Gone)?;
        let a = e
            .attachment_by_name(name)
            .ok_or_else(|| ResolveError::NotFound(name.to_string()))?;
        Ok(zeroize::Zeroizing::new(a.data.get().clone()))
    }

    /// Add (or replace) an attachment; the previous state goes to history.
    pub fn add_attachment(
        &mut self,
        id: EntryId,
        name: &str,
        data: Vec<u8>,
    ) -> Result<(), WriteError> {
        self.ensure_writable()?;
        let mut em = self.db.entry_mut(id).ok_or(WriteError::Gone)?;
        em.edit_tracking(|t| {
            t.as_mut().remove_attachment_by_name(name);
            t.add_attachment(name.to_string(), Value::protected(data));
        });
        self.touch();
        Ok(())
    }

    /// Remove an attachment; the previous state goes to history.
    pub fn remove_attachment(&mut self, id: EntryId, name: &str) -> Result<(), WriteError> {
        self.ensure_writable()?;
        let exists = self
            .db
            .entry(id)
            .ok_or(WriteError::Gone)?
            .attachment_by_name(name)
            .is_some();
        if !exists {
            return Err(ResolveError::NotFound(name.to_string()).into());
        }
        let mut em = self.db.entry_mut(id).ok_or(WriteError::Gone)?;
        em.edit_tracking(|t| {
            t.as_mut().remove_attachment_by_name(name);
            t.times.last_modification = Some(Times::now());
        });
        self.touch();
        Ok(())
    }

    // ----- key --------------------------------------------------------------

    /// `passwd`: change the master password and/or key file. Takes effect on the next save.
    pub fn change_credentials(&mut self, creds: &Credentials) -> Result<(), WriteError> {
        self.ensure_writable()?;
        self.key = creds
            .to_key()
            .map_err(|e| WriteError::Save(SaveError::Open(e)))?;
        self.keyfile = creds.keyfile.clone();
        self.db.meta.master_key_changed = Some(Times::now());
        self.touch();
        Ok(())
    }

    // ----- upgrade ----------------------------------------------------------

    /// `upgrade`: convert an in-memory KDBX3 database to KDBX4 with strong KDF settings.
    /// Nothing touches the disk until `save`, which keeps a `.bak` of the old file.
    /// Returns false if the database was already KDBX4.
    pub fn upgrade_to_kdbx4(&mut self) -> Result<bool, WriteError> {
        if self.read_only {
            return Err(WriteError::ReadOnly);
        }
        if matches!(self.version(), DatabaseVersion::KDB4(_)) {
            return Ok(false);
        }
        let cfg = strong_config();
        self.db.config.version = cfg.version;
        self.db.config.kdf_config = cfg.kdf_config;
        self.db.config.outer_cipher_config = cfg.outer_cipher_config;
        self.db.config.inner_cipher_config = cfg.inner_cipher_config;
        self.db.config.compression_config = cfg.compression_config;
        self.touch();
        Ok(true)
    }

    // ----- save -------------------------------------------------------------

    /// Serialize, verify by re-parsing, then atomically replace the file.
    pub fn save(&mut self, opts: SaveOptions) -> Result<SaveReport, SaveError> {
        let path = self.path.clone();
        self.save_to(&path, opts)
    }

    /// `saveas`: like `save` but to a new path, which becomes the vault's path.
    pub fn save_to(&mut self, path: &Path, opts: SaveOptions) -> Result<SaveReport, SaveError> {
        self.ensure_writable().map_err(Box::new)?;
        let same_file = path == self.path;
        if same_file && !opts.force {
            if let (Some(before), Some(now)) = (self.disk_state, DiskState::read(path)) {
                if before != now {
                    return Err(SaveError::ChangedOnDisk(path.to_path_buf()));
                }
            }
        }
        // keepass-rs serializes KDBX 4.1 only. KeePassXC writes 4.0 when no 4.1 feature
        // is in use and reads 4.1 without complaint, so bump the minor version here.
        if let DatabaseVersion::KDB4(minor) = self.db.config.version {
            if minor < 1 {
                self.db.config.version = DatabaseVersion::KDB4(1);
            }
        }
        let mut buf = Vec::new();
        self.db.save(&mut buf, self.key.clone())?;
        let verified = if opts.verify {
            let reparsed = Database::parse(&buf, self.key.clone())?;
            let diff = Fingerprint::of(&self.db).diff(&Fingerprint::of(&reparsed));
            if !diff.is_empty() {
                return Err(SaveError::Verify(diff));
            }
            true
        } else {
            false
        };
        let backup = write_atomic(path, &buf, opts.backup)?;
        self.path = path.to_path_buf();
        self.disk_state = DiskState::read(path);
        self.modified = false;
        Ok(SaveReport {
            path: path.to_path_buf(),
            backup,
            bytes: buf.len(),
            verified,
        })
    }

    /// `newdb`: create a KDBX4 database with strong KDF settings and save it.
    /// Fails if `path` already exists.
    pub fn create(path: &Path, creds: &Credentials, name: &str) -> Result<Vault, SaveError> {
        if path.exists() {
            return Err(SaveError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "file exists"),
            });
        }
        let key = creds.to_key()?;
        let mut db = Database::with_config(strong_config());
        db.meta.database_name = Some(name.to_string());
        db.meta.generator = Some(format!("chiave {}", env!("CARGO_PKG_VERSION")));
        db.root_mut().name = name.to_string();
        let cwd = db.root().id();
        let mut v = Vault {
            db,
            key,
            path: path.to_path_buf(),
            keyfile: creds.keyfile.clone(),
            read_only: false,
            cwd,
            listing: Vec::new(),
            modified: true,
            disk_state: None,
        };
        v.ensure_recycle_bin().map_err(Box::new)?;
        v.save(SaveOptions {
            backup: false,
            ..SaveOptions::default()
        })?;
        Ok(v)
    }
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> SaveError + '_ {
    move |source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Write `data` next to `path`, fsync, optionally back up the old file, rename into place.
fn write_atomic(path: &Path, data: &[u8], backup: bool) -> Result<Option<PathBuf>, SaveError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".chiave-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(io_err(path))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o600)).map_err(io_err(path))?;
    }
    tmp.write_all(data).map_err(io_err(path))?;
    tmp.as_file().sync_all().map_err(io_err(path))?;

    let mut backup_path = None;
    if backup && path.exists() {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".bak");
        let bak = path.with_file_name(name);
        fs::copy(path, &bak).map_err(io_err(&bak))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&bak, fs::Permissions::from_mode(0o600));
        }
        backup_path = Some(bak);
    }
    tmp.persist(path).map_err(|e| SaveError::Io {
        path: path.to_path_buf(),
        source: e.error,
    })?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(backup_path)
}

/// Keep `OpenOptions` referenced for platforms where we may want exclusive create later.
#[allow(dead_code)]
fn _unused(_: OpenOptions) {}
