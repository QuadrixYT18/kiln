//! `kiln template save`: turn an existing project into a reusable template by
//! replacing project-specific values with placeholders.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::custom::{MANIFEST, valid_name};
use crate::project::catalog;
use crate::project::gradle;
use crate::registry::version::Version;
use crate::util::paths;

pub struct SaveOptions {
    pub name: String,
    pub from: PathBuf,
    pub project_name: Option<String>,
    pub package: Option<String>,
    pub force: bool,
}

pub struct SaveReport {
    pub dir: PathBuf,
    pub files: usize,
    pub package: Option<String>,
    pub project_name: String,
}

const SKIP_DIRS: &[&str] =
    &[".git", ".gradle", "build", "out", "target", ".idea", ".vscode", ".kotlin", "node_modules", ".fleet"];

fn skip_file(rel: &str, name: &str) -> bool {
    matches!(name, ".DS_Store" | "local.properties" | "gradlew" | "gradlew.bat" | "Thumbs.db")
        || name.ends_with(".class")
        || name.ends_with(".iml")
        || name.ends_with(".log")
        || name.starts_with("hs_err_pid")
        || rel.starts_with("gradle/wrapper/")
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for e in std::fs::read_dir(dir).with_context(|| format!("could not read {}", dir.display()))? {
        let e = e?;
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect(root, &p, out)?;
        } else {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            let rel =
                rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
            if !skip_file(&rel, &name) {
                out.push((rel, p));
            }
        }
    }
    Ok(())
}

/// Reads the `package x.y.z` declaration of the first Java/Kotlin source file.
pub fn detect_package(root: &Path) -> Option<String> {
    for src in ["src/main/java", "src/main/kotlin"] {
        let mut files = Vec::new();
        let base = root.join(src);
        if !base.is_dir() {
            continue;
        }
        let _ = collect(&base, &base, &mut files);
        files.sort();
        for (_, p) in files {
            if !matches!(p.extension().and_then(|e| e.to_str()), Some("java" | "kt")) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                for line in text.lines() {
                    if let Some(rest) = line.trim().strip_prefix("package ") {
                        let pkg = rest.trim().trim_end_matches(';').trim();
                        if !pkg.is_empty() {
                            return Some(pkg.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Replaces whole-word occurrences (neighbours must not be alphanumeric).
pub fn replace_word(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(needle) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + needle.len()..].chars().next();
        let ok = !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric());
        out.push_str(&rest[..i]);
        out.push_str(if ok { replacement } else { needle });
        rest = &rest[i + needle.len()..];
    }
    out.push_str(rest);
    out
}

pub fn pascal_case(name: &str) -> String {
    let mut out = String::new();
    for part in name.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()) {
        let mut cs = part.chars();
        if let Some(f) = cs.next() {
            out.extend(f.to_uppercase());
            out.push_str(cs.as_str());
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'P');
    }
    out
}

fn replace_java_version(text: &str) -> String {
    let mut out = text.to_string();
    for pat in ["jvmToolchain(", "JavaLanguageVersion.of(", "options.release.set(", "options.release = "] {
        let mut result = String::new();
        let mut rest = out.as_str();
        while let Some(i) = rest.find(pat) {
            let after = &rest[i + pat.len()..];
            let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
            result.push_str(&rest[..i + pat.len()]);
            if digits > 0 {
                result.push_str("{{java_version}}");
                rest = &after[digits..];
            } else {
                rest = after;
            }
        }
        result.push_str(rest);
        out = result;
    }
    out
}

/// Turns pinned catalog versions into live lookups.
fn unpin_catalog(text: &str) -> String {
    let Ok(cat) = catalog::parse(text) else { return text.to_string() };
    let mut out = text.to_string();
    let opt = |v: &str| {
        let ver = Version::new(v);
        if ver.is_stable() {
            ""
        } else if ver.allowed(crate::registry::version::Stability::AllowSnapshot) {
            "?snapshot"
        } else {
            "?pre"
        }
    };
    for (key, value) in &cat.versions {
        let lib = cat.libraries.iter().find(|l| l.version == catalog::CatVersion::Ref(key.clone()));
        let plugin = cat.plugins.iter().find(|p| p.version == catalog::CatVersion::Ref(key.clone()));
        let token = match (lib, plugin) {
            (Some(l), _) => Some(format!("{{{{latest:{}{}}}}}", l.coord, opt(value))),
            (None, Some(p)) => Some(format!("{{{{plugin:{}{}}}}}", p.id, opt(value))),
            _ => None,
        };
        if let Some(t) = token
            && let Ok(n) = catalog::set_version_key(&out, key, &t)
        {
            out = n;
        }
    }
    for l in &cat.libraries {
        if let catalog::CatVersion::Literal(v) = &l.version {
            let t = format!("{{{{latest:{}{}}}}}", l.coord, opt(v));
            if let Ok(n) = catalog::set_inline_version(&out, catalog::Table_::Libraries, &l.alias, &t) {
                out = n;
            }
        }
    }
    for p in &cat.plugins {
        if let catalog::CatVersion::Literal(v) = &p.version {
            let t = format!("{{{{plugin:{}{}}}}}", p.id, opt(v));
            if let Ok(n) = catalog::set_inline_version(&out, catalog::Table_::Plugins, &p.alias, &t) {
                out = n;
            }
        }
    }
    out
}

/// Value of `rootProject.name = "…"` in a settings script.
pub fn root_project_name(text: &str) -> Option<String> {
    let m = gradle::mask(text);
    let i = text.find("rootProject.name")?;
    let lits = gradle::string_literals(&m, i..text.len().min(i + 200));
    lits.first().map(|r| text[r.clone()].to_string())
}

pub fn save(opts: &SaveOptions) -> Result<SaveReport> {
    if !valid_name(&opts.name) {
        bail!("invalid template name `{}` (use letters, digits, `-`, `_`, `.`)", opts.name);
    }
    let from = opts.from.canonicalize().with_context(|| format!("{} does not exist", opts.from.display()))?;
    let target = paths::templates_dir()?.join(&opts.name);
    if target.exists() {
        if !opts.force {
            bail!("template `{}` already exists (use --force to overwrite)", opts.name);
        }
        std::fs::remove_dir_all(&target)?;
    }

    // Project facts.
    let dir_name = from.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut project_name = opts.project_name.clone().unwrap_or_else(|| dir_name.clone());
    if opts.project_name.is_none()
        && let Some(s) = crate::project::detect::gradle_settings(&from)
        && let Ok(text) = std::fs::read_to_string(&s)
        && let Some(v) = root_project_name(&text)
    {
        project_name = v;
    }
    let package = opts.package.clone().or_else(|| detect_package(&from));
    let group = crate::project::detect::gradle_build(&from)
        .and_then(|b| std::fs::read_to_string(&b).ok().map(|t| (t, b)))
        .and_then(|(t, b)| gradle::scan(&t, gradle::is_kotlin_script(&b)).vars.get("group").map(|l| l.value.clone()));
    let class_name = pascal_case(&project_name);

    let mut files = Vec::new();
    collect(&from, &from, &mut files)?;
    if files.is_empty() {
        bail!("no files found in {}", from.display());
    }

    let mut count = 0;
    for (rel, path) in &files {
        let mut out_rel = rel.clone();
        if let Some(pkg) = &package {
            let pkg_path = pkg.replace('.', "/");
            out_rel = out_rel.replace(&pkg_path, "{{package_path}}");
        }
        out_rel = replace_word(&out_rel, &class_name, "{{class_name}}");
        let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        let data = match std::str::from_utf8(&bytes) {
            Ok(text) if !bytes.contains(&0) => {
                let mut t = text.to_string();
                if let Some(pkg) = &package {
                    t = replace_word(&t, pkg, "{{package}}");
                    // Slash form (resource paths, META-INF entries).
                    t = t.replace(&pkg.replace('.', "/"), "{{package_path}}");
                }
                if let Some(g) = &group
                    && !g.is_empty()
                {
                    t = replace_word(&t, g, "{{group}}");
                }
                if !project_name.is_empty() && project_name != "name" {
                    t = replace_word(&t, &project_name, "{{name}}");
                }
                if class_name.len() > 2 && class_name != project_name {
                    t = replace_word(&t, &class_name, "{{class_name}}");
                }
                if rel.ends_with(".gradle.kts") || rel.ends_with(".gradle") {
                    t = replace_java_version(&t);
                }
                if rel == "gradle/libs.versions.toml" {
                    t = unpin_catalog(&t);
                }
                t.into_bytes()
            }
            _ => bytes,
        };
        let dest = out_rel.split('/').fold(target.clone(), |acc, c| acc.join(c));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, data).with_context(|| format!("could not write {}", dest.display()))?;
        count += 1;
    }
    let manifest = format!("description = \"Saved from {}\"\n", dir_name.replace('"', "'"));
    std::fs::write(target.join(MANIFEST), manifest)?;
    Ok(SaveReport { dir: target, files: count, package, project_name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_replacement_respects_boundaries() {
        assert_eq!(replace_word("app application my-app app.jar", "app", "X"), "X application my-X X.jar");
        assert_eq!(replace_word("com.example.app.Main", "com.example.app", "{{package}}"), "{{package}}.Main");
    }

    #[test]
    fn finds_root_project_name() {
        assert_eq!(root_project_name("rootProject.name = \"demo\"\n").as_deref(), Some("demo"));
        assert_eq!(root_project_name("rootProject.name = 'g'\n").as_deref(), Some("g"));
        assert_eq!(root_project_name("include(\"a\")").as_deref(), None);
    }

    #[test]
    fn pascal() {
        assert_eq!(pascal_case("my-cool_plugin"), "MyCoolPlugin");
        assert_eq!(pascal_case("2fast"), "P2fast");
    }

    #[test]
    fn java_version_is_templated() {
        assert_eq!(replace_java_version("kotlin { jvmToolchain(21) }"), "kotlin { jvmToolchain({{java_version}}) }");
        assert_eq!(replace_java_version("JavaLanguageVersion.of(17)"), "JavaLanguageVersion.of({{java_version}})");
    }

    #[test]
    fn catalog_versions_become_lookups() {
        let cat = "[versions]\nhikari = \"5.1.0\"\nkotlin = \"2.0.0\"\n[libraries]\nhikari = { module = \"com.zaxxer:HikariCP\", version.ref = \"hikari\" }\nguava = \"com.google.guava:guava:33.0.0-jre\"\n[plugins]\nkotlin = { id = \"org.jetbrains.kotlin.jvm\", version.ref = \"kotlin\" }\n";
        let out = unpin_catalog(cat);
        assert!(out.contains("hikari = \"{{latest:com.zaxxer:HikariCP}}\""), "{out}");
        assert!(out.contains("kotlin = \"{{plugin:org.jetbrains.kotlin.jvm}}\""), "{out}");
        assert!(out.contains("guava = \"com.google.guava:guava:{{latest:com.google.guava:guava}}\""), "{out}");
    }
}
