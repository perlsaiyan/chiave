# Local patches to keepass-rs 0.13.25

Vendored from crates.io (MIT) and wired in with `[patch.crates-io]` in the
workspace `Cargo.toml`. Every change is marked `chiave patch` in the source.
Diff against the verbatim import: `git diff de7bf51 -- vendor/keepass-rs/src`.
Intended for an upstream pull request to github.com/sseemayer/keepass-rs.

## Attachment reference integrity

Found by chiave's post-save verification (re-parse and compare) when removing
an attachment from an entry that had history.

1. **Dump wrote `Ref` as the internal attachment id, but binaries are written
   positionally** in ascending-id order (`to_xml`). After any removal leaves a
   gap in the ids, every attachment with a higher id was silently dropped on
   reload. Fix: `Entry::db_to_xml` writes the position of the id in sorted
   order; dangling references are skipped instead of written.
2. **Attachment back-references were never populated on load.** Only custom
   icons had a rebuild pass in `KeePassFile::xml_to_db`. With empty
   back-reference sets, removing an attachment from one entry treated the
   binary as orphaned and deleted it from the pool even when other entries or
   historical versions still used it (issue #360 family). Fix: rebuild pass
   after parsing, `Database::rebuild_attachment_backrefs_for`.
3. **History snapshots did not maintain back-references.** `EntryTrack::drop`
   inserts the snapshot at history index 0, shifting every existing
   `(entry, Some(i))` reference, and the snapshot's own references were never
   registered. Fix: `Database::rebuild_entry_backrefs` (attachments and icons)
   after the insert.
4. **Removal dropped this entry's historical references too.**
   `remove_attachment_by_name`/`_by_id` used `retain(entry_id != id)`, which
   also discarded valid references from the same entry's history. Fix:
   recompute the entry's back-references, then remove the binary only if the
   set is empty (`remove_attachment_if_orphaned`).
5. `add_attachment` on a historical `EntryMut` registered `(id, None)` instead
   of `(id, Some(index))`.
