//! The collapsible group tree shown in the left pane.

use std::collections::HashSet;

use chiave_core::{GroupId, Vault};

/// One visible line of the tree.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub id: GroupId,
    pub depth: usize,
    pub name: String,
    pub has_children: bool,
    pub expanded: bool,
    pub is_recycle_bin: bool,
}

/// Flattened tree plus the expansion state that produced it.
#[derive(Debug, Default)]
pub struct Tree {
    pub rows: Vec<TreeRow>,
    pub selected: usize,
    expanded: HashSet<GroupId>,
}

impl Tree {
    pub fn new() -> Self {
        Tree::default()
    }

    pub fn selected_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.selected)
    }

    pub fn selected_id(&self) -> Option<GroupId> {
        self.selected_row().map(|r| r.id)
    }

    pub fn is_expanded(&self, id: GroupId) -> bool {
        self.expanded.contains(&id)
    }

    /// Rebuild the flattened rows from the vault, keeping the selected group if it survives.
    pub fn rebuild(&mut self, vault: &Vault, root_label: &str) {
        let keep = self.selected_id();
        let root = vault.root();
        self.expanded.insert(root);
        self.rows.clear();
        self.push(vault, root, 0, root_label.to_string());
        self.selected = keep
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
    }

    fn push(&mut self, vault: &Vault, id: GroupId, depth: usize, label: String) {
        let bin = vault.recycle_bin_id();
        let mut kids = vault.children(id).map(|l| l.groups).unwrap_or_default();
        // `children` sorts case-insensitively already; the recycle bin goes last.
        kids.sort_by_key(|g| (Some(g.id) == bin, g.name.to_lowercase()));
        let expanded = self.expanded.contains(&id);
        self.rows.push(TreeRow {
            id,
            depth,
            name: label,
            has_children: !kids.is_empty(),
            expanded,
            is_recycle_bin: Some(id) == bin,
        });
        if expanded {
            for k in kids {
                self.push(vault, k.id, depth + 1, k.name);
            }
        }
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_id(&mut self, id: GroupId) {
        if let Some(i) = self.rows.iter().position(|r| r.id == id) {
            self.selected = i;
        }
    }

    /// Expand the selected group, or step into its first child when already open.
    /// Returns true when the flattened rows need rebuilding.
    pub fn expand_selected(&mut self) -> bool {
        let Some(row) = self.rows.get(self.selected) else {
            return false;
        };
        if row.has_children && !row.expanded {
            self.expanded.insert(row.id);
            true
        } else {
            if row.has_children {
                self.select_next();
            }
            false
        }
    }

    /// Collapse the selected group, or jump to its parent when already closed.
    /// Returns true when the flattened rows need rebuilding.
    pub fn collapse_selected(&mut self) -> bool {
        let Some(row) = self.rows.get(self.selected) else {
            return false;
        };
        if row.expanded && row.has_children {
            let id = row.id;
            self.expanded.remove(&id);
            true
        } else {
            let depth = row.depth;
            if let Some(parent) = self.rows[..self.selected]
                .iter()
                .rposition(|r| r.depth + 1 == depth)
            {
                self.selected = parent;
            }
            false
        }
    }

    /// Make sure every ancestor of `id` is open, so `id` becomes visible.
    pub fn reveal(&mut self, vault: &Vault, id: GroupId) {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        while let Some(g) = cur {
            chain.push(g);
            cur = vault
                .db()
                .group(g)
                .and_then(|gr| gr.parent().map(|p| p.id()));
        }
        for g in chain {
            self.expanded.insert(g);
        }
    }

    /// Every group in the vault, depth-first, for the move picker.
    pub fn all_groups(vault: &Vault, root_label: &str) -> Vec<(GroupId, usize, String)> {
        let mut out = Vec::new();
        collect(vault, vault.root(), 0, root_label.to_string(), &mut out);
        out
    }
}

fn collect(
    vault: &Vault,
    id: GroupId,
    depth: usize,
    label: String,
    out: &mut Vec<(GroupId, usize, String)>,
) {
    let bin = vault.recycle_bin_id();
    out.push((id, depth, label));
    let mut kids = vault.children(id).map(|l| l.groups).unwrap_or_default();
    kids.sort_by_key(|g| (Some(g.id) == bin, g.name.to_lowercase()));
    for k in kids {
        collect(vault, k.id, depth + 1, k.name, out);
    }
}
