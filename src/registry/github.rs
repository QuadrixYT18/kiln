//! Release notes for major updates, discovered via the POM's SCM metadata and
//! the GitHub Releases API.

use anyhow::Result;

use super::{Coord, Registry, Repo};
use crate::project::xml::{Ev, events, unescape};

#[derive(Debug, Clone)]
pub struct ReleaseNotes {
    pub title: String,
    pub url: String,
    pub summary: Vec<String>,
}

/// Extracts `(owner, repo)` from any GitHub URL flavour found in POM metadata.
pub fn parse_github_url(s: &str) -> Option<(String, String)> {
    let idx = s.find("github.com")?;
    let rest = &s[idx + "github.com".len()..];
    let rest = rest.trim_start_matches([':', '/']);
    let mut parts = rest.split(['/', '?', '#']);
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim().trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

pub fn github_repo_from_pom(pom: &str) -> Option<(String, String)> {
    let evs = events(pom).ok()?;
    let mut path: Vec<&str> = Vec::new();
    let mut candidates: Vec<(u8, String)> = Vec::new();
    for ev in &evs {
        match ev {
            Ev::Start { name, .. } => path.push(name),
            Ev::End { .. } => {
                path.pop();
            }
            Ev::Text { start, end } => {
                let rank = match path.as_slice() {
                    ["project", "scm", "url"] => 0,
                    ["project", "scm", "connection"] => 1,
                    ["project", "scm", "developerConnection"] => 2,
                    ["project", "url"] => 3,
                    _ => continue,
                };
                candidates.push((rank, unescape(&pom[*start..*end])));
            }
            Ev::Empty { .. } => {}
        }
    }
    candidates.sort_by_key(|(r, _)| *r);
    candidates.iter().find_map(|(_, u)| parse_github_url(u))
}

/// Condenses a Markdown release body into a few readable lines.
pub fn summarize(body: &str, max_lines: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let l = line.trim().trim_start_matches('#').trim();
        if l.is_empty() || l.starts_with("<!--") || l.starts_with("---") || l.starts_with("```") {
            continue;
        }
        let mut l = l.replace("**", "").replace('`', "");
        if l.chars().count() > 110 {
            l = l.chars().take(107).collect::<String>() + "...";
        }
        out.push(l);
        if out.len() >= max_lines {
            break;
        }
    }
    out
}

impl Registry {
    pub async fn release_notes(&self, coord: &Coord, version: &str, repos: &[Repo]) -> Option<ReleaseNotes> {
        self.release_notes_inner(coord, version, repos).await.ok().flatten()
    }

    async fn release_notes_inner(&self, coord: &Coord, version: &str, repos: &[Repo]) -> Result<Option<ReleaseNotes>> {
        let mut pom = None;
        for repo in repos {
            let url = format!("{}/{}/{version}/{}-{version}.pom", repo.0, coord.path(), coord.artifact);
            if let Ok(r) = self.http.get(&url).await
                && r.ok()
            {
                pom = Some(r.body);
                break;
            }
        }
        let Some(pom) = pom else { return Ok(None) };
        let Some((owner, repo)) = github_repo_from_pom(&pom) else { return Ok(None) };

        let mut headers = vec![("Accept", "application/vnd.github+json".to_string())];
        if let Ok(token) = std::env::var("GITHUB_TOKEN")
            && !token.is_empty()
        {
            headers.push(("Authorization", format!("Bearer {token}")));
        }
        let tags = [
            format!("v{version}"),
            version.to_string(),
            format!("{}-{version}", coord.artifact),
            format!("release-{version}"),
            format!("r{version}"),
        ];
        for tag in tags {
            let url = format!("{}/repos/{owner}/{repo}/releases/tags/{tag}", self.endpoints.github_api);
            let Ok(r) = self.http.get_with(&url, &headers).await else { continue };
            if !r.ok() {
                continue;
            }
            #[derive(serde::Deserialize)]
            struct Rel {
                name: Option<String>,
                tag_name: String,
                body: Option<String>,
                html_url: String,
            }
            let Ok(rel) = serde_json::from_str::<Rel>(&r.body) else { continue };
            let summary = summarize(rel.body.as_deref().unwrap_or(""), 8);
            return Ok(Some(ReleaseNotes {
                title: rel.name.filter(|n| !n.is_empty()).unwrap_or(rel.tag_name),
                url: rel.html_url,
                summary,
            }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_urls() {
        for u in [
            "https://github.com/brettwooldridge/HikariCP",
            "scm:git:git://github.com/brettwooldridge/HikariCP.git",
            "git@github.com:brettwooldridge/HikariCP.git",
            "https://github.com/brettwooldridge/HikariCP/tree/main",
        ] {
            assert_eq!(parse_github_url(u), Some(("brettwooldridge".into(), "HikariCP".into())), "{u}");
        }
        assert_eq!(parse_github_url("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn finds_repo_in_pom() {
        let pom = "<project><url>https://example.org</url><scm><connection>scm:git:https://github.com/a/b.git</connection></scm></project>";
        assert_eq!(github_repo_from_pom(pom), Some(("a".into(), "b".into())));
    }

    #[test]
    fn summarizes_markdown() {
        let s = summarize("## Highlights\n\n- **Faster** startup\n\n```\ncode\n```\n- more", 2);
        assert_eq!(s, vec!["Highlights", "- Faster startup"]);
    }
}
