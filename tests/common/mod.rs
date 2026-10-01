#![allow(dead_code)]
//! Test harness: a tiny local HTTP server standing in for Maven repositories,
//! the Maven Central search API, the Gradle Plugin Portal and GitHub, plus
//! helpers to copy fixture projects into temp directories.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use assert_cmd::Command;

type Routes = Arc<Mutex<HashMap<String, (u16, String)>>>;

pub struct Mock {
    pub port: u16,
    routes: Routes,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

fn handle(mut stream: TcpStream, routes: Routes, hits: Arc<AtomicUsize>) {
    let mut buf = [0u8; 8192];
    let mut data = Vec::new();
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        data.extend_from_slice(&buf[..n]);
        if data.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let req = String::from_utf8_lossy(&data);
    let target = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("/").to_string();
    let path = target.split('?').next().unwrap_or("/").to_string();
    hits.fetch_add(1, Ordering::SeqCst);
    let found = routes.lock().unwrap().get(&path).cloned();
    let (status, body) = match found {
        Some(r) => r,
        // Unknown artifacts: generic metadata, so template lookups always succeed.
        None if path.ends_with("/maven-metadata.xml") && (path.starts_with("/maven2/") || path.starts_with("/plugins/")) => (
            200,
            "<metadata><versioning><release>1.2.3</release><versions><version>1.0.0</version><version>1.2.3</version></versions></versioning></metadata>".to_string(),
        ),
        None => (404, String::new()),
    };
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

impl Mock {
    pub fn start() -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (routes, hits, stop) = (routes.clone(), hits.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((s, _)) => {
                            s.set_nonblocking(false).ok();
                            let (r, h) = (routes.clone(), hits.clone());
                            thread::spawn(move || handle(s, r, h));
                        }
                        Err(_) => thread::sleep(std::time::Duration::from_millis(5)),
                    }
                }
            });
        }
        let m = Mock { port, routes, hits, stop };
        m.standard_routes();
        m
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn route(&self, path: &str, body: &str) {
        self.routes.lock().unwrap().insert(path.to_string(), (200, body.to_string()));
    }

    /// Registers `maven-metadata.xml` for `group:artifact` under `prefix`.
    pub fn artifact(&self, prefix: &str, group: &str, artifact: &str, versions: &[&str]) {
        let items: String = versions.iter().map(|v| format!("<version>{v}</version>")).collect();
        let latest = versions.last().copied().unwrap_or("");
        let xml = format!(
            "<?xml version=\"1.0\"?><metadata><groupId>{group}</groupId><artifactId>{artifact}</artifactId><versioning><latest>{latest}</latest><release>{latest}</release><versions>{items}</versions></versioning></metadata>"
        );
        self.route(&format!("{prefix}/{}/{artifact}/maven-metadata.xml", group.replace('.', "/")), &xml);
    }

    pub fn remove(&self, path: &str) {
        self.routes.lock().unwrap().remove(path);
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn standard_routes(&self) {
        let c = "/maven2";
        self.artifact(c, "com.zaxxer", "HikariCP", &["5.0.1", "5.1.0", "6.0.0-beta1", "6.0.0"]);
        self.artifact(c, "org.slf4j", "slf4j-api", &["2.0.7", "2.0.9", "2.0.13", "2.1.0-alpha1"]);
        self.artifact(
            c,
            "com.google.guava",
            "guava",
            &["32.0.0-jre", "32.1.3-jre", "32.1.3-android", "33.2.1-jre", "33.2.1-android"],
        );
        self.artifact(c, "org.junit.jupiter", "junit-jupiter", &["5.10.0", "5.10.3", "5.11.0", "6.0.1"]);
        self.artifact(c, "com.fasterxml.jackson.core", "jackson-databind", &["2.15.0", "2.15.4", "2.17.1"]);
        self.artifact(c, "org.apache.maven.plugins", "maven-compiler-plugin", &["3.11.0", "3.13.0"]);
        self.artifact(c, "redis.clients", "jedis", &["5.1.0", "5.2.0"]);
        self.artifact(c, "it.unimi.dsi", "fastutil", &["8.5.12", "8.5.14"]);
        self.artifact(
            "/plugins",
            "com.gradleup.shadow",
            "com.gradleup.shadow.gradle.plugin",
            &["8.3.0", "8.3.5", "9.0.0"],
        );
        self.artifact("/papermc", "io.papermc.paper", "paper-api", &["1.20.4-R0.1-SNAPSHOT", "1.21.4-R0.1-SNAPSHOT"]);
        self.route(
            "/gradle/versions/current",
            &format!(r#"{{"version":"9.8.0","checksumUrl":"http://127.0.0.1:{}/gradle/checksum"}}"#, self.port),
        );
        self.route("/gradle/checksum", &"ab".repeat(32));
        self.artifact("/papermc", "com.velocitypowered", "velocity-api", &["3.3.0-SNAPSHOT", "3.4.0-SNAPSHOT"]);
        self.artifact(c, "org.jetbrains.kotlin", "kotlin-stdlib", &["2.0.0", "2.1.0", "2.2.0-Beta1"]);
        self.route(
            "/search",
            r#"{"response":{"numFound":2,"docs":[
                {"id":"it.unimi.dsi:fastutil","g":"it.unimi.dsi","a":"fastutil","latestVersion":"8.5.14","timestamp":1700000000000,"versionCount":120},
                {"id":"org.other:fastutil-extras","g":"org.other","a":"fastutil-extras","latestVersion":"1.0","timestamp":1500000000000,"versionCount":3}
            ]}}"#,
        );
        // Release notes for the junit major update.
        self.route(
            "/maven2/org/junit/jupiter/junit-jupiter/6.0.1/junit-jupiter-6.0.1.pom",
            "<project><scm><url>https://github.com/junit-team/junit5</url></scm></project>",
        );
        self.route(
            "/github/repos/junit-team/junit5/releases/tags/v6.0.1",
            r#"{"name":"JUnit 6.0.1","tag_name":"v6.0.1","html_url":"https://github.com/junit-team/junit5/releases/tag/v6.0.1","body":"Highlights\n\n- Removed deprecated APIs\n- Requires Java 17"}"#,
        );
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub struct Env {
    pub mock: Mock,
    pub dir: tempfile::TempDir,
    pub cache: tempfile::TempDir,
    pub config: tempfile::TempDir,
}

impl Env {
    pub fn new(fixture: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture), dir.path());
        Env { mock: Mock::start(), dir, cache: tempfile::tempdir().unwrap(), config: tempfile::tempdir().unwrap() }
    }

    pub fn empty() -> Env {
        Env {
            mock: Mock::start(),
            dir: tempfile::tempdir().unwrap(),
            cache: tempfile::tempdir().unwrap(),
            config: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).unwrap()
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    /// Converts all text files of the fixture to CRLF.
    pub fn to_crlf(&self) {
        convert_crlf(self.path());
    }

    pub fn kiln(&self) -> Command {
        let base = self.mock.base();
        let mut c = Command::cargo_bin("kiln").unwrap();
        c.current_dir(self.path())
            .env("KILN_CENTRAL_URL", format!("{base}/maven2"))
            .env("KILN_PLUGIN_PORTAL_URL", format!("{base}/plugins"))
            .env("KILN_SEARCH_URL", format!("{base}/search"))
            .env("KILN_PAPERMC_URL", format!("{base}/papermc"))
            .env("KILN_GITHUB_API_URL", format!("{base}/github"))
            .env("KILN_GRADLE_API_URL", format!("{base}/gradle"))
            .env("KILN_CACHE_DIR", self.cache.path())
            .env("KILN_CONFIG_DIR", self.config.path())
            .env("NO_COLOR", "1")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .env_remove("HTTPS_PROXY")
            .env_remove("https_proxy")
            .env_remove("HTTP_PROXY")
            .env_remove("http_proxy")
            .env_remove("ALL_PROXY")
            .env_remove("KILN_OFFLINE")
            .env_remove("GITHUB_TOKEN")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
            .env("GIT_CONFIG_SYSTEM", if cfg!(windows) { "NUL" } else { "/dev/null" });
        c
    }
}

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target: PathBuf = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn convert_crlf(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            convert_crlf(&p);
        } else if let Ok(s) = std::fs::read_to_string(&p) {
            std::fs::write(&p, s.replace("\r\n", "\n").replace('\n', "\r\n")).unwrap();
        }
    }
}

/// Asserts that `new` only adds lines compared to `old` (nothing removed or reordered).
pub fn assert_only_additions(old: &str, new: &str) {
    let diff = similar::TextDiff::from_lines(old, new);
    for c in diff.iter_all_changes() {
        assert_ne!(c.tag(), similar::ChangeTag::Delete, "line removed: {:?}\n--- new ---\n{new}", c.value());
    }
}

pub fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}
