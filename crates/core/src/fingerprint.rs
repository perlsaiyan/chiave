//! A structural summary of a database, used to prove that what we wrote back
//! parses to the same content as what we had in memory.

use std::collections::BTreeMap;

use chrono::{NaiveDateTime, SubsecRound};
use keepass::db::GroupRef;
use keepass::Database;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupFp {
    pub name: String,
    pub parent: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryFp {
    pub parent: Uuid,
    /// field name -> (value, protected)
    pub fields: BTreeMap<String, (String, bool)>,
    pub tags: Vec<String>,
    pub attachments: BTreeMap<String, Vec<u8>>,
    pub history: usize,
    pub expires: Option<bool>,
    pub expiry: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub name: Option<String>,
    pub recycle_bin: Option<Uuid>,
    pub groups: BTreeMap<Uuid, GroupFp>,
    pub entries: BTreeMap<Uuid, EntryFp>,
}

impl Fingerprint {
    pub fn of(db: &Database) -> Fingerprint {
        let mut fp = Fingerprint {
            name: db.meta.database_name.clone(),
            recycle_bin: db.meta.recyclebin_uuid,
            groups: BTreeMap::new(),
            entries: BTreeMap::new(),
        };
        fp.visit(db.root());
        fp
    }

    fn visit(&mut self, g: GroupRef<'_>) {
        self.groups.insert(
            g.id().uuid(),
            GroupFp {
                name: g.name.clone(),
                parent: g.parent().map(|p| p.id().uuid()),
                notes: g.notes.clone(),
            },
        );
        for e in g.entries() {
            let fields = e
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), (v.get().clone(), v.is_protected())))
                .collect();
            let attachments = e
                .attachments_named()
                .map(|(n, a)| (n.to_string(), a.data.get().clone()))
                .collect();
            self.entries.insert(
                e.id().uuid(),
                EntryFp {
                    parent: g.id().uuid(),
                    fields,
                    tags: e.tags.clone(),
                    attachments,
                    history: e
                        .history
                        .as_ref()
                        .map(|h| h.get_entries().len())
                        .unwrap_or(0),
                    expires: e.times.expires,
                    expiry: e.times.expiry.map(|t| t.trunc_subsecs(0)),
                },
            );
        }
        for c in g.groups() {
            self.visit(c);
        }
    }

    /// Human-readable list of differences, empty when equal.
    pub fn diff(&self, other: &Fingerprint) -> Vec<String> {
        let mut out = Vec::new();
        if self.name != other.name {
            out.push("database name".into());
        }
        if self.recycle_bin != other.recycle_bin {
            out.push("recycle bin uuid".into());
        }
        for (id, g) in &self.groups {
            match other.groups.get(id) {
                None => out.push(format!("group {id} ({}) missing", g.name)),
                Some(o) if o != g => out.push(format!("group {id} ({}) differs", g.name)),
                _ => {}
            }
        }
        for id in other.groups.keys() {
            if !self.groups.contains_key(id) {
                out.push(format!("group {id} unexpected"));
            }
        }
        for (id, e) in &self.entries {
            let title = e.fields.get("Title").map(|(v, _)| v.as_str()).unwrap_or("");
            match other.entries.get(id) {
                None => out.push(format!("entry {id} ({title}) missing")),
                Some(o) if o != e => out.push(format!("entry {id} ({title}) differs")),
                _ => {}
            }
        }
        for id in other.entries.keys() {
            if !self.entries.contains_key(id) {
                out.push(format!("entry {id} unexpected"));
            }
        }
        out
    }
}
