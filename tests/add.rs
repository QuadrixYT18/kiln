mod common;
use common::*;

#[test]
fn add_alias_to_gradle_catalog_project() {
    let env = Env::new("gradle-catalog");
    let old_build = env.read("build.gradle.kts");
    let old_cat = env.read("gradle/libs.versions.toml");
    let out = env.kiln().args(["add", "hikari"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout(&out).contains("Added com.zaxxer:HikariCP 6.0.0"), "{}", stdout(&out));

    let build = env.read("build.gradle.kts");
    let cat = env.read("gradle/libs.versions.toml");
    assert_only_additions(&old_build, &build);
    assert_only_additions(&old_cat, &cat);
    assert!(build.contains("    implementation(libs.hikaricp)\n"), "{build}");
    assert!(cat.contains("hikaricp = \"6.0.0\""), "{cat}");
    assert!(cat.contains("hikaricp = { module = \"com.zaxxer:HikariCP\", version.ref = \"hikaricp\" }"), "{cat}");
    // placed after the last implementation, before the test dependency
    let impl_pos = build.find("libs.hikaricp").unwrap();
    assert!(impl_pos < build.find("testImplementation").unwrap());
}

#[test]
fn add_skips_pre_releases_unless_asked() {
    let env = Env::new("gradle-kts");
    env.kiln().args(["add", "hikari"]).assert().success();
    assert!(env.read("build.gradle.kts").contains("\"com.zaxxer:HikariCP:6.0.0\""));

    let env = Env::new("gradle-kts");
    env.mock.artifact("/maven2", "com.zaxxer", "HikariCP", &["5.1.0", "7.0.0-beta1"]);
    env.kiln().args(["add", "hikari", "--pre"]).assert().success();
    assert!(env.read("build.gradle.kts").contains("\"com.zaxxer:HikariCP:7.0.0-beta1\""));
}

#[test]
fn add_scopes_for_gradle_kts() {
    let env = Env::new("gradle-kts");
    env.kiln().args(["add", "jedis", "--test"]).assert().success();
    let build = env.read("build.gradle.kts");
    let test_pos = build.find("testImplementation(\"redis.clients:jedis:5.2.0\")").expect(&build);
    assert!(test_pos > build.find("junit-jupiter").unwrap());

    env.kiln().args(["add", "hikari", "--compile-only"]).assert().success();
    assert!(env.read("build.gradle.kts").contains("compileOnly(\"com.zaxxer:HikariCP:6.0.0\")"));
    env.kiln().args(["add", "postgres", "--runtime"]).output().unwrap(); // not mocked: must fail cleanly
}

#[test]
fn add_is_idempotent_and_preserves_comments() {
    let env = Env::new("gradle-kts");
    let old = env.read("build.gradle.kts");
    env.kiln().args(["add", "guava"]).assert().success().stdout(predicates::str::contains("already declared"));
    assert_eq!(env.read("build.gradle.kts"), old);
    env.kiln().args(["add", "jedis"]).assert().success();
    let new = env.read("build.gradle.kts");
    assert_only_additions(&old, &new);
    assert!(new.contains("// logging"));
    assert!(new.contains("// fat jar"));
}

#[test]
fn add_dry_run_prints_diff_and_writes_nothing() {
    let env = Env::new("gradle-kts");
    let old = env.read("build.gradle.kts");
    let out = env.kiln().args(["add", "jedis", "--dry-run"]).output().unwrap();
    assert!(out.status.success());
    let s = stdout(&out);
    assert!(s.contains("+    implementation(\"redis.clients:jedis:5.2.0\")"), "{s}");
    assert!(s.contains("--- a/build.gradle.kts"), "{s}");
    assert_eq!(env.read("build.gradle.kts"), old);
}

#[test]
fn add_groovy_dsl_uses_existing_style() {
    let env = Env::new("gradle-groovy");
    env.kiln().args(["add", "jedis"]).assert().success();
    let build = env.read("build.gradle");
    assert!(build.contains("    implementation 'redis.clients:jedis:5.2.0'\n"), "{build}");
}

#[test]
fn add_to_maven_uses_version_property() {
    let env = Env::new("maven");
    let old = env.read("pom.xml");
    env.kiln().args(["add", "jedis"]).assert().success();
    let pom = env.read("pom.xml");
    assert_only_additions(&old, &pom);
    assert!(pom.contains("        <jedis.version>5.2.0</jedis.version>\n    </properties>"), "{pom}");
    assert!(pom.contains(
        "        <dependency>\n            <groupId>redis.clients</groupId>\n            <artifactId>jedis</artifactId>\n            <version>${jedis.version}</version>\n        </dependency>\n    </dependencies>"
    ), "{pom}");
    env.kiln().args(["add", "jedis"]).assert().success().stdout(predicates::str::contains("already declared"));
}

#[test]
fn add_maven_test_scope() {
    let env = Env::new("maven");
    env.kiln().args(["add", "jedis", "--test"]).assert().success();
    assert!(env.read("pom.xml").contains("<scope>test</scope>\n        </dependency>\n    </dependencies>"));
}

#[test]
fn crlf_files_stay_crlf() {
    for fixture in ["gradle-kts", "gradle-catalog", "maven"] {
        let env = Env::new(fixture);
        env.to_crlf();
        env.kiln().args(["add", "jedis"]).assert().success();
        for rel in ["build.gradle.kts", "gradle/libs.versions.toml", "pom.xml"] {
            let p = env.path().join(rel);
            if p.exists() {
                let t = std::fs::read_to_string(p).unwrap();
                assert_eq!(t.matches('\n').count(), t.matches("\r\n").count(), "{fixture}/{rel} has bare LF");
            }
        }
    }
}

#[test]
fn unknown_names_use_search_and_pick_top_hit_non_interactively() {
    let env = Env::new("gradle-kts");
    let out = env.kiln().args(["add", "fastutil"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(env.read("build.gradle.kts").contains("it.unimi.dsi:fastutil:8.5.14"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("ambiguous"));
}

#[test]
fn explicit_coordinates_and_pinned_versions() {
    let env = Env::new("gradle-kts");
    env.kiln().args(["add", "redis.clients:jedis@5.1.0"]).assert().success();
    assert!(env.read("build.gradle.kts").contains("redis.clients:jedis:5.1.0"));
}

#[test]
fn multi_module_requires_a_module() {
    let env = Env::new("multi-module");
    let out = env.kiln().args(["add", "jedis"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--module"));
    env.kiln().args(["add", "jedis", "--module", "lib"]).assert().success();
    assert!(env.read("lib/build.gradle.kts").contains("jedis"));
    assert!(!env.read("app/build.gradle.kts").contains("jedis"));
    // running inside a module directory selects it
    let out = env.kiln().current_dir(env.path().join("app")).args(["add", "hikari"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(env.read("app/build.gradle.kts").contains("libs.hikaricp"));
}

#[test]
fn project_repositories_are_consulted() {
    let env = Env::new("gradle-kts");
    let base = env.mock.base();
    let old = env.read("build.gradle.kts");
    env.write(
        "build.gradle.kts",
        &old.replace("mavenCentral()", &format!("mavenCentral()\n    maven(\"{base}/papermc\")")),
    );
    env.kiln().args(["add", "io.papermc.paper:paper-api", "--compile-only"]).arg("--pre").assert().success();
    // SNAPSHOT-only artifact needs --pre; latest picked from the custom repo
    assert!(env.read("build.gradle.kts").contains("compileOnly(\"io.papermc.paper:paper-api:1.21.4-R0.1-SNAPSHOT\")"));
}

#[test]
fn alias_repository_is_added_to_gradle_and_maven_builds() {
    let env = Env::new("gradle-kts");
    let base = env.mock.base();
    env.kiln().args(["add", "paper-api", "--compile-only", "--pre"]).assert().success();
    let build = env.read("build.gradle.kts");
    assert!(build.contains(&format!("    maven(\"{base}/papermc/\")\n}}")), "{build}");
    assert!(build.contains("compileOnly(\"io.papermc.paper:paper-api:1.21.4-R0.1-SNAPSHOT\")"), "{build}");

    let env = Env::new("maven");
    env.kiln().args(["add", "paper-api", "--compile-only", "--pre"]).assert().success();
    let pom = env.read("pom.xml");
    assert!(pom.contains("<repositories>") && pom.contains("/papermc/</url>"), "{pom}");
    assert!(pom.contains("<scope>provided</scope>"), "{pom}");
    // A second alias from the same repository does not duplicate it.
    env.kiln().args(["add", "velocity-api", "--compile-only", "--pre"]).assert().success();
    assert_eq!(env.read("pom.xml").matches("<repository>").count(), 1);
}

#[test]
fn maven_annotation_processor_goes_into_the_compiler_plugin() {
    let env = Env::new("maven");
    env.kiln().args(["add", "org.projectlombok:lombok", "--annotation-processor"]).assert().success();
    let pom = env.read("pom.xml");
    assert!(pom.contains("<annotationProcessorPaths>") && pom.contains("<artifactId>lombok</artifactId>"), "{pom}");
    // it is registered in the existing compiler plugin, not as a dependency
    let compiler = pom.find("maven-compiler-plugin").unwrap();
    assert!(pom.find("<annotationProcessorPaths>").unwrap() > compiler, "{pom}");
    assert_eq!(pom.matches("<dependency>").count(), 2, "{pom}");
}

#[test]
fn search_falls_back_to_the_central_website_api() {
    let env = Env::new("gradle-kts");
    env.mock.remove("/search");
    env.mock.route(
        "/fallback",
        r#"{"components":[{"namespace":"it.unimi.dsi","name":"fastutil","latestVersionInfo":{"version":"8.5.14","timestampUnixWithMS":1700000000000}}]}"#,
    );
    let base = env.mock.base();
    let out = env
        .kiln()
        .env("KILN_SEARCH_FALLBACK_URL", format!("{base}/fallback"))
        .args(["add", "fastutil"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(env.read("build.gradle.kts").contains("it.unimi.dsi:fastutil:8.5.14"));
}

#[test]
fn network_errors_hide_urls_unless_verbose() {
    let env = Env::new("gradle-catalog");
    let dead = "http://127.0.0.1:9/maven2";
    let short = env.kiln().env("KILN_CENTRAL_URL", dead).arg("outdated").output().unwrap();
    let text = format!("{}{}", stdout(&short), String::from_utf8_lossy(&short.stderr));
    assert!(text.contains("could not connect") && !text.contains("maven-metadata.xml"), "{text}");
    let verbose = env.kiln().env("KILN_CENTRAL_URL", dead).args(["-v", "outdated"]).output().unwrap();
    let text = format!("{}{}", stdout(&verbose), String::from_utf8_lossy(&verbose.stderr));
    assert!(text.contains("maven-metadata.xml"), "{text}");
}
