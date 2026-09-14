//! TOTP sources: the KeePassXC `otp` field (an otpauth:// URI, the KDBX4-native way)
//! and kpcli's legacy `2FA-TOTP: <base32>` line in the notes.

use keepass::db::{fields, EntryRef, TOTP};

/// Where an entry's TOTP seed lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtpSource {
    /// The `otp` field with an otpauth:// URI (KeePassXC convention).
    Field,
    /// A kpcli-style `2FA-TOTP[-ALGO]: SECRET` line in the notes.
    Notes(NotesOtp),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesOtp {
    /// Base32 seed exactly as written in the notes.
    pub secret: String,
    /// SHA1 (default), SHA256 or SHA512.
    pub algorithm: String,
    /// The whole matched line, used when migrating it out of the notes.
    pub line: String,
}

/// Parse kpcli's `2FA-TOTP: SECRET` / `2FA-TOTP-SHA256: SECRET` line, if present.
pub fn parse_notes(notes: &str) -> Option<NotesOtp> {
    for line in notes.lines() {
        let trimmed = line.trim_end_matches('\r');
        let Some(rest) = trimmed.strip_prefix("2FA-TOTP") else {
            continue;
        };
        let (algo, rest) = match rest.strip_prefix('-') {
            Some(r) => {
                let idx = r.find(':')?;
                (r[..idx].to_string(), &r[idx..])
            }
            None => ("SHA1".to_string(), rest),
        };
        let Some(value) = rest.strip_prefix(':') else {
            continue;
        };
        let secret = value.split_whitespace().next()?.to_string();
        if secret.is_empty() {
            continue;
        }
        let algorithm = match algo.to_ascii_uppercase().as_str() {
            "SHA" | "SHA1" => "SHA1",
            "SHA256" => "SHA256",
            "SHA512" => "SHA512",
            _ => "SHA1",
        }
        .to_string();
        return Some(NotesOtp {
            secret,
            algorithm,
            line: trimmed.to_string(),
        });
    }
    None
}

/// Turn a bare base32 seed into an otpauth URI, or pass an existing URI through.
pub fn to_otpauth(value: &str, label: &str, algorithm: &str) -> String {
    let v = value.trim();
    if v.starts_with("otpauth://") {
        return v.to_string();
    }
    let secret: String = v
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let label = percent_encode(if label.is_empty() { "chiave" } else { label });
    format!(
        "otpauth://totp/{label}?secret={secret}&period=30&digits=6&algorithm={}",
        algorithm.to_ascii_uppercase()
    )
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Redact every occurrence of the notes seed.
pub fn redact(notes: &str, seed: &str) -> String {
    if seed.is_empty() {
        return notes.to_string();
    }
    notes.replace(seed, "<redacted>")
}

/// Detect the TOTP source of an entry: the `otp` field wins, then the notes line.
pub fn source_of(e: &EntryRef<'_>) -> Option<OtpSource> {
    if e.get_raw_otp_value()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        return Some(OtpSource::Field);
    }
    e.get(fields::NOTES)
        .and_then(parse_notes)
        .map(OtpSource::Notes)
}

/// Build the TOTP generator for an entry from whichever source it has.
pub fn totp_of(e: &EntryRef<'_>) -> Option<Result<TOTP, keepass::db::TOTPError>> {
    match source_of(e)? {
        OtpSource::Field => Some(e.get_otp()),
        OtpSource::Notes(n) => {
            let uri = to_otpauth(&n.secret, e.get_title().unwrap_or("entry"), &n.algorithm);
            Some(uri.parse::<TOTP>())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kpcli_lines() {
        let n = parse_notes("account notes\n2FA-TOTP: JBSWY3DPEHPK3PXP\nmore").unwrap();
        assert_eq!(n.secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(n.algorithm, "SHA1");
        assert_eq!(n.line, "2FA-TOTP: JBSWY3DPEHPK3PXP");
        let n = parse_notes("2FA-TOTP-SHA256:ABCDEFGH (30, 10)").unwrap();
        assert_eq!(n.secret, "ABCDEFGH");
        assert_eq!(n.algorithm, "SHA256");
        assert!(parse_notes("nothing here").is_none());
        assert!(parse_notes("2FA-TOTP:").is_none());
        assert!(parse_notes("some 2FA-TOTP: X in the middle").is_none());
    }

    #[test]
    fn builds_uris() {
        assert_eq!(
            to_otpauth("jbsw y3dp-ehpk3pxp", "GitHub tom", "SHA1"),
            "otpauth://totp/GitHub%20tom?secret=JBSWY3DPEHPK3PXP&period=30&digits=6&algorithm=SHA1"
        );
        assert_eq!(
            to_otpauth(" otpauth://totp/x?secret=A ", "y", "SHA1"),
            "otpauth://totp/x?secret=A"
        );
    }

    #[test]
    fn redacts() {
        assert_eq!(
            redact("a\n2FA-TOTP: SECRET\nb", "SECRET"),
            "a\n2FA-TOTP: <redacted>\nb"
        );
    }
}
