//! Gradle version catalog (`gradle/libs.versions.toml`) reading and editing.
//!
//! Editing goes through `toml_edit`, which preserves comments, ordering and
//! whitespace of everything we do not touch.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use crate::registry::Coord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatVersion {
    None,
    /// `version.ref = "key"`
    Ref(String),
    /// Plain string version, e.g. `version = "1.0"` or `"g:a:1.0"`.
    Literal(String),
    /// `version = { require/strictly/prefer = "x" }`: the key and its value.
    Rich(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatLib {
    pub alias: String,
    pub coord: Coord,
    pub version: CatVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatPlugin {
    pub alias: String,
    pub id: String,
    pub version: CatVersion,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub versions: BTreeMap<String, String>,
    pub libraries: Vec<CatLib>,
    pub plugins: Vec<CatPlugin>,
    /// bundle name -> library aliases
    pub bundles: BTreeMap<String, Vec<String>>,
}

/// Gradle turns `-`, `_` and `.` in aliases into accessor segments.
pub fn accessor(alias: &str) -> String {
    alias.replace(['-', '_'], ".")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table_ {
    Libraries,
    Plugins,
}

impl Table_ {
    fn key(self) -> &'static str {
        match self {
            Table_::Libraries => "libraries",
            Table_::Plugins => "plugins",
        }
    }
}

fn read_version(tbl: &dyn toml_edit::TableLike) -> CatVersion {
    match tbl.get("version") {
        None => CatVersion::None,
        Some(item) => {
            if let Some(s) = item.as_str() {
                return CatVersion::Literal(s.to_string());
            }
            if let Some(t) = item.as_table_like() {
                if let Some(r) = t.get("ref").and_then(Item::as_str) {
                    return CatVersion::Ref(r.to_string());
                }
                for k in ["strictly", "require", "prefer"] {
                    if let Some(v) = t.get(k).and_then(Item::as_str) {
                        return CatVersion::Rich(k.to_string(), v.to_string());
                    }
                }
            }
            CatVersion::None
        }
    }
}

pub fn parse(text: &str) -> Result<Catalog> {
    let doc: DocumentMut = text.parse().context("invalid TOML in version catalog")?;
    let mut cat = Catalog::default();
    if let Some(versions) = doc.get("versions").and_then(Item::as_table_like) {
        for (k, v) in versions.iter() {
            if let Some(s) = v.as_str() {
                cat.versions.insert(k.to_string(), s.to_string());
            } else if let Some(t) = v.as_table_like() {
                for key in ["strictly", "require", "prefer"] {
                    if let Some(s) = t.get(key).and_then(Item::as_str) {
                        cat.versions.insert(k.to_string(), s.to_string());
                        break;
                    }
                }
            }
        }
    }
    if let Some(libs) = doc.get("libraries").and_then(Item::as_table_like) {
        for (alias, item) in libs.iter() {
            if let Some(s) = item.as_str() {
                if let Some((coord, v)) = Coord::parse(s) {
                    cat.libraries.push(CatLib {
                        alias: alias.to_string(),
                        coord,
                        version: v.map(CatVersion::Literal).unwrap_or(CatVersion::None),
                    });
                }
                continue;
            }
            let Some(t) = item.as_table_like() else { continue };
            let coord = if let Some(m) = t.get("module").and_then(Item::as_str) {
                Coord::parse(m).map(|(c, _)| c)
            } else {
                match (t.get("group").and_then(Item::as_str), t.get("name").and_then(Item::as_str)) {
                    (Some(g), Some(n)) => Some(Coord::new(g, n)),
                    _ => None,
                }
            };
            if let Some(coord) = coord {
                cat.libraries.push(CatLib { alias: alias.to_string(), coord, version: read_version(t) });
            }
        }
    }
    if let Some(plugins) = doc.get("plugins").and_then(Item::as_table_like) {
        for (alias, item) in plugins.iter() {
            if let Some(s) = item.as_str() {
                let mut it = s.splitn(2, ':');
                if let Some(id) = it.next() {
                    cat.plugins.push(CatPlugin {
                        alias: alias.to_string(),
                        id: id.to_string(),
                        version: it.next().map(|v| CatVersion::Literal(v.to_string())).unwrap_or(CatVersion::None),
                    });
                }
                continue;
            }
            let Some(t) = item.as_table_like() else { continue };
            if let Some(id) = t.get("id").and_then(Item::as_str) {
                cat.plugins.push(CatPlugin { alias: alias.to_string(), id: id.to_string(), version: read_version(t) });
            }
        }
    }
    if let Some(bundles) = doc.get("bundles").and_then(Item::as_table_like) {
        for (name, item) in bundles.iter() {
            if let Some(arr) = item.as_array() {
                cat.bundles.insert(name.to_string(), arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            }
        }
    }
    Ok(cat)
}

fn replace_string(v: &mut Value, new: &str) {
    let decor = v.decor().clone();
    *v = Value::from(new);
    *v.decor_mut() = decor;
}

fn parse_doc(text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>().context("invalid TOML in version catalog")
}

/// Sets the value of a `[versions]` key (keeps decoration/comments).
pub fn set_version_key(text: &str, key: &str, new: &str) -> Result<String> {
    let mut doc = parse_doc(text)?;
    let item = doc
        .get_mut("versions")
        .and_then(Item::as_table_like_mut)
        .and_then(|t| t.get_mut(key))
        .with_context(|| format!("version key `{key}` not found in catalog"))?;
    if let Some(v) = item.as_value_mut() {
        if v.is_str() {
            replace_string(v, new);
        } else if let Some(t) = v.as_inline_table_mut() {
            set_rich(t, new)?;
        }
    } else if let Some(t) = item.as_table_like_mut() {
        for k in ["strictly", "require", "prefer"] {
            if let Some(v) = t.get_mut(k).and_then(Item::as_value_mut) {
                replace_string(v, new);
                return Ok(doc.to_string());
            }
        }
        bail!("unsupported version declaration for `{key}`");
    }
    Ok(doc.to_string())
}

fn set_rich(t: &mut InlineTable, new: &str) -> Result<()> {
    for k in ["strictly", "require", "prefer"] {
        if let Some(v) = t.get_mut(k) {
            replace_string(v, new);
            return Ok(());
        }
    }
    bail!("unsupported rich version declaration")
}

/// Rewrites a version that is declared inline in a library/plugin entry.
pub fn set_inline_version(text: &str, table: Table_, alias: &str, new: &str) -> Result<String> {
    let mut doc = parse_doc(text)?;
    let item = doc
        .get_mut(table.key())
        .and_then(Item::as_table_like_mut)
        .and_then(|t| t.get_mut(alias))
        .with_context(|| format!("catalog entry `{alias}` not found"))?;
    if let Some(v) = item.as_value_mut() {
        if let Some(s) = v.as_str() {
            // "g:a:1.0" / "id:1.0" shorthand.
            let mut parts: Vec<String> = s.split(':').map(String::from).collect();
            let idx = if table == Table_::Libraries { 2 } else { 1 };
            if parts.len() > idx {
                parts[idx] = new.to_string();
            } else {
                parts.push(new.to_string());
            }
            replace_string(v, &parts.join(":"));
            return Ok(doc.to_string());
        }
    }
    let tbl = item.as_table_like_mut().context("unsupported catalog entry")?;
    let ver = tbl.get_mut("version").with_context(|| format!("`{alias}` has no inline version"))?;
    if let Some(v) = ver.as_value_mut() {
        if v.is_str() {
            replace_string(v, new);
        } else if let Some(t) = v.as_inline_table_mut() {
            set_rich(t, new)?;
        }
    } else if let Some(t) = ver.as_table_like_mut() {
        for k in ["strictly", "require", "prefer"] {
            if let Some(v) = t.get_mut(k).and_then(Item::as_value_mut) {
                replace_string(v, new);
                break;
            }
        }
    }
    Ok(doc.to_string())
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Item {
    if !doc.contains_key(key) {
        let mut t = Table::new();
        t.set_implicit(false);
        doc.insert(key, Item::Table(t));
    }
    doc.get_mut(key).expect("just inserted")
}

/// Picks a catalog alias for `coord` that does not clash with existing entries.
pub fn pick_alias(cat: &Catalog, coord: &Coord) -> String {
    let base = coord.artifact.to_ascii_lowercase().replace(['_', '.'], "-");
    if let Some(existing) = cat.libraries.iter().find(|l| l.alias == base) {
        if existing.coord == *coord {
            return base;
        }
        let prefix = coord.group.rsplit('.').next().unwrap_or("lib").to_ascii_lowercase();
        let candidate = format!("{prefix}-{base}");
        if !cat.libraries.iter().any(|l| l.alias == candidate) {
            return candidate;
        }
        let mut i = 2;
        loop {
            let c = format!("{candidate}-{i}");
            if !cat.libraries.iter().any(|l| l.alias == c) {
                return c;
            }
            i += 1;
        }
    }
    base
}

/// Adds `alias = { module = "g:a", version.ref = "alias" }` and the matching
/// `[versions]` entry. Reuses an identical version key when present. Pass
/// `version = None` for BOM-managed libraries.
pub fn add_library(text: &str, alias: &str, coord: &Coord, version: Option<&str>) -> Result<String> {
    let mut doc = parse_doc(text)?;
    let mut entry = InlineTable::new();
    entry.insert("module", Value::from(coord.to_string()));
    if let Some(version) = version {
        // Prefer an existing version key with the same value to avoid duplicates.
        let cat = parse(text)?;
        let key = if cat.versions.get(alias).map(String::as_str) == Some(version) || !cat.versions.contains_key(alias) {
            alias.to_string()
        } else {
            let mut i = 2;
            loop {
                let k = format!("{alias}-{i}");
                if !cat.versions.contains_key(&k) {
                    break k;
                }
                i += 1;
            }
        };
        if !cat.versions.contains_key(&key) {
            ensure_table(&mut doc, "versions")
                .as_table_like_mut()
                .context("`versions` is not a table")?
                .insert(&key, toml_edit::value(version));
        }
        let mut r = InlineTable::new();
        r.insert("ref", Value::from(key));
        r.set_dotted(true);
        entry.insert("version", Value::InlineTable(r));
    }
    entry.fmt();
    ensure_table(&mut doc, "libraries")
        .as_table_like_mut()
        .context("`libraries` is not a table")?
        .insert(alias, Item::Value(Value::InlineTable(entry)));
    Ok(doc.to_string())
}

pub fn remove_library(text: &str, alias: &str) -> Result<String> {
    let mut doc = parse_doc(text)?;
    if let Some(t) = doc.get_mut("libraries").and_then(Item::as_table_like_mut) {
        t.remove(alias);
    }
    // Drop the alias from bundles as well.
    if let Some(bundles) = doc.get_mut("bundles").and_then(Item::as_table_like_mut) {
        for (_, item) in bundles.iter_mut() {
            if let Some(arr) = item.as_array_mut() {
                let idx: Vec<usize> =
                    arr.iter().enumerate().filter(|(_, v)| v.as_str() == Some(alias)).map(|(i, _)| i).collect();
                for i in idx.into_iter().rev() {
                    arr.remove(i);
                }
            }
        }
    }
    Ok(doc.to_string())
}

pub fn remove_version_key(text: &str, key: &str) -> Result<String> {
    let mut doc = parse_doc(text)?;
    if let Some(t) = doc.get_mut("versions").and_then(Item::as_table_like_mut) {
        t.remove(key);
    }
    Ok(doc.to_string())
}

/// Whether any library/plugin entry still references the version key.
pub fn version_key_in_use(cat: &Catalog, key: &str) -> bool {
    cat.libraries.iter().any(|l| l.version == CatVersion::Ref(key.to_string()))
        || cat.plugins.iter().any(|p| p.version == CatVersion::Ref(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# catalog
[versions]
kotlin = "2.0.0" # the compiler
hikari = "5.1.0"

[libraries]
hikari = { module = "com.zaxxer:HikariCP", version.ref = "hikari" }
guava = "com.google.guava:guava:33.0.0-jre"
slf4j = { group = "org.slf4j", name = "slf4j-api", version = "2.0.9" }
junit-bom = { module = "org.junit:junit-bom" }

[plugins]
kotlin-jvm = { id = "org.jetbrains.kotlin.jvm", version.ref = "kotlin" }

[bundles]
db = ["hikari", "guava"]
"#;

    #[test]
    fn parses_all_forms() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.versions["kotlin"], "2.0.0");
        assert_eq!(c.libraries.len(), 4);
        assert_eq!(c.libraries[0].version, CatVersion::Ref("hikari".into()));
        assert_eq!(c.libraries[1].version, CatVersion::Literal("33.0.0-jre".into()));
        assert_eq!(c.libraries[2].coord, Coord::new("org.slf4j", "slf4j-api"));
        assert_eq!(c.libraries[3].version, CatVersion::None);
        assert_eq!(c.plugins[0].id, "org.jetbrains.kotlin.jvm");
        assert_eq!(c.bundles["db"], vec!["hikari", "guava"]);
    }

    #[test]
    fn version_edit_keeps_comments() {
        let out = set_version_key(SAMPLE, "kotlin", "2.1.0").unwrap();
        assert!(out.contains("kotlin = \"2.1.0\" # the compiler"));
        assert!(out.starts_with("# catalog"));
    }

    #[test]
    fn inline_edits() {
        let out = set_inline_version(SAMPLE, Table_::Libraries, "guava", "33.1.0-jre").unwrap();
        assert!(out.contains("guava = \"com.google.guava:guava:33.1.0-jre\""));
        let out = set_inline_version(SAMPLE, Table_::Libraries, "slf4j", "2.0.10").unwrap();
        assert!(out.contains("version = \"2.0.10\""));
    }

    #[test]
    fn adds_library_with_version() {
        let out = add_library(SAMPLE, "jedis", &Coord::new("redis.clients", "jedis"), Some("5.1.0")).unwrap();
        assert!(out.contains("jedis = \"5.1.0\""), "{out}");
        assert!(out.contains("jedis = { module = \"redis.clients:jedis\", version.ref = \"jedis\" }"), "{out}");
        // Everything else is untouched.
        assert!(out.contains("kotlin = \"2.0.0\" # the compiler"));
    }

    #[test]
    fn adds_to_empty_catalog() {
        let out = add_library("", "gson", &Coord::new("com.google.code.gson", "gson"), Some("2.10")).unwrap();
        let c = parse(&out).unwrap();
        assert_eq!(c.versions["gson"], "2.10");
        assert_eq!(c.libraries[0].alias, "gson");
    }

    #[test]
    fn removal() {
        let out = remove_library(SAMPLE, "hikari").unwrap();
        let c = parse(&out).unwrap();
        assert!(c.libraries.iter().all(|l| l.alias != "hikari"));
        assert_eq!(c.bundles["db"], vec!["guava"]);
        assert!(!version_key_in_use(&c, "hikari"));
        let out = remove_version_key(&out, "hikari").unwrap();
        assert!(!parse(&out).unwrap().versions.contains_key("hikari"));
    }

    #[test]
    fn alias_collision() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(pick_alias(&c, &Coord::new("com.zaxxer", "HikariCP")), "hikaricp");
        assert_eq!(pick_alias(&c, &Coord::new("other.group", "guava")), "group-guava");
        assert_eq!(pick_alias(&c, &Coord::new("com.google.guava", "guava")), "guava");
    }

    #[test]
    fn accessor_names() {
        assert_eq!(accessor("kotlin-jvm_x"), "kotlin.jvm.x");
    }
}
