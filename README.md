# chiave

*chiave* (Italian: key) is a KeePass KDBX shell and TUI for the terminal, a
compiled successor to the Perl tool kpcli. One binary, three modes:

- `chiave shell` — kpcli-compatible interactive shell (`ls`, `cd`, `show`, `xp`, `find`, `new`, `edit` ...)
- `chiave <command>` — one-shot subcommands for scripting
- `chiave` — full-screen TUI

Written in Rust on top of [keepass-rs](https://github.com/sseemayer/keepass-rs).
Designed to replace 1Password on [omarchy](https://omarchy.org), with KeePassXC
as the companion for browser and SSH-agent integration.

Status: phase 1 complete (read-only shell, one-shot CLI, clipboard); the
core write path is done and the write commands and TUI are next. See
[PLAN.md](PLAN.md).

## Quick start

```sh
cargo build --release
target/release/chiave --kdb ~/vault.kdbx ls            # one-shot
target/release/chiave --kdb ~/vault.kdbx               # interactive shell
target/release/chiave --kdb ~/vault.kdbx --command "find github" --command "xp 1"
```

Configuration lives in `~/.config/chiave/config.toml`:

```toml
database = "~/vault.kdbx"
keyfile = "~/vault.keyx"   # optional
clip_timeout = 10           # seconds before a copied secret is cleared
timeout = 300               # idle seconds before the shell re-asks the password
```

Environment: `CHIAVE_KDB`, `CHIAVE_KEYFILE`, `CHIAVE_PASSWORD` (scripts only),
`CHIAVE_CLIPBOARD=wayland|x11|wl-copy|none`.

## Development

```sh
cargo test --workspace
# Interop oracle: run KeePassXC's CLI against what chiave writes
CHIAVE_KPXC_CLI=/path/to/keepassxc-cli cargo test --workspace
```

`tests/fixtures/keepassxc` is KeePassXC's own test corpus. `vendor/keepass-rs`
is keepass-rs 0.13.25 with local fixes for attachment reference handling
(`vendor/keepass-rs/CHIAVE-PATCHES.md`, `patches/`), applied through
`[patch.crates-io]`. A KeePassXC AppImage extracted to `.tools/keepassxc/`
(gitignored) provides `keepassxc-cli` for the oracle tests on machines
without KeePassXC installed.
