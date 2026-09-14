-- chiave hotkey snippet for omarchy quattro (4.x, Lua bindings).
--
-- Append the lines below to ~/.config/hypr/bindings.lua (inside the same
-- table/section as the other `o.bind` / `o.rebind` calls; the exact table
-- name varies by quattro revision, so match the surrounding style rather
-- than pasting this as a standalone file).
--
-- NOTE ON UNCERTAINTY: `chiave tui` is the intended zero-arg TUI mode
-- described in PLAN.md phase 3, but as of this writing (see README.md,
-- "phase 1 complete... TUI next") the TUI is not wired up yet and there is
-- no `tui` subcommand in src/main.rs (only `shell` and one-shot commands).
-- These bindings will fail with "no such subcommand" until that lands.
-- Once it does, `chiave` with no arguments is also expected to open the
-- TUI directly (see PLAN.md src/main.rs dispatch table: "no args -> TUI"),
-- but we bind `chiave tui` explicitly here so the binding keeps working
-- even if that default ever changes.

-- Replace 1Password's Super+Shift+/ with chiave's TUI.
-- o.rebind unbinds whatever is currently on this chord (1Password's
-- `{ omarchy = "1password" }` spec, if you followed docs/omarchy.md and
-- ran omarchy-remove-service-1password first) and binds the new spec.
-- `{ tui = "chiave tui", focus = true }` resolves to
-- `omarchy-launch-or-focus-tui "chiave tui"`, which runs the command in a
-- floating terminal (see contrib/omarchy/window.lua for the window rule)
-- and focuses the existing window instead of opening a second one if it
-- is already running.
-- Bare `chiave` opens the TUI. Keep it a single word: omarchy-launch-tui builds the
-- window app-id from `basename` of the command, and the window rule in window.lua
-- matches org.omarchy.chiave.
o.rebind("SUPER + SHIFT + SLASH", "Passwords", { tui = "chiave", focus = true })

-- OPTIONAL second binding: drop straight into the kpcli-style shell
-- instead of the TUI (handy for scripting-style lookups, e.g. `find`,
-- `xp`, piping output). Pick a chord that is free on your machine --
-- SUPER + SHIFT + P is used here only as an example and may already be
-- bound to something else in your bindings.lua; check first with
-- `hyprctl binds` or by reading the file.
o.bind("SUPER + SHIFT + P", "Password Shell", { tui = "chiave shell", focus = true })
