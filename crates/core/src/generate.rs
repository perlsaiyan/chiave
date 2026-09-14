//! Password generation from the OS random source.

use secrecy::SecretString;
use thiserror::Error;
use zeroize::Zeroizing;

pub const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
pub const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
pub const DIGITS: &str = "0123456789";
/// Default special characters: common, rarely rejected by web forms.
pub const SPECIAL: &str = "!@#$%^&*()-_=+[]{}:,.?";
const AMBIGUOUS: &str = "0O1lI|`'\"";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GenError {
    #[error("no character classes enabled")]
    NoClasses,
    #[error("length {0} is too short for the required classes")]
    TooShort(usize),
    #[error("random source unavailable: {0}")]
    Random(String),
    #[error("word list is empty")]
    NoWords,
}

#[derive(Debug, Clone)]
pub struct CharOptions {
    pub length: usize,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    /// Special characters to draw from; empty disables the class.
    pub special: String,
    /// Minimum number of special characters (kpcli --pwscmin, default 1).
    pub min_special: usize,
    /// Maximum number of special characters (kpcli --pwscmax); None = no cap.
    pub max_special: Option<usize>,
    /// Drop characters that are easy to misread (0 O 1 l I | ` ' ").
    pub exclude_ambiguous: bool,
}

impl Default for CharOptions {
    fn default() -> Self {
        CharOptions {
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            special: SPECIAL.to_string(),
            min_special: 1,
            max_special: None,
            exclude_ambiguous: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WordOptions {
    pub words: usize,
    pub separator: String,
    pub capitalize: bool,
    /// Append one random digit group to satisfy sites that demand a number.
    pub add_digits: usize,
}

impl Default for WordOptions {
    fn default() -> Self {
        WordOptions {
            words: 5,
            separator: "-".into(),
            capitalize: false,
            add_digits: 0,
        }
    }
}

fn random_below(n: usize) -> Result<usize, GenError> {
    debug_assert!(n > 0);
    let n64 = n as u64;
    let zone = u64::MAX - (u64::MAX % n64);
    loop {
        let mut buf = [0u8; 8];
        getrandom::fill(&mut buf).map_err(|e| GenError::Random(e.to_string()))?;
        let v = u64::from_le_bytes(buf);
        if v < zone {
            return Ok((v % n64) as usize);
        }
    }
}

fn pick(set: &[char]) -> Result<char, GenError> {
    Ok(set[random_below(set.len())?])
}

fn shuffle(v: &mut [char]) -> Result<(), GenError> {
    for i in (1..v.len()).rev() {
        let j = random_below(i + 1)?;
        v.swap(i, j);
    }
    Ok(())
}

fn class(chars: &str, exclude_ambiguous: bool) -> Vec<char> {
    chars
        .chars()
        .filter(|c| !exclude_ambiguous || !AMBIGUOUS.contains(*c))
        .collect()
}

/// Generate a random character password. Every enabled class appears at least once
/// and the special-character count respects `min_special`/`max_special`.
pub fn password(o: &CharOptions) -> Result<SecretString, GenError> {
    let mut classes: Vec<Vec<char>> = Vec::new();
    if o.lower {
        classes.push(class(LOWER, o.exclude_ambiguous));
    }
    if o.upper {
        classes.push(class(UPPER, o.exclude_ambiguous));
    }
    if o.digits {
        classes.push(class(DIGITS, o.exclude_ambiguous));
    }
    let special = class(&o.special, o.exclude_ambiguous);
    classes.retain(|c| !c.is_empty());
    let has_special = !special.is_empty();
    if classes.is_empty() && !has_special {
        return Err(GenError::NoClasses);
    }
    let min_special = if has_special { o.min_special.max(1) } else { 0 };
    let max_special = if has_special {
        o.max_special
            .unwrap_or(o.length)
            .max(min_special)
            .min(o.length)
    } else {
        0
    };
    if o.length < classes.len() + min_special {
        return Err(GenError::TooShort(o.length));
    }

    let mut out: Zeroizing<Vec<char>> = Zeroizing::new(Vec::with_capacity(o.length));
    for c in &classes {
        out.push(pick(c)?);
    }
    let n_special = if has_special {
        min_special + random_below(max_special - min_special + 1)?
    } else {
        0
    };
    let n_special = n_special.min(o.length - classes.len());
    for _ in 0..n_special {
        out.push(pick(&special)?);
    }
    let pool: Vec<char> = if classes.is_empty() {
        special.clone()
    } else {
        classes.concat()
    };
    while out.len() < o.length {
        out.push(pick(&pool)?);
    }
    shuffle(&mut out)?;
    Ok(SecretString::from(out.iter().collect::<String>()))
}

/// Generate a passphrase from a word list (diceware style).
pub fn passphrase(wordlist: &[String], o: &WordOptions) -> Result<SecretString, GenError> {
    if wordlist.is_empty() {
        return Err(GenError::NoWords);
    }
    let mut parts: Zeroizing<Vec<String>> = Zeroizing::new(Vec::with_capacity(o.words + 1));
    for _ in 0..o.words.max(1) {
        let w = &wordlist[random_below(wordlist.len())?];
        let w = if o.capitalize {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        } else {
            w.clone()
        };
        parts.push(w);
    }
    if o.add_digits > 0 {
        let mut d = String::new();
        for _ in 0..o.add_digits {
            d.push(char::from(b'0' + random_below(10)? as u8));
        }
        parts.push(d);
    }
    Ok(SecretString::from(parts.join(&o.separator)))
}

/// Approximate entropy in bits for a character password with these options.
pub fn entropy_bits(o: &CharOptions) -> f64 {
    let mut n = 0usize;
    if o.lower {
        n += class(LOWER, o.exclude_ambiguous).len();
    }
    if o.upper {
        n += class(UPPER, o.exclude_ambiguous).len();
    }
    if o.digits {
        n += class(DIGITS, o.exclude_ambiguous).len();
    }
    n += class(&o.special, o.exclude_ambiguous).len();
    if n == 0 {
        return 0.0;
    }
    o.length as f64 * (n as f64).log2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn default_password_has_every_class() {
        for _ in 0..50 {
            let p = password(&CharOptions::default()).unwrap();
            let s = p.expose_secret();
            assert_eq!(s.chars().count(), 20);
            assert!(s.chars().any(|c| LOWER.contains(c)));
            assert!(s.chars().any(|c| UPPER.contains(c)));
            assert!(s.chars().any(|c| DIGITS.contains(c)));
            assert!(s.chars().any(|c| SPECIAL.contains(c)));
        }
    }

    #[test]
    fn special_bounds_and_kpcli_defaults() {
        let o = CharOptions {
            special: "_".into(),
            min_special: 2,
            max_special: Some(2),
            length: 12,
            ..Default::default()
        };
        for _ in 0..50 {
            let p = password(&o).unwrap();
            assert_eq!(p.expose_secret().matches('_').count(), 2);
        }
        let none = CharOptions {
            special: String::new(),
            ..Default::default()
        };
        let p = password(&none).unwrap();
        assert!(p.expose_secret().chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn digits_only_and_errors() {
        let o = CharOptions {
            lower: false,
            upper: false,
            special: String::new(),
            length: 6,
            ..Default::default()
        };
        let p = password(&o).unwrap();
        assert!(p.expose_secret().chars().all(|c| c.is_ascii_digit()));
        let bad = CharOptions {
            lower: false,
            upper: false,
            digits: false,
            special: String::new(),
            ..Default::default()
        };
        assert_eq!(password(&bad).unwrap_err(), GenError::NoClasses);
        let short = CharOptions {
            length: 2,
            ..Default::default()
        };
        assert_eq!(password(&short).unwrap_err(), GenError::TooShort(2));
    }

    #[test]
    fn ambiguous_excluded() {
        let o = CharOptions {
            exclude_ambiguous: true,
            length: 64,
            ..Default::default()
        };
        for _ in 0..20 {
            let p = password(&o).unwrap();
            assert!(!p.expose_secret().chars().any(|c| AMBIGUOUS.contains(c)));
        }
    }

    #[test]
    fn passphrases() {
        let words: Vec<String> = ["alpha", "bravo", "charlie"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let p = passphrase(
            &words,
            &WordOptions {
                words: 4,
                separator: ".".into(),
                capitalize: true,
                add_digits: 2,
            },
        )
        .unwrap();
        let parts: Vec<&str> = p.expose_secret().split('.').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[..4].iter().all(|w| w.starts_with(char::is_uppercase)));
        assert!(parts[4].chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            passphrase(&[], &WordOptions::default()).unwrap_err(),
            GenError::NoWords
        );
    }

    #[test]
    fn entropy() {
        let bits = entropy_bits(&CharOptions::default());
        assert!(bits > 120.0 && bits < 135.0, "{bits}");
    }
}
