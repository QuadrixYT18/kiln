//! Shared data model for projects and dependencies.

use std::ops::Range;
use std::path::PathBuf;

use crate::registry::Coord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scope {
    #[default]
    Compile,
    Test,
    Runtime,
    CompileOnly,
    AnnotationProcessor,
}

impl Scope {
    pub fn gradle_config(self) -> &'static str {
        match self {
            Scope::Compile => "implementation",
            Scope::Test => "testImplementation",
            Scope::Runtime => "runtimeOnly",
            Scope::CompileOnly => "compileOnly",
            Scope::AnnotationProcessor => "annotationProcessor",
        }
    }

    /// Maven `<scope>`; `None` means the default (compile).
    pub fn maven_scope(self) -> Option<&'static str> {
        match self {
            Scope::Compile => None,
            Scope::Test => Some("test"),
            Scope::Runtime => Some("runtime"),
            Scope::CompileOnly | Scope::AnnotationProcessor => Some("provided"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    Gradle,
    Maven,
}

#[derive(Debug, Clone)]
pub struct Module {
    /// Gradle path (`:app`, `:` for the root) or Maven artifactId.
    pub name: String,
    pub dir: PathBuf,
    pub build_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub kind: BuildKind,
    pub modules: Vec<Module>,
    /// `gradle/libs.versions.toml` when it exists.
    pub catalog: Option<PathBuf>,
    /// `settings.gradle(.kts)`.
    pub settings: Option<PathBuf>,
}

impl Project {
    pub fn is_multi_module(&self) -> bool {
        self.modules.len() > 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DepKind {
    Library,
    /// Gradle plugin (resolved through its marker artifact).
    Plugin,
    /// Maven build plugin.
    MavenPlugin,
    /// Maven `<parent>`.
    Parent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogTable {
    Libraries,
    Plugins,
}

/// Where the version of a dependency lives, i.e. what an update has to rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VersionSite {
    /// A literal in a text file (Gradle string, `<version>`, property, `val`).
    Text { file: PathBuf, start: usize, end: usize },
    /// `[versions]` key in the version catalog.
    CatalogVersion { file: PathBuf, key: String },
    /// Version declared inline in a catalog `[libraries]` / `[plugins]` entry.
    CatalogInline { file: PathBuf, plugin: bool, alias: String },
    /// No version in the build file (managed by a BOM / plugin).
    Managed,
    /// Dynamic or unresolvable selector (`1.+`, `[1,2)`, unknown variable).
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub coord: Coord,
    pub kind: DepKind,
    pub current: Option<String>,
    pub site: VersionSite,
    /// Human-friendly place of declaration (module name, `libs.versions.toml`, …).
    pub module: String,
    pub file: PathBuf,
    pub config: Option<String>,
}

impl Dependency {
    pub fn display_name(&self) -> String {
        match self.kind {
            DepKind::Plugin => format!("plugin {}", self.coord.group),
            DepKind::MavenPlugin => format!("plugin {}", self.coord),
            DepKind::Parent => format!("parent {}", self.coord),
            DepKind::Library => self.coord.to_string(),
        }
    }
}
