//! High-level edits: add / remove dependencies and apply version updates.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};

use super::catalog::{self, Table_, accessor};
use super::gradle::{self, GSource};
use super::maven::{self, PomEditor};
use super::model::{BuildKind, Module, Project, Scope, VersionSite};
use super::workspace::Workspace;
use crate::registry::{Coord, Repo};
use crate::util::text::{Edit, apply_edits};

const PROVIDER_METHODS: &[&str] = &["get", "asProvider", "map", "flatMap", "orElse", "zip", "getOrElse"];

/// Does the accessor chain (`hikari`, `hikari.get`, `db.pool`) refer to `alias`?
pub fn chain_uses_alias(chain: &str, alias: &str) -> bool {
    let acc = accessor(alias);
    if chain == acc {
        return true;
    }
    chain
        .strip_prefix(&format!("{acc}."))
        .is_some_and(|rest| PROVIDER_METHODS.contains(&rest.split('.').next().unwrap_or("")))
}

fn version_chain_uses(chain: &str, key: &str) -> bool {
    chain_uses_alias(chain.strip_prefix("versions.").unwrap_or("\u{0}"), key)
}

// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

/// Applies `(site, new_version)` pairs. Edits against the same file are
/// computed from a single scan and applied back-to-front.
pub fn apply_updates(ws: &mut Workspace, updates: &[(VersionSite, String)]) -> Result<()> {
    let mut by_file: BTreeMap<PathBuf, Vec<Edit>> = BTreeMap::new();
    let mut catalog_ops: Vec<(&VersionSite, &String)> = Vec::new();
    for (site, v) in updates {
        match site {
            VersionSite::Text { file, start, end } => {
                let edits = by_file.entry(file.clone()).or_default();
                if !edits.iter().any(|e| e.range == (*start..*end)) {
                    edits.push(Edit { range: *start..*end, replacement: v.clone() });
                }
            }
            VersionSite::CatalogVersion { .. } | VersionSite::CatalogInline { .. } => catalog_ops.push((site, v)),
            VersionSite::Managed | VersionSite::Unsupported(_) => {}
        }
    }
    for (file, edits) in by_file {
        let text = ws.text(&file)?;
        let new = apply_edits(&text, &edits)?;
        ws.set(&file, new)?;
    }
    for (site, v) in catalog_ops {
        match site {
            VersionSite::CatalogVersion { file, key } => {
                let text = ws.text(file)?;
                ws.set(file, catalog::set_version_key(&text, key, v)?)?;
            }
            VersionSite::CatalogInline { file, plugin, alias } => {
                let text = ws.text(file)?;
                let table = if *plugin { Table_::Plugins } else { Table_::Libraries };
                ws.set(file, catalog::set_inline_version(&text, table, alias, v)?)?;
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Add
// ---------------------------------------------------------------------------

pub struct AddRequest<'a> {
    pub coord: &'a Coord,
    pub version: Option<&'a str>,
    pub scope: Scope,
    pub module: &'a Module,
    /// Repository required to resolve the dependency (alias hint), if any.
    pub repo: Option<&'a Repo>,
    /// Repositories the project already knows about.
    pub known_repos: &'a [Repo],
}

pub enum AddOutcome {
    Added { notes: Vec<String> },
    AlreadyPresent { version: Option<String> },
}

fn host_id(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("repo").replace('.', "-")
}

pub fn add_dependency(ws: &mut Workspace, project: &Project, req: &AddRequest<'_>) -> Result<AddOutcome> {
    match project.kind {
        BuildKind::Gradle => add_gradle(ws, project, req),
        BuildKind::Maven => add_maven(ws, req),
    }
}

fn repo_known(req: &AddRequest<'_>, repo: &Repo) -> bool {
    req.known_repos.iter().any(|r| r == repo) || *repo == Repo::new(crate::registry::DEFAULT_CENTRAL)
}

fn add_gradle(ws: &mut Workspace, project: &Project, req: &AddRequest<'_>) -> Result<AddOutcome> {
    let file = &req.module.build_file;
    let kotlin = gradle::is_kotlin_script(file);
    let text = ws.text(file)?;
    let gs = gradle::scan(&text, kotlin);
    let mut notes = Vec::new();

    let cat = match &project.catalog {
        Some(p) => Some((p.clone(), catalog::parse(&ws.text(p)?)?)),
        None => None,
    };

    // Already declared in this module?
    for d in gs.deps.iter().filter(|d| !d.in_buildscript) {
        match &d.source {
            GSource::Gav { coord, version } if coord == req.coord => {
                return Ok(AddOutcome::AlreadyPresent { version: version.as_ref().map(|v| v.value.clone()) });
            }
            GSource::Catalog { chain } => {
                if let Some((_, c)) = &cat
                    && let Some(lib) = c.libraries.iter().find(|l| &l.coord == req.coord)
                    && chain_uses_alias(chain, &lib.alias)
                {
                    let version = match &lib.version {
                        catalog::CatVersion::Ref(k) => c.versions.get(k).cloned(),
                        catalog::CatVersion::Literal(v) | catalog::CatVersion::Rich(_, v) => Some(v.clone()),
                        catalog::CatVersion::None => None,
                    };
                    return Ok(AddOutcome::AlreadyPresent { version });
                }
            }
            _ => {}
        }
    }

    // Repository for alias hints (e.g. repo.papermc.io).
    if let Some(repo) = req.repo
        && !repo_known(req, repo)
    {
        let url = format!("{}/", repo.0);
        let settings_text = match &project.settings {
            Some(s) => Some((s.clone(), ws.text(s)?)),
            None => None,
        };
        let mut done = false;
        if let Some((sp, st)) = settings_text {
            let sk = gradle::is_kotlin_script(&sp);
            let sscan = gradle::scan(&st, sk);
            if !sscan.repo_blocks.is_empty()
                && let Some(new) = gradle::ensure_repository(&st, sk, &url)?
            {
                ws.set(&sp, new)?;
                notes.push(format!(
                    "added repository {url} to {}",
                    sp.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                ));
                done = true;
            }
        }
        if !done {
            let text = ws.text(file)?;
            if let Some(new) = gradle::ensure_repository(&text, kotlin, &url)? {
                ws.set(file, new)?;
                notes.push(format!("added repository {url}"));
            }
        }
    }

    let text = ws.text(file)?;
    let gs = gradle::scan(&text, kotlin);
    let kapt =
        gs.plugins.iter().any(|p| matches!(&p.source, gradle::GPluginSource::Id { id, .. } if id.contains("kapt")));
    let config =
        if req.scope == Scope::AnnotationProcessor && kotlin && kapt { "kapt" } else { req.scope.gradle_config() };

    let new_text = match cat {
        Some((cat_path, c)) => {
            let existing = c.libraries.iter().find(|l| &l.coord == req.coord).map(|l| l.alias.clone());
            let alias = match existing {
                Some(a) => a,
                None => {
                    let a = catalog::pick_alias(&c, req.coord);
                    let ctext = ws.text(&cat_path)?;
                    ws.set(&cat_path, catalog::add_library(&ctext, &a, req.coord, req.version)?)?;
                    notes.push(format!(
                        "added `{a}` to {}",
                        cat_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                    ));
                    a
                }
            };
            gradle::add_statement(&text, kotlin, config, Some(&accessor(&alias)), None)?
        }
        None => {
            let gav = match req.version {
                Some(v) => format!("{}:{}", req.coord, v),
                None => req.coord.to_string(),
            };
            gradle::add_statement(&text, kotlin, config, None, Some(&gav))?
        }
    };
    ws.set(file, new_text)?;
    Ok(AddOutcome::Added { notes })
}

fn add_maven(ws: &mut Workspace, req: &AddRequest<'_>) -> Result<AddOutcome> {
    let file = &req.module.build_file;
    let text = ws.text(file)?;
    let pom = maven::scan(&text)?;
    if let Some(d) = pom.dependencies.iter().find(|d| !d.managed && d.coord().as_ref() == Some(req.coord)) {
        return Ok(AddOutcome::AlreadyPresent { version: d.version.as_ref().map(|v| v.value.clone()) });
    }
    let mut notes = Vec::new();
    let mut ed = PomEditor::new(&text);
    if let Some(repo) = req.repo
        && !repo_known(req, repo)
        && ed.ensure_repository(&format!("{}/", repo.0), &host_id(&repo.0))?
    {
        notes.push(format!("added repository {}/", repo.0));
    }
    let prop = req.version.map(|v| {
        let base = format!("{}.version", req.coord.artifact.to_ascii_lowercase());
        match pom.properties.get(&base) {
            Some(t) if t.value != v => {
                format!("{}.{base}", req.coord.group.rsplit('.').next().unwrap_or("lib").to_ascii_lowercase())
            }
            _ => base,
        }
    });
    if req.scope == Scope::AnnotationProcessor {
        notes.push("Maven has no annotationProcessor scope; added with scope `provided` (annotation processors on the classpath are picked up automatically)".to_string());
    }
    ed.add_dependency(req.coord, req.version, req.scope, prop.as_deref())?;
    ws.set(file, ed.text)?;
    Ok(AddOutcome::Added { notes })
}

// ---------------------------------------------------------------------------
// Remove
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct RemoveOutcome {
    pub removed_from: Vec<String>,
    pub notes: Vec<String>,
}

/// Matches a user-supplied selector against declared coordinates.
pub struct Selector {
    pub coord: Option<Coord>,
    /// artifactId / catalog alias text for loose matching.
    pub loose: String,
}

impl Selector {
    pub fn new(input: &str, aliases: &BTreeMap<String, crate::registry::aliases::Alias>) -> Self {
        if let Some((c, _)) = Coord::parse(input) {
            return Self { coord: Some(c), loose: input.to_string() };
        }
        let key = input.to_ascii_lowercase();
        Self { coord: aliases.get(&key).map(|a| a.coord.clone()), loose: input.to_string() }
    }

    pub fn matches(&self, coord: &Coord) -> bool {
        if let Some(c) = &self.coord
            && c == coord
        {
            return true;
        }
        coord.artifact.eq_ignore_ascii_case(&self.loose)
    }

    pub fn matches_alias(&self, alias: &str) -> bool {
        accessor(alias).eq_ignore_ascii_case(&accessor(&self.loose))
    }
}

pub fn remove_dependency(
    ws: &mut Workspace,
    project: &Project,
    modules: &[&Module],
    selector: &Selector,
) -> Result<RemoveOutcome> {
    match project.kind {
        BuildKind::Gradle => remove_gradle(ws, project, modules, selector),
        BuildKind::Maven => remove_maven(ws, project, modules, selector),
    }
}

fn remove_gradle(ws: &mut Workspace, project: &Project, modules: &[&Module], sel: &Selector) -> Result<RemoveOutcome> {
    let mut out = RemoveOutcome::default();
    let cat = match &project.catalog {
        Some(p) => Some((p.clone(), catalog::parse(&ws.text(p)?)?)),
        None => None,
    };
    let mut removed_aliases: Vec<String> = Vec::new();
    for m in modules {
        let text = ws.text(&m.build_file)?;
        let gs = gradle::scan(&text, gradle::is_kotlin_script(&m.build_file));
        let mut ranges = Vec::new();
        for d in &gs.deps {
            let hit = match &d.source {
                GSource::Gav { coord, .. } => sel.matches(coord),
                GSource::Catalog { chain } => cat.as_ref().is_some_and(|(_, c)| {
                    c.libraries.iter().any(|l| {
                        chain_uses_alias(chain, &l.alias) && (sel.matches(&l.coord) || sel.matches_alias(&l.alias))
                    })
                }),
            };
            if hit {
                if let GSource::Catalog { chain } = &d.source
                    && let Some((_, c)) = &cat
                    && let Some(l) = c.libraries.iter().find(|l| chain_uses_alias(chain, &l.alias))
                {
                    removed_aliases.push(l.alias.clone());
                }
                ranges.push(d.stmt.clone());
            }
        }
        if !ranges.is_empty() {
            let new = gradle::remove_statements(&text, &ranges);
            ws.set(&m.build_file, new)?;
            out.removed_from.push(m.name.clone());
        }
    }

    // Catalog cleanup.
    if let Some((cat_path, c)) = &cat {
        // Entries that match but were never referenced from a build file are dead entries.
        let mut candidates: Vec<String> = removed_aliases.clone();
        if out.removed_from.is_empty() {
            candidates.extend(
                c.libraries
                    .iter()
                    .filter(|l| sel.matches(&l.coord) || sel.matches_alias(&l.alias))
                    .map(|l| l.alias.clone()),
            );
        }
        candidates.sort();
        candidates.dedup();
        // Gather usage across all build files of the project (current in-memory state).
        let mut chains: Vec<String> = Vec::new();
        let mut scripts: Vec<PathBuf> = project.modules.iter().map(|m| m.build_file.clone()).collect();
        scripts.extend(project.settings.clone());
        for s in scripts {
            let t = ws.text(&s)?;
            chains.extend(gradle::scan(&t, gradle::is_kotlin_script(&s)).catalog_refs);
        }
        for alias in candidates {
            let still_used = chains.iter().any(|ch| chain_uses_alias(ch, &alias));
            let in_bundles: Vec<&String> =
                c.bundles.iter().filter(|(_, v)| v.contains(&alias)).map(|(k, _)| k).collect();
            if still_used {
                out.notes.push(format!("catalog entry `{alias}` is still used elsewhere and was kept"));
                continue;
            }
            if !in_bundles.is_empty() {
                out.notes.push(format!(
                    "catalog entry `{alias}` is part of bundle(s) {} and was kept",
                    in_bundles.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ));
                continue;
            }
            let lib = c.libraries.iter().find(|l| l.alias == alias);
            let ctext = ws.text(cat_path)?;
            let mut new = catalog::remove_library(&ctext, &alias)?;
            out.notes.push(format!("removed `{alias}` from libs.versions.toml"));
            if let Some(catalog::CatLib { version: catalog::CatVersion::Ref(key), .. }) = lib {
                let c2 = catalog::parse(&new)?;
                let key_used_in_scripts = chains.iter().any(|ch| version_chain_uses(ch, key));
                if !catalog::version_key_in_use(&c2, key) && !key_used_in_scripts {
                    new = catalog::remove_version_key(&new, key)?;
                    out.notes.push(format!("removed unused version `{key}`"));
                }
            }
            ws.set(cat_path, new)?;
            if out.removed_from.is_empty() {
                out.removed_from.push("libs.versions.toml".to_string());
            }
        }
    }
    Ok(out)
}

fn remove_maven(ws: &mut Workspace, project: &Project, modules: &[&Module], sel: &Selector) -> Result<RemoveOutcome> {
    let mut out = RemoveOutcome::default();
    let mut props_to_check: Vec<(PathBuf, String)> = Vec::new();
    for m in modules {
        let text = ws.text(&m.build_file)?;
        let pom = maven::scan(&text)?;
        let hits: Vec<_> =
            pom.dependencies.iter().filter(|d| !d.managed && d.coord().is_some_and(|c| sel.matches(&c))).collect();
        if hits.is_empty() {
            continue;
        }
        let mut ed = PomEditor::new(&text);
        // Remove back-to-front so earlier ranges stay valid.
        let mut ranges: Vec<_> = hits.iter().map(|d| d.element.clone()).collect();
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        for h in &hits {
            if let Some(v) = &h.version
                && let Some((name, t)) = maven::resolve_property(&v.value, &pom.properties)
            {
                let _ = t;
                props_to_check.push((m.build_file.clone(), name));
            }
        }
        for r in ranges {
            ed.remove_element(r);
        }
        ws.set(&m.build_file, ed.text)?;
        out.removed_from.push(m.name.clone());
    }
    // Drop properties that nothing references any more.
    let all_texts: Vec<String> = {
        let mut v = Vec::new();
        for m in &project.modules {
            v.push(ws.text(&m.build_file)?);
        }
        v
    };
    for (file, name) in props_to_check {
        let needle = format!("${{{name}}}");
        if all_texts.iter().any(|t| t.contains(&needle)) {
            continue;
        }
        let text = ws.text(&file)?;
        let mut ed = PomEditor::new(&text);
        ed.remove_property(&name)?;
        ws.set(&file, ed.text)?;
        out.notes.push(format!("removed unused property `{name}`"));
    }
    if out.removed_from.is_empty() {
        bail!("no matching dependency found");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::detect::detect;
    use crate::project::model::Scope;

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn req<'a>(coord: &'a Coord, version: Option<&'a str>, scope: Scope, module: &'a Module) -> AddRequest<'a> {
        AddRequest { coord, version, scope, module, repo: None, known_repos: &[] }
    }

    #[test]
    fn alias_chain_matching() {
        assert!(chain_uses_alias("hikari", "hikari"));
        assert!(chain_uses_alias("hikari.get", "hikari"));
        assert!(chain_uses_alias("db.pool", "db-pool"));
        assert!(!chain_uses_alias("hikari.extra", "hikari"));
        assert!(!chain_uses_alias("hikari2", "hikari"));
    }

    #[test]
    fn add_with_catalog_then_remove_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "settings.gradle.kts", "rootProject.name = \"x\"\n");
        write(r, "build.gradle.kts", "plugins { java }\ndependencies {\n    implementation(libs.guava)\n}\n");
        write(
            r,
            "gradle/libs.versions.toml",
            "[versions]\nguava = \"33.0.0-jre\"\n\n[libraries]\nguava = { module = \"com.google.guava:guava\", version.ref = \"guava\" }\n",
        );
        let project = detect(r).unwrap();
        let module = &project.modules[0];
        let mut ws = Workspace::new();
        let coord = Coord::new("com.zaxxer", "HikariCP");
        let res = add_dependency(&mut ws, &project, &req(&coord, Some("6.0.0"), Scope::Compile, module)).unwrap();
        assert!(matches!(res, AddOutcome::Added { .. }));
        let build = ws.text(&module.build_file).unwrap();
        assert!(build.contains("    implementation(libs.hikaricp)\n"), "{build}");
        let cat = ws.text(project.catalog.as_ref().unwrap()).unwrap();
        assert!(cat.contains("hikaricp = { module = \"com.zaxxer:HikariCP\", version.ref = \"hikaricp\" }"), "{cat}");
        // adding again is a no-op
        let again = add_dependency(&mut ws, &project, &req(&coord, Some("6.0.0"), Scope::Compile, module)).unwrap();
        assert!(matches!(again, AddOutcome::AlreadyPresent { .. }));

        let sel = Selector::new("hikaricp", &BTreeMap::new());
        let res = remove_dependency(&mut ws, &project, &[module], &sel).unwrap();
        assert_eq!(res.removed_from, vec![":"]);
        let build = ws.text(&module.build_file).unwrap();
        assert!(!build.contains("hikaricp"));
        let cat = ws.text(project.catalog.as_ref().unwrap()).unwrap();
        assert!(!cat.contains("hikaricp"), "{cat}");
        assert!(cat.contains("guava"), "{cat}");
    }

    #[test]
    fn remove_keeps_catalog_entry_used_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "settings.gradle.kts", "include(\"a\", \"b\")\n");
        write(r, "a/build.gradle.kts", "dependencies {\n    implementation(libs.guava)\n}\n");
        write(r, "b/build.gradle.kts", "dependencies {\n    implementation(libs.guava)\n}\n");
        write(r, "gradle/libs.versions.toml", "[libraries]\nguava = \"com.google.guava:guava:33.0.0-jre\"\n");
        let project = detect(r).unwrap();
        let a = project.modules.iter().find(|m| m.name == ":a").unwrap();
        let mut ws = Workspace::new();
        let res = remove_dependency(&mut ws, &project, &[a], &Selector::new("guava", &BTreeMap::new())).unwrap();
        assert!(res.notes.iter().any(|n| n.contains("still used")));
        assert!(ws.text(project.catalog.as_ref().unwrap()).unwrap().contains("guava"));
    }

    #[test]
    fn add_without_catalog_uses_literal_and_respects_style() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "build.gradle",
            "plugins { id 'java' }\ndependencies {\n    implementation 'com.google.guava:guava:33.0.0-jre'\n}\n",
        );
        let project = detect(r).unwrap();
        let module = &project.modules[0];
        let mut ws = Workspace::new();
        let coord = Coord::new("redis.clients", "jedis");
        add_dependency(&mut ws, &project, &req(&coord, Some("5.1.0"), Scope::Test, module)).unwrap();
        let t = ws.text(&module.build_file).unwrap();
        assert!(t.contains("    testImplementation 'redis.clients:jedis:5.1.0'\n"), "{t}");
    }

    #[test]
    fn maven_add_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let pom = "<project>\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>g</groupId>\n  <artifactId>a</artifactId>\n  <version>1</version>\n</project>\n";
        write(r, "pom.xml", pom);
        let project = detect(r).unwrap();
        let module = &project.modules[0];
        let mut ws = Workspace::new();
        let coord = Coord::new("redis.clients", "jedis");
        add_dependency(&mut ws, &project, &req(&coord, Some("5.1.0"), Scope::Compile, module)).unwrap();
        let t = ws.text(&module.build_file).unwrap();
        assert!(t.contains("<jedis.version>5.1.0</jedis.version>"), "{t}");
        assert!(t.contains("<version>${jedis.version}</version>"), "{t}");
        let res = remove_dependency(&mut ws, &project, &[module], &Selector::new("jedis", &BTreeMap::new())).unwrap();
        assert!(res.notes.iter().any(|n| n.contains("jedis.version")));
        let t = ws.text(&module.build_file).unwrap();
        assert!(!t.contains("jedis"), "{t}");
        assert!(maven::scan(&t).is_ok());
    }

    #[test]
    fn updates_hit_exact_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("build.gradle.kts");
        std::fs::write(&p, "implementation(\"a:b:1.0\") // keep\nval v = \"2.0\"\n").unwrap();
        let mut ws = Workspace::new();
        let text = ws.text(&p).unwrap();
        let s1 = text.find("1.0").unwrap();
        let s2 = text.rfind("2.0").unwrap();
        apply_updates(
            &mut ws,
            &[
                (VersionSite::Text { file: p.clone(), start: s1, end: s1 + 3 }, "1.1".into()),
                (VersionSite::Text { file: p.clone(), start: s2, end: s2 + 3 }, "2.5".into()),
            ],
        )
        .unwrap();
        assert_eq!(ws.text(&p).unwrap(), "implementation(\"a:b:1.1\") // keep\nval v = \"2.5\"\n");
    }
}
