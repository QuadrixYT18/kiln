//! `maven-metadata.xml` parsing.

use anyhow::{Result, bail};

use crate::project::xml::{Ev, events, unescape};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub versions: Vec<String>,
    pub latest: Option<String>,
    pub release: Option<String>,
}

pub fn parse(xml: &str) -> Result<Metadata> {
    let evs = events(xml)?;
    let mut path: Vec<&str> = Vec::new();
    let mut md = Metadata::default();
    for ev in &evs {
        match ev {
            Ev::Start { name, .. } => path.push(name),
            Ev::End { .. } => {
                path.pop();
            }
            Ev::Text { start, end } => {
                let value = unescape(&xml[*start..*end]);
                if value.is_empty() {
                    continue;
                }
                match path.as_slice() {
                    ["metadata", "versioning", "versions", "version"] => md.versions.push(value),
                    ["metadata", "versioning", "latest"] => md.latest = Some(value),
                    ["metadata", "versioning", "release"] => md.release = Some(value),
                    _ => {}
                }
            }
            Ev::Empty { .. } => {}
        }
    }
    if md.versions.is_empty() && md.latest.is_none() && md.release.is_none() {
        bail!("maven-metadata.xml contains no versions");
    }
    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>com.zaxxer</groupId>
  <artifactId>HikariCP</artifactId>
  <versioning>
    <latest>6.0.0</latest>
    <release>6.0.0</release>
    <versions>
      <version>5.1.0</version>
      <version>6.0.0</version>
    </versions>
    <lastUpdated>20240101000000</lastUpdated>
  </versioning>
</metadata>"#;
        let md = parse(xml).unwrap();
        assert_eq!(md.versions, vec!["5.1.0", "6.0.0"]);
        assert_eq!(md.release.as_deref(), Some("6.0.0"));
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("<metadata></metadata>").is_err());
    }
}
