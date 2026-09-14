# Plan: chiave, a compiled kpcli successor with a TUI

Date: 2026-09-14. Research basis: kpcli 4.1.3 source, keepass-rs / gokeepasslib
status as of this week, the omarchy repo (master 3.8.5 and quattro 4.x branches),
and a survey of existing KDBX terminal tools.

## 1. Goal

Replace 1Password on omarchy with a KeePass (KDBX4) vault, driven from the
terminal by one binary that offers:

1. kpcli-compatible interactive shell (ls / cd / show / xp / find / new / edit ...)
2. one-shot subcommands for scripting (`chiave show /web/github -f`)
3. a full-screen TUI for browsing, searching, copying and editing
4. Wayland-native clipboard with auto-clear and clipboard-manager hinting
5. omarchy wiring: hotkey, floating terminal, window rules, package

KeePassXC (GUI) stays the companion for browser extension, ssh-agent and the
one-time import from 1Password. Both tools share the same .kdbx file.

## 2. Buy vs build (frank version)

Nothing existing covers shell + TUI + write + TOTP + Wayland in one maintained
binary. What exists:

| Tool | Covers | Gap |
|---|---|---|
| keepassxc-cli (C++, Arch extra) | one-shot + REPL, full write, KDBX4, TOTP, clip auto-clear | no browsable TUI |
| keepmenu (Python, AUR) | Wayland picker + autotype + TOTP + write | menu only, no shell/TUI |
| kpxhs (Haskell) | real TUI over keepassxc-cli | read-only |
| daxartio/kdbx (Rust) | fzf-style picker, TOTP, clip | add-only writes, experimental save path |
| KeePassTUI, keepass-tui, kpcli-rust, Keepass-TUI | on paper everything | 0 to 2 commits, unreleased |

Zero-code path that works today: KeePassXC + keepassxc-cli + keepmenu bound to
Super+Shift+/. That replaces 1Password's core functions. The new tool is
justified by the unified terminal UX, not by necessity. Build it if the UX is
the point.

## 3. Language decision: Rust

| Concern | Rust | Go |
|---|---|---|
| KDBX lib | keepass-rs 0.13.25, active (commits Aug 2026), KDBX3 read, KDBX4 read/write ("experimental" label on write), TOTP built in, KDB v1 read | gokeepasslib 3.7.0, active (today), KDBX3.1/4 read/write first-class, no TOTP, open attachment bug #86 |
| Secret hygiene | zeroize + ownership, compile-time enforceable | GC copies strings; memguard mitigates only |
| Readline for shell | rustyline 18 / reedline 0.51, both active | chzyer/readline, go-prompt, liner all stale 1 to 3 yrs |
| TUI | ratatui 0.30 | bubbletea / tview, equally good |
| Wayland clipboard | wl-clipboard-rs native, or wl-copy | shell out to wl-copy |
| Prior art competition | open field | tobischo/kp2 and keydex already exist |

Rust wins on the two things that matter most for a password tool: memory
hygiene and an actively maintained readline. Go would be faster to write.

Rejected option C: a TUI wrapper around keepassxc-cli as the engine (kpxhs
model). Safe for vault data but slow, awkward for edits, and needs a persistent
child REPL to avoid re-entering the password. keepassxc-cli is used instead as
the interop test oracle.

## 4. Architecture

Single binary, Cargo workspace, three front ends over one core:

```
crates/
  core/    vault model, open/save, path resolution (kpcli fs metaphor),
           search, TOTP, password generator, recycle bin, history,
           secret types (zeroize), atomic save + backup + verify
  shell/   kpcli-compatible REPL (reedline), command table, completion,
           numbered ls references, --command batch mode
  cli/     clap one-shot subcommands sharing the shell's command impls
  tui/     ratatui app: group tree | entry list | detail pane,
           fuzzy search, copy shortcuts, edit forms, idle lock
  clip/    wl-copy adapter: x-kde-passwordManagerHint mime, --paste-once,
           N-second auto-clear, fallback to xclip
src/main.rs  mode dispatch: no args -> TUI; `shell`; any subcommand -> cli
```

Key design rules:

- Save path is the risk. Never overwrite in place: write temp, fsync, rename,
  keep `<db>.bak`. After every save, reopen the file and diff against the
  in-memory model. If keepassxc-cli is on PATH, optionally verify with it too.
- Writes are disabled until the round-trip suite passes against the KeePassXC
  `tests/data/` corpus (Format300/400, all key-file variants, Twofish, recycle
  bin, corrupt files). Vendor that corpus into `tests/fixtures/`.
- keepass-rs 0.13 saves KDBX4 only. Opening a KDBX3 file must put chiave in
  read-only mode with an explicit `upgrade` command that rewrites as KDBX4
  (after a backup). KDBX 2.x files do not open at all; point users at KeePassXC.
- keepass-rs does not verify the KDBX3 header hash (BrokenHeaderHash.kdbx
  opens). Add the check in chiave-core when reading KDBX3.
- keepass-rs `Database::new()` defaults to Argon2d with 50 rounds and 1 MB of
  memory (keepassxc-cli db-info confirms). `newdb` must set KeePassXC-grade
  parameters (Argon2id, 64 MB, 10+ iterations, tuned to ~1s) before saving.
- Watch keepass-rs issues #360 (attachments dangling in history) and #336
  (merge drops attachments). Add regression tests for both before shipping
  attachment edits.
- All secrets live in `SecretString`/`Zeroizing<Vec<u8>>`; never `String`.
  Optional mlock via memsec.
- Clipboard: always offer `x-kde-passwordManagerHint` so cliphist and the
  quattro clipboard plugin skip it; default 10s clear like keepassxc-cli.
- Idle lock like kpcli's --timeout: re-prompt for master password after N
  seconds, safe commands exempt.
- Duplicate titles: unlike kpcli, tolerate them (address by number or UUID
  suffix) since KeePassXC creates them freely.
- Lock file: honor KeePassXC's `.lock` convention so both apps coexist.

## 5. kpcli command parity target

Phase 1 (read-only): open, close, ls/dir, cd/chdir, cl, pwd, show (-f -a),
get, find (-a -expired), otp, xu xw xp xo xpx xx, stats, ver/vers, cls, help,
history, quit/exit.

Phase 2 (write): new, edit, set, clone, copy/cp, mv, rm, mkdir, rmdir, rename,
save, saveas, newdb, passwd, attach, purge, icons, pwck, export.

Deferred or dropped: import (use KeePassXC), reroot, mktestdb (becomes a test
helper), utf8 toggle (always on), autosave (replaced by atomic save).

Launch flags to keep: --kdb, --key, --pwfile, --readonly, --timeout,
--command (repeatable), --histfile, --no-recycle, --xpxsecs, password
generator flags. Add: --tui, --shell, --clip-timeout, --no-clip-hint.

## 6. Phases

Progress (2026-09-14): phases 0 to 3 built. Core (read, write with history,
recycle bin, attachments, atomic verified save, create, upgrade, generator,
pwck, purge), kpcli shell with read and write commands, one-shot CLI with
auto-save, clipboard backends verified on omarchy quattro, ratatui TUI
(tree, entries, detail, search, copy, edit forms, lock), omarchy contrib
files and AUR PKGBUILD. keepass-rs is vendored with attachment-reference
fixes (vendor/keepass-rs/CHIAVE-PATCHES.md). Not yet: TUI editing of
attachments, tags and OTP secrets; TUI history view; pwck/purge shell
commands; KDBX3 header-hash check; phase 5 items.

0. Bootstrap (half a day): pick name, `cargo new --workspace`, rustup on the
   Ubuntu dev box, CI, fetch KeePassXC fixtures, write a throwaway KDBX4 vault
   with keepassxc-cli for local testing.
1. Core read + shell + CLI read-only (Phase 1 command list), TOTP, clipboard.
   Milestone: daily use for lookups.
2. Write path with atomic save, round-trip suite, history entries, recycle bin,
   Phase 2 commands. Milestone: stop opening the KeePassXC GUI for edits.
3. TUI: tree/list/detail, `/` search, y/u/p/o copy keys, e edit, n new,
   idle lock. Milestone: Super+Shift+/ opens it in a floating terminal.
4. omarchy integration: `o.rebind("SUPER + SHIFT + SLASH", "Passwords",
   "omarchy-launch-or-focus-tui chiave")` (quattro) or bindings.conf unbind +
   bindd (3.8.5), window rule with `no_screen_share`, install script modeled
   on omarchy-install-service-1password, PKGBUILD + AUR. Note: omarchy updates
   have stomped user bindings before (issues #6933, #1802). Document
   re-applying.
5. Later: unlock-cache daemon (ssh-agent style socket) so repeated shell
   invocations do not re-prompt; ssh-agent from vault keys; Elephant/Walker or
   quattro menu provider; KDB v1 import; merge.

Migration from 1Password: export 1PUX in 1Password, import in the KeePassXC
GUI (native 1PUX importer), remove 1Password with
`omarchy-remove-service-1password` (not the generic remover, which leaves
native-messaging manifests behind per issue #8916).

## 7. Size and agent plan

Estimate: 10k to 15k lines of Rust plus tests. Sequential single-session build
is feasible. Parallelism helps in phases 2 and 3.

| Work | Who | Why |
|---|---|---|
| Core model, save path, secret types, security review | Fable (this session) | judgment-heavy, data-loss risk |
| Shell command implementations from the parity table | Opus subagent(s), 2 in parallel | well specified, mechanical |
| TUI screens from a written spec | Opus subagent | contained, visual iteration |
| Fixture harness, PKGBUILD, omarchy scripts, docs | Sonnet subagent | routine |

No cloud Workflow needed. Review every subagent PR against the design rules in
section 4 before merge.

## 8. Naming constraints

Taken or confusing: kpcli (Perl original, Python rebkwok/kpcli, Go
robertranjan/kpcli on AUR), kpcli-rust, kpcli-go, kp / kp2, kpxc, keydex,
kdbx, tresor, KeePassTUI, keepass-tui, jaiba, kpass, passlane. Check crates.io,
AUR and GitHub before committing to a name.

## 9. Open questions

- Target machine runs omarchy quattro (4.x, Lua bindings). Phase 4 uses
  o.rebind in ~/.config/hypr/bindings.lua and omarchy-launch-or-focus-tui.
- Is YubiKey challenge-response needed? keepass-rs does not do it; defer.
- Key file in use today, or password only?
