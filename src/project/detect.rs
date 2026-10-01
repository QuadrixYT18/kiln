//! Locating the project root and its modules.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::gradle::{mask, matching_close, string_literals};
use super::maven;
use super::model::{BuildKind, Module, Project};

const GRADLE_SETTINGS: [&str; 2] = ["settings.gradle.kts", "settings.gradle"];
const GRADLE_BUILD: [&str; 2] = ["build.gradle.kts", "build.gradle"];

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

pub fn gradle_settings(dir: &Path) -> Option<PathBuf> {
    first_existing(dir, &GRADLE_SETTINGS)
}

pub fn gradle_build(dir: &Path) -> Option<PathBuf> {
    first_existing(dir, &GRADLE_BUILD)
}

/// Finds the project that contains `start` and loads its module list.
pub fn detect(start: &Path) -> Result<Project> {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    // Windows canonicalize yields `\\?\` paths; strip them for friendlier output.
    let start = strip_verbatim(start);
    let mut dir = Some(start.as_path());
    while let Some(d) = dir {
        if gradle_settings(d).is_some() {
            return load_gradle(d);
        }
        dir = d.parent();
    }
    // Gradle without settings file (single project) or Maven.
    let mut dir = Some(start.as_path());
    let mut found: Option<PathBuf> = None;
    while let Some(d) = dir {
        if d.join("pom.xml").is_file() || gradle_build(d).is_some() {
            found = Some(d.to_path_buf());
            break;
        }
        dir = d.parent();
    }
    let Some(mut root) = found else {
        bail!(
            "no Gradle or Maven project found in {} or any parent directory\n  hint: run kiln inside a project, or use `kiln new` to create one",
            start.display()
        );
    };
    if root.join("pom.xml").is_file() && gradle_build(&root).is_none() {
        // Walk up while the parent pom lists us as a module.
        loop {
            let Some(parent) = root.parent() else { break };
            let pom = parent.join("pom.xml");
            let Ok(text) = std::fs::read_to_string(&pom) else { break };
            let Ok(p) = maven::scan(&text) else { break };
            let me = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if p.modules.iter().any(|m| {
                m.trim_end_matches('/') == me || Path::new(m).file_name().is_some_and(|f| f.to_string_lossy() == me)
            }) {
                root = parent.to_path_buf();
            } else {
                break;
            }
        }
        return load_maven(&root);
    }
    if gradle_build(&root).is_some() {
        return load_gradle(&root);
    }
    load_maven(&root)
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    p
}

fn load_gradle(root: &Path) -> Result<Project> {
    let settings = gradle_settings(root);
    let catalog = Some(root.join("gradle").join("libs.versions.toml")).filter(|p| p.is_file());
    let mut modules = Vec::new();
    if let Some(b) = gradle_build(root) {
        modules.push(Module { name: ":".to_string(), dir: root.to_path_buf(), build_file: b });
    }
    if let Some(s) = &settings {
        let text = std::fs::read_to_string(s)?;
        for path in included_projects(&text) {
            let rel: PathBuf = path.split(':').filter(|p| !p.is_empty()).collect();
            let dir = root.join(&rel);
            if let Some(b) = gradle_build(&dir) {
                let name = format!(":{}", path.trim_start_matches(':'));
                if !modules.iter().any(|m| m.name == name) {
                    modules.push(Module { name, dir, build_file: b });
                }
            }
        }
    }
    Ok(Project { root: root.to_path_buf(), kind: BuildKind::Gradle, modules, catalog, settings })
}

/// Project paths from `include(...)` calls in a settings script.
pub fn included_projects(src: &str) -> Vec<String> {
    let m = mask(src);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 7 <= m.len() {
        if &m[i..i + 7] == b"include" && (i == 0 || !(m[i - 1].is_ascii_alphanumeric() || m[i - 1] == b'_')) {
            let after = i + 7;
            let next = m.get(after).copied().unwrap_or(b' ');
            // `include(` or `include ` — not `includeBuild`.
            if next == b'(' || next == b' ' || next == b'\t' {
                let (range_start, range_end) = if next == b'(' {
                    match matching_close(&m, after) {
                        Some(c) => (after + 1, c),
                        None => (after, m.len()),
                    }
                } else {
                    // Groovy: arguments until end of statement (allow trailing-comma continuation).
                    let mut e = after;
                    while e < m.len() {
                        if m[e] == b'\n' {
                            let prev = m[..e].iter().rev().find(|c| !c.is_ascii_whitespace()).copied();
                            if prev != Some(b',') {
                                break;
                            }
                        }
                        e += 1;
                    }
                    (after, e)
                };
                for lit in string_literals(&m, range_start..range_end) {
                    let s = src[lit].trim().to_string();
                    if !s.is_empty() {
                        out.push(s);
                    }
                }
                i = range_end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn load_maven(root: &Path) -> Result<Project> {
    let mut modules = Vec::new();
    collect_maven(root, &mut modules, 0)?;
    Ok(Project { root: root.to_path_buf(), kind: BuildKind::Maven, modules, catalog: None, settings: None })
}

fn collect_maven(dir: &Path, out: &mut Vec<Module>, depth: usize) -> Result<()> {
    let pom_path = dir.join("pom.xml");
    let text = std::fs::read_to_string(&pom_path)?;
    let pom = maven::scan(&text)?;
    let name = pom
        .artifact_id
        .clone()
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    out.push(Module { name, dir: dir.to_path_buf(), build_file: pom_path });
    if depth > 8 {
        return Ok(());
    }
    for m in &pom.modules {
        let sub = dir.join(m.trim_end_matches('/'));
        // A module may point directly at a pom file.
        let sub_dir = if sub.is_file() { sub.parent().map(Path::to_path_buf).unwrap_or(sub.clone()) } else { sub };
        if sub_dir.join("pom.xml").is_file() && !out.iter().any(|x| x.dir == sub_dir) {
            collect_maven(&sub_dir, out, depth + 1)?;
        }
    }
    Ok(())
}

impl Project {
    /// Picks the module `kiln add` should edit.
    pub fn select_module(&self, name: Option<&str>, cwd: &Path) -> Result<&Module> {
        if let Some(n) = name {
            let wanted = n.trim_start_matches(':');
            return self
                .modules
                .iter()
                .find(|m| {
                    m.name.trim_start_matches(':') == wanted
                        || m.dir.file_name().is_some_and(|f| f.to_string_lossy() == wanted)
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "module `{n}` not found; available: {}",
                        self.modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
                    )
                });
        }
        let cwd = strip_verbatim(cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()));
        // Deepest module containing the working directory.
        let best = self.modules.iter().filter(|m| cwd.starts_with(&m.dir)).max_by_key(|m| m.dir.components().count());
        match (best, self.modules.len()) {
            (Some(m), _) if m.dir != self.root || self.modules.len() == 1 => Ok(m),
            (_, 1) => Ok(&self.modules[0]),
            (Some(m), _) if self.modules.iter().filter(|x| x.dir != self.root).count() == 0 => Ok(m),
            _ => bail!(
                "this is a multi-module project; choose a module with --module ({})",
                self.modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    pub fn gradle_wrapper(&self) -> Option<PathBuf> {
        let name = if cfg!(windows) { "gradlew.bat" } else { "gradlew" };
        Some(self.root.join(name)).filter(|p| p.is_file())
    }

    pub fn maven_wrapper(&self) -> Option<PathBuf> {
        let name = if cfg!(windows) { "mvnw.cmd" } else { "mvnw" };
        Some(self.root.join(name)).filter(|p| p.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_includes() {
        let kts =
            "rootProject.name = \"x\"\ninclude(\":app\", \"lib:core\")\ninclude(\"api\")\nincludeBuild(\"../other\")\n";
        assert_eq!(included_projects(kts), vec![":app", "lib:core", "api"]);
        let groovy = "include ':app', ':lib'\ninclude 'a',\n    'b'\n";
        assert_eq!(included_projects(groovy), vec![":app", ":lib", "a", "b"]);
    }

    #[test]
    fn detects_gradle_multi_module() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("settings.gradle.kts"), "include(\"app\", \"lib\")\n").unwrap();
        std::fs::write(root.join("build.gradle.kts"), "").unwrap();
        for m in ["app", "lib"] {
            std::fs::create_dir_all(root.join(m)).unwrap();
            std::fs::write(root.join(m).join("build.gradle.kts"), "").unwrap();
        }
        let p = detect(&root.join("app")).unwrap();
        assert_eq!(p.kind, BuildKind::Gradle);
        assert_eq!(p.modules.len(), 3);
        assert_eq!(p.select_module(None, &root.join("app")).unwrap().name, ":app");
        assert!(p.select_module(None, root).is_err());
        assert_eq!(p.select_module(Some("lib"), root).unwrap().name, ":lib");
    }

    #[test]
    fn detects_maven_modules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("pom.xml"),
            "<project><artifactId>parent</artifactId><modules><module>core</module></modules></project>",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("core")).unwrap();
        std::fs::write(root.join("core/pom.xml"), "<project><artifactId>core</artifactId></project>").unwrap();
        let p = detect(&root.join("core")).unwrap();
        assert_eq!(p.kind, BuildKind::Maven);
        assert_eq!(p.modules.len(), 2);
        assert_eq!(p.root.file_name(), root.file_name());
    }

    #[test]
    fn no_project_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_err());
    }
}
