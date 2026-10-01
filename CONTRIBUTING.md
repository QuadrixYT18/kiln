# Contributing to kiln

Thanks for your interest in improving kiln! This document explains how to set up
the project, what we expect from a pull request, and how releases are made.

By participating you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).
Security issues should be reported privately, see [SECURITY.md](SECURITY.md).

## Getting started

```sh
git clone https://github.com/QuadrixYT18/kiln.git
cd kiln
cargo build
cargo test
```

You need a stable Rust toolchain (the minimum supported version is in
`Cargo.toml` as `rust-version`) and `git`. Gradle/Maven are **not** required to run
the test-suite.

Useful commands:

```sh
cargo fmt --all                                  # format
cargo clippy --all-targets -- -D warnings        # lint (CI fails on warnings)
cargo test                                       # unit + integration tests
cargo run -- add hikari --dry-run                # try it in any Gradle/Maven project
```

## Project layout

| Path | Purpose |
|------|---------|
| `src/cli.rs` | clap definitions and `--help` examples |
| `src/commands/` | one file per command (`add`, `outdated`, `update`, `remove`, `new`, ...) |
| `src/project/` | detection and **formatting-preserving** editors: `gradle.rs`, `catalog.rs`, `maven.rs`, `edit.rs` |
| `src/registry/` | HTTP client (cache, offline, rate limits), Maven metadata, search, GitHub, version logic, aliases |
| `src/templates/` | template engine, built-in templates, custom templates, Gradle wrapper |
| `templates/` | the built-in template files (embedded at compile time) |
| `tests/` | integration tests with fixture projects and a local mock registry |

### Ground rules for build-file editing

kiln must never reformat a user's files. Edits are splices on the original text
(or `toml_edit` for catalogs). When you touch an editor:

- keep indentation, comments, quote style and line endings (CRLF stays CRLF),
- add a fixture or test that asserts the file is otherwise byte-identical
  (`assert_only_additions` in `tests/common/mod.rs` helps),
- never assume `/` or `~` in paths; use `std::path` and `dirs`.

## Tests

- Unit tests live next to the code.
- Integration tests in `tests/` run the real binary against **fixture projects**
  (`tests/fixtures/`: Gradle Kotlin DSL, Groovy DSL, version catalog, Maven,
  multi-module) and a **local mock HTTP server** (`tests/common/mod.rs`). Tests must
  not touch the real network; every endpoint can be redirected with `KILN_*_URL`
  environment variables.
- When fixing a bug, add a failing test first.

## Adding aliases and templates

- Short names (`hikari` ...) live in `src/registry/aliases.rs`. Make sure the
  coordinates exist; aliases for artifacts outside Maven Central need a repository URL.
- Built-in templates live in `templates/<id>/` and are registered in
  `src/templates/builtin.rs`. Dependency versions must be written as live lookups
  (`{{latest:group:artifact}}`, `{{plugin:id}}`), never hardcoded. Please verify that a
  generated project builds (`./gradlew build`) before opening the PR.

## Commit messages and pull requests

We use [Conventional Commits](https://www.conventionalcommits.org/); the release
tooling derives the next version and the changelog from them.

```
feat(add): support --annotation-processor for Maven
fix(update): keep CRLF when editing version catalogs
docs: explain custom template placeholders
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`.
Use `feat!:` / a `BREAKING CHANGE:` footer for incompatible changes. Squash-merging
uses the PR title as commit message, so please write it in this format.

Before opening a PR make sure `cargo fmt`, `cargo clippy` and `cargo test` pass.
CI runs the tests on Windows, macOS (Apple Silicon and Intel) and Linux.

## Releases (maintainers)

1. Merge pull requests into `main` using Conventional Commit titles.
2. [release-please](https://github.com/googleapis/release-please) keeps a **Release PR**
   open with the next version and `CHANGELOG.md` entry.
3. Merging the Release PR creates the `vX.Y.Z` tag and the GitHub Release.
4. The tag starts the `Release` workflow ([cargo-dist](https://opensource.axo.dev/cargo-dist/)):
   it builds the binaries, installers and checksums, attests build provenance, and
   publishes to crates.io, the Homebrew tap, the Scoop bucket (and winget, when enabled).

See the README section on releasing for the required repository secrets.
