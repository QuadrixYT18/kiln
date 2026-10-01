# Security Policy

## Supported versions

Only the latest released version of kiln receives security fixes. Please upgrade
before reporting an issue, if possible.

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report vulnerabilities privately through GitHub:
<https://github.com/QuadrixYT18/kiln/security/advisories/new>

Include:

- the kiln version (`kiln --version`) and your operating system,
- a description of the issue and its impact,
- steps or a minimal project that reproduces it.

You can expect an acknowledgement within a few days. We will keep you informed
about the progress, agree on a disclosure date with you, and credit you in the
release notes unless you prefer to stay anonymous.

## Scope and design notes

kiln edits build files in your project and talks to public package registries
(Maven repositories, the Maven Central search API, the Gradle Plugin Portal,
GitHub Releases). Things worth reporting:

- path traversal or writes outside the project / template directories,
- command execution that is not initiated by the user (`kiln update --verify`
  intentionally runs your project's own Gradle/Maven wrapper),
- TLS or integrity problems in downloads (kiln uses rustls and verifies
  certificates; the Gradle wrapper checksum is written when available),
- supply-chain issues in the release process.

Release binaries are published with GitHub build provenance attestations. Verify
a download with `gh attestation verify <file> --repo QuadrixYT18/kiln`.
