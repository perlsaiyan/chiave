//! Field-name aliases accepted on the command line, mapped to KDBX field keys.

pub use keepass::db::fields::{NOTES, OTP, PASSWORD, TITLE, URL, USERNAME};

/// Map a user-typed field name to the canonical KDBX key. Unknown names are
/// returned unchanged (they are custom fields).
pub fn canonical(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "title" | "name" => TITLE.into(),
        "username" | "uname" | "user" | "login" => USERNAME.into(),
        "password" | "pass" | "pw" => PASSWORD.into(),
        "url" | "link" | "website" => URL.into(),
        "notes" | "comments" | "comment" | "note" => NOTES.into(),
        "otp" | "totp" => OTP.into(),
        _ => name.to_string(),
    }
}

/// Fields that are always stored memory-protected.
pub fn is_secret(key: &str) -> bool {
    key == PASSWORD || key == OTP
}

pub fn is_standard(key: &str) -> bool {
    keepass::db::fields::KNOWN_FIELDS.contains(&key) || key == OTP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases() {
        assert_eq!(canonical("uname"), "UserName");
        assert_eq!(canonical("Comments"), "Notes");
        assert_eq!(canonical("PIN"), "PIN");
        assert!(is_secret("Password"));
        assert!(!is_secret("URL"));
    }
}
