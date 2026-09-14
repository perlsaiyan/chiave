use std::fs::File;
use std::path::{Path, PathBuf};

use keepass::{Database, DatabaseKey};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenError {
    #[error("cannot read {path}")]
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
#[derive(Default, Clone)]
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

    /// Build the keepass-rs key. Reads the key file from disk.
    pub fn to_key(&self) -> Result<DatabaseKey, OpenError> {
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

/// Open a KDBX database from disk with the given key.
pub fn open_with_key(path: &Path, key: DatabaseKey) -> Result<Database, OpenError> {
    let mut f = File::open(path).map_err(|source| OpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Database::open(&mut f, key)?)
}

/// Open a KDBX database from disk.
pub fn open(path: &Path, creds: &Credentials) -> Result<Database, OpenError> {
    open_with_key(path, creds.to_key()?)
}
