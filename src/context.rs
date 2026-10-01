//! Process-wide context shared by commands.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::cli::Cli;
use crate::registry::http::{Http, HttpOptions};
use crate::registry::{Endpoints, Registry};
use crate::util::config::Config;
use crate::util::paths;

pub struct Ctx {
    pub cwd: PathBuf,
    pub config: Config,
    pub offline: bool,
    pub no_cache: bool,
}

impl Ctx {
    pub fn new(cli: &Cli) -> Result<Self> {
        let cwd = match &cli.path {
            Some(p) => p.clone(),
            None => std::env::current_dir().context("could not determine the current directory")?,
        };
        Ok(Self { cwd, config: Config::load()?, offline: cli.offline, no_cache: cli.no_cache })
    }

    pub fn http(&self) -> Result<Arc<Http>> {
        let ttl = std::env::var("KILN_CACHE_TTL")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(self.config.cache_ttl_secs)
            .unwrap_or(3600);
        let opts = HttpOptions { offline: self.offline, no_cache: self.no_cache, ttl_secs: ttl };
        Ok(Arc::new(Http::new(paths::cache_dir()?, opts)?))
    }

    pub fn registry(&self) -> Result<Registry> {
        Ok(Registry::new(self.http()?, Endpoints::from_env()))
    }
}
