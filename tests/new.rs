mod common;
use common::*;

fn exists(env: &Env, rel: &str) -> bool {
    env.path().join(rel).exists()
}

fn new(env: &Env, args: &[&str]) -> std::process::Output {
    let mut c = env.kiln();
    c.arg("new");
    c.args(args);
    c.output().unwrap()
}

fn ok(out: &std::process::Output) {
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn library_java_has_everything() {
    let env = Env::empty();
    ok(&new(&env, &["demo-lib", "--template", "library", "--java", "21", "--group", "org.acme"]));
    for f in [
        "demo-lib/settings.gradle.kts",
        "demo-lib/build.gradle.kts",
        "demo-lib/gradle/libs.versions.toml",
        "demo-lib/gradlew",
        "demo-lib/gradlew.bat",
        "demo-lib/gradle/wrapper/gradle-wrapper.jar",
        "demo-lib/gradle/wrapper/gradle-wrapper.properties",
        "demo-lib/.gitignore",
        "demo-lib/.gitattributes",
        "demo-lib/.editorconfig",
        "demo-lib/README.md",
        "demo-lib/src/main/java/org/acme/demolib/Greeter.java",
        "demo-lib/src/test/java/org/acme/demolib/GreeterTest.java",
    ] {
        assert!(exists(&env, f), "missing {f}");
    }
    let props = env.read("demo-lib/gradle/wrapper/gradle-wrapper.properties");
    assert!(
        props.contains("gradle-9.8.0-bin.zip") && props.contains(&format!("distributionSha256Sum={}", "ab".repeat(32))),
        "{props}"
    );
    let cat = env.read("demo-lib/gradle/libs.versions.toml");
    assert!(cat.contains("junit = \"1.2.3\""), "versions are looked up live: {cat}");
    let build = env.read("demo-lib/build.gradle.kts");
    assert!(build.contains("JavaLanguageVersion.of(21)") && build.contains("group = \"org.acme\""), "{build}");
    assert!(env.read("demo-lib/settings.gradle.kts").contains("rootProject.name = \"demo-lib\""));
    assert!(env.read("demo-lib/settings.gradle.kts").contains("foojay-resolver-convention\") version \"1.2.3\""));
    assert!(!env.read("demo-lib/src/main/java/org/acme/demolib/Greeter.java").contains("{{"));
    let bat = env.read("demo-lib/gradlew.bat");
    assert_eq!(bat.matches('\n').count(), bat.matches("\r\n").count(), "gradlew.bat must be CRLF");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(env.path().join("demo-lib/gradlew")).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "gradlew must be executable");
    }
    // git repository with a first commit
    let log = std::process::Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(env.path().join("demo-lib"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&log.stdout).contains("initial commit"));
}

#[test]
fn library_kotlin_and_flags() {
    let env = Env::empty();
    ok(&new(
        &env,
        &[
            "kt-lib",
            "--template",
            "library",
            "--lang",
            "kotlin",
            "--no-git",
            "--ci",
            "github",
            "--renovate",
            "--license",
            "mit-or-apache-2.0",
        ],
    ));
    assert!(exists(&env, "kt-lib/src/main/kotlin/com/example/ktlib/Greeter.kt"));
    assert!(env.read("kt-lib/build.gradle.kts").contains("jvmToolchain(21)"));
    assert!(env.read("kt-lib/gradle/libs.versions.toml").contains("kotlin-jvm"));
    assert!(!exists(&env, "kt-lib/.git"));
    assert!(env.read("kt-lib/.github/workflows/ci.yml").contains("java-version: 21"));
    assert!(exists(&env, "kt-lib/renovate.json"));
    assert!(env.read("kt-lib/LICENSE-MIT").contains("MIT License"));
    assert!(env.read("kt-lib/LICENSE-APACHE").contains("Apache License"));
    assert!(env.read("kt-lib/README.md").contains("MIT OR Apache-2.0"));
}

#[test]
fn paper_plugin_java_and_kotlin() {
    let env = Env::empty();
    ok(&new(&env, &["my-plugin", "--template", "paper", "--no-git", "--description", "Says \"hi\""]));
    let build = env.read("my-plugin/build.gradle.kts");
    assert!(build.contains("compileOnly(libs.paper.api)"), "{build}");
    assert!(build.contains("minecraftVersion(\"1.21.4\")"), "{build}");
    assert!(build.contains("description = \"Says \\\"hi\\\"\""), "{build}");
    let yml = env.read("my-plugin/src/main/resources/paper-plugin.yml");
    assert!(yml.contains("main: com.example.myplugin.MyPlugin") && yml.contains("api-version: '1.21'"), "{yml}");
    assert!(yml.contains("version: '${version}'"), "{yml}");
    assert!(yml.contains("description: 'Says \"hi\"'"), "{yml}");
    let cat = env.read("my-plugin/gradle/libs.versions.toml");
    assert!(cat.contains("paper-api = \"1.21.4-R0.1-SNAPSHOT\""), "{cat}");
    assert!(exists(&env, "my-plugin/src/main/java/com/example/myplugin/MyPlugin.java"));
    assert!(env.read("my-plugin/.gitignore").contains("run/"));

    ok(&new(&env, &["kt-plugin", "--template", "paper", "--lang", "kotlin", "--paperweight", "--no-git"]));
    let build = env.read("kt-plugin/build.gradle.kts");
    assert!(build.contains("paperweight.paperDevBundle(libs.versions.paper.api)"), "{build}");
    assert!(!build.contains("compileOnly(libs.paper.api)"));
    assert!(exists(&env, "kt-plugin/src/main/kotlin/com/example/ktplugin/KtPlugin.kt"));
    let cat = env.read("kt-plugin/gradle/libs.versions.toml");
    assert!(cat.contains("paperweight-userdev") && !cat.contains("[libraries]"), "{cat}");
}

#[test]
fn velocity_plugin() {
    let env = Env::empty();
    ok(&new(&env, &["proxy-tools", "--template", "velocity", "--no-git"]));
    let src = env.read("proxy-tools/src/main/java/com/example/proxytools/ProxyTools.java");
    assert!(src.contains("@Plugin(") && src.contains("id = \"proxy-tools\""), "{src}");
    assert!(env.read("proxy-tools/build.gradle.kts").contains("annotationProcessor(libs.velocity.api)"));
    assert!(env.read("proxy-tools/gradle/libs.versions.toml").contains("velocity = \"3.4.0-SNAPSHOT\""));

    ok(&new(&env, &["kproxy", "--template", "velocity", "--lang", "kotlin", "--no-git"]));
    assert!(env.read("kproxy/build.gradle.kts").contains("kapt(libs.velocity.api)"));
    assert!(env.read("kproxy/gradle/libs.versions.toml").contains("kotlin-kapt"));
}

#[test]
fn backend_ktor_with_docker_services() {
    let env = Env::empty();
    ok(&new(&env, &["api", "--template", "backend", "--framework", "ktor", "--db", "postgres,redis", "--no-git"]));
    assert!(exists(&env, "api/src/main/kotlin/com/example/api/Application.kt"));
    assert!(exists(&env, "api/src/test/kotlin/com/example/api/ApplicationTest.kt"));
    let docker = env.read("api/Dockerfile");
    assert!(
        docker.contains("FROM gradle:jdk21") && docker.contains("buildFatJar") && docker.contains("-all.jar"),
        "{docker}"
    );
    let compose = env.read("api/docker-compose.yml");
    assert!(
        compose.contains("postgres:")
            && compose.contains("redis:")
            && compose.contains("DATABASE_URL: jdbc:postgresql://postgres:5432/api"),
        "{compose}"
    );
    assert!(compose.contains("volumes:\n  postgres-data:"), "{compose}");
    assert!(env.read("api/build.gradle.kts").contains("mainClass.set(\"com.example.api.ApplicationKt\")"));
    assert!(env.read("api/.gitignore").contains(".env"));
}

#[test]
fn backend_spring_java_and_kotlin() {
    let env = Env::empty();
    ok(&new(
        &env,
        &["svc", "--template", "backend", "--framework", "spring", "--lang", "java", "--docker", "--no-git"],
    ));
    assert!(exists(&env, "svc/src/main/java/com/example/svc/Application.java"));
    assert!(exists(&env, "svc/src/main/java/com/example/svc/HelloController.java"));
    let docker = env.read("svc/Dockerfile");
    assert!(docker.contains("bootJar") && !docker.contains("buildFatJar"), "{docker}");
    assert!(env.read("svc/build.gradle.kts").contains("alias(libs.plugins.spring.boot)"));

    ok(&new(&env, &["ksvc", "--template", "backend", "--framework", "spring", "--lang", "kotlin", "--no-git"]));
    assert!(exists(&env, "ksvc/src/main/kotlin/com/example/ksvc/Application.kt"));
    assert!(
        env.read("ksvc/build.gradle.kts").contains("kotlin.spring")
            || env.read("ksvc/build.gradle.kts").contains("libs.plugins.kotlin.spring")
    );
}

#[test]
fn ktor_is_kotlin_only() {
    let env = Env::empty();
    let out = new(&env, &["api", "--template", "backend", "--framework", "ktor", "--lang", "java"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Kotlin-only"));
}

#[test]
fn empty_template_and_bare_mode() {
    let env = Env::empty();
    ok(&new(&env, &["blank", "--template", "empty", "--bare", "--no-git"]));
    assert!(exists(&env, "blank/build.gradle.kts") && exists(&env, "blank/src/main/java/.gitkeep"));
    assert!(
        !exists(&env, "blank/.gitignore") && !exists(&env, "blank/README.md") && !exists(&env, "blank/.editorconfig")
    );
    assert!(exists(&env, "blank/gradlew"));
}

#[test]
fn gitlab_ci() {
    let env = Env::empty();
    ok(&new(&env, &["lab", "--template", "empty", "--ci", "gitlab", "--java", "17", "--no-git"]));
    assert!(env.read("lab/.gitlab-ci.yml").contains("eclipse-temurin:17-jdk"));
}

#[test]
fn refuses_to_overwrite_and_validates_input() {
    let env = Env::empty();
    ok(&new(&env, &["one", "--template", "empty", "--no-git"]));
    let out = new(&env, &["one", "--template", "empty", "--no-git"]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("already exists"));
    for bad in ["bad name", "a/b", "..", "-x"] {
        let out = new(&env, &[bad, "--template", "empty", "--no-git"]);
        assert!(!out.status.success(), "{bad}");
    }
    let out = new(&env, &["x", "--template", "nope"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown template"));
    let out = new(&env, &["--template", "empty"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing project name"));
    let out = new(&env, &["y"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("--template"));
}

#[test]
fn failed_lookup_leaves_no_partial_project() {
    let env = Env::empty();
    // The PaperMC repository no longer knows paper-api and Central is unreachable -> lookup fails.
    env.mock.remove("/papermc/io/papermc/paper/paper-api/maven-metadata.xml");
    let base = env.mock.base();
    let out = env
        .kiln()
        .env("KILN_CENTRAL_URL", format!("{base}/nothing"))
        .args(["new", "broken", "--template", "paper", "--no-git"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!exists(&env, "broken"));
}

#[test]
fn paths_with_spaces_and_umlauts() {
    let env = Env::empty();
    let out_dir = env.path().join("mein Ordner").join("äöü");
    ok(&new(&env, &["Müller-App", "--template", "empty", "--output", out_dir.to_str().unwrap(), "--no-git"]));
    assert!(out_dir.join("Müller-App/build.gradle.kts").exists());
    assert!(std::fs::read_to_string(out_dir.join("Müller-App/settings.gradle.kts")).unwrap().contains("Müller-App"));
}

#[test]
fn custom_templates_roundtrip() {
    let env = Env::empty();
    ok(&new(&env, &["orig-app", "--template", "library", "--lang", "kotlin", "--group", "org.acme", "--no-git"]));
    let proj = env.path().join("orig-app");
    // Save it as a template.
    let out = env.kiln().args(["template", "save", "my-lib"]).current_dir(&proj).output().unwrap();
    ok(&out);
    let stdout_text = stdout(&out);
    assert!(stdout_text.contains("org.acme.origapp"), "{stdout_text}");
    let tpl = env.config.path().join("templates/my-lib");
    assert!(tpl.join("build.gradle.kts").exists());
    assert!(tpl.join("src/main/kotlin/{{package_path}}/Greeter.kt").exists());
    assert!(!tpl.join("gradlew").exists(), "wrapper is regenerated, not stored");
    let cat = std::fs::read_to_string(tpl.join("gradle/libs.versions.toml")).unwrap();
    assert!(cat.contains("{{latest:org.junit:junit-bom}}"), "{cat}");
    let settings = std::fs::read_to_string(tpl.join("settings.gradle.kts")).unwrap();
    assert!(settings.contains("rootProject.name = \"{{name}}\""), "{settings}");

    // list shows it
    let list = stdout(&env.kiln().args(["template", "list"]).output().unwrap());
    assert!(
        list.contains("my-lib") && list.contains("custom") && list.contains("paper") && list.contains("built-in"),
        "{list}"
    );

    // use it
    ok(&new(&env, &["fresh", "--template", "my-lib", "--group", "net.example", "--no-git"]));
    let g = env.read("fresh/src/main/kotlin/net/example/fresh/Greeter.kt");
    assert!(g.contains("package net.example.fresh"), "{g}");
    assert!(env.read("fresh/settings.gradle.kts").contains("rootProject.name = \"fresh\""));
    assert!(env.read("fresh/gradle/libs.versions.toml").contains("junit = \"1.2.3\""));
    assert!(exists(&env, "fresh/gradlew"));

    // saving twice needs --force
    let again = env.kiln().args(["template", "save", "my-lib"]).current_dir(&proj).output().unwrap();
    assert!(!again.status.success());
    ok(&env.kiln().args(["template", "save", "my-lib", "--force"]).current_dir(&proj).output().unwrap());

    ok(&env.kiln().args(["template", "remove", "my-lib"]).output().unwrap());
    assert!(!tpl.exists());
}

#[test]
fn custom_template_placeholders_are_detected() {
    let env = Env::empty();
    let tpl = env.config.path().join("templates/greeting");
    std::fs::create_dir_all(tpl.join("docs")).unwrap();
    std::fs::write(
        tpl.join("README.md"),
        "# {{name}}\nOwner: {{owner}}\nTeam: {{team}}\n{{#if fancy}}\nfancy mode\n{{/if}}\n",
    )
    .unwrap();
    std::fs::write(tpl.join("docs/{{name}}.txt"), "hello {{owner}}").unwrap();
    std::fs::write(tpl.join("logo.bin"), [0u8, 159, 146, 150, 255]).unwrap();
    std::fs::write(
        tpl.join("kiln-template.toml"),
        "description = \"greeting\"\n[variables.team]\ndefault = \"core\"\n",
    )
    .unwrap();

    // Missing value without a terminal -> clear error naming the placeholder.
    let out = new(&env, &["p1", "--template", "greeting", "--no-git"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("owner") && err.contains("--var"), "{err}");
    assert!(!err.contains("team"), "defaults from the manifest are used: {err}");

    ok(&new(&env, &["p1", "--template", "greeting", "--no-git", "--var", "owner=Jane", "--var", "fancy=true"]));
    let readme = env.read("p1/README.md");
    assert_eq!(readme, "# p1\nOwner: Jane\nTeam: core\nfancy mode\n");
    assert_eq!(env.read("p1/docs/p1.txt"), "hello Jane");
    assert_eq!(
        std::fs::read(env.path().join("p1/logo.bin")).unwrap(),
        vec![0u8, 159, 146, 150, 255],
        "binary files are copied verbatim"
    );
    assert!(!exists(&env, "p1/gradlew"), "no Gradle settings in the template, so no wrapper");
    assert!(!exists(&env, "p1/kiln-template.toml"));
    assert!(env.read("p1/.gitignore").contains(".gradle/"));
}

#[test]
fn templates_resolve_versions_from_cache_when_offline() {
    let env = Env::empty();
    ok(&new(&env, &["online", "--template", "library", "--no-git"]));
    let hits = env.mock.hits();
    let out = env.kiln().args(["--offline", "new", "cached", "--template", "library", "--no-git"]).output().unwrap();
    ok(&out);
    assert_eq!(env.mock.hits(), hits);
    assert!(env.read("cached/gradle/libs.versions.toml").contains("junit = \"1.2.3\""));
}

#[test]
fn defaults_come_from_config() {
    let env = Env::empty();
    std::fs::write(
        env.config.path().join("config.toml"),
        "[new]\ngroup = \"io.acme\"\njava = 17\nlicense = \"mit\"\nauthor = \"Ada\"\n",
    )
    .unwrap();
    ok(&new(&env, &["cfg", "--template", "library", "--no-git"]));
    assert!(exists(&env, "cfg/src/main/java/io/acme/cfg/Greeter.java"));
    assert!(env.read("cfg/build.gradle.kts").contains("JavaLanguageVersion.of(17)"));
    assert!(env.read("cfg/LICENSE").contains("Ada"));
}

#[test]
fn template_output_cannot_escape_the_project_directory() {
    let env = Env::empty();
    let tpl = env.config.path().join("templates/sneaky");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("{{owner}}.txt"), "x").unwrap();
    let out = new(&env, &["safe", "--template", "sneaky", "--no-git", "--var", "owner=../../escaped"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusing to write outside"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!exists(&env, "safe"));
    assert!(!env.path().parent().unwrap().join("escaped.txt").exists());
}
