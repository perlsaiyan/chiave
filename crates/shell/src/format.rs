//! Rendering of listings, entries and search hits. Nothing here exposes a secret
//! unless it is explicitly asked to.

use std::fmt::Write as _;

use chiave_core::path::escape;
use chiave_core::{EntryView, ExposeSecret, FieldValue, FindHit, Listing};
use chrono::NaiveDateTime;

pub const MASK: &str = "********";
const EXPIRED: &str = "*EXPIRED*";
const OTP_MARK: &str = "[otp]";
const OLD: &str = "*OLD*";

fn time(t: Option<NaiveDateTime>) -> String {
    match t {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "-".to_string(),
    }
}

/// `ls` output for one group.
pub fn listing(l: &Listing) -> String {
    let mut out = String::new();
    out.push_str("=== Groups ===\n");
    for g in &l.groups {
        let _ = writeln!(out, "{}/", escape(&g.name));
    }
    out.push_str("=== Entries ===\n");
    let mut left = Vec::with_capacity(l.entries.len());
    for e in &l.entries {
        let mut s = format!("{}. {}", e.number, escape(&e.title));
        if e.has_otp {
            s.push(' ');
            s.push_str(OTP_MARK);
        }
        if e.expired {
            s.push(' ');
            s.push_str(EXPIRED);
        }
        left.push(s);
    }
    let width = left.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    for (s, e) in left.iter().zip(&l.entries) {
        match &e.username {
            Some(u) if !u.is_empty() => {
                let pad = width.saturating_sub(s.chars().count());
                let _ = writeln!(out, "{s}{:pad$}  {u}", "");
            }
            _ => {
                let _ = writeln!(out, "{s}");
            }
        }
    }
    out
}

/// `find` output.
pub fn hits(hits: &[FindHit]) -> String {
    let mut out = String::new();
    for h in hits {
        let _ = write!(out, "{}. {}", h.number, h.path);
        if let Some(u) = h.username.as_deref().filter(|u| !u.is_empty()) {
            let _ = write!(out, " ({u})");
        }
        if h.in_recycle_bin {
            let _ = write!(out, " {OLD}");
        }
        if h.expired {
            let _ = write!(out, " {EXPIRED}");
        }
        out.push('\n');
    }
    if hits.is_empty() {
        out.push_str("no matches\n");
    }
    out
}

/// `show` output. `full` reveals the password and protected custom fields,
/// `all` adds times, icon, history count and UUID.
pub fn entry(e: &EntryView, full: bool, all: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Path: {}", e.path);
    let _ = write!(out, "Title: {}", e.title);
    if e.expired {
        let _ = write!(out, " {EXPIRED}");
    }
    out.push('\n');
    let _ = writeln!(out, "Uname: {}", e.username.as_deref().unwrap_or(""));
    let pass = match (&e.password, full) {
        (Some(p), true) => p.expose_secret().to_string(),
        (Some(_), false) => MASK.to_string(),
        (None, _) => String::new(),
    };
    let _ = writeln!(out, "Pass: {pass}");
    let _ = writeln!(out, "URL: {}", e.url.as_deref().unwrap_or(""));
    if let Some(n) = &e.notes {
        let _ = writeln!(out, "Notes: {}", indented(n, 7));
    }
    for (name, value) in &e.custom {
        let shown = match value {
            FieldValue::Plain(v) => v.clone(),
            FieldValue::Protected(v) if full => v.expose_secret().to_string(),
            FieldValue::Protected(_) => MASK.to_string(),
        };
        let _ = writeln!(out, "{name}: {shown}");
    }
    if !e.tags.is_empty() {
        let _ = writeln!(out, "Tags: {}", e.tags.join(", "));
    }
    if !e.attachments.is_empty() {
        let _ = writeln!(out, "Attachments: {}", e.attachments.join(", "));
    }
    if e.has_otp {
        out.push_str("OTP: configured\n");
    }
    if all {
        let _ = writeln!(out, "Created: {}", time(e.created));
        let _ = writeln!(out, "Modified: {}", time(e.modified));
        let _ = writeln!(out, "Accessed: {}", time(e.accessed));
        let expires = match e.expires {
            Some(t) => time(Some(t)),
            None => "never".to_string(),
        };
        let _ = writeln!(out, "Expires: {expires}");
        let _ = writeln!(out, "Icon: {}", e.icon.as_deref().unwrap_or("-"));
        let _ = writeln!(out, "History: {}", e.history_count);
        let _ = writeln!(out, "UUID: {}", e.uuid);
    }
    out
}

/// Continuation lines of a multi-line value line up under the first one.
fn indented(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    text.replace('\n', &format!("\n{pad}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_continuations() {
        assert_eq!(indented("a\nb", 3), "a\n   b");
    }
}
