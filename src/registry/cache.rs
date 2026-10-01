//! Tiny on-disk HTTP response cache (one file per URL).
//!
//! File layout: a JSON header line followed by the raw body. Entries are
//! written atomically (temp file + rename) so parallel kiln processes never
//! observe half-written files.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Header {
    url: String,
    status: u16,
    fetched_at: u64,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: u16,
    pub body: String,
    pub age_secs: u64,
}

#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// FNV-1a: stable across Rust versions and platforms (unlike `DefaultHasher`).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir: dir.join("http") }
    }

    fn file_for(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{:016x}.cache", fnv1a(url)))
    }

    pub fn get(&self, url: &str) -> Option<CacheEntry> {
        let raw = std::fs::read_to_string(self.file_for(url)).ok()?;
        let (head, body) = raw.split_once('\n')?;
        let header: Header = serde_json::from_str(head).ok()?;
        if header.url != url {
            return None;
        }
        Some(CacheEntry {
            status: header.status,
            body: body.to_string(),
            age_secs: now().saturating_sub(header.fetched_at),
        })
    }

    pub fn put(&self, url: &str, status: u16, body: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("could not create cache directory {}", self.dir.display()))?;
        let header = serde_json::to_string(&Header { url: url.to_string(), status, fetched_at: now() })?;
        let target = self.file_for(url);
        let tmp = self.dir.join(format!("{:016x}.{}.tmp", fnv1a(url), std::process::id()));
        std::fs::write(&tmp, format!("{header}\n{body}"))?;
        std::fs::rename(&tmp, &target).or_else(|_| {
            // Windows cannot rename over an existing file that is open elsewhere.
            let _ = std::fs::remove_file(&target);
            std::fs::rename(&tmp, &target)
        })?;
        Ok(())
    }

    /// Removes all cached responses; returns how many files were deleted.
    pub fn clear(&self) -> Result<usize> {
        let mut n = 0;
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Ok(0) };
        for e in rd.flatten() {
            if std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::new(dir.path().to_path_buf());
        assert!(c.get("https://x/y").is_none());
        c.put("https://x/y", 200, "line1\nline2").unwrap();
        let e = c.get("https://x/y").unwrap();
        assert_eq!(e.status, 200);
        assert_eq!(e.body, "line1\nline2");
        c.put("https://x/y", 404, "").unwrap();
        assert_eq!(c.get("https://x/y").unwrap().status, 404);
        assert_eq!(c.clear().unwrap(), 1);
        assert!(c.get("https://x/y").is_none());
    }
}
