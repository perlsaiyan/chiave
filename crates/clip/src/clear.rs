//! "Clear only if the clipboard still holds our value."
//!
//! The decision is a pure function over a [`ReadClear`] so it can be exercised in
//! CI with a fake backend, with no display of any kind.

use zeroize::Zeroizing;

use crate::ClipError;

/// The minimum a backend must provide for the auto-clear check.
pub trait ReadClear {
    /// Current clipboard content, or `Ok(None)` when the clipboard is empty.
    fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError>;
    /// Clear the clipboard.
    fn clear_now(&self) -> Result<(), ClipError>;
}

/// What [`clear_if_unchanged`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearOutcome {
    /// The clipboard still held our value and was cleared.
    Cleared,
    /// The clipboard was already empty; nothing to do.
    AlreadyEmpty,
    /// Someone else has copied since; left untouched.
    Changed,
    /// Read-back failed, so the clipboard was cleared anyway (fail safe).
    ClearedUnverified,
}

/// Normalise for comparison: a single trailing newline is ignored.
///
/// `wl-paste` appends a newline unless `--no-newline` is passed, and some
/// consumers round-trip text with one, so a lone trailing `\n` must not be read
/// as "the user copied something else".
fn normalize(bytes: &[u8]) -> &[u8] {
    match bytes.strip_suffix(b"\n") {
        Some(rest) => rest,
        None => bytes,
    }
}

/// Clear the clipboard only if it still holds `expected`.
///
/// If reading back fails the clipboard is cleared regardless: leaving a password
/// in the clipboard is worse than clobbering an unrelated copy.
pub fn clear_if_unchanged(
    backend: &dyn ReadClear,
    expected: &[u8],
) -> Result<ClearOutcome, ClipError> {
    match backend.read_current() {
        Ok(None) => Ok(ClearOutcome::AlreadyEmpty),
        Ok(Some(current)) => {
            if normalize(&current) == normalize(expected) {
                backend.clear_now()?;
                Ok(ClearOutcome::Cleared)
            } else {
                Ok(ClearOutcome::Changed)
            }
        }
        Err(_) => {
            backend.clear_now()?;
            Ok(ClearOutcome::ClearedUnverified)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        content: Mutex<Option<Vec<u8>>>,
        read_fails: bool,
        clear_fails: bool,
        clears: Mutex<usize>,
    }

    impl Fake {
        fn holding(value: &str) -> Self {
            Fake {
                content: Mutex::new(Some(value.as_bytes().to_vec())),
                ..Default::default()
            }
        }
        fn clears(&self) -> usize {
            *self.clears.lock().unwrap()
        }
    }

    impl ReadClear for Fake {
        fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
            if self.read_fails {
                return Err(ClipError::Backend("read failed".into()));
            }
            Ok(self.content.lock().unwrap().clone().map(Zeroizing::new))
        }
        fn clear_now(&self) -> Result<(), ClipError> {
            if self.clear_fails {
                return Err(ClipError::Backend("clear failed".into()));
            }
            *self.clears.lock().unwrap() += 1;
            *self.content.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn clears_when_value_is_still_ours() {
        let fake = Fake::holding("hunter2");
        assert_eq!(
            clear_if_unchanged(&fake, b"hunter2").unwrap(),
            ClearOutcome::Cleared
        );
        assert_eq!(fake.clears(), 1);
        assert!(fake.content.lock().unwrap().is_none());
    }

    #[test]
    fn leaves_someone_elses_copy_alone() {
        let fake = Fake::holding("a shopping list");
        assert_eq!(
            clear_if_unchanged(&fake, b"hunter2").unwrap(),
            ClearOutcome::Changed
        );
        assert_eq!(fake.clears(), 0);
        assert_eq!(
            fake.content.lock().unwrap().as_deref(),
            Some(&b"a shopping list"[..])
        );
    }

    #[test]
    fn empty_clipboard_is_left_alone() {
        let fake = Fake::default();
        assert_eq!(
            clear_if_unchanged(&fake, b"hunter2").unwrap(),
            ClearOutcome::AlreadyEmpty
        );
        assert_eq!(fake.clears(), 0);
    }

    #[test]
    fn trailing_newline_is_not_a_change() {
        let fake = Fake::holding("hunter2\n");
        assert_eq!(
            clear_if_unchanged(&fake, b"hunter2").unwrap(),
            ClearOutcome::Cleared
        );
        assert_eq!(fake.clears(), 1);
    }

    #[test]
    fn prefix_is_not_a_match() {
        let fake = Fake::holding("hunter2x");
        assert_eq!(
            clear_if_unchanged(&fake, b"hunter2").unwrap(),
            ClearOutcome::Changed
        );
    }

    #[test]
    fn read_failure_clears_anyway() {
        let fake = Fake {
            read_fails: true,
            ..Fake::holding("hunter2")
        };
        assert_eq!(
            clear_if_unchanged(&fake, b"hunter2").unwrap(),
            ClearOutcome::ClearedUnverified
        );
        assert_eq!(fake.clears(), 1);
    }

    #[test]
    fn clear_failure_propagates() {
        let fake = Fake {
            clear_fails: true,
            ..Fake::holding("hunter2")
        };
        assert!(clear_if_unchanged(&fake, b"hunter2").is_err());
    }
}
