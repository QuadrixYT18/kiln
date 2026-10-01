//! Optional user configuration (`config.toml` in the config directory).
//!
//! ```toml
//! cache_ttl_secs = 3600
//!
//! [aliases]
//! hikari = "com.zaxxer:HikariCP"
//!
//! [new]
//! group = "com.example"
//! java = 21
//! license = "MIT OR Apache-2.0"
//! author = "Jane Doe"
//! ```

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use super::paths;

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    pub cache_ttl_secs: Option<u64>,
    pub aliases: BTreeMap<String, String>,
    /// Extra Maven repositories consulted for every lookup.
    pub repositories: Vec<String>,
    pub new: NewDefaults,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct NewDefaults {
    pub group: Option<String>,
    pub java: Option<u32>,
    pub license: Option<String>,
    pub author: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = paths::config_file()?;
        match std::fs::read_to_string(&path) {
            Ok(s) => {
                toml_edit::de::from_str(&s).with_context(|| format!("invalid configuration in {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let cfg: Config = toml_edit::de::from_str(
            "cache_ttl_secs = 10\n[aliases]\nfoo = \"a:b\"\n[new]\ngroup = \"x.y\"\njava = 17\n",
        )
        .unwrap();
        assert_eq!(cfg.cache_ttl_secs, Some(10));
        assert_eq!(cfg.aliases["foo"], "a:b");
        assert_eq!(cfg.new.java, Some(17));
    }
}
