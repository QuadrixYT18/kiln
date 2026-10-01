//! Collects every versioned dependency, plugin and repository of a project.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::catalog::{self, CatVersion, Catalog};
use super::gradle::{self, GPluginSource, GSource, Lit};
use super::maven::{self, Pom, Text};
use super::model::{BuildKind, DepKind, Dependency, Project, VersionSite};
use super::workspace::Workspace;
use crate::registry::version::Version;
use crate::registry::{Coord, Repo};

#[derive(Default)]
pub struct Scan {
    pub deps: Vec<Dependency>,
    /// Project repositories (without Maven Central / the plugin portal).
    pub repos: Vec<Repo>,
    pub catalog: Option<Catalog>,
}

type Vars = BTreeMap<String, (PathBuf, Lit)>;

fn resolve_literal(lit: &Lit, file: &Path, vars: &Vars) -> (Option<String>, VersionSite) {
    if let Some(name) = gradle::var_reference(&lit.value) {
        return match vars.get(name) {
            Some((f, l)) => {
                (Some(l.value.clone()), VersionSite::Text { file: f.clone(), start: l.range.start, end: l.range.end })
            }
            None => (None, VersionSite::Unsupported(format!("unresolved variable `{name}`"))),
        };
    }
    if Version::is_dynamic(&lit.value) {
        return (Some(lit.value.clone()), VersionSite::Unsupported("dynamic version".to_string()));
    }
    (
        Some(lit.value.clone()),
        VersionSite::Text { file: file.to_path_buf(), start: lit.range.start, end: lit.range.end },
    )
}

fn add_repo(scan: &mut Scan, url: &str) {
    let r = Repo::new(url);
    let skip = [crate::registry::DEFAULT_CENTRAL, crate::registry::DEFAULT_PLUGIN_PORTAL];
    if skip.iter().any(|s| Repo::new(s) == r) || scan.repos.contains(&r) {
        return;
    }
    scan.repos.push(r);
}

pub fn scan_project(project: &Project, ws: &mut Workspace, extra_repos: &[String]) -> Result<Scan> {
    let mut scan = Scan::default();
    match project.kind {
        BuildKind::Gradle => scan_gradle(project, ws, &mut scan)?,
        BuildKind::Maven => scan_maven(project, ws, &mut scan)?,
    }
    for r in extra_repos {
        add_repo(&mut scan, r);
    }
    Ok(scan)
}

fn load_props(ws: &mut Workspace, dir: &Path, vars: &mut Vars) {
    let p = dir.join("gradle.properties");
    if p.is_file()
        && let Ok(text) = ws.text(&p)
    {
        for (k, lit) in gradle::parse_properties(&text) {
            vars.entry(k).or_insert((p.clone(), lit));
        }
    }
}

fn scan_gradle(project: &Project, ws: &mut Workspace, scan: &mut Scan) -> Result<()> {
    let mut props: Vars = BTreeMap::new();
    load_props(ws, &project.root, &mut props);

    let plugin_site = |scan: &mut Scan, vars: &Vars, file: &Path, module: &str, gs: &gradle::GradleScan| {
        for p in &gs.plugins {
            if let GPluginSource::Id { id, version: Some(v) } = &p.source {
                let (current, site) = resolve_literal(v, file, vars);
                scan.deps.push(Dependency {
                    coord: Coord::plugin_marker(id),
                    kind: DepKind::Plugin,
                    current,
                    site,
                    module: module.to_string(),
                });
            }
        }
    };

    // settings.gradle(.kts): plugins + repositories
    if let Some(settings) = &project.settings {
        let text = ws.text(settings)?;
        let gs = gradle::scan(&text, gradle::is_kotlin_script(settings));
        let mut vars = props.clone();
        for (k, l) in &gs.vars {
            vars.insert(k.clone(), (settings.clone(), l.clone()));
        }
        plugin_site(scan, &vars, settings, "settings", &gs);
        for r in &gs.repos {
            add_repo(scan, r);
        }
    }

    // Version catalog
    if let Some(cat_path) = &project.catalog {
        let text = ws.text(cat_path)?;
        let cat = catalog::parse(&text)?;
        let file_name = "libs.versions.toml".to_string();
        let site_for = |v: &CatVersion, alias: &str, plugin: bool| -> (Option<String>, VersionSite) {
            match v {
                CatVersion::None => (None, VersionSite::Managed),
                CatVersion::Ref(key) => match cat.versions.get(key) {
                    Some(val) if Version::is_dynamic(val) => {
                        (Some(val.clone()), VersionSite::Unsupported("dynamic version".into()))
                    }
                    Some(val) => {
                        (Some(val.clone()), VersionSite::CatalogVersion { file: cat_path.clone(), key: key.clone() })
                    }
                    None => (None, VersionSite::Unsupported(format!("unknown version key `{key}`"))),
                },
                CatVersion::Literal(val) | CatVersion::Rich(_, val) => {
                    if Version::is_dynamic(val) {
                        (Some(val.clone()), VersionSite::Unsupported("dynamic version".into()))
                    } else {
                        (
                            Some(val.clone()),
                            VersionSite::CatalogInline { file: cat_path.clone(), plugin, alias: alias.to_string() },
                        )
                    }
                }
            }
        };
        for lib in &cat.libraries {
            let (current, site) = site_for(&lib.version, &lib.alias, false);
            scan.deps.push(Dependency {
                coord: lib.coord.clone(),
                kind: DepKind::Library,
                current,
                site,
                module: file_name.clone(),
            });
        }
        for pl in &cat.plugins {
            let (current, site) = site_for(&pl.version, &pl.alias, true);
            scan.deps.push(Dependency {
                coord: Coord::plugin_marker(&pl.id),
                kind: DepKind::Plugin,
                current,
                site,
                module: file_name.clone(),
            });
        }
        scan.catalog = Some(cat);
    }

    // Build scripts
    let root_build = project.modules.iter().find(|m| m.dir == project.root).map(|m| m.build_file.clone());
    let mut root_vars: Vars = BTreeMap::new();
    if let Some(rb) = &root_build {
        let text = ws.text(rb)?;
        let gs = gradle::scan(&text, gradle::is_kotlin_script(rb));
        for (k, l) in gs.vars {
            root_vars.insert(k, (rb.clone(), l));
        }
    }
    for module in &project.modules {
        let text = ws.text(&module.build_file)?;
        let gs = gradle::scan(&text, gradle::is_kotlin_script(&module.build_file));
        let mut vars = props.clone();
        load_props(ws, &module.dir, &mut vars);
        for (k, v) in &root_vars {
            vars.insert(k.clone(), v.clone());
        }
        for (k, l) in &gs.vars {
            vars.insert(k.clone(), (module.build_file.clone(), l.clone()));
        }
        for d in &gs.deps {
            let GSource::Gav { coord, version } = &d.source else { continue };
            let (current, site) = match version {
                Some(v) => resolve_literal(v, &module.build_file, &vars),
                None => (None, VersionSite::Managed),
            };
            scan.deps.push(Dependency {
                coord: coord.clone(),
                kind: DepKind::Library,
                current,
                site,
                module: module.name.clone(),
            });
        }
        plugin_site(scan, &vars, &module.build_file, &module.name, &gs);
        for r in &gs.repos {
            add_repo(scan, r);
        }
    }
    Ok(())
}

fn maven_site(version: &Text, file: &Path, props: &BTreeMap<String, (PathBuf, Text)>) -> (Option<String>, VersionSite) {
    let v = &version.value;
    if v.starts_with("${") {
        let name = v.trim_start_matches("${").trim_end_matches('}');
        // Resolve chains of `${a}` -> `${b}`.
        let mut cur = name.to_string();
        for _ in 0..5 {
            match props.get(&cur) {
                Some((_, t)) if t.value.starts_with("${") => {
                    cur = t.value.trim_start_matches("${").trim_end_matches('}').to_string()
                }
                Some((f, t)) => {
                    if Version::is_dynamic(&t.value) {
                        return (Some(t.value.clone()), VersionSite::Unsupported("dynamic version".into()));
                    }
                    return (
                        Some(t.value.clone()),
                        VersionSite::Text { file: f.clone(), start: t.range.start, end: t.range.end },
                    );
                }
                None => break,
            }
        }
        return (None, VersionSite::Unsupported(format!("unresolved property `{name}`")));
    }
    if Version::is_dynamic(v) {
        return (Some(v.clone()), VersionSite::Unsupported("version range".into()));
    }
    (
        Some(v.clone()),
        VersionSite::Text { file: file.to_path_buf(), start: version.range.start, end: version.range.end },
    )
}

fn scan_maven(project: &Project, ws: &mut Workspace, scan: &mut Scan) -> Result<()> {
    let mut poms: Vec<(usize, Pom)> = Vec::new();
    for (i, m) in project.modules.iter().enumerate() {
        let text = ws.text(&m.build_file)?;
        poms.push((i, maven::scan(&text)?));
    }
    for (i, pom) in &poms {
        let module = &project.modules[*i];
        // Properties visible from this module: ancestors first, own last.
        let mut props: BTreeMap<String, (PathBuf, Text)> = BTreeMap::new();
        let mut ancestors: Vec<usize> =
            (0..project.modules.len()).filter(|j| j != i && module.dir.starts_with(&project.modules[*j].dir)).collect();
        ancestors.sort_by_key(|j| project.modules[*j].dir.components().count());
        for j in ancestors.into_iter().chain([*i]) {
            let (_, p) = &poms[j];
            for (k, t) in &p.properties {
                props.insert(k.clone(), (project.modules[j].build_file.clone(), t.clone()));
            }
        }
        for d in &pom.dependencies {
            let Some(coord) = d.coord() else { continue };
            let (current, site) = match &d.version {
                Some(v) => maven_site(v, &module.build_file, &props),
                None => (None, VersionSite::Managed),
            };
            scan.deps.push(Dependency { coord, kind: DepKind::Library, current, site, module: module.name.clone() });
        }
        for p in &pom.plugins {
            let Some(v) = &p.version else { continue };
            let (current, site) = maven_site(v, &module.build_file, &props);
            scan.deps.push(Dependency {
                coord: Coord::new(&p.group, &p.artifact),
                kind: DepKind::MavenPlugin,
                current,
                site,
                module: module.name.clone(),
            });
        }
        if let Some(parent) = &pom.parent
            && let Some(v) = &parent.version
        {
            let (current, site) = maven_site(v, &module.build_file, &props);
            // Only when the parent is outside this project (not a sibling module).
            let local = project.modules.iter().any(|m| m.name == parent.artifact);
            if !local {
                scan.deps.push(Dependency {
                    coord: Coord::new(&parent.group, &parent.artifact),
                    kind: DepKind::Parent,
                    current,
                    site,
                    module: module.name.clone(),
                });
            }
        }
        for r in &pom.repositories {
            add_repo(scan, r);
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub coord: Coord,
    /// (module, version)
    pub versions: Vec<(String, String)>,
}

/// Libraries declared with different versions in different places.
pub fn conflicts(deps: &[Dependency]) -> Vec<Conflict> {
    let mut by: BTreeMap<&Coord, Vec<&Dependency>> = BTreeMap::new();
    for d in deps.iter().filter(|d| d.kind == DepKind::Library && d.current.is_some()) {
        by.entry(&d.coord).or_default().push(d);
    }
    let mut out = Vec::new();
    for (coord, ds) in by {
        let distinct: BTreeSet<&str> = ds.iter().filter_map(|d| d.current.as_deref()).collect();
        if distinct.len() > 1 {
            out.push(Conflict {
                coord: coord.clone(),
                versions: ds.iter().map(|d| (d.module.clone(), d.current.clone().unwrap_or_default())).collect(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::detect::detect;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn scans_gradle_with_catalog_vars_and_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "settings.gradle.kts", "rootProject.name = \"x\"\ninclude(\"app\")\n");
        write(
            r,
            "build.gradle.kts",
            "plugins { kotlin(\"jvm\") version \"2.0.0\" }\nval gson = \"2.10\"\ndependencies { implementation(\"com.google.code.gson:gson:$gson\") }\n",
        );
        write(
            r,
            "app/build.gradle.kts",
            "dependencies {\n implementation(libs.hikari)\n implementation(\"org.x:y:1.+\")\n implementation(\"org.x:managed\")\n}\n",
        );
        write(r, "gradle.properties", "other=1\n");
        write(
            r,
            "gradle/libs.versions.toml",
            "[versions]\nhikari = \"5.0.1\"\n[libraries]\nhikari = { module = \"com.zaxxer:HikariCP\", version.ref = \"hikari\" }\n[plugins]\nshadow = { id = \"com.gradleup.shadow\", version = \"8.3.0\" }\n",
        );
        let project = detect(r).unwrap();
        let mut ws = Workspace::new();
        let scan = scan_project(&project, &mut ws, &[]).unwrap();
        let find = |name: &str| scan.deps.iter().find(|d| d.coord.to_string().contains(name)).unwrap();
        let gson = find("gson");
        assert_eq!(gson.current.as_deref(), Some("2.10"));
        assert!(matches!(gson.site, VersionSite::Text { .. }));
        assert!(matches!(find("HikariCP").site, VersionSite::CatalogVersion { .. }));
        assert!(matches!(find("org.x:y").site, VersionSite::Unsupported(_)));
        assert!(matches!(find("org.x:managed").site, VersionSite::Managed));
        assert_eq!(find("org.jetbrains.kotlin.jvm").kind, DepKind::Plugin);
        assert!(matches!(find("com.gradleup.shadow").site, VersionSite::CatalogInline { plugin: true, .. }));
    }

    #[test]
    fn scans_maven_with_properties_and_parent_props() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "pom.xml",
            "<project><artifactId>parent</artifactId><modules><module>core</module></modules><properties><jackson.version>2.15.0</jackson.version></properties><build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><version>3.11.0</version></plugin></plugins></build></project>",
        );
        write(
            r,
            "core/pom.xml",
            "<project><artifactId>core</artifactId><parent><groupId>g</groupId><artifactId>parent</artifactId><version>1</version></parent><dependencies><dependency><groupId>com.fasterxml.jackson.core</groupId><artifactId>jackson-databind</artifactId><version>${jackson.version}</version></dependency></dependencies></project>",
        );
        let project = detect(r).unwrap();
        let mut ws = Workspace::new();
        let scan = scan_project(&project, &mut ws, &[]).unwrap();
        let jackson = scan.deps.iter().find(|d| d.coord.artifact == "jackson-databind").unwrap();
        assert_eq!(jackson.current.as_deref(), Some("2.15.0"));
        let VersionSite::Text { file, start, end } = &jackson.site else { panic!() };
        assert_eq!(file, &project.modules[0].build_file);
        assert_eq!(&std::fs::read_to_string(file).unwrap()[*start..*end], "2.15.0");
        let plugin = scan.deps.iter().find(|d| d.kind == DepKind::MavenPlugin).unwrap();
        assert_eq!(plugin.coord.group, "org.apache.maven.plugins");
        // parent inside the project is not an updatable dependency
        assert!(scan.deps.iter().all(|d| d.kind != DepKind::Parent));
    }

    #[test]
    fn finds_conflicts() {
        let mk = |m: &str, v: &str| Dependency {
            coord: Coord::new("a", "b"),
            kind: DepKind::Library,
            current: Some(v.into()),
            site: VersionSite::Managed,
            module: m.into(),
        };
        let c = conflicts(&[mk("x", "1"), mk("y", "2")]);
        assert_eq!(c.len(), 1);
        assert!(conflicts(&[mk("x", "1"), mk("y", "1")]).is_empty());
    }
}
