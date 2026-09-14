-- chiave window rule for omarchy quattro (4.x, Lua Hyprland config).
--
-- Append inside ~/.config/hypr/apps/ (following the pattern of
-- default/hypr/apps/terminals.lua and the 1Password rule) or into your own
-- windows.lua override, wherever your bindings.lua's o.window calls live.
--
-- Style copied from 1Password's rule:
--   o.window("^(1[pP]assword|com\\.onepassword\\.OnePassword)$",
--     { no_screen_share = true, tag = "+floating-window" })
--
-- chiave is launched via omarchy-launch-tui / omarchy-launch-or-focus-tui,
-- which runs `xdg-terminal-exec --app-id=org.omarchy.<basename> -e "<cmd>"`.
-- For `chiave tui` the basename is "chiave", so the resulting app-id is
-- "org.omarchy.chiave". That already matches the generic
-- `org\.omarchy\..*` terminal-tagging rule in terminals.lua, so this rule
-- only needs to add the password-manager-specific bits: float it, size and
-- center it, and keep it out of screen shares/recordings.
--
-- UNCERTAIN: the exact field names/shape for size + centering
-- (`size`/`position` vs a `float_size` style key, and whether centering is
-- automatic for floating windows or needs an explicit rule) were not
-- confirmed against the quattro source for this task. Verify against
-- `default/hypr/apps/*.lua` on the target machine and adjust the table
-- below -- the app-id match and `no_screen_share` are the load-bearing
-- parts and should be correct as written.
o.window("^org\\.omarchy\\.chiave", {
	no_screen_share = true, -- keep the vault contents out of screen shares/recordings
	tag = "+floating-window", -- same tag 1Password uses; float instead of tiling
	size = { 1100, 700 }, -- approximate size in pixels; adjust to taste
	position = "center", -- UNCERTAIN: confirm this is how quattro centers a float
})
