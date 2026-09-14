//! Fuzzy search over every entry in the vault.
//!
//! A small subsequence scorer rather than a dependency: characters must appear
//! in order, runs of adjacent characters and matches at word starts score
//! higher, and earlier matches beat later ones.

use chiave_core::{EntryId, GroupId, Vault};

/// One searchable entry, flattened out of the tree once per query.
#[derive(Debug, Clone)]
pub struct SearchItem {
    pub id: EntryId,
    pub group: GroupId,
    pub path: String,
    pub title: String,
    pub username: Option<String>,
    pub expired: bool,
    pub has_otp: bool,
    pub in_recycle_bin: bool,
    /// url and tags, joined, for matching only.
    pub extra: String,
}

/// The `/` search: the query being typed and the hits it produced.
#[derive(Debug, Default)]
pub struct Search {
    pub query: String,
    pub cursor: usize,
    /// Results are showing in the middle pane.
    pub active: bool,
    pub hits: Vec<SearchItem>,
    pub selected: usize,
}

impl Search {
    pub fn clear(&mut self) {
        self.query.clear();
        self.cursor = 0;
        self.active = false;
        self.hits.clear();
        self.selected = 0;
    }

    pub fn selected_item(&self) -> Option<&SearchItem> {
        self.hits.get(self.selected)
    }

    /// Re-run the query over the vault.
    pub fn run(&mut self, vault: &Vault) {
        let items = all_entries(vault);
        let mut scored: Vec<(i32, SearchItem)> = items
            .into_iter()
            .filter_map(|it| item_score(&self.query, &it).map(|s| (s, it)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
        self.hits = scored.into_iter().map(|(_, it)| it).collect();
        self.selected = self.selected.min(self.hits.len().saturating_sub(1));
        self.active = true;
    }
}

fn item_score(query: &str, it: &SearchItem) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut best = None;
    for (weight, hay) in [
        (3, it.title.as_str()),
        (2, it.username.as_deref().unwrap_or("")),
        (1, it.extra.as_str()),
        (1, it.path.as_str()),
    ] {
        if let Some(s) = score(query, hay) {
            let s = s * weight;
            best = Some(best.map_or(s, |b: i32| b.max(s)));
        }
    }
    best
}

/// Subsequence score, or `None` when `needle` does not fuzzy-match `hay`.
pub fn score(needle: &str, hay: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay_chars: Vec<char> = hay.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle_chars: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    let mut total = 0i32;
    let mut pos = 0usize;
    let mut last: Option<usize> = None;
    for nc in needle_chars {
        let found = hay_chars[pos..].iter().position(|h| *h == nc)? + pos;
        let mut points = 10;
        if Some(found) == last.map(|l| l + 1) {
            points += 8; // adjacent run
        }
        let starts_word = found == 0
            || matches!(
                hay_chars.get(found.wrapping_sub(1)),
                Some(' ' | '/' | '.' | '-' | '_' | ':')
            );
        if starts_word {
            points += 6;
        }
        points -= (found as i32 - last.map(|l| l as i32).unwrap_or(-1) - 1).min(8);
        total += points;
        last = Some(found);
        pos = found + 1;
    }
    Some(total)
}

/// Every entry in the vault, flattened.
pub fn all_entries(vault: &Vault) -> Vec<SearchItem> {
    let mut out = Vec::new();
    let mut stack = vec![vault.root()];
    while let Some(gid) = stack.pop() {
        let Some(g) = vault.db().group(gid) else {
            continue;
        };
        let in_bin = vault.is_in_recycle_bin(gid);
        for e in g.entries() {
            let id = e.id();
            let mut extra = String::new();
            if let Some(u) = e.get_url() {
                extra.push_str(u);
            }
            for t in &e.tags {
                extra.push(' ');
                extra.push_str(t);
            }
            out.push(SearchItem {
                id,
                group: gid,
                path: vault.entry_path(id),
                title: e.get_title().unwrap_or("").to_string(),
                username: e
                    .get_username()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                expired: chiave_core::is_expired(&e.times),
                has_otp: e.get_raw_otp_value().is_some(),
                in_recycle_bin: in_bin,
                extra,
            });
        }
        stack.extend(g.groups().map(|c| c.id()));
    }
    out.sort_by_key(|a| a.path.to_lowercase());
    out
}
