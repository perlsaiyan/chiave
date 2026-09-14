//! Asking the user for the master password.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chiave_core::SecretString;

/// How the shell asks for a master password (idle unlock, `open`).
pub trait PasswordPrompt {
    fn prompt(&self, msg: &str) -> io::Result<SecretString>;
}

/// Reads from the terminal without echo.
#[derive(Debug, Default, Clone, Copy)]
pub struct RpasswordPrompt;

impl PasswordPrompt for RpasswordPrompt {
    fn prompt(&self, msg: &str) -> io::Result<SecretString> {
        let pw = rpassword::prompt_password(msg)?;
        Ok(SecretString::from(pw))
    }
}

/// Always answers with the same password; for tests and `--pwfile`-style flows.
/// Clones share the call counter.
#[derive(Debug, Clone)]
pub struct FixedPrompt {
    password: SecretString,
    calls: Arc<AtomicUsize>,
}

impl FixedPrompt {
    pub fn new(password: impl Into<String>) -> Self {
        FixedPrompt {
            password: SecretString::from(password.into()),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many times the shell has asked for a password.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PasswordPrompt for FixedPrompt {
    fn prompt(&self, _msg: &str) -> io::Result<SecretString> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.password.clone())
    }
}
