# chiave

*chiave* (Italian: key) is a KeePass KDBX shell and TUI for the terminal, a
compiled successor to the Perl tool kpcli. One binary, three modes:

- `chiave shell` — kpcli-compatible interactive shell (`ls`, `cd`, `show`, `xp`, `find`, `new`, `edit` ...)
- `chiave <command>` — one-shot subcommands for scripting
- `chiave` — full-screen TUI

Written in Rust on top of [keepass-rs](https://github.com/sseemayer/keepass-rs).
Designed to replace 1Password on [omarchy](https://omarchy.org), with KeePassXC
as the companion for browser and SSH-agent integration.

Status: phase 0 (bootstrap). See [PLAN.md](PLAN.md).

## Build

```sh
cargo build --release
cargo test --workspace
```
