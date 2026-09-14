//! Backend selection: which clipboard implementation this session should use.
//!
//! The choice is factored into a pure function over an environment snapshot so the
//! precedence rules can be tested without mutating the process environment.

/// Environment variable that forces a backend.
pub const OVERRIDE_VAR: &str = "CHIAVE_CLIPBOARD";

/// The available backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Native Wayland via `wl-clipboard-rs` (ext/wlr-data-control).
    Wayland,
    /// X11 via the `xclip`/`xsel` command line tools.
    X11,
    /// Wayland via the external `wl-copy`/`wl-paste` binaries.
    WlCopy,
    /// No clipboard.
    None,
}

impl BackendKind {
    /// Name reported by `Clipboard::name()` and accepted by `CHIAVE_CLIPBOARD`.
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Wayland => "wayland",
            BackendKind::X11 => "x11",
            BackendKind::WlCopy => "wl-copy",
            BackendKind::None => "none",
        }
    }

    /// Parse a `CHIAVE_CLIPBOARD` / `--backend` value. Case-insensitive.
    pub fn parse(value: &str) -> Option<BackendKind> {
        match value.trim().to_ascii_lowercase().as_str() {
            "wayland" | "wl" => Some(BackendKind::Wayland),
            "x11" | "xclip" => Some(BackendKind::X11),
            "wl-copy" | "wlcopy" => Some(BackendKind::WlCopy),
            "none" | "null" | "off" => Some(BackendKind::None),
            _ => None,
        }
    }
}

/// The environment inputs that decide the backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    /// `CHIAVE_CLIPBOARD`.
    pub override_value: Option<String>,
    /// `WAYLAND_DISPLAY`.
    pub wayland_display: Option<String>,
    /// `DISPLAY`.
    pub display: Option<String>,
}

fn non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

impl EnvSnapshot {
    /// Read the relevant variables from the process environment. Empty values count
    /// as unset.
    pub fn from_env() -> Self {
        EnvSnapshot {
            override_value: non_empty(OVERRIDE_VAR),
            wayland_display: non_empty("WAYLAND_DISPLAY"),
            display: non_empty("DISPLAY"),
        }
    }
}

/// Decide which backend to use.
///
/// Order: `CHIAVE_CLIPBOARD` override, then Wayland (`WAYLAND_DISPLAY` set), then
/// X11 (`DISPLAY` set), then none. An unrecognised override value is ignored and
/// auto-detection proceeds, because `detect()` has no way to report an error.
pub fn choose_backend(env: &EnvSnapshot) -> BackendKind {
    if let Some(kind) = env
        .override_value
        .as_deref()
        .filter(|v| !v.is_empty())
        .and_then(BackendKind::parse)
    {
        return kind;
    }
    if env
        .wayland_display
        .as_deref()
        .is_some_and(|v| !v.is_empty())
    {
        return BackendKind::Wayland;
    }
    if env.display.as_deref().is_some_and(|v| !v.is_empty()) {
        return BackendKind::X11;
    }
    BackendKind::None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(o: Option<&str>, w: Option<&str>, x: Option<&str>) -> EnvSnapshot {
        EnvSnapshot {
            override_value: o.map(str::to_string),
            wayland_display: w.map(str::to_string),
            display: x.map(str::to_string),
        }
    }

    #[test]
    fn nothing_set_means_no_clipboard() {
        assert_eq!(choose_backend(&env(None, None, None)), BackendKind::None);
    }

    #[test]
    fn wayland_wins_over_x11() {
        assert_eq!(
            choose_backend(&env(None, Some("wayland-1"), Some(":0"))),
            BackendKind::Wayland
        );
    }

    #[test]
    fn x11_when_only_display_is_set() {
        assert_eq!(
            choose_backend(&env(None, None, Some(":0"))),
            BackendKind::X11
        );
    }

    #[test]
    fn override_beats_detection() {
        for (value, want) in [
            ("wayland", BackendKind::Wayland),
            ("x11", BackendKind::X11),
            ("wl-copy", BackendKind::WlCopy),
            ("none", BackendKind::None),
            ("WAYLAND", BackendKind::Wayland),
            (" x11 ", BackendKind::X11),
        ] {
            assert_eq!(
                choose_backend(&env(Some(value), Some("wayland-1"), Some(":0"))),
                want,
                "override {value:?}"
            );
        }
    }

    #[test]
    fn unknown_override_falls_through_to_detection() {
        assert_eq!(
            choose_backend(&env(Some("banana"), None, Some(":0"))),
            BackendKind::X11
        );
        assert_eq!(
            choose_backend(&env(Some("banana"), None, None)),
            BackendKind::None
        );
    }

    #[test]
    fn empty_values_are_treated_as_unset() {
        assert_eq!(
            choose_backend(&env(Some(""), Some(""), Some(""))),
            BackendKind::None
        );
    }

    #[test]
    fn names_round_trip() {
        for kind in [
            BackendKind::Wayland,
            BackendKind::X11,
            BackendKind::WlCopy,
            BackendKind::None,
        ] {
            assert_eq!(BackendKind::parse(kind.as_str()), Some(kind));
        }
    }
}
