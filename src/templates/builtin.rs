//! Built-in templates, embedded at compile time from the repository's
//! `templates/` directory.

use super::render::{Vars, truthy};

pub struct BuiltinFile {
    /// Target path (itself a template, may use `{{package_path}}` etc.).
    pub target: &'static str,
    pub content: &'static str,
    /// Condition(s) joined by `&`, each optionally negated with `!` (`kotlin&!paperweight`).
    pub when: &'static str,
}

pub struct Builtin {
    pub id: &'static str,
    pub description: &'static str,
    pub default_description: &'static str,
    /// Languages offered for this template (first one is the default).
    pub langs: &'static [&'static str],
    /// Extra repositories for `{{latest:…}}` lookups.
    pub repos: &'static [&'static str],
    /// Variables derived from a version lookup before rendering.
    pub derived: &'static [(&'static str, &'static str)],
    pub files: Vec<BuiltinFile>,
}

const PAPER_REPO: &str = "https://repo.papermc.io/repository/maven-public/";

macro_rules! f {
    ($target:expr, $path:literal) => {
        BuiltinFile { target: $target, content: include_str!(concat!("../../templates/", $path)), when: "" }
    };
    ($target:expr, $path:literal, $when:expr) => {
        BuiltinFile { target: $target, content: include_str!(concat!("../../templates/", $path)), when: $when }
    };
}

pub fn all() -> Vec<Builtin> {
    vec![
        Builtin {
            id: "paper",
            description: "Paper (Minecraft) plugin with paper-plugin.yml, run-paper and optional paperweight",
            default_description: "A Paper plugin",
            langs: &["java", "kotlin"],
            repos: &[PAPER_REPO],
            derived: &[
                ("mc_version", "latest:io.papermc.paper:paper-api?snapshot|before:-R0"),
                ("api_version", "latest:io.papermc.paper:paper-api?snapshot|before:-R0|major_minor"),
            ],
            files: vec![
                f!("build.gradle.kts", "paper/build.gradle.kts"),
                f!("gradle/libs.versions.toml", "paper/libs.versions.toml"),
                f!("src/main/resources/paper-plugin.yml", "paper/paper-plugin.yml"),
                f!("src/main/java/{{package_path}}/{{class_name}}.java", "paper/Main.java", "!kotlin"),
                f!("src/main/kotlin/{{package_path}}/{{class_name}}.kt", "paper/Main.kt", "kotlin"),
            ],
        },
        Builtin {
            id: "velocity",
            description: "Velocity proxy plugin with annotation-processed plugin descriptor",
            default_description: "A Velocity proxy plugin",
            langs: &["java", "kotlin"],
            repos: &[PAPER_REPO],
            derived: &[],
            files: vec![
                f!("build.gradle.kts", "velocity/build.gradle.kts"),
                f!("gradle/libs.versions.toml", "velocity/libs.versions.toml"),
                f!("src/main/java/{{package_path}}/{{class_name}}.java", "velocity/Main.java", "!kotlin"),
                f!("src/main/kotlin/{{package_path}}/{{class_name}}.kt", "velocity/Main.kt", "kotlin"),
            ],
        },
        Builtin {
            id: "library",
            description: "Java or Kotlin library with JUnit 5 tests and maven-publish",
            default_description: "A JVM library",
            langs: &["java", "kotlin"],
            repos: &[],
            derived: &[],
            files: vec![
                f!("build.gradle.kts", "library/build.gradle.kts"),
                f!("gradle/libs.versions.toml", "library/libs.versions.toml"),
                f!("src/main/java/{{package_path}}/Greeter.java", "library/Greeter.java", "!kotlin"),
                f!("src/test/java/{{package_path}}/GreeterTest.java", "library/GreeterTest.java", "!kotlin"),
                f!("src/main/kotlin/{{package_path}}/Greeter.kt", "library/Greeter.kt", "kotlin"),
                f!("src/test/kotlin/{{package_path}}/GreeterTest.kt", "library/GreeterTest.kt", "kotlin"),
            ],
        },
        Builtin {
            id: "backend",
            description: "Backend service with Ktor (Kotlin) or Spring Boot (Java/Kotlin)",
            default_description: "A backend service",
            langs: &["kotlin", "java"],
            repos: &[],
            derived: &[],
            files: vec![
                // Ktor
                f!("build.gradle.kts", "backend/ktor/build.gradle.kts", "ktor"),
                f!("gradle/libs.versions.toml", "backend/ktor/libs.versions.toml", "ktor"),
                f!("src/main/kotlin/{{package_path}}/Application.kt", "backend/ktor/Application.kt", "ktor"),
                f!("src/main/resources/logback.xml", "backend/ktor/logback.xml", "ktor"),
                f!("src/test/kotlin/{{package_path}}/ApplicationTest.kt", "backend/ktor/ApplicationTest.kt", "ktor"),
                // Spring Boot
                f!("build.gradle.kts", "backend/spring/build.gradle.kts", "spring"),
                f!("gradle/libs.versions.toml", "backend/spring/libs.versions.toml", "spring"),
                f!(
                    "src/main/java/{{package_path}}/Application.java",
                    "backend/spring/Application.java",
                    "spring&!kotlin"
                ),
                f!(
                    "src/main/java/{{package_path}}/HelloController.java",
                    "backend/spring/HelloController.java",
                    "spring&!kotlin"
                ),
                f!(
                    "src/test/java/{{package_path}}/ApplicationTests.java",
                    "backend/spring/ApplicationTests.java",
                    "spring&!kotlin"
                ),
                f!("src/main/kotlin/{{package_path}}/Application.kt", "backend/spring/Application.kt", "spring&kotlin"),
                f!(
                    "src/main/kotlin/{{package_path}}/HelloController.kt",
                    "backend/spring/HelloController.kt",
                    "spring&kotlin"
                ),
                f!(
                    "src/test/kotlin/{{package_path}}/ApplicationTests.kt",
                    "backend/spring/ApplicationTests.kt",
                    "spring&kotlin"
                ),
            ],
        },
        Builtin {
            id: "empty",
            description: "Empty Gradle project with a version catalog",
            default_description: "A Gradle project",
            langs: &["java", "kotlin"],
            repos: &[],
            derived: &[],
            files: vec![
                f!("build.gradle.kts", "empty/build.gradle.kts"),
                f!("gradle/libs.versions.toml", "empty/libs.versions.toml"),
                f!("src/main/{{src_dir}}/.gitkeep", "empty/gitkeep"),
            ],
        },
    ]
}

pub fn find(id: &str) -> Option<Builtin> {
    all().into_iter().find(|b| b.id == id)
}

/// Evaluates a `when` condition against flag variables.
pub fn condition_holds(when: &str, vars: &Vars) -> bool {
    when.split('&').filter(|c| !c.is_empty()).all(|c| match c.strip_prefix('!') {
        Some(n) => !truthy(vars.get(n)),
        None => truthy(vars.get(c)),
    })
}

/// Shared files, written for every built-in template (and for custom templates
/// when the files do not already exist).
pub const SETTINGS: &str = include_str!("../../templates/common/settings.gradle.kts");
pub const GRADLE_PROPERTIES: &str = include_str!("../../templates/common/gradle.properties");
pub const EDITORCONFIG: &str = include_str!("../../templates/common/editorconfig");
pub const GITIGNORE: &str = include_str!("../../templates/common/gitignore");
pub const GITATTRIBUTES: &str = include_str!("../../templates/common/gitattributes");
pub const RENOVATE: &str = include_str!("../../templates/common/renovate.json");
pub const README: &str = include_str!("../../templates/common/README.md");
pub const GITHUB_CI: &str = include_str!("../../templates/common/github-ci.yml");
pub const GITLAB_CI: &str = include_str!("../../templates/common/gitlab-ci.yml");
pub const DOCKERFILE_BACKEND: &str = include_str!("../../templates/common/Dockerfile.backend");
pub const DOCKER_COMPOSE: &str = include_str!("../../templates/common/docker-compose.yml");
pub const LICENSE_MIT: &str = include_str!("../../templates/common/LICENSE-MIT.txt");
pub const LICENSE_APACHE: &str = include_str!("../../LICENSE-APACHE");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions() {
        let mut v = Vars::new();
        v.insert("kotlin".into(), "true".into());
        assert!(condition_holds("", &v));
        assert!(condition_holds("kotlin", &v));
        assert!(!condition_holds("!kotlin", &v));
        assert!(condition_holds("kotlin&!spring", &v));
        assert!(!condition_holds("kotlin&spring", &v));
    }

    #[test]
    fn ids_are_unique() {
        let all = all();
        let mut ids: Vec<_> = all.iter().map(|b| b.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
    }
}
