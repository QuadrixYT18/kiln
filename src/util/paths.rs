//! Platform-correct directories. Never hardcode `/` or `~`.
//!
//! | OS      | config                                   | cache                          |
//! |---------|------------------------------------------|--------------------------------|
//! | macOS   | `~/Library/Application Support/kiln`     | `~/Library/Caches/kiln`        |
//! | Windows | `%APPDATA%\kiln`                         | `%LOCALAPPDATA%\kiln\cache`    |
//! | Linux   | `~/.config/kiln`                         | `~/.cache/kiln`                |
//!
//! `KILN_CONFIG_DIR` and `KILN_CACHE_DIR` override the locations (used by the
//! test-suite and handy for portable setups).
//!
//! Note on long paths: Rust's `std::fs` transparently switches to the `\\?\`
//! form on Windows for absolute paths beyond `MAX_PATH`, so plain `PathBuf`s
//! are sufficient here. Spaces and non-ASCII characters are safe because paths
//! are passed as `OsStr`, never through a shell.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("KILN_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    dirs::config_dir().map(|d| d.join("kiln")).ok_or_else(|| anyhow!("could not determine the configuration directory"))
}

pub fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("KILN_CACHE_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let base = dirs::cache_dir().ok_or_else(|| anyhow!("could not determine the cache directory"))?;
    if cfg!(windows) { Ok(base.join("kiln").join("cache")) } else { Ok(base.join("kiln")) }
}

pub fn templates_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("templates"))
}

pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}
