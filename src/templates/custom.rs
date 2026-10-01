//! User-defined templates stored in `<config dir>/templates/<name>/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::util::paths;

pub const MANIFEST: &str = "kiln-template.toml";

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub description: String,
    /// Extra repositories for `{{latest:…}}` lookups.
    pub repositories: Vec<String>,
    pub variables: BTreeMap<String, VariableSpec>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct VariableSpec {
    pub prompt: Option<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Custom {
    pub name: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.')
}

fn load(dir: &Path) -> Option<Custom> {
    let name = dir.file_name()?.to_string_lossy().to_string();
    let manifest = std::fs::read_to_string(dir.join(MANIFEST))
        .ok()
        .and_then(|s| toml_edit::de::from_str::<Manifest>(&s).ok())
        .unwrap_or_default();
    Some(Custom { name, dir: dir.to_path_buf(), manifest })
}

pub fn list() -> Result<Vec<Custom>> {
    let root = paths::templates_dir()?;
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&root) else { return Ok(out) };
    for e in rd.flatten() {
        if e.path().is_dir()
            && let Some(c) = load(&e.path())
        {
            out.push(c);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn find(name: &str) -> Result<Option<Custom>> {
    if !valid_name(name) {
        return Ok(None);
    }
    let dir = paths::templates_dir()?.join(name);
    Ok(if dir.is_dir() { load(&dir) } else { None })
}

/// All files of a template as `(relative path with '/', bytes)`, excluding the manifest.
pub fn files(c: &Custom) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    walk(&c.dir, &c.dir, &mut out)?;
    out.retain(|(p, _)| p != MANIFEST);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for e in std::fs::read_dir(dir).with_context(|| format!("could not read {}", dir.display()))? {
        let e = e?;
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out)?;
        } else {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            let rel =
                rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
            out.push((rel, std::fs::read(&p).with_context(|| format!("could not read {}", p.display()))?));
        }
    }
    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    let Some(c) = find(name)? else { bail!("custom template `{name}` not found") };
    std::fs::remove_dir_all(&c.dir).with_context(|| format!("could not delete {}", c.dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation() {
        assert!(valid_name("my-service_2.0"));
        assert!(!valid_name(""));
        assert!(!valid_name("../evil"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(".hidden"));
        assert!(!valid_name("sp ace"));
    }

    #[test]
    fn manifest_parses() {
        let m: Manifest = toml_edit::de::from_str(
            "description = \"x\"\nrepositories = [\"https://r\"]\n[variables.author]\nprompt = \"Who?\"\ndefault = \"me\"\n",
        )
        .unwrap();
        assert_eq!(m.variables["author"].default.as_deref(), Some("me"));
        assert_eq!(m.repositories, vec!["https://r"]);
    }
}
