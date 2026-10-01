mod common;
use common::*;

#[test]
fn outdated_reports_levels() {
    let env = Env::new("gradle-catalog");
    let out = env.kiln().arg("outdated").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let s = stdout(&out);
    // slf4j 2.0.9: patch 2.0.13 available, 2.1.0-alpha1 ignored
    let slf4j = s.lines().find(|l| l.contains("org.slf4j:slf4j-api")).unwrap();
    assert!(slf4j.contains("2.0.13") && !slf4j.contains("alpha"), "{s}");
    // guava stays within the -jre family
    let guava = s.lines().find(|l| l.contains("com.google.guava:guava")).unwrap();
    assert!(guava.contains("32.1.3-jre") && guava.contains("33.2.1-jre") && !guava.contains("android"), "{s}");
    // junit has a major update
    let junit = s.lines().find(|l| l.contains("junit-jupiter")).unwrap();
    assert!(junit.contains("MAJOR") && junit.contains("6.0.1") && junit.contains("5.11.0"), "{s}");
    // plugins are checked through their marker artifacts
    let shadow = s.lines().find(|l| l.contains("plugin com.gradleup.shadow")).unwrap();
    assert!(shadow.contains("8.3.5") && shadow.contains("9.0.0"), "{s}");
    assert!(s.contains("4 dependencies checked, 4 with updates (3 major)"), "{s}");
}

#[test]
fn outdated_only_outdated_and_up_to_date() {
    let env = Env::new("gradle-catalog");
    env.mock.artifact("/maven2", "org.slf4j", "slf4j-api", &["2.0.9"]);
    let out = env.kiln().args(["outdated", "--only-outdated"]).output().unwrap();
    assert!(!stdout(&out).contains("slf4j"));
    let out = env.kiln().arg("outdated").output().unwrap();
    assert!(stdout(&out).contains("up to date"));
}

#[test]
fn outdated_inline_gradle_versions_and_variables() {
    let env = Env::new("gradle-kts");
    let s = stdout(&env.kiln().arg("outdated").output().unwrap());
    assert!(s.contains("org.slf4j:slf4j-api"), "{s}");
    assert!(s.contains("32.0.0-jre"), "{s}");
}

#[test]
fn update_minor_applies_catalog_changes_and_keeps_formatting() {
    let env = Env::new("gradle-catalog");
    let old = env.read("gradle/libs.versions.toml");
    let out = env.kiln().args(["update", "--minor"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let cat = env.read("gradle/libs.versions.toml");
    assert!(cat.starts_with("# Version catalog\n"));
    assert!(cat.contains("slf4j = \"2.0.13\""), "{cat}");
    assert!(cat.contains("guava = \"32.1.3-jre\""), "{cat}");
    assert!(cat.contains("junit = \"5.11.0\""), "{cat}");
    assert!(cat.contains("shadow = \"8.3.5\""), "{cat}");
    // only version strings changed
    assert_eq!(old.lines().count(), cat.lines().count());
    assert_eq!(
        env.read("build.gradle.kts"),
        std::fs::read_to_string(Env::new("gradle-catalog").path().join("build.gradle.kts")).unwrap()
    );
}

#[test]
fn update_patch_only() {
    let env = Env::new("gradle-catalog");
    env.kiln().args(["update", "--patch"]).assert().success();
    let cat = env.read("gradle/libs.versions.toml");
    assert!(cat.contains("slf4j = \"2.0.13\""), "{cat}");
    assert!(cat.contains("junit = \"5.10.3\""), "{cat}");
    assert!(cat.contains("guava = \"32.0.0-jre\""), "no patch available for guava: {cat}");
}

#[test]
fn update_all_includes_major_and_shows_release_notes() {
    let env = Env::new("gradle-catalog");
    let out = env.kiln().args(["update", "--all"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let s = stdout(&out);
    assert!(s.contains("major update"), "{s}");
    assert!(s.contains("JUnit 6.0.1") && s.contains("Removed deprecated APIs"), "{s}");
    let cat = env.read("gradle/libs.versions.toml");
    assert!(cat.contains("junit = \"6.0.1\""), "{cat}");
    assert!(cat.contains("guava = \"33.2.1-jre\""), "{cat}");
    assert!(cat.contains("shadow = \"9.0.0\""), "{cat}");
}

#[test]
fn update_inline_versions_and_variables_in_kts() {
    let env = Env::new("gradle-kts");
    let old = env.read("build.gradle.kts");
    env.kiln().args(["update", "--minor"]).assert().success();
    let new = env.read("build.gradle.kts");
    assert!(new.contains("val guavaVersion = \"32.1.3-jre\""), "{new}");
    assert!(new.contains("implementation(\"org.slf4j:slf4j-api:2.0.13\")"), "{new}");
    assert!(new.contains("// logging") && new.contains("// fat jar"));
    assert!(new.contains("id(\"com.gradleup.shadow\") version \"8.3.5\" // fat jar"), "{new}");
    assert_eq!(old.lines().count(), new.lines().count());
}

#[test]
fn update_dry_run_writes_nothing() {
    let env = Env::new("gradle-catalog");
    let old = env.read("gradle/libs.versions.toml");
    let out = env.kiln().args(["update", "--all", "--dry-run"]).output().unwrap();
    assert!(out.status.success());
    let s = stdout(&out);
    assert!(s.contains("-slf4j = \"2.0.9\"") && s.contains("+slf4j = \"2.0.13\""), "{s}");
    assert_eq!(env.read("gradle/libs.versions.toml"), old);
}

#[test]
fn update_filter_by_name() {
    let env = Env::new("gradle-catalog");
    env.kiln().args(["update", "guava", "--minor"]).assert().success();
    let cat = env.read("gradle/libs.versions.toml");
    assert!(cat.contains("guava = \"32.1.3-jre\""));
    assert!(cat.contains("slf4j = \"2.0.9\""));
}

#[test]
fn update_without_flags_needs_a_terminal() {
    let env = Env::new("gradle-catalog");
    let out = env.kiln().arg("update").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--patch"));
}

#[test]
fn update_maven_properties_plugins_and_inline() {
    let env = Env::new("maven");
    env.kiln().args(["update", "--minor"]).assert().success();
    let pom = env.read("pom.xml");
    assert!(pom.contains("<slf4j.version>2.0.13</slf4j.version>"), "{pom}");
    assert!(pom.contains("<version>5.11.0</version>"), "{pom}");
    assert!(pom.contains("<version>3.13.0</version>"), "{pom}");
    assert!(pom.contains("<!-- logging -->"));
}

#[test]
fn update_maven_property_defined_in_parent_pom() {
    let env = Env::new("maven-multi");
    env.kiln().args(["update", "--minor"]).assert().success();
    assert!(env.read("pom.xml").contains("<jackson.version>2.17.1</jackson.version>"));
    assert!(env.read("core/pom.xml").contains("${jackson.version}"));
}

#[test]
fn update_multi_module_gradle_resolves_conflicts() {
    let env = Env::new("multi-module");
    let out = env.kiln().arg("outdated").output().unwrap();
    assert!(stdout(&out).contains("slf4j-api"));
    env.kiln().args(["update", "--minor"]).assert().success();
    assert!(env.read("gradle/libs.versions.toml").contains("guava = \"32.1.3-jre\""));
    assert!(env.read("lib/build.gradle.kts").contains("slf4j-api:2.0.13"));
}

#[test]
fn offline_mode_uses_the_cache() {
    let env = Env::new("gradle-catalog");
    env.kiln().arg("outdated").assert().success();
    let hits = env.mock.hits();
    assert!(hits > 0);
    // Second run must be served entirely from the cache.
    let out = env.kiln().args(["--offline", "outdated"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout(&out).contains("2.0.13"));
    assert_eq!(env.mock.hits(), hits, "offline run must not touch the network");
}

#[test]
fn offline_without_cache_reports_a_clear_error() {
    let env = Env::new("gradle-catalog");
    let out = env.kiln().args(["--offline", "outdated"]).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("offline"), "{err}");
}

#[test]
fn remove_gradle_catalog_dependency() {
    let env = Env::new("gradle-catalog");
    let old_build = env.read("build.gradle.kts");
    env.kiln().args(["remove", "guava"]).assert().success();
    let build = env.read("build.gradle.kts");
    let cat = env.read("gradle/libs.versions.toml");
    assert!(!build.contains("libs.guava"));
    assert!(!cat.contains("guava"), "{cat}");
    assert!(cat.contains("slf4j = \"2.0.9\"") && cat.contains("# Version catalog"));
    assert_eq!(build.lines().count() + 1, old_build.lines().count());
}

#[test]
fn remove_keeps_catalog_entry_when_used_in_another_module() {
    let env = Env::new("multi-module");
    let out = env.kiln().args(["remove", "guava", "--module", "app"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!env.read("app/build.gradle.kts").contains("libs.guava"));
    assert!(env.read("gradle/libs.versions.toml").contains("guava"));
    assert!(stdout(&out).contains("still used"));
    env.kiln().args(["remove", "guava"]).assert().success();
    assert!(!env.read("gradle/libs.versions.toml").contains("guava"));
}

#[test]
fn remove_inline_and_maven() {
    let env = Env::new("gradle-kts");
    env.kiln().args(["remove", "slf4j"]).assert().success();
    assert!(!env.read("build.gradle.kts").contains("slf4j"));

    let env = Env::new("maven");
    let old = env.read("pom.xml");
    env.kiln().args(["remove", "slf4j-api"]).assert().success();
    let pom = env.read("pom.xml");
    assert!(!pom.contains("slf4j"), "{pom}");
    assert!(pom.contains("<maven.compiler.release>21</maven.compiler.release>"));
    assert!(pom.lines().count() < old.lines().count());
    env.kiln().args(["remove", "does-not-exist"]).assert().failure();
}

#[test]
fn remove_dry_run() {
    let env = Env::new("maven");
    let old = env.read("pom.xml");
    let out = env.kiln().args(["remove", "slf4j", "--dry-run"]).output().unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).contains("-            <artifactId>slf4j-api</artifactId>"), "{}", stdout(&out));
    assert_eq!(env.read("pom.xml"), old);
}

#[test]
fn misc_cli() {
    let env = Env::new("gradle-kts");
    env.kiln().arg("--version").assert().success().stdout(predicates::str::contains("kiln "));
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let out = env.kiln().args(["completions", shell]).output().unwrap();
        assert!(out.status.success());
        assert!(stdout(&out).contains("kiln"), "{shell}");
    }
    env.kiln().args(["cache", "path"]).assert().success();
    env.kiln().args(["cache", "clear"]).assert().success();
    env.kiln().arg("--help").assert().success().stdout(predicates::str::contains("EXAMPLES"));
    env.kiln()
        .args(["add", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("kiln add hikari postgres jedis"));
}

#[test]
fn no_project_gives_a_helpful_error() {
    let env = Env::new("gradle-kts");
    let empty = tempfile::tempdir().unwrap();
    let out = env.kiln().current_dir(empty.path()).arg("outdated").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no Gradle or Maven project found"));
}
