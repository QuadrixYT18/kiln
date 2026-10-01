//! Looks up newer versions for scanned dependencies (parallel, cached).

use std::collections::BTreeMap;

use futures::StreamExt;

use crate::project::model::{DepKind, Dependency, VersionSite};
use crate::registry::version::{Candidates, Stability, candidates};
use crate::registry::{Coord, Registry, Repo};
use crate::util::term;

#[derive(Debug, Clone)]
pub struct Checked {
    pub dep: Dependency,
    pub candidates: Candidates,
    /// Lookup failed (offline without cache, unknown artifact, …).
    pub error: Option<String>,
}

impl Checked {
    /// Declarations we can read and rewrite.
    pub fn is_checkable(dep: &Dependency) -> bool {
        dep.current.is_some()
            && matches!(
                dep.site,
                VersionSite::Text { .. } | VersionSite::CatalogVersion { .. } | VersionSite::CatalogInline { .. }
            )
    }
}

/// Repositories to consult for a dependency kind: Maven Central (or the plugin
/// portal first for Gradle plugins), then the project's own repositories.
pub fn repos_for(reg: &Registry, kind: DepKind, project_repos: &[Repo], extra: &[Repo]) -> Vec<Repo> {
    let mut v = Vec::new();
    if kind == DepKind::Plugin {
        v.push(reg.plugin_portal());
    }
    v.push(reg.central());
    v.extend(project_repos.iter().cloned());
    v.extend(extra.iter().cloned());
    let mut seen = Vec::new();
    v.retain(|r| {
        let dup = seen.contains(r);
        seen.push(r.clone());
        !dup
    });
    v
}

type LookupResult = ((Coord, DepKind), Result<Vec<String>, String>);

pub async fn check_all(
    reg: &Registry,
    deps: Vec<Dependency>,
    project_repos: &[Repo],
    stability: Stability,
    show_progress: bool,
) -> Vec<Checked> {
    let mut keys: Vec<(Coord, DepKind)> = Vec::new();
    for d in deps.iter().filter(|d| Checked::is_checkable(d)) {
        let k = (d.coord.clone(), if d.kind == DepKind::Plugin { DepKind::Plugin } else { DepKind::Library });
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let pb = if show_progress {
        term::progress(keys.len() as u64, "Checking versions")
    } else {
        indicatif::ProgressBar::hidden()
    };
    let results: Vec<LookupResult> = futures::stream::iter(keys)
        .map(|(coord, kind)| {
            let repos = repos_for(reg, kind, project_repos, &[]);
            let pb = pb.clone();
            async move {
                let r = reg.versions(&coord, &repos).await.map_err(|e| e.to_string());
                pb.inc(1);
                ((coord, kind), r)
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    pb.finish_and_clear();
    let lookup: BTreeMap<(Coord, DepKind), Result<Vec<String>, String>> = results.into_iter().collect();

    deps.into_iter()
        .map(|dep| {
            if !Checked::is_checkable(&dep) {
                return Checked { dep, candidates: Candidates::default(), error: None };
            }
            let key = (dep.coord.clone(), if dep.kind == DepKind::Plugin { DepKind::Plugin } else { DepKind::Library });
            match lookup.get(&key) {
                Some(Ok(avail)) => {
                    let c = candidates(dep.current.as_deref().unwrap_or(""), avail, stability);
                    Checked { dep, candidates: c, error: None }
                }
                Some(Err(e)) => Checked { dep, candidates: Candidates::default(), error: Some(e.clone()) },
                None => Checked { dep, candidates: Candidates::default(), error: None },
            }
        })
        .collect()
}
