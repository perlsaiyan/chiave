//! Vault hygiene: password quality checks (`pwck`) and age-based purging (`purge`).

use std::collections::HashMap;

use chrono::{Duration, NaiveDateTime};
use keepass::db::{EntryId, GroupId, Times};

use crate::vault::{is_expired, Vault};
use crate::write::WriteError;

/// One entry's password assessment.
#[derive(Debug, Clone)]
pub struct PwckRow {
    pub id: EntryId,
    pub path: String,
    /// zxcvbn score 0 (worst) to 4 (best); None when the entry has no password.
    pub score: Option<u8>,
    pub guesses_log10: f64,
    pub warning: Option<String>,
    pub suggestions: Vec<String>,
    /// Other entries using the same password.
    pub reused_with: Vec<String>,
    pub expired: bool,
    pub empty: bool,
}

impl PwckRow {
    /// Weak means score below 3, empty, or reused.
    pub fn is_weak(&self) -> bool {
        self.empty || self.score.map(|s| s < 3).unwrap_or(false) || !self.reused_with.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeField {
    Created,
    Modified,
    Accessed,
    Expiry,
}

#[derive(Debug, Clone, Copy)]
pub struct PurgeOptions {
    pub field: AgeField,
    /// Entries whose `field` is older than this many days are purged
    /// (for Expiry: expired at least this many days ago).
    pub older_than_days: i64,
    pub recursive: bool,
    /// Delete permanently instead of moving to the recycle bin.
    pub permanent: bool,
}

#[derive(Debug, Clone)]
pub struct PurgedEntry {
    pub id: EntryId,
    pub path: String,
    pub when: NaiveDateTime,
}

fn collect_entries(v: &Vault, group: GroupId, recursive: bool, out: &mut Vec<EntryId>) {
    let Some(g) = v.db().group(group) else { return };
    out.extend(g.entries().map(|e| e.id()));
    if recursive {
        let children: Vec<GroupId> = g.groups().map(|c| c.id()).collect();
        for c in children {
            collect_entries(v, c, true, out);
        }
    }
}

impl Vault {
    /// Assess password quality for one entry, or a group (optionally recursive).
    /// Reuse is detected across the whole vault regardless of scope.
    pub fn pwck(&self, scope: GroupId, recursive: bool) -> Vec<PwckRow> {
        let mut ids = Vec::new();
        collect_entries(self, scope, recursive, &mut ids);
        self.pwck_entries(&ids)
    }

    pub fn pwck_entries(&self, ids: &[EntryId]) -> Vec<PwckRow> {
        // Map password -> paths over the whole vault for reuse detection, excluding the bin.
        let mut all = Vec::new();
        collect_entries(self, self.root(), true, &mut all);
        let mut by_password: HashMap<String, Vec<(EntryId, String)>> = HashMap::new();
        for id in &all {
            if let Some(e) = self.db().entry(*id) {
                if let Some(p) = e.get_password().filter(|p| !p.is_empty()) {
                    by_password
                        .entry(p.to_string())
                        .or_default()
                        .push((*id, self.entry_path(*id)));
                }
            }
        }
        let mut rows = Vec::new();
        for id in ids {
            let Some(e) = self.db().entry(*id) else {
                continue;
            };
            let path = self.entry_path(*id);
            let password = e.get_password().unwrap_or("");
            if password.is_empty() {
                rows.push(PwckRow {
                    id: *id,
                    path,
                    score: None,
                    guesses_log10: 0.0,
                    warning: None,
                    suggestions: Vec::new(),
                    reused_with: Vec::new(),
                    expired: is_expired(&e.times),
                    empty: true,
                });
                continue;
            }
            let inputs: Vec<&str> = [e.get_title(), e.get_username(), e.get_url()]
                .into_iter()
                .flatten()
                .collect();
            let est = zxcvbn::zxcvbn(password, &inputs);
            let (warning, suggestions) = match est.feedback() {
                Some(f) => (
                    f.warning().map(|w| w.to_string()),
                    f.suggestions().iter().map(|s| s.to_string()).collect(),
                ),
                None => (None, Vec::new()),
            };
            let reused_with = by_password
                .get(password)
                .map(|v| {
                    v.iter()
                        .filter(|(o, _)| o != id)
                        .map(|(_, p)| p.clone())
                        .collect()
                })
                .unwrap_or_default();
            rows.push(PwckRow {
                id: *id,
                path,
                score: Some(est.score() as u8),
                guesses_log10: est.guesses_log10(),
                warning,
                suggestions,
                reused_with,
                expired: is_expired(&e.times),
                empty: false,
            });
        }
        rows.sort_by_key(|r| r.path.to_lowercase());
        rows
    }

    /// Entries in `group` that `purge` would remove, without removing them.
    pub fn purge_candidates(&self, group: GroupId, opts: PurgeOptions) -> Vec<PurgedEntry> {
        let cutoff = Times::now() - Duration::days(opts.older_than_days);
        let mut ids = Vec::new();
        collect_entries(self, group, opts.recursive, &mut ids);
        let mut out = Vec::new();
        for id in ids {
            let Some(e) = self.db().entry(id) else {
                continue;
            };
            let when = match opts.field {
                AgeField::Created => e.times.creation,
                AgeField::Modified => e.times.last_modification,
                AgeField::Accessed => e.times.last_access,
                AgeField::Expiry => {
                    if e.times.expires == Some(true) {
                        e.times.expiry
                    } else {
                        None
                    }
                }
            };
            if let Some(when) = when {
                if when < cutoff {
                    out.push(PurgedEntry {
                        id,
                        path: self.entry_path(id),
                        when,
                    });
                }
            }
        }
        out.sort_by_key(|r| r.path.to_lowercase());
        out
    }

    /// `purge`: remove entries older than a threshold. Returns what was removed.
    pub fn purge(
        &mut self,
        group: GroupId,
        opts: PurgeOptions,
    ) -> Result<Vec<PurgedEntry>, WriteError> {
        let victims = self.purge_candidates(group, opts);
        for v in &victims {
            self.rm_entry(v.id, opts.permanent)?;
        }
        Ok(victims)
    }
}
