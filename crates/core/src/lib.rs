//! chiave-core: the vault model shared by the shell, CLI and TUI front ends.
//!
//! Phase 0: open a KDBX file and walk it. Everything else comes later.

use std::fs::File;
use std::path::{Path, PathBuf};

use keepass::db::{EntryRef, GroupRef};
use keepass::{Database, DatabaseKey};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("no password or key file given")]
    NoCredentials,
    #[error("cannot open database: {0}")]
    Database(#[from] keepass::error::DatabaseOpenError),
}

/// Credentials used to unlock a database.
#[derive(Default)]
pub struct Credentials {
    pub password: Option<SecretString>,
    pub keyfile: Option<PathBuf>,
}

impl Credentials {
    pub fn password(p: impl Into<String>) -> Self {
        Credentials {
            password: Some(SecretString::from(p.into())),
            keyfile: None,
        }
    }

    pub fn with_keyfile(mut self, path: impl Into<PathBuf>) -> Self {
        self.keyfile = Some(path.into());
        self
    }

    fn to_key(&self) -> Result<DatabaseKey, OpenError> {
        let mut key = DatabaseKey::new();
        if let Some(pw) = &self.password {
            key = key.with_password(pw.expose_secret());
        }
        if let Some(kf) = &self.keyfile {
            let mut f = File::open(kf).map_err(|source| OpenError::Io {
                path: kf.clone(),
                source,
            })?;
            key = key.with_keyfile(&mut f).map_err(|source| OpenError::Io {
                path: kf.clone(),
                source,
            })?;
        }
        if key.is_empty() {
            return Err(OpenError::NoCredentials);
        }
        Ok(key)
    }
}

/// Open a KDBX database from disk.
pub fn open(path: &Path, creds: &Credentials) -> Result<Database, OpenError> {
    let mut f = File::open(path).map_err(|source| OpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let key = creds.to_key()?;
    Ok(Database::open(&mut f, key)?)
}

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

fn walk_group(group: GroupRef<'_>, prefix: &str, out: &mut Vec<WalkItem>) {
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
            path: format!("{here}/{}", entry_title(&e)),
            is_group: false,
        });
    }
}

fn entry_title(e: &EntryRef<'_>) -> String {
    e.get_title().unwrap_or("(untitled)").to_string()
}
