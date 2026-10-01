use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();

    let hash = std::env::var("KILN_COMMIT_HASH")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("GITHUB_SHA").ok().filter(|s| !s.is_empty()))
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .map(|h| h.chars().take(9).collect::<String>());
    let date = git(&["show", "-s", "--format=%cs", "HEAD"]);

    let long = match (hash, date) {
        (Some(h), Some(d)) => format!("{version} ({h} {d})"),
        (Some(h), None) => format!("{version} ({h})"),
        _ => version,
    };
    println!("cargo:rustc-env=KILN_LONG_VERSION={long}");
    println!("cargo:rerun-if-env-changed=KILN_COMMIT_HASH");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
}
