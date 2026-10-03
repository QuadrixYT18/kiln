//! HTTP client with caching, offline mode, rate limiting and retries.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use tokio::sync::Semaphore;

use super::cache::Cache;
use crate::util::term;

pub const USER_AGENT: &str = concat!("kiln/", env!("CARGO_PKG_VERSION"), " (+https://github.com/QuadrixYT18/kiln)");

#[derive(Debug, Clone)]
pub struct HttpOptions {
    pub offline: bool,
    pub no_cache: bool,
    pub ttl_secs: u64,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self { offline: false, no_cache: false, ttl_secs: 3600 }
    }
}

pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        self.status == 200
    }
}

pub struct Http {
    client: reqwest::Client,
    cache: Cache,
    opts: HttpOptions,
    permits: Semaphore,
    last_hit: Mutex<HashMap<String, Instant>>,
}

/// Minimum spacing between two requests to the same (rate-limited) host.
fn min_interval(host: &str) -> Duration {
    match host {
        "search.maven.org" | "central.sonatype.com" => Duration::from_millis(300),
        "api.github.com" => Duration::from_millis(250),
        _ => Duration::ZERO,
    }
}

/// `host` normally, the full URL with `--verbose`.
fn where_(url: &str) -> String {
    if term::verbose() { url.to_string() } else { host_of(url).to_string() }
}

fn net_err(url: &str, e: &reqwest::Error) -> anyhow::Error {
    if term::verbose() {
        return anyhow!("request to {url} failed: {e:?}");
    }
    let why = if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "could not connect"
    } else {
        "request failed"
    };
    anyhow!("{}: {why} (use --verbose for details)", host_of(url))
}

fn host_of(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("")
}

impl Http {
    pub fn new(cache_dir: std::path::PathBuf, opts: HttpOptions) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client,
            cache: Cache::new(cache_dir),
            opts,
            permits: Semaphore::new(8),
            last_hit: Mutex::new(HashMap::new()),
        })
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    async fn throttle(&self, host: &str) {
        let gap = min_interval(host);
        if gap.is_zero() {
            return;
        }
        let wait = {
            let mut map = self.last_hit.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let next = map.get(host).map(|t| *t + gap).filter(|t| *t > now).unwrap_or(now);
            map.insert(host.to_string(), next);
            next.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// POST a JSON body (never cached). Used only for the search fallback.
    pub async fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<Response> {
        if self.opts.offline {
            bail!("offline mode: search needs network access");
        }
        let host = host_of(url).to_string();
        let _permit = self.permits.acquire().await.map_err(|e| anyhow!(e))?;
        self.throttle(&host).await;
        let resp = self.client.post(url).json(body).send().await.map_err(|e| net_err(url, &e))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| anyhow!("reading response from {}: {e}", where_(url)))?;
        Ok(Response { status, body: text })
    }

    /// GET `url`; 200 and 404 are returned (and cached), other statuses are errors.
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.get_with(url, &[]).await
    }

    pub async fn get_with(&self, url: &str, headers: &[(&str, String)]) -> Result<Response> {
        if !self.opts.no_cache
            && let Some(hit) = self.cache.get(url)
            && (self.opts.offline || hit.age_secs <= self.opts.ttl_secs)
        {
            return Ok(Response { status: hit.status, body: hit.body });
        }
        if self.opts.offline {
            if term::verbose() {
                bail!("offline mode: no cached response for {url}");
            }
            bail!("offline mode: no cached data for {} (run without --offline once to fill the cache)", host_of(url));
        }

        let host = host_of(url).to_string();
        let _permit = self.permits.acquire().await.map_err(|e| anyhow!(e))?;
        let mut last_err = None;
        for attempt in 0..4u32 {
            self.throttle(&host).await;
            let mut req = self.client.get(url);
            for (k, v) in headers {
                req = req.header(*k, v);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 429 || (500..600).contains(&status) {
                        let wait = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .map(|s| Duration::from_secs(s.min(30)))
                            .unwrap_or_else(|| Duration::from_millis(500 * 2u64.pow(attempt)));
                        last_err = Some(anyhow!("{} returned HTTP {status}", where_(url)));
                        tokio::time::sleep(wait).await;
                        continue;
                    }
                    if status != 200 && status != 404 {
                        bail!("{} returned HTTP {status}", where_(url));
                    }
                    let body = resp.text().await.map_err(|e| anyhow!("reading {url}: {e}"))?;
                    if !self.opts.no_cache {
                        let _ = self.cache.put(url, status, &body);
                    }
                    return Ok(Response { status, body });
                }
                Err(e) => {
                    last_err = Some(net_err(url, &e));
                    tokio::time::sleep(Duration::from_millis(300 * 2u64.pow(attempt))).await;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("request to {url} failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_extraction() {
        assert_eq!(host_of("https://search.maven.org/solrsearch/select?q=a"), "search.maven.org");
        assert_eq!(host_of("http://127.0.0.1:8080/x"), "127.0.0.1:8080");
    }

    #[test]
    fn user_agent_contains_project_url() {
        assert!(USER_AGENT.contains("github.com/QuadrixYT18/kiln"));
    }
}
