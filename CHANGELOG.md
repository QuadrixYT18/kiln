# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file is maintained by [release-please](https://github.com/googleapis/release-please)
from [Conventional Commits](https://www.conventionalcommits.org/); please do not edit
released sections by hand.

## [Unreleased]

### Added

- `kiln new`: create Paper, Velocity, library, backend (Ktor / Spring Boot) and empty
  Gradle projects with live dependency versions, Gradle wrapper and optional Docker,
  CI, Renovate, license and git setup; custom templates via `kiln template`.
- `kiln add`, `kiln remove`: edit Gradle (Kotlin/Groovy DSL, version catalogs) and Maven
  build files without touching formatting, comments or line endings.
- `kiln outdated`, `kiln update`: parallel, cached version checks with patch/minor/major
  classification, release-note summaries for major updates, `--verify` with rollback.
- Shell completions, offline mode, response cache.
