//! `pom.xml` scanning and surgical editing.
//!
//! The scanner records byte ranges for everything we may touch; edits are
//! splices into the original text, so indentation, comments and line endings
//! stay exactly as the user wrote them.

use std::collections::BTreeMap;
use std::ops::Range;

use anyhow::{Result, bail};

use super::model::Scope;
use super::xml::{Ev, escape, events, unescape};
use crate::registry::Coord;
use crate::util::text::{LineEnding, line_end_inclusive, line_indent, line_start};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub value: String,
    /// Range of the (trimmed) text inside the element.
    pub range: Range<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct PomDep {
    pub group: Option<Text>,
    pub artifact: Option<Text>,
    pub version: Option<Text>,
    pub scope: Option<Text>,
    pub kind: Option<Text>,
    /// Whole `<dependency>…</dependency>` element.
    pub element: Range<usize>,
    pub managed: bool,
}

impl PomDep {
    pub fn coord(&self) -> Option<Coord> {
        Some(Coord::new(self.group.as_ref()?.value.clone(), self.artifact.as_ref()?.value.clone()))
    }
}

#[derive(Debug, Clone)]
pub struct PomPlugin {
    pub group: String,
    pub artifact: String,
    pub version: Option<Text>,
}

#[derive(Debug, Clone, Default)]
pub struct Pom {
    pub group_id: Option<String>,
    pub artifact_id: Option<String>,
    pub parent: Option<PomPlugin>,
    pub dependencies: Vec<PomDep>,
    pub plugins: Vec<PomPlugin>,
    pub properties: BTreeMap<String, Text>,
    pub modules: Vec<String>,
    pub repositories: Vec<String>,
    /// Offset of `<dependencies>` of the top-level `<project><dependencies>`.
    pub deps_open: Option<usize>,
    /// Offset of `</dependencies>` of the top-level `<project><dependencies>`.
    pub deps_close: Option<usize>,
    /// Offset of `</properties>` of `<project><properties>`.
    pub props_close: Option<usize>,
    /// Offset of `</repositories>`.
    pub repos_close: Option<usize>,
    /// Offset of `</project>`.
    pub project_close: Option<usize>,
    /// End offset of `<project …>` start tag.
    pub project_open_end: Option<usize>,
    /// Offset of `<build>` if present (top-level).
    pub build_open: Option<usize>,
    /// End of the last of modelVersion/groupId/artifactId/version/packaging/name/description at top level.
    pub header_end: Option<usize>,
}

fn trimmed(text: &str, start: usize, end: usize) -> Text {
    let raw = &text[start..end];
    let lead = raw.len() - raw.trim_start().len();
    let trail = raw.len() - raw.trim_end().len();
    let (s, e) = (start + lead, end - trail);
    let (s, e) = if s <= e { (s, e) } else { (start, start) };
    Text { value: unescape(&text[s..e]), range: s..e }
}

pub fn scan(text: &str) -> Result<Pom> {
    let evs = events(text)?;
    let mut pom = Pom::default();
    let mut path: Vec<String> = Vec::new();
    let mut cur_dep: Option<(PomDep, usize)> = None;
    let mut cur_plugin: Option<(String, Option<String>, Option<Text>)> = None;
    let mut cur_parent: Option<(String, String, Option<Text>)> = None;
    let mut pending_start: Option<usize> = None; // start offset of current element's start tag end
    let _ = &mut pending_start;

    for ev in &evs {
        match ev {
            Ev::Start { name, start, end } => {
                let parent_path = path.join("/");
                if name == "dependency"
                    && matches!(
                        parent_path.as_str(),
                        "project/dependencies" | "project/dependencyManagement/dependencies"
                    )
                {
                    cur_dep = Some((
                        PomDep { managed: parent_path.contains("dependencyManagement"), ..Default::default() },
                        *start,
                    ));
                }
                if name == "plugin"
                    && matches!(
                        parent_path.as_str(),
                        "project/build/plugins" | "project/build/pluginManagement/plugins"
                    )
                {
                    cur_plugin = Some((String::new(), None, None));
                    cur_plugin.as_mut().unwrap().0 = "org.apache.maven.plugins".into();
                }
                if name == "parent" && parent_path == "project" {
                    cur_parent = Some((String::new(), String::new(), None));
                }
                if name == "project" && path.is_empty() {
                    pom.project_open_end = Some(*end);
                }
                if name == "dependencies" && parent_path == "project" {
                    pom.deps_open = Some(*start);
                }
                if name == "build" && parent_path == "project" {
                    pom.build_open = Some(*start);
                }
                path.push(name.clone());
            }
            Ev::End { name, start, end } => {
                let full = path.join("/");
                match full.as_str() {
                    "project/dependencies" => pom.deps_close = Some(*start),
                    "project/properties" => pom.props_close = Some(*start),
                    "project/repositories" => pom.repos_close = Some(*start),
                    "project" => pom.project_close = Some(*start),
                    _ => {}
                }
                if path.len() == 2
                    && path[0] == "project"
                    && matches!(
                        name.as_str(),
                        "modelVersion"
                            | "groupId"
                            | "artifactId"
                            | "version"
                            | "packaging"
                            | "name"
                            | "description"
                            | "url"
                            | "parent"
                    )
                {
                    pom.header_end = Some(*end);
                }
                if name == "dependency"
                    && let Some((mut dep, s)) = cur_dep.take()
                {
                    dep.element = s..*end;
                    pom.dependencies.push(dep);
                }
                if name == "plugin"
                    && let Some((g, a, v)) = cur_plugin.take()
                    && let Some(a) = a
                {
                    pom.plugins.push(PomPlugin { group: g, artifact: a, version: v });
                }
                if name == "parent"
                    && let Some((g, a, v)) = cur_parent.take()
                    && !g.is_empty()
                {
                    pom.parent = Some(PomPlugin { group: g, artifact: a, version: v });
                }
                path.pop();
            }
            Ev::Empty { .. } => {}
            Ev::Text { start, end } => {
                let t = trimmed(text, *start, *end);
                if t.value.is_empty() {
                    continue;
                }
                let full = path.join("/");
                let leaf = path.last().map(String::as_str).unwrap_or("");
                if let Some((dep, _)) = cur_dep.as_mut() {
                    let depth_ok = full.ends_with(&format!("dependency/{leaf}"));
                    if depth_ok {
                        match leaf {
                            "groupId" => dep.group = Some(t.clone()),
                            "artifactId" => dep.artifact = Some(t.clone()),
                            "version" => dep.version = Some(t.clone()),
                            "scope" => dep.scope = Some(t.clone()),
                            "type" => dep.kind = Some(t.clone()),
                            _ => {}
                        }
                    }
                    continue;
                }
                if let Some((g, a, v)) = cur_plugin.as_mut() {
                    if full.ends_with(&format!("plugin/{leaf}")) {
                        match leaf {
                            "groupId" => *g = t.value.clone(),
                            "artifactId" => *a = Some(t.value.clone()),
                            "version" => *v = Some(t.clone()),
                            _ => {}
                        }
                    }
                    continue;
                }
                if let Some((g, a, v)) = cur_parent.as_mut() {
                    if full.ends_with(&format!("parent/{leaf}")) {
                        match leaf {
                            "groupId" => *g = t.value.clone(),
                            "artifactId" => *a = t.value.clone(),
                            "version" => *v = Some(t.clone()),
                            _ => {}
                        }
                    }
                    continue;
                }
                match full.as_str() {
                    "project/groupId" => pom.group_id = Some(t.value),
                    "project/artifactId" => pom.artifact_id = Some(t.value),
                    "project/modules/module" => pom.modules.push(t.value),
                    "project/repositories/repository/url" => pom.repositories.push(t.value),
                    p if p.starts_with("project/properties/") && p.matches('/').count() == 2 => {
                        pom.properties.insert(leaf.to_string(), t);
                    }
                    _ => {}
                }
            }
        }
    }
    if pom.project_close.is_none() {
        bail!("pom.xml has no <project> root element");
    }
    Ok(pom)
}

/// Resolves `${prop}` references against `props` (single level + chains).
pub fn resolve_property<'a>(value: &str, props: &'a BTreeMap<String, Text>) -> Option<(String, &'a Text)> {
    let name = value.strip_prefix("${")?.strip_suffix('}')?;
    let t = props.get(name)?;
    if t.value.starts_with("${") {
        return resolve_property(&t.value, props);
    }
    Some((name.to_string(), t))
}

fn unit_indent(text: &str) -> String {
    // Detect whether the file uses tabs or N spaces for the first nested level.
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('\t')
            && rest.starts_with('<')
        {
            return "\t".into();
        }
        let n = line.len() - line.trim_start_matches(' ').len();
        if n > 0 && line.trim_start().starts_with('<') {
            return " ".repeat(n);
        }
    }
    "  ".into()
}

/// Ensures `<project>` has the parts we need and returns the edited text.
pub struct PomEditor {
    pub text: String,
    eol: LineEnding,
}

impl PomEditor {
    pub fn new(text: &str) -> Self {
        let eol = LineEnding::detect(text);
        Self { text: text.to_string(), eol }
    }

    fn nl(&self) -> &'static str {
        self.eol.as_str()
    }

    /// Adds a dependency. When `version_property` is given, the version is
    /// written as `${name}` and the property is added/updated in `<properties>`.
    pub fn add_dependency(
        &mut self,
        coord: &Coord,
        version: Option<&str>,
        scope: Scope,
        version_property: Option<&str>,
    ) -> Result<()> {
        let unit = unit_indent(&self.text);
        let pom = scan(&self.text)?;
        let nl = self.nl().to_string();

        if let (Some(prop), Some(version)) = (version_property, version) {
            self.set_property(prop, version)?;
        }
        let pom_after = if version_property.is_some() { scan(&self.text)? } else { pom };

        let (indent_dep, deps_close) = match pom_after.deps_close {
            Some(close) => {
                let open_indent = line_indent(&self.text, close).to_string();
                // Indent of existing entries if there are any, else one level deeper.
                let existing = pom_after
                    .dependencies
                    .iter()
                    .find(|d| !d.managed)
                    .map(|d| line_indent(&self.text, d.element.start).to_string());
                (existing.unwrap_or_else(|| format!("{open_indent}{unit}")), Some(close))
            }
            None => (String::new(), None),
        };

        let inner = format!("{indent_dep}{unit}");
        let mut block = String::new();
        block.push_str(&format!("{indent_dep}<dependency>{nl}"));
        block.push_str(&format!("{inner}<groupId>{}</groupId>{nl}", escape(&coord.group)));
        block.push_str(&format!("{inner}<artifactId>{}</artifactId>{nl}", escape(&coord.artifact)));
        if let Some(v) = version {
            let v = version_property.map(|p| format!("${{{p}}}")).unwrap_or_else(|| escape(v));
            block.push_str(&format!("{inner}<version>{v}</version>{nl}"));
        }
        if let Some(s) = scope.maven_scope() {
            block.push_str(&format!("{inner}<scope>{s}</scope>{nl}"));
        }
        block.push_str(&format!("{indent_dep}</dependency>{nl}"));

        match deps_close {
            Some(close) => {
                // Insert before the indentation of `</dependencies>`.
                let ls = line_start(&self.text, close);
                let only_ws = self.text[ls..close].trim().is_empty();
                if only_ws {
                    self.text.insert_str(ls, &block);
                } else {
                    // `</dependencies>` shares a line with other content.
                    let ins = format!("{nl}{}{nl}{}", block.trim_end(), line_indent(&self.text, close));
                    self.text.insert_str(close, &ins);
                }
            }
            None => {
                let base = pom_after.project_close.map(|c| line_indent(&self.text, c).to_string()).unwrap_or_default();
                let ind1 = format!("{base}{unit}");
                // Re-indent properly: block was built with empty indent_dep.
                let mut section2 = format!("{ind1}<dependencies>{nl}");
                for line in block.lines() {
                    section2.push_str(&format!("{ind1}{unit}{line}{nl}"));
                }
                section2.push_str(&format!("{ind1}</dependencies>{nl}"));
                let at = match pom_after.build_open {
                    Some(b) => line_start(&self.text, b),
                    None => pom_after
                        .project_close
                        .map(|c| line_start(&self.text, c))
                        .ok_or_else(|| anyhow::anyhow!("no </project>"))?,
                };
                self.text.insert_str(at, &section2);
            }
        }
        Ok(())
    }

    /// Adds or updates `<properties><name>value</name></properties>`.
    pub fn set_property(&mut self, name: &str, value: &str) -> Result<()> {
        let pom = scan(&self.text)?;
        let nl = self.nl().to_string();
        let unit = unit_indent(&self.text);
        if let Some(existing) = pom.properties.get(name) {
            self.text.replace_range(existing.range.clone(), &escape(value));
            return Ok(());
        }
        let base = pom.project_close.map(|c| line_indent(&self.text, c).to_string()).unwrap_or_default();
        if let Some(close) = pom.props_close {
            let ind = pom
                .properties
                .values()
                .next()
                .map(|t| line_indent(&self.text, t.range.start).to_string())
                .unwrap_or_else(|| format!("{}{unit}", line_indent(&self.text, close)));
            let line = format!("{ind}<{name}>{}</{name}>{nl}", escape(value));
            let ls = line_start(&self.text, close);
            if self.text[ls..close].trim().is_empty() {
                self.text.insert_str(ls, &line);
            } else {
                self.text.insert_str(close, &format!("{nl}{line}{}", line_indent(&self.text, close)));
            }
            return Ok(());
        }
        let ind1 = format!("{base}{unit}");
        let ind2 = format!("{ind1}{unit}");
        let section =
            format!("{ind1}<properties>{nl}{ind2}<{name}>{}</{name}>{nl}{ind1}</properties>{nl}", escape(value));
        let at = match pom.header_end {
            Some(end) => {
                let after = line_end_inclusive(&self.text, end);
                // Keep a blank-line-free insertion right after the header block.
                after
            }
            None => pom
                .project_open_end
                .map(|e| line_end_inclusive(&self.text, e))
                .ok_or_else(|| anyhow::anyhow!("no <project>"))?,
        };
        self.text.insert_str(at, &section);
        Ok(())
    }

    /// Adds `<repository>` unless the URL is already declared.
    pub fn ensure_repository(&mut self, url: &str, id: &str) -> Result<bool> {
        let pom = scan(&self.text)?;
        let norm = |s: &str| s.trim().trim_end_matches('/').to_ascii_lowercase();
        if pom.repositories.iter().any(|r| norm(r) == norm(url)) {
            return Ok(false);
        }
        let nl = self.nl().to_string();
        let unit = unit_indent(&self.text);
        let base = pom.project_close.map(|c| line_indent(&self.text, c).to_string()).unwrap_or_default();
        let ind1 = format!("{base}{unit}");
        let ind2 = format!("{ind1}{unit}");
        let ind3 = format!("{ind2}{unit}");
        let repo = format!(
            "{ind2}<repository>{nl}{ind3}<id>{}</id>{nl}{ind3}<url>{}</url>{nl}{ind2}</repository>{nl}",
            escape(id),
            escape(url)
        );
        if let Some(close) = pom.repos_close {
            let ls = line_start(&self.text, close);
            self.text.insert_str(ls, &repo);
        } else {
            let section = format!("{ind1}<repositories>{nl}{repo}{ind1}</repositories>{nl}");
            let at = pom
                .deps_open
                .or(pom.build_open)
                .or(pom.project_close)
                .map(|c| line_start(&self.text, c))
                .unwrap_or(self.text.len());
            self.text.insert_str(at, &section);
        }
        Ok(true)
    }

    /// Removes the dependency element (and its line when it sits alone on it).
    pub fn remove_element(&mut self, range: Range<usize>) {
        let ls = line_start(&self.text, range.start);
        let le = line_end_inclusive(&self.text, range.end);
        let before_ws = self.text[ls..range.start].trim().is_empty();
        let after_ws = self.text[range.end..le].trim().is_empty();
        if before_ws && after_ws {
            self.text.replace_range(ls..le, "");
        } else {
            self.text.replace_range(range, "");
        }
    }

    pub fn remove_property(&mut self, name: &str) -> Result<()> {
        let pom = scan(&self.text)?;
        let Some(t) = pom.properties.get(name) else { return Ok(()) };
        // Expand to the whole `<name>…</name>` element.
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = self.text[..t.range.start].rfind(&open);
        let end = self.text[t.range.end..].find(&close).map(|i| t.range.end + i + close.len());
        if let (Some(s), Some(e)) = (start, end) {
            self.remove_element(s..e);
        }
        Ok(())
    }
}

/// Where `annotationProcessorPaths` can be added to the build.
#[derive(Debug, Default)]
struct CompilerSpots {
    /// `</annotationProcessorPaths>` of the compiler plugin configuration.
    paths_close: Option<usize>,
    /// `</configuration>` of the compiler plugin.
    config_close: Option<usize>,
    /// `</plugin>` of the compiler plugin.
    plugin_close: Option<usize>,
    /// `</plugins>` of `<project><build><plugins>`.
    plugins_close: Option<usize>,
    /// `</build>` of `<project><build>`.
    build_close: Option<usize>,
}

fn compiler_spots(text: &str) -> Result<CompilerSpots> {
    let evs = events(text)?;
    let mut spots = CompilerSpots::default();
    let mut path: Vec<String> = Vec::new();
    // Start offset of every currently open `plugin` element with its artifactId.
    let mut plugin_artifact: Option<String> = None;
    let mut in_compiler = false;
    for ev in &evs {
        match ev {
            Ev::Start { name, .. } => path.push(name.clone()),
            Ev::End { start, .. } => {
                let full = path.join("/");
                match full.as_str() {
                    "project/build/plugins" => spots.plugins_close = Some(*start),
                    "project/build" => spots.build_close = Some(*start),
                    "project/build/plugins/plugin" if in_compiler => spots.plugin_close = Some(*start),
                    "project/build/plugins/plugin/configuration" if in_compiler => spots.config_close = Some(*start),
                    "project/build/plugins/plugin/configuration/annotationProcessorPaths" if in_compiler => {
                        spots.paths_close = Some(*start)
                    }
                    _ => {}
                }
                if full == "project/build/plugins/plugin" {
                    plugin_artifact = None;
                    in_compiler = false;
                }
                path.pop();
            }
            Ev::Text { start, end } => {
                if path.join("/") == "project/build/plugins/plugin/artifactId" {
                    plugin_artifact = Some(unescape(&text[*start..*end]));
                    in_compiler = plugin_artifact.as_deref() == Some("maven-compiler-plugin");
                }
            }
            Ev::Empty { .. } => {}
        }
    }
    let _ = plugin_artifact;
    Ok(spots)
}

impl PomEditor {
    /// Registers an annotation processor in `maven-compiler-plugin`'s
    /// `annotationProcessorPaths`, creating the plugin/configuration if needed.
    ///
    /// Note: the `artifactId` of the compiler plugin must come before its
    /// `<configuration>`, which is the common layout.
    pub fn add_annotation_processor(&mut self, coord: &Coord, version: Option<&str>) -> Result<()> {
        let unit = unit_indent(&self.text);
        let nl = self.nl().to_string();
        let spots = compiler_spots(&self.text)?;
        let pom = scan(&self.text)?;
        let base = pom.project_close.map(|c| line_indent(&self.text, c).to_string()).unwrap_or_default();

        let path_block = |ind: &str| {
            let inner = format!("{ind}{unit}");
            let mut s = format!("{ind}<path>{nl}");
            s.push_str(&format!("{inner}<groupId>{}</groupId>{nl}", escape(&coord.group)));
            s.push_str(&format!("{inner}<artifactId>{}</artifactId>{nl}", escape(&coord.artifact)));
            if let Some(v) = version {
                s.push_str(&format!("{inner}<version>{}</version>{nl}", escape(v)));
            }
            s.push_str(&format!("{ind}</path>{nl}"));
            s
        };
        let insert_before_line = |text: &mut String, close: usize, block: &str| {
            let ls = line_start(text, close);
            if text[ls..close].trim().is_empty() {
                text.insert_str(ls, block);
            } else {
                text.insert_str(close, &format!("{nl}{}{}", block.trim_end(), nl));
            }
        };

        if let Some(close) = spots.paths_close {
            let ind = format!("{}{unit}", line_indent(&self.text, close));
            let block = path_block(&ind);
            insert_before_line(&mut self.text, close, &block);
            return Ok(());
        }
        if let Some(close) = spots.config_close {
            let ind1 = format!("{}{unit}", line_indent(&self.text, close));
            let ind2 = format!("{ind1}{unit}");
            let block = format!(
                "{ind1}<annotationProcessorPaths>{nl}{}{ind1}</annotationProcessorPaths>{nl}",
                path_block(&ind2)
            );
            insert_before_line(&mut self.text, close, &block);
            return Ok(());
        }
        let plugin_xml = |ind: &str| {
            let i1 = format!("{ind}{unit}");
            let i2 = format!("{i1}{unit}");
            let i3 = format!("{i2}{unit}");
            format!(
                "{ind}<plugin>{nl}{i1}<groupId>org.apache.maven.plugins</groupId>{nl}{i1}<artifactId>maven-compiler-plugin</artifactId>{nl}{i1}<configuration>{nl}{i2}<annotationProcessorPaths>{nl}{}{i2}</annotationProcessorPaths>{nl}{i1}</configuration>{nl}{ind}</plugin>{nl}",
                path_block(&i3)
            )
        };
        if let Some(close) = spots.plugin_close {
            // Compiler plugin without <configuration>.
            let i1 = format!("{}{unit}", line_indent(&self.text, close));
            let i2 = format!("{i1}{unit}");
            let i3 = format!("{i2}{unit}");
            let block = format!(
                "{i1}<configuration>{nl}{i2}<annotationProcessorPaths>{nl}{}{i2}</annotationProcessorPaths>{nl}{i1}</configuration>{nl}",
                path_block(&i3)
            );
            insert_before_line(&mut self.text, close, &block);
            return Ok(());
        }
        if let Some(close) = spots.plugins_close {
            let ind = format!("{}{unit}", line_indent(&self.text, close));
            insert_before_line(&mut self.text, close, &plugin_xml(&ind));
            return Ok(());
        }
        if let Some(close) = spots.build_close {
            let ind1 = format!("{}{unit}", line_indent(&self.text, close));
            let ind2 = format!("{ind1}{unit}");
            let block = format!("{ind1}<plugins>{nl}{}{ind1}</plugins>{nl}", plugin_xml(&ind2));
            insert_before_line(&mut self.text, close, &block);
            return Ok(());
        }
        let at =
            pom.project_close.map(|c| line_start(&self.text, c)).ok_or_else(|| anyhow::anyhow!("no </project>"))?;
        let ind1 = format!("{base}{unit}");
        let ind2 = format!("{ind1}{unit}");
        let ind3 = format!("{ind2}{unit}");
        let block =
            format!("{ind1}<build>{nl}{ind2}<plugins>{nl}{}{ind2}</plugins>{nl}{ind1}</build>{nl}", plugin_xml(&ind3));
        self.text.insert_str(at, &block);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>demo</artifactId>
    <version>1.0.0</version>

    <properties>
        <java.version>21</java.version>
        <hikari.version>5.0.1</hikari.version>
    </properties>

    <dependencies>
        <!-- pool -->
        <dependency>
            <groupId>com.zaxxer</groupId>
            <artifactId>HikariCP</artifactId>
            <version>${hikari.version}</version>
        </dependency>
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>5.10.0</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>
"#;

    #[test]
    fn scans_pom() {
        let p = scan(POM).unwrap();
        assert_eq!(p.dependencies.len(), 2);
        assert_eq!(p.dependencies[0].coord().unwrap(), Coord::new("com.zaxxer", "HikariCP"));
        assert_eq!(p.dependencies[1].scope.as_ref().unwrap().value, "test");
        assert_eq!(p.properties["hikari.version"].value, "5.0.1");
        let v = p.dependencies[0].version.as_ref().unwrap();
        let (name, t) = resolve_property(&v.value, &p.properties).unwrap();
        assert_eq!(name, "hikari.version");
        assert_eq!(&POM[t.range.clone()], "5.0.1");
    }

    #[test]
    fn adds_dependency_with_matching_indent() {
        let mut ed = PomEditor::new(POM);
        ed.add_dependency(&Coord::new("redis.clients", "jedis"), Some("5.1.0"), Scope::Compile, Some("jedis.version"))
            .unwrap();
        let out = ed.text;
        assert!(out.contains("        <dependency>\n            <groupId>redis.clients</groupId>\n            <artifactId>jedis</artifactId>\n            <version>${jedis.version}</version>\n        </dependency>\n    </dependencies>"), "{out}");
        assert!(out.contains("        <jedis.version>5.1.0</jedis.version>\n    </properties>"), "{out}");
        // Everything before is byte-identical.
        assert!(out.contains("<!-- pool -->"));
        let p = scan(&out).unwrap();
        assert_eq!(p.dependencies.len(), 3);
    }

    #[test]
    fn crlf_is_preserved() {
        let crlf = POM.replace('\n', "\r\n");
        let mut ed = PomEditor::new(&crlf);
        ed.add_dependency(&Coord::new("a", "b"), Some("1"), Scope::Test, None).unwrap();
        assert_eq!(ed.text.matches('\n').count(), ed.text.matches("\r\n").count());
        assert!(ed.text.contains("<scope>test</scope>"));
    }

    #[test]
    fn creates_dependencies_section() {
        let pom = "<project>\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>g</groupId>\n  <artifactId>a</artifactId>\n  <version>1</version>\n</project>\n";
        let mut ed = PomEditor::new(pom);
        ed.add_dependency(&Coord::new("x", "y"), Some("2"), Scope::Compile, None).unwrap();
        let p = scan(&ed.text).unwrap();
        assert_eq!(p.dependencies.len(), 1, "{}", ed.text);
        assert!(ed.text.contains("  <dependencies>\n    <dependency>\n      <groupId>x</groupId>"), "{}", ed.text);
    }

    #[test]
    fn removes_dependency_line() {
        let p = scan(POM).unwrap();
        let mut ed = PomEditor::new(POM);
        ed.remove_element(p.dependencies[1].element.clone());
        assert!(!ed.text.contains("junit-jupiter"));
        assert!(ed.text.contains("</dependency>\n    </dependencies>"));
    }

    #[test]
    fn removes_property() {
        let mut ed = PomEditor::new(POM);
        ed.remove_property("hikari.version").unwrap();
        assert!(!ed.text.contains("hikari.version>5"));
        assert!(ed.text.contains("<java.version>21</java.version>"));
    }

    #[test]
    fn annotation_processor_paths_in_all_layouts() {
        let coord = Coord::new("org.projectlombok", "lombok");
        // existing compiler plugin without configuration (POM fixture)
        let mut ed = PomEditor::new(POM);
        ed.add_annotation_processor(&coord, Some("1.18.30")).unwrap();
        assert!(ed.text.contains("<annotationProcessorPaths>"), "{}", ed.text);
        assert!(ed.text.contains("<artifactId>lombok</artifactId>"));
        assert!(scan(&ed.text).is_ok());
        // a second processor joins the same list
        ed.add_annotation_processor(&Coord::new("org.mapstruct", "mapstruct-processor"), Some("1.5.5.Final")).unwrap();
        assert_eq!(ed.text.matches("<annotationProcessorPaths>").count(), 1);
        assert_eq!(ed.text.matches("<path>").count(), 2);
        // no build section at all
        let bare = "<project>\n  <modelVersion>4.0.0</modelVersion>\n</project>\n";
        let mut ed = PomEditor::new(bare);
        ed.add_annotation_processor(&coord, None).unwrap();
        assert!(ed.text.contains("<build>") && ed.text.contains("maven-compiler-plugin"), "{}", ed.text);
        assert!(scan(&ed.text).is_ok());
        // build without plugins, and plugins without the compiler
        let mut ed = PomEditor::new("<project>\n  <build>\n  </build>\n</project>\n");
        ed.add_annotation_processor(&coord, None).unwrap();
        assert!(ed.text.contains("<plugins>"), "{}", ed.text);
        let mut ed = PomEditor::new(
            "<project>\n  <build>\n    <plugins>\n      <plugin>\n        <artifactId>other</artifactId>\n      </plugin>\n    </plugins>\n  </build>\n</project>\n",
        );
        ed.add_annotation_processor(&coord, None).unwrap();
        assert!(
            ed.text.contains("maven-compiler-plugin") && ed.text.contains("<artifactId>other</artifactId>"),
            "{}",
            ed.text
        );
        assert!(scan(&ed.text).is_ok());
    }

    #[test]
    fn adds_repository() {
        let mut ed = PomEditor::new(POM);
        assert!(ed.ensure_repository("https://repo.papermc.io/repository/maven-public/", "papermc").unwrap());
        assert!(!ed.ensure_repository("https://repo.papermc.io/repository/maven-public", "papermc").unwrap());
        assert_eq!(scan(&ed.text).unwrap().repositories.len(), 1);
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use crate::util::fuzz::{Rng, mutate, rounds};

    const SNIPPETS: &[&str] = &[
        "<",
        ">",
        "</",
        "/>",
        "&",
        "&amp;",
        "<!--",
        "-->",
        "<![CDATA[",
        "]]>",
        "ü",
        "\n",
        "\r\n",
        "<dependency>",
        "</dependencies>",
        "${",
        "}",
    ];
    const SEEDS: &[&str] = &[
        include_str!("../../tests/fixtures/maven/pom.xml"),
        include_str!("../../tests/fixtures/maven-multi/core/pom.xml"),
        include_str!("../../tests/fixtures/maven-multi/pom.xml"),
    ];

    #[test]
    fn pom_scanning_and_editing_never_panics_on_broken_input() {
        let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
        for round in 0..rounds() {
            let text = mutate(&mut rng, SEEDS[round % SEEDS.len()], SNIPPETS);
            let Ok(pom) = scan(&text) else { continue };
            let coord = Coord::new("a.b", "c");
            let mut ed = PomEditor::new(&text);
            let _ = ed.add_dependency(&coord, Some("1.0"), Scope::Test, Some("c.version"));
            let mut ed = PomEditor::new(&text);
            let _ = ed.ensure_repository("https://example.org/m", "example");
            let _ = ed.set_property("x.version", "2");
            let _ = ed.remove_property("x.version");
            for d in &pom.dependencies {
                let mut ed = PomEditor::new(&text);
                ed.remove_element(d.element.clone());
            }
        }
    }
}
