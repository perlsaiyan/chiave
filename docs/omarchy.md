# Migrating from 1Password to chiave on omarchy

This is the step-by-step for replacing 1Password with chiave + KeePassXC on
an omarchy (quattro, 4.x) machine, per [PLAN.md](../PLAN.md) section 6 phase
4. KeePassXC stays installed as the companion for the browser extension and
ssh-agent; chiave and KeePassXC share one `.kdbx` file.

Status check first: chiave's write path and TUI are still in progress (see
[README.md](../README.md)). Don't remove 1Password until you have verified
you can *read* your real vault with chiave (`chiave --kdb ... ls`, `find`,
`show`) and are comfortable with the shell. Treat this guide as the target
end state, not a same-day migration.

## 1. Export from 1Password

1. Open 1Password's desktop app (or `op` CLI) and export **all vaults** in
   **1PUX** format (File -> Export -> All Items, format "1Password Unencrypted
   Export (.1pux)"). 1PUX is the format KeePassXC's importer understands
   natively; do not use the older CSV export, which loses TOTP secrets,
   attachments and item history.
2. The export is unencrypted on disk. Put it somewhere you control (not a
   synced folder, not `/tmp` on a multi-user box) and delete it as soon as
   step 2 is done.

## 2. Import into KeePassXC (GUI)

1. Install KeePassXC if it isn't already there: `omarchy-pkg-add keepassxc`
   (or `contrib/omarchy/omarchy-install-chiave`, which installs both
   keepassxc and chiave).
2. Create the new database first: Database -> New Database.
   - Format: **KDBX 4** (not KDBX 3.1 -- KDBX4 is what chiave's keepass-rs
     backend writes; KDBX3 files open read-only in chiave until you run
     `upgrade`, so start on KDBX4 and skip that step entirely).
   - Key Derivation Function: **Argon2id**.
   - Leave the memory/iteration defaults KeePassXC suggests (it tunes to
     roughly 1 second on your hardware, similar to what PLAN.md asks
     chiave's own `newdb`/`create` command to target: 64 MB memory, 10+
     iterations) unless you have a specific reason to raise them further.
   - Set a strong master password. Add a key file too if you want a second
     factor tied to a specific machine/USB stick; chiave supports key files
     via `--key` / `keyfile =` in its config.
3. Database -> Import -> 1Password 1pux File, point it at the export from
   step 1.
4. Spot-check the import: entry count matches 1Password, TOTP entries still
   generate correct-looking codes, attachments opened. Then securely delete
   the `.1pux` file (`shred -u` or move it to an encrypted vault you control
   if you want to keep it as a one-time backup).

## 3. Configure chiave

Install chiave (AUR once published; until then `cargo install --path .`
from a checkout, or run `contrib/omarchy/omarchy-install-chiave` which does
this for you). Then write `~/.config/chiave/config.toml`:

```toml
database = "~/vault.kdbx"      # path to the KDBX4 file you just created
# keyfile = "~/vault.keyx"     # uncomment if you added a key file above
clip_timeout = 10               # seconds before a copied secret is cleared
timeout = 300                   # idle seconds before chiave re-asks the password
```

Verify with a read-only smoke test before trusting it day to day:

```sh
chiave --kdb ~/vault.kdbx ls
chiave --kdb ~/vault.kdbx find github
chiave --kdb ~/vault.kdbx show /web/github -f
```

## 4. Wire up the hotkey

Append the snippet in [`contrib/omarchy/bindings.lua`](../contrib/omarchy/bindings.lua)
to `~/.config/hypr/bindings.lua`, and the rule in
[`contrib/omarchy/window.lua`](../contrib/omarchy/window.lua) to your window
rules (`~/.config/hypr/apps/` or wherever your other `o.window` calls live).
`contrib/omarchy/omarchy-install-chiave` prints both snippets and will
append them automatically if it recognizes an include-file layout in your
bindings.lua; otherwise it just prints instructions, since blindly editing
someone's hand-tuned Hyprland config is a good way to break it.

Reload with `hyprctl reload` (or re-log) and confirm `SUPER + SHIFT + /`
opens chiave in a floating terminal, and that pressing it again focuses the
same window instead of opening a second one.

**`omarchy update` has stomped user-added bindings before** (upstream issues
#6933 and #1802): an update can overwrite `bindings.lua` wholesale and drop
anything you appended by hand. After any `omarchy update`, check
`~/.config/hypr/bindings.lua` for your chiave lines and re-append the
snippet if they're gone. Keeping a copy of `contrib/omarchy/bindings.lua` in
this repo (or your dotfiles) makes that a two-minute fix instead of a
from-memory rewrite.

## 5. Verify clipboard hygiene

chiave's clipboard adapter sets the `x-kde-passwordManagerHint` MIME type on
every copy, same as keepassxc-cli. quattro's clipboard history plugin
(`shell/plugins/clipboard/capture.sh`) is written to skip entries carrying
that hint (and anything copied while `CLIPBOARD_STATE=sensitive`), so a
copied password should never land in clipboard history. Verify this
yourself rather than trusting the doc:

```sh
chiave --kdb ~/vault.kdbx xp /web/github     # or your TUI's copy shortcut
wl-paste --list-types                        # should include x-kde-passwordManagerHint
```

Then open your clipboard history (whatever quattro binds it to, typically
`SUPER + V` or similar) and confirm the secret you just copied is **not**
listed. If it is, something changed upstream in the clipboard plugin or in
how chiave sets the hint -- treat that as a bug, not a documentation gap.

Also confirm the auto-clear: after `clip_timeout` seconds, `wl-paste`
should no longer return the secret.

## 6. Remove 1Password

Use the dedicated remover, not the generic one:

```sh
omarchy-remove-service-1password
```

The generic `omarchy-remove-*` path leaves 1Password's Chromium
native-messaging manifest behind (upstream issue #8916), which can leave
stale "1Password wants to connect" prompts in your browser. The dedicated
script cleans that up along with the package itself.
`contrib/omarchy/omarchy-install-chiave` offers to run this for you at the
end of the install, or run it manually once you trust chiave day to day.

Keep 1Password's original vault data around (the 1PUX export, or a
1Password backup) somewhere safe until you're confident in the new setup --
don't delete your only copy on day one.

## 7. Keep KeePassXC for browser + ssh-agent

chiave does not do browser autofill or SSH agent duty, and PLAN.md does not
plan to add either any time soon (an ssh-agent-style unlock-cache daemon is
listed as later/phase 5 work, not committed). Keep KeePassXC running
alongside chiave:

- **Browser extension**: install the KeePassXC-Browser extension in your
  browser and enable "Browser Integration" in KeePassXC's settings, pointed
  at the same `.kdbx` file.
- **SSH agent**: enable KeePassXC's "SSH Agent" integration (Settings ->
  SSH Agent) and store SSH keys as attachments/entries in the same
  database, same as you would have with 1Password's SSH agent.

Both chiave and KeePassXC honor KeePassXC's `.lock` file convention, so it's
safe to have both open against the same database, but avoid *editing* the
same entry in both at once -- the usual "last save wins" caveat with a
shared file applies. Locking on write is atomic on the chiave side (write
temp, fsync, rename, keep a `.bak`), but concurrent edits from two
apps are still a you-problem, not a tooling problem.

## 8. Sync and backups

Options, roughly in order of how much you already have to maintain:

- **Syncthing** (or similar): sync the `.kdbx` file (and its `.keyx` key
  file, if used) between machines like any other file. Simple, no vendor
  lock-in, works with both chiave and KeePassXC's own native "sync with
  Syncthing" affordances. Exclude the `.bak` files from sync if you don't
  want sync conflicts on them too (see below).
- **KeePassXC's built-in sync helpers** (if you use their WebDAV/cloud
  storage integration) work the same way regardless of which tool last
  wrote the file, since both write standard KDBX4.
- **Git**, if you really want history on a binary file -- workable but not
  recommended; a dedicated sync tool or your existing cloud-storage habit
  is simpler.

**`.bak` behavior**: every chiave save keeps `<db>.bak` as the
previous version (per PLAN.md's atomic-save design: write temp, fsync,
rename, verify by reopening). This is a local safety net, not a backup
strategy -- it only ever holds the *immediately previous* revision, and
`*.kdbx.bak` / `*.lock.kdbx` are gitignored in this repo for that reason
(see `.gitignore`). If your sync tool free-for-alls on conflict, having a
one-generation `.bak` next to the live file can help you recover, but don't
rely on it in place of real backups or version history elsewhere.

## Uncertainties in this guide

- The exact Argon2id memory/iteration numbers KeePassXC's "New Database"
  wizard defaults to were not pinned down for this task -- described above
  qualitatively ("tunes to ~1s"); check what your installed KeePassXC
  version actually proposes and don't feel obligated to match chiave's own
  `create` defaults exactly, as long as it's Argon2id with a real memory
  cost (tens of MB, not KeePassXC's old 2019-era defaults).
- Whether quattro's `bindings.lua` supports a separate include file (so
  `omarchy update` can't clobber your additions) was not confirmed against
  the actual quattro source for this task -- see the note in
  `contrib/omarchy/omarchy-install-chiave`.
- `chiave tui` / bare `chiave` opening the TUI directly does not exist yet
  as of this writing (phase 3 in PLAN.md); the hotkey binding in
  `contrib/omarchy/bindings.lua` will not work until that ships.
