# chiave-clip

Clipboard access for secrets, for Linux terminals. Wayland first (omarchy /
Hyprland), X11 as a fallback.

The job is narrower than "put a string on the clipboard": a password must be
marked so clipboard managers do not archive it, it must disappear on a timer,
and it must still be there after the one-shot `chiave xp` process has exited.
Those three requirements drive the whole design.

## Public contract

```rust
pub trait Clipboard: Send + Sync {
    fn name(&self) -> &'static str;
    fn copy_secret(&self, secret: &SecretString, clear_after: Option<Duration>) -> Result<(), ClipError>;
    fn copy_text(&self, text: &str) -> Result<(), ClipError>;
    fn clear(&self) -> Result<(), ClipError>;
    // added, both have default bodies:
    fn supports_sensitive_hint(&self) -> bool;
    fn read_back(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError>;
}

pub fn detect(opts: DetectOptions) -> Box<dyn Clipboard>;
pub fn helper_main(args: Vec<String>) -> ExitCode;   // `chiave __clip-serve ...`
```

`DetectOptions` carries `helper_exe: Option<PathBuf>` and `paste_once: bool`.

Also public, mostly so the behaviour is testable and inspectable:
`BackendKind`, `EnvSnapshot`, `choose_backend`, `OVERRIDE_VAR`,
`build_offers`/`MimeOffer`/`OfferPayload`/`TEXT_MIME_TYPES`/`SENSITIVE_MIME`/
`SENSITIVE_VALUE`/`primary_text_mime`, `HelperArgs`/`parse_helper_args`/
`HELPER_SUBCOMMAND`, and `ReadClear`/`ClearOutcome`/`clear_if_unchanged`.

## Backends

| `name()` | chosen when | sensitive hint | survives caller exit |
|---|---|---|---|
| `wayland` | `WAYLAND_DISPLAY` is set | **yes** | only via the helper process |
| `wayland (wl-copy fallback)` | native path hit a compositor without data-control | no | yes (`wl-copy` forks) |
| `x11` | `DISPLAY` is set and Wayland is not | no | yes (`xclip` forks) |
| `wl-copy` | forced with `CHIAVE_CLIPBOARD=wl-copy` | no | yes |
| `none` | no display | n/a | n/a |

Detection order, implemented as the pure function `choose_backend(&EnvSnapshot)`:

1. `CHIAVE_CLIPBOARD` — `wayland` \| `x11` \| `wl-copy` \| `none`
   (case-insensitive; an unrecognised value is ignored and detection continues,
   because `detect()` has no channel to report an error on)
2. `WAYLAND_DISPLAY` set and non-empty → `wayland`
3. `DISPLAY` set and non-empty → `x11`
4. otherwise → `none` (`NullClipboard`, every call returns `ClipError::Unavailable`)

## The sensitive hint

`copy_secret` on the native Wayland backend offers the value under six MIME
types in a single `copy_multi` call:

```
text/plain;charset=utf-8
text/plain
UTF8_STRING
STRING
TEXT
x-kde-passwordManagerHint   -> the literal bytes "secret"
```

The last one is the convention KeePassXC established. `wl-paste` exports
`CLIPBOARD_STATE=sensitive` when it sees it, which is what makes **cliphist**,
**KDE Klipper** and the **omarchy quattro clipboard plugin** skip the entry
instead of writing it to their history. The text types come first in the offer
list so that a consumer asking for "any text" always gets the password and never
the literal string `secret`.

`copy_text` (usernames, URLs) offers the five text types and no hint, so those
*do* land in clipboard history — which is what you want.

The offer list is built by the pure function `build_offers(sensitive: bool)` and
is unit-tested. `wl-clipboard-rs`' own "add the common text types for you"
behaviour is switched off (`omit_additional_text_mime_types(true)`) so the offer
set is exactly what `build_offers` says.

## The helper process

`wl-clipboard-rs` 0.9 serves paste requests from a **thread inside the copying
process** (`copy_multi` spawns it; `foreground(true)` blocks instead). Older
versions of the crate forked; this one does not. So for a long-running `chiave
shell` or the TUI an in-process copy is fine, but `chiave xp github` would put
the password on the clipboard and then take it away again on exit.

So whenever `DetectOptions::helper_exe` is set, the Wayland backend copies
through a detached helper instead — and any backend does when auto-clear is
requested:

```
chiave xp ──fork/exec, setsid──▶ chiave __clip-serve --clear-after 10 --backend wayland
          ──secret on stdin───▶
          ◀──"ok" on stdout, once the selection is actually taken──
   exits immediately                 stdio ▶ /dev/null
                                     sleep 10
                                     read the clipboard back
                                     clear only if it is still ours
                                     exit 0
```

Details that matter:

- **The secret travels on stdin only.** Never argv (`/proc/*/cmdline` is world
  readable), never the environment. There is a unit test asserting the rendered
  argv cannot contain it.
- **`setsid`** in a `pre_exec` hook puts the helper in its own session, so
  Ctrl-C in the caller's process group or closing the terminal does not kill it.
- **Readiness handshake.** The helper prints `ok` once it owns the clipboard, or
  `err <message>`, and only then redirects its stdio to `/dev/null`. The spawner
  blocks on that one line, so a copy failure is still reported synchronously to
  the user instead of vanishing into a background process.
- **No zombies.** The spawner reaps the child on a detached thread rather than
  leaving it un-waited, which matters for the long-running shell.
- **`--clear-after 0` / absent** means "hold the selection until another
  application takes it". On Wayland the helper blocks on the serving loop and
  exits when it loses the selection, exactly like `wl-copy`. On the command
  backends (`xclip`, `wl-copy`), which fork their own selection owner, the
  helper simply exits at once and is not spawned at all unless auto-clear was
  asked for.
- **`--paste-once`** (from `DetectOptions::paste_once`, secrets only) maps to
  `ServeRequests::Only(1)` natively, `wl-copy --paste-once`, and `xclip -loops 1`.
  `xsel` has no equivalent and ignores it. Off by default: XWayland clients are
  known to have trouble pasting a one-shot offer.

Helper command line: `chiave __clip-serve [--clear-after SECS] [--backend
wayland|x11|wl-copy|none] [--paste-once] [--text|--secret]`. Both `--flag value`
and `--flag=value` are accepted. `--text` drops the sensitive hint.

### Clear only if unchanged

Auto-clear never blindly wipes the clipboard. `clear_if_unchanged` reads the
clipboard back through the backend and compares:

| read-back | action | `ClearOutcome` |
|---|---|---|
| still our value | clear | `Cleared` |
| empty | nothing | `AlreadyEmpty` |
| something else | nothing | `Changed` |
| read failed | clear anyway | `ClearedUnverified` |

A single trailing newline is ignored in the comparison (`wl-paste` adds one
unless `--no-newline`). Read failure clears anyway: leaving a password on the
clipboard is worse than clobbering an unrelated copy. The function is pure over
a `ReadClear` trait, so CI exercises every branch against a fake backend with no
display.

### When there is no helper executable

`helper_exe: None` falls back to copying in-process and running the timer on a
detached thread. **That thread dies with the process**, so a one-shot invocation
that exits before the delay elapses leaves the secret on the clipboard (and on
Wayland, loses it entirely). Long-running modes are unaffected. If spawning the
helper fails (missing or broken executable), the same in-process path is used
rather than failing the copy; if that also fails, both errors are reported.

## Secret hygiene

- Secrets arrive as `SecretString` and are turned into `Zeroizing<Vec<u8>>`
  immediately; every buffer this crate owns — the copy payload, the helper's
  stdin read, every read-back — is `Zeroizing` and wiped on drop.
- The helper zeroizes before exit (the `Zeroizing` buffer's destructor runs on
  the normal return path out of `helper_main`).
- **Limitation:** once the bytes are handed to `wl-clipboard-rs` they live in a
  plain `Arc<[u8]>` inside the crate's data source and are not zeroized when the
  offer is dropped, and for the command backends they are written into a pipe to
  another process. Neither is under this crate's control. No `mlock` either, so
  pages can be swapped; PLAN.md's optional memsec `mlock` would cover that.
- Comparison in `clear_if_unchanged` is a plain `==`, not constant-time. It runs
  against a value this process already holds, so there is nothing to leak.

## Known limitations

- **Native Wayland needs `ext-data-control` or `wlr-data-control`.** Hyprland,
  Sway and wlroots compositors have it; GNOME/Mutter does not. On failure the
  backend degrades at the first failing call to shelling out to `wl-copy`, if
  that binary is on `PATH`, and `name()` becomes `wayland (wl-copy fallback)`.
- **The `wl-copy` fallback cannot offer the hint.** `wl-copy` takes a single
  `--type` per invocation and a second invocation *replaces* the selection
  instead of adding to it, so `x-kde-passwordManagerHint` cannot sit next to the
  text types. This backend copies as `text/plain;charset=utf-8` only, which means
  **clipboard managers will record the secret**. `supports_sensitive_hint()`
  returns `false` so the shell can warn; auto-clear still applies.
- **X11 has no hint at all.** `xclip`/`xsel` own one target per invocation, so
  the same caveat applies, permanently. Auto-clear is the only protection there.
- **Read-back needs a tool.** The `wl-copy` backend needs `wl-paste` installed
  to verify before clearing; without it the read fails and auto-clear falls into
  the `ClearedUnverified` branch (it clears regardless).
- **Primary selection** (middle-click) is never touched; only `CLIPBOARD`.
- **No image or attachment support.** Text only.
- One helper process per copy while it is holding the selection. They supersede
  each other (taking the selection makes the previous owner's serving loop exit),
  so they do not accumulate.

## Verifying on omarchy

Build the manual test tool. It doubles as the `__clip-serve` dispatcher, so it
exercises the real detached-helper path:

```sh
cargo build -p chiave-clip --example clip-demo
DEMO=./target/debug/examples/clip-demo
```

1. **The hint is offered.** The demo exits immediately; the helper keeps the
   clipboard.

   ```sh
   $DEMO secret hunter2 30
   wl-paste --list-types
   ```

   Expect all six types, including `x-kde-passwordManagerHint`:

   ```
   text/plain;charset=utf-8
   text/plain
   UTF8_STRING
   STRING
   TEXT
   x-kde-passwordManagerHint
   ```

   And `wl-paste` should hand back the password itself:

   ```sh
   wl-paste --no-newline; echo
   ```

2. **Clipboard managers skip it.**

   ```sh
   cliphist list | grep hunter2   # must print nothing, exit 1
   ```

   The quattro clipboard plugin and KDE Klipper honour the same hint. Check the
   history UI too if you use one.

3. **It survives the caller exiting.** Step 1 already proves this: `$DEMO`
   returned before `wl-paste` ran. Confirm the helper is the one holding it:

   ```sh
   ps -o pid,sid,stat,cmd -C clip-demo
   # PID == SID and STAT Ss: its own session, detached from the terminal
   ```

4. **Auto-clear fires.**

   ```sh
   $DEMO secret hunter2 5; sleep 6; wl-paste --list-types   # "Nothing is copied"
   ```

5. **Auto-clear leaves a newer copy alone.**

   ```sh
   $DEMO secret hunter2 5
   echo "something else" | wl-copy
   sleep 6
   wl-paste --no-newline; echo    # still "something else"
   ```

6. **Plain text is not marked sensitive.**

   ```sh
   $DEMO text tom
   wl-paste --list-types | grep -c passwordManagerHint   # 0
   cliphist list | grep -c tom                           # >= 1
   ```

7. **The ignored round-trip tests**, on a machine with a session:

   ```sh
   cargo test -p chiave-clip -- --ignored --nocapture
   ```

To force a backend while debugging: `CHIAVE_CLIPBOARD=wl-copy $DEMO secret x 10`.

## What is and is not verified

Developed and tested on a headless Ubuntu box with no Wayland and no X11.

Verified there:

- All unit and integration tests (`cargo test -p chiave-clip`), including the
  offer list, helper argument parsing and round-tripping, the
  clear-only-if-unchanged decision table against a fake backend, and the
  detection order both as a pure function and through `detect()` with real
  environment variables.
- The **whole detached-helper path** end to end, using stub `wl-copy`/`wl-paste`
  and `xclip` scripts backed by a file: spawn with `setsid`, secret over stdin
  (and confirmed absent from `/proc` argv), readiness handshake, caller exiting
  while the helper holds the value, the timer firing, and the timer correctly
  declining to clear after a foreign copy.
- `cargo build -p chiave-clip` needs **no system libraries**: `wl-clipboard-rs`
  is used with default features, so `wayland-backend` is the pure-Rust client
  (the `native_lib`/`dlopen` features that link `libwayland-client.so` are off).

**Not verified** — no Wayland session was available:

- The native `wl-clipboard-rs` code path against a real compositor: that the
  six-MIME `copy_multi` offer appears as expected in `wl-paste --list-types`,
  that cliphist/Klipper/quattro actually skip it, that `paste::get_contents`
  read-back and `copy::clear` behave as assumed, and that the serving thread
  holds the selection for the helper's lifetime.
- The `is_protocol_missing` → `wl-copy` fallback trigger: the string matching
  against `wl-clipboard-rs`' `MissingProtocol`/`NoSeats` error text was checked
  against the crate source, not against a live GNOME session.
- Real `xclip`/`xsel`/`wl-copy`/`wl-paste` binaries (stubs were used, matching
  their documented argument and exit-status behaviour). In particular the
  assumption that all three fork a background selection owner and exit promptly
  — so waiting on them does not block — is from their documentation, not from
  an observed run.
- `xclip -loops 1` and `wl-copy --paste-once` semantics.

Work through the "Verifying on omarchy" list on the target machine before
trusting the Wayland path.
