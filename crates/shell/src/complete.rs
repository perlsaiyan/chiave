//! Tab completion: command names at word 0, vault paths afterwards.

use chiave_core::path::{escape, split_for_completion};
use chiave_core::Vault;

use crate::command::command_names;
use crate::shell::Shell;

/// Commands whose argument can only be a group.
const GROUP_ONLY: &[&str] = &["cd", "chdir", "ls", "dir", "cl"];

/// Complete `line[..pos]`. Returns the byte offset where the replacement starts
/// and the candidates, already quoted for [`shell_words`].
pub fn complete(shell: &Shell, line: &str, pos: usize) -> (usize, Vec<String>) {
    let head = &line[..pos.min(line.len())];
    let start = word_start(head);
    let word = &head[start..];
    let before = head[..start].trim_start();

    if before.is_empty() {
        let mut out: Vec<String> = command_names()
            .into_iter()
            .filter(|n| n.starts_with(word))
            .collect();
        out.sort();
        return (start, out);
    }

    let Some(vault) = shell.vault() else {
        return (start, Vec::new());
    };
    let verb = before.split_whitespace().next().unwrap_or("");
    let groups_only = GROUP_ONLY.contains(&verb);
    (start, paths(vault, word, groups_only))
}

/// Candidate paths for a partially typed spec.
fn paths(vault: &Vault, word: &str, groups_only: bool) -> Vec<String> {
    let typed = unquote(word);
    let (parent, partial) = split_for_completion(&typed);
    let group = if parent.is_empty() {
        vault.cwd()
    } else {
        match vault.resolve_group(parent) {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        }
    };
    let Some(listing) = vault.children(group) else {
        return Vec::new();
    };
    let needle = partial.to_lowercase();
    let mut out = Vec::new();
    for g in &listing.groups {
        let name = escape(&g.name);
        if name.to_lowercase().starts_with(&needle) {
            out.push(quote(&format!("{parent}{name}/")));
        }
    }
    if !groups_only {
        for e in &listing.entries {
            let name = escape(&e.title);
            if name.to_lowercase().starts_with(&needle) {
                out.push(quote(&format!("{parent}{name}")));
            }
        }
    }
    out.dedup();
    out
}

/// Byte offset of the word being completed, honouring quotes and backslashes.
fn word_start(head: &str) -> usize {
    let mut start = 0usize;
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in head.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if q == '"' && c == '\\' {
                    escaped = true;
                }
            }
            None => match c {
                ' ' | '\t' => in_word = false,
                _ => {
                    if !in_word {
                        in_word = true;
                        start = i;
                    }
                    match c {
                        '\\' => escaped = true,
                        '\'' | '"' => quote = Some(c),
                        _ => {}
                    }
                }
            },
        }
    }
    if in_word {
        start
    } else {
        head.len()
    }
}

/// Undo the shell quoting of a partially typed word, keeping path escapes intact.
fn unquote(word: &str) -> String {
    let body = match word.chars().next() {
        Some('\'') | Some('"') => &word[1..],
        _ => word,
    };
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&' ') {
            out.push(' ');
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// Quote a candidate so `shell_words` hands it back unchanged.
fn quote(s: &str) -> String {
    if !s.contains([' ', '\t', '\\', '\'', '"']) {
        return s.to_string();
    }
    if !s.contains('\'') {
        return format!("'{s}'");
    }
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\\' | '\'' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_word_start() {
        assert_eq!(word_start("cd Inte"), 3);
        assert_eq!(word_start("cd"), 0);
        assert_eq!(word_start("show Sample\\ En"), 5);
        assert_eq!(word_start("ls "), 3);
        assert_eq!(word_start("show '/Sample En"), 5);
        assert_eq!(word_start(r#"show "a b"#), 5);
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(quote("Internet/"), "Internet/");
        assert_eq!(quote("Sample Entry"), "'Sample Entry'");
        assert_eq!(quote(r"Comcast\/Xfinity"), r"'Comcast\/Xfinity'");
    }

    #[test]
    fn unquotes_typed_word() {
        assert_eq!(unquote("'Sample En"), "Sample En");
        assert_eq!(unquote(r"Sample\ En"), "Sample En");
        assert_eq!(unquote(r"Comcast\/X"), r"Comcast\/X");
    }
}
