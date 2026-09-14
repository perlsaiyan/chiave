//! `~/.config/chiave/config.toml` (or `$XDG_CONFIG_HOME/chiave/config.toml`).
//!
//! Every key is optional; command-line flags and environment variables win.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Database to open when `--kdb` and `$CHIAVE_KDB` are absent.
    pub database: Option<PathBuf>,
    /// Key file that goes with it.
    pub keyfile: Option<PathBuf>,
    /// Seconds a copied secret stays on the clipboard; 0 never clears.
    pub clip_timeout: Option<u64>,
    /// Idle seconds before the vault re-locks; 0 disables the idle lock.
    pub timeout: Option<u64>,
    /// REPL history file; `/dev/null` disables it.
    pub histfile: Option<PathBuf>,
}

/// The directory holding chiave's configuration.
pub fn config_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("chiave"))
}

/// The configuration file path, whether or not it exists.
pub fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("config.toml"))
}

/// Read the configuration file. A missing file is not an error.
pub fn load() -> anyhow::Result<Config> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    load_from(&path)
}

/// Read one specific configuration file. A missing file is not an error.
pub fn load_from(path: &Path) -> anyhow::Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut cfg: Config =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    cfg.database = cfg.database.map(|p| expand_tilde(&p));
    cfg.keyfile = cfg.keyfile.map(|p| expand_tilde(&p));
    cfg.histfile = cfg.histfile.map(|p| expand_tilde(&p));
    Ok(cfg)
}

/// Expand a leading `~/` using `$HOME`.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_key() {
        let dir = std::env::temp_dir().join("chiave-config-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "database = \"/tmp/v.kdbx\"\nkeyfile = \"/tmp/v.key\"\nclip_timeout = 20\ntimeout = 300\nhistfile = \"/dev/null\"\n",
        )
        .expect("write config");
        let cfg = load_from(&path).expect("load");
        assert_eq!(cfg.database.as_deref(), Some(Path::new("/tmp/v.kdbx")));
        assert_eq!(cfg.keyfile.as_deref(), Some(Path::new("/tmp/v.key")));
        assert_eq!(cfg.clip_timeout, Some(20));
        assert_eq!(cfg.timeout, Some(300));
        assert_eq!(cfg.histfile.as_deref(), Some(Path::new("/dev/null")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_is_the_default() {
        let cfg = load_from(Path::new("/nonexistent/chiave/config.toml")).expect("load");
        assert!(cfg.database.is_none());
    }
}
