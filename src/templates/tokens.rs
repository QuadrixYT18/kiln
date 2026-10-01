//! Live version lookups embedded in templates.
//!
//! Syntax (inside `{{ … }}`):
//!
//! ```text
//! latest:group:artifact            newest stable version
//! latest:group:artifact?pre        newest version including pre-releases
//! latest:group:artifact?snapshot   newest stable or -SNAPSHOT version
//! plugin:plugin.id                 newest Gradle plugin version (Plugin Portal)
//! …|before:-R                      cut the result at the first `-R`
//! …|major_minor                    keep only `major.minor`
//! ```

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;

use crate::registry::version::Stability;
use crate::registry::{Coord, Registry, Repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    Before(String),
    MajorMinor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub body: String,
    pub coord: Coord,
    pub plugin: bool,
    pub stability: Stability,
    pub filters: Vec<Filter>,
}

pub fn parse(body: &str) -> Result<Token> {
    let mut parts = body.split('|');
    let head = parts.next().unwrap_or("");
    let (head, opt) = match head.split_once('?') {
        Some((h, o)) => (h, Some(o)),
        None => (head, None),
    };
    let stability = match opt {
        None => Stability::Stable,
        Some("pre") => Stability::Any,
        Some("snapshot") => Stability::AllowSnapshot,
        Some(other) => bail!("unknown option `?{other}` in `{{{{{body}}}}}`"),
    };
    let (plugin, rest) = if let Some(r) = head.strip_prefix("plugin:") {
        (true, r)
    } else if let Some(r) = head.strip_prefix("latest:") {
        (false, r)
    } else {
        bail!("not a version lookup: {body}")
    };
    let coord = if plugin {
        Coord::plugin_marker(rest)
    } else {
        Coord::parse(rest).map(|(c, _)| c).ok_or_else(|| anyhow!("expected group:artifact in `{{{{{body}}}}}`"))?
    };
    let mut filters = Vec::new();
    for f in parts {
        match f.split_once(':') {
            Some(("before", s)) => filters.push(Filter::Before(s.to_string())),
            None if f == "major_minor" => filters.push(Filter::MajorMinor),
            _ => bail!("unknown filter `{f}` in `{{{{{body}}}}}`"),
        }
    }
    Ok(Token { body: body.to_string(), coord, plugin, stability, filters })
}

pub fn apply_filters(mut v: String, filters: &[Filter]) -> String {
    for f in filters {
        match f {
            Filter::Before(s) => {
                if let Some(i) = v.find(s.as_str()) {
                    v.truncate(i);
                }
            }
            Filter::MajorMinor => {
                let parts: Vec<&str> = v.split('.').take(2).collect();
                v = parts.join(".");
            }
        }
    }
    v
}

/// Resolves all tokens concurrently. `repos` are consulted after Maven Central
/// (and the Plugin Portal for plugin tokens).
pub async fn resolve(reg: &Registry, bodies: &[String], repos: &[Repo]) -> Result<BTreeMap<String, String>> {
    let tokens: Vec<Token> = bodies.iter().map(|b| parse(b)).collect::<Result<_>>()?;
    let results: Vec<(String, Result<String>)> = futures::stream::iter(tokens)
        .map(|t| async move {
            let mut rs = Vec::new();
            if t.plugin {
                rs.push(reg.plugin_portal());
            }
            rs.push(reg.central());
            rs.extend(repos.iter().cloned());
            let r =
                reg.latest(&t.coord, &rs, t.stability).await.map(|v| apply_filters(v, &t.filters)).with_context(|| {
                    format!(
                        "while resolving the latest version of {}",
                        if t.plugin { format!("plugin {}", t.coord.group) } else { t.coord.to_string() }
                    )
                });
            (t.body, r)
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut map = BTreeMap::new();
    for (body, r) in results {
        map.insert(body, r?);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tokens() {
        let t = parse("latest:io.papermc.paper:paper-api?snapshot|before:-R0|major_minor").unwrap();
        assert_eq!(t.coord, Coord::new("io.papermc.paper", "paper-api"));
        assert_eq!(t.stability, Stability::AllowSnapshot);
        assert_eq!(t.filters, vec![Filter::Before("-R0".into()), Filter::MajorMinor]);
        let p = parse("plugin:org.jetbrains.kotlin.jvm").unwrap();
        assert!(p.plugin);
        assert_eq!(p.coord.artifact, "org.jetbrains.kotlin.jvm.gradle.plugin");
        assert!(parse("latest:foo").is_err());
        assert!(parse("latest:a:b?weird").is_err());
        assert!(parse("latest:a:b|nope").is_err());
    }

    #[test]
    fn filters() {
        assert_eq!(apply_filters("1.21.4-R0.1-SNAPSHOT".into(), &[Filter::Before("-R0".into())]), "1.21.4");
        assert_eq!(apply_filters("1.21.4".into(), &[Filter::MajorMinor]), "1.21");
    }
}
