//! Talking to Maven repositories, Maven Central search, the Gradle Plugin
//! Portal and GitHub (release notes). Everything goes through [`http::Http`] so
//! caching, offline mode and rate limiting apply uniformly.

pub mod aliases;
pub mod cache;
pub mod github;
pub mod http;
pub mod metadata;
pub mod version;

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use self::http::Http;
use self::version::{Stability, Version};
use crate::util::term;

pub const DEFAULT_CENTRAL: &str = "https://repo.maven.apache.org/maven2";
pub const DEFAULT_PLUGIN_PORTAL: &str = "https://plugins.gradle.org/m2";
pub const DEFAULT_SEARCH: &str = "https://search.maven.org/solrsearch/select";
/// Fallback used by the Maven Central website (best effort, undocumented).
pub const DEFAULT_SEARCH_FALLBACK: &str = "https://central.sonatype.com/api/internal/browse/components";
pub const DEFAULT_GITHUB_API: &str = "https://api.github.com";
pub const DEFAULT_GRADLE_API: &str = "https://services.gradle.org";

/// Built-in repository URLs may be redirected (mirrors, tests): currently
/// `KILN_PAPERMC_URL` replaces https://repo.papermc.io/repository/maven-public/.
pub fn repo_override(url: &str) -> String {
    if url.contains("repo.papermc.io")
        && let Ok(v) = std::env::var("KILN_PAPERMC_URL")
        && !v.is_empty()
    {
        return v;
    }
    url.to_string()
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

/// Service endpoints; every one can be redirected with an environment variable
/// (used by the test-suite and by corporate mirrors).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub central: String,
    pub plugin_portal: String,
    pub search: String,
    pub search_fallback: String,
    pub github_api: String,
    pub gradle_api: String,
}

impl Endpoints {
    pub fn from_env() -> Self {
        Self {
            central: env_or("KILN_CENTRAL_URL", DEFAULT_CENTRAL),
            plugin_portal: env_or("KILN_PLUGIN_PORTAL_URL", DEFAULT_PLUGIN_PORTAL),
            search: env_or("KILN_SEARCH_URL", DEFAULT_SEARCH),
            search_fallback: env_or("KILN_SEARCH_FALLBACK_URL", DEFAULT_SEARCH_FALLBACK),
            github_api: env_or("KILN_GITHUB_API_URL", DEFAULT_GITHUB_API),
            gradle_api: env_or("KILN_GRADLE_API_URL", DEFAULT_GRADLE_API),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Coord {
    pub group: String,
    pub artifact: String,
}

impl Coord {
    pub fn new(group: impl Into<String>, artifact: impl Into<String>) -> Self {
        Self { group: group.into(), artifact: artifact.into() }
    }

    /// `group:artifact` or `group:artifact:version`; returns the optional version too.
    pub fn parse(s: &str) -> Option<(Coord, Option<String>)> {
        let mut it = s.split(':');
        let g = it.next()?.trim();
        let a = it.next()?.trim();
        let v = it.next().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if g.is_empty() || a.is_empty() || it.next().is_some() {
            return None;
        }
        // Reject things like `http://…`.
        if g.contains('/') || a.contains('/') {
            return None;
        }
        Some((Coord::new(g, a), v))
    }

    pub fn path(&self) -> String {
        format!("{}/{}", self.group.replace('.', "/"), self.artifact)
    }

    /// Gradle plugin marker artifact for a plugin id.
    pub fn plugin_marker(plugin_id: &str) -> Coord {
        Coord::new(plugin_id, format!("{plugin_id}.gradle.plugin"))
    }
}

impl fmt::Display for Coord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.group, self.artifact)
    }
}

/// A Maven repository base URL (no trailing slash).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Repo(pub String);

impl Repo {
    pub fn new(url: &str) -> Self {
        Repo(url.trim().trim_end_matches('/').to_string())
    }
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub coord: Coord,
    pub latest: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub version_count: u64,
    pub score: f64,
}

pub struct Registry {
    pub http: Arc<Http>,
    pub endpoints: Endpoints,
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Registry {
    pub fn new(http: Arc<Http>, endpoints: Endpoints) -> Self {
        Self { http, endpoints }
    }

    pub fn central(&self) -> Repo {
        Repo::new(&self.endpoints.central)
    }

    pub fn plugin_portal(&self) -> Repo {
        Repo::new(&self.endpoints.plugin_portal)
    }

    /// Fetches the merged list of versions of `coord` across `repos`.
    ///
    /// Fails if no repository knows the artifact. Repositories that merely
    /// don't have it (404) are skipped; real network errors are only reported
    /// when no repository could answer.
    pub async fn versions(&self, coord: &Coord, repos: &[Repo]) -> Result<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut versions = Vec::new();
        let mut errors = Vec::new();
        let mut found = false;
        let mut tried: Vec<&Repo> = Vec::new();
        for repo in repos {
            if tried.contains(&repo) {
                continue;
            }
            tried.push(repo);
            let url = format!("{}/{}/maven-metadata.xml", repo.0, coord.path());
            match self.http.get(&url).await {
                Ok(r) if r.ok() => match metadata::parse(&r.body) {
                    Ok(md) => {
                        found = true;
                        let mut all = md.versions;
                        all.extend(md.release);
                        all.extend(md.latest);
                        for v in all {
                            if seen.insert(v.clone()) {
                                versions.push(v);
                            }
                        }
                    }
                    Err(e) => errors.push(format!("{url}: {e}")),
                },
                Ok(_) => {}
                Err(e) => errors.push(e.to_string()),
            }
        }
        if found {
            return Ok(versions);
        }
        if let Some(e) = errors.first() {
            bail!("could not look up {coord}: {e}");
        }
        bail!(
            "{coord} was not found in any repository ({})",
            repos.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", ")
        )
    }

    /// Latest version of `coord` honoring `stability`.
    pub async fn latest(&self, coord: &Coord, repos: &[Repo], stability: Stability) -> Result<String> {
        let versions = self.versions(coord, repos).await?;
        version::latest(&versions, stability).ok_or_else(|| {
            anyhow!(
                "{coord} has no {} version (try --pre)",
                if stability == Stability::Stable { "stable" } else { "matching" }
            )
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        #[derive(serde::Deserialize)]
        struct Resp {
            response: Inner,
        }
        #[derive(serde::Deserialize)]
        struct Inner {
            docs: Vec<Doc>,
        }
        #[derive(serde::Deserialize)]
        struct Doc {
            g: String,
            a: String,
            #[serde(rename = "latestVersion")]
            latest_version: Option<String>,
            timestamp: Option<u64>,
            #[serde(rename = "versionCount")]
            version_count: Option<u64>,
        }

        let mut hits: Vec<SearchHit> = Vec::new();
        let queries = [format!("a:\"{query}\""), query.to_string()];
        let mut last_err = None;
        let mut any_ok = false;
        for q in queries {
            let url = format!("{}?q={}&rows=20&wt=json", self.endpoints.search, percent_encode(&q));
            match self.http.get(&url).await {
                Ok(r) if r.ok() => {
                    let parsed: Resp = match serde_json::from_str(&r.body) {
                        Ok(p) => p,
                        Err(e) => {
                            last_err = Some(anyhow!("unexpected search response: {e}"));
                            continue;
                        }
                    };
                    any_ok = true;
                    for d in parsed.response.docs {
                        let coord = Coord::new(d.g, d.a);
                        if hits.iter().any(|h| h.coord == coord) {
                            continue;
                        }
                        hits.push(SearchHit {
                            score: 0.0,
                            coord,
                            latest: d.latest_version,
                            timestamp_ms: d.timestamp,
                            version_count: d.version_count.unwrap_or(0),
                        });
                    }
                }
                Ok(r) => last_err = Some(anyhow!("search returned HTTP {}", r.status)),
                Err(e) => last_err = Some(e),
            }
        }
        if !any_ok {
            // The primary API is down or changed: try the Central website's own search.
            match self.search_fallback(query).await {
                Ok(h) if !h.is_empty() => hits = h,
                Ok(_) => {}
                Err(fallback_err) => {
                    if let Some(e) = last_err {
                        return Err(if term::verbose() {
                            anyhow!("{e}; fallback search failed: {fallback_err}")
                        } else {
                            e
                        });
                    }
                }
            }
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        for h in &mut hits {
            h.score = score_hit(query, h, now_ms);
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(hits)
    }

    async fn search_fallback(&self, query: &str) -> Result<Vec<SearchHit>> {
        let body = serde_json::json!({ "size": 20, "searchTerm": query });
        let r = self.http.post_json(&self.endpoints.search_fallback, &body).await?;
        if r.status != 200 {
            bail!("search fallback returned HTTP {}", r.status);
        }
        let v: serde_json::Value =
            serde_json::from_str(&r.body).map_err(|e| anyhow!("unexpected fallback response: {e}"))?;
        let comps = v.get("components").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        Ok(comps
            .iter()
            .filter_map(|c| {
                let group = c.get("namespace")?.as_str()?;
                let artifact = c.get("name")?.as_str()?;
                let info = c.get("latestVersionInfo");
                Some(SearchHit {
                    coord: Coord::new(group, artifact),
                    latest: info.and_then(|i| i.get("version")).and_then(|x| x.as_str()).map(String::from),
                    timestamp_ms: info.and_then(|i| i.get("timestampUnixWithMS")).and_then(|x| x.as_u64()),
                    version_count: 0,
                    score: 0.0,
                })
            })
            .collect())
    }

    /// Current Gradle release (version + optional wrapper checksum).
    pub async fn gradle_current(&self) -> Result<GradleRelease> {
        #[derive(serde::Deserialize)]
        struct Cur {
            version: String,
            #[serde(rename = "wrapperChecksumUrl")]
            wrapper_checksum_url: Option<String>,
            #[serde(rename = "checksumUrl")]
            checksum_url: Option<String>,
        }
        let url = format!("{}/versions/current", self.endpoints.gradle_api);
        let r = self.http.get(&url).await?;
        if !r.ok() {
            bail!("could not determine the current Gradle version ({url} returned {})", r.status);
        }
        let cur: Cur = serde_json::from_str(&r.body).map_err(|e| anyhow!("unexpected response from {url}: {e}"))?;
        let _ = cur.wrapper_checksum_url;
        let sha256 = match cur.checksum_url {
            Some(u) => self.http.get(&u).await.ok().filter(|r| r.ok()).map(|r| r.body.trim().to_string()),
            None => None,
        };
        Ok(GradleRelease { version: cur.version, sha256 })
    }
}

#[derive(Debug, Clone)]
pub struct GradleRelease {
    pub version: String,
    /// SHA-256 of the `-bin` distribution.
    pub sha256: Option<String>,
}

fn score_hit(query: &str, hit: &SearchHit, now_ms: u64) -> f64 {
    let q = query.to_ascii_lowercase();
    let a = hit.coord.artifact.to_ascii_lowercase();
    let g = hit.coord.group.to_ascii_lowercase();
    let mut score = 0.0;
    if a == q {
        score += 100.0;
    } else if a.starts_with(&q) {
        score += 60.0;
    } else if a.contains(&q) {
        score += 40.0;
    }
    if g.contains(&q) {
        score += 15.0;
    }
    score += ((hit.version_count as f64) + 1.0).ln() * 3.0;
    if let Some(ts) = hit.timestamp_ms {
        let age_days = now_ms.saturating_sub(ts) / 86_400_000;
        score += match age_days {
            0..=365 => 12.0,
            366..=730 => 8.0,
            731..=1460 => 3.0,
            _ => 0.0,
        };
    }
    if let Some(latest) = &hit.latest
        && !Version::new(latest).is_stable()
    {
        score -= 2.0;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_coords() {
        assert_eq!(Coord::parse("a.b:c").unwrap(), (Coord::new("a.b", "c"), None));
        assert_eq!(Coord::parse("a.b:c:1.0").unwrap().1.as_deref(), Some("1.0"));
        assert!(Coord::parse("hikari").is_none());
        assert!(Coord::parse("https://x").is_none());
        assert!(Coord::parse("a:b:c:d").is_none());
    }

    #[test]
    fn coord_path_and_marker() {
        assert_eq!(Coord::new("com.zaxxer", "HikariCP").path(), "com/zaxxer/HikariCP");
        let m = Coord::plugin_marker("org.jetbrains.kotlin.jvm");
        assert_eq!(m.artifact, "org.jetbrains.kotlin.jvm.gradle.plugin");
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode("a:\"b c\""), "a%3A%22b%20c%22");
    }

    #[test]
    fn exact_artifact_match_ranks_first() {
        let mk = |g: &str, a: &str, n| SearchHit {
            coord: Coord::new(g, a),
            latest: Some("1.0".into()),
            timestamp_ms: None,
            version_count: n,
            score: 0.0,
        };
        let exact = mk("org.x", "jedis", 5);
        let loose = mk("org.y", "jedis-extras-big", 500);
        assert!(score_hit("jedis", &exact, 0) > score_hit("jedis", &loose, 0));
    }
}
