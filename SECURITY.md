# Security policy

## Supported versions

Only the latest release receives fixes during the 0.x series.

## Reporting a vulnerability

Please **do not open a public issue** for vulnerabilities that could lead to
data loss, file overwrite, writing outside authorized folders, or code
execution. Use GitHub's private vulnerability reporting ("Security" tab →
"Report a vulnerability") on this repository. Include:

- the MediaFlow version and OS / file system,
- steps to reproduce with generated test files (never send real media),
- what you expected and what happened.

We aim to acknowledge reports within 7 days and to publish a fix and an
advisory once users can update.

## Scope

In scope: the engine (`crates/mediaflow-core`), the CLI, the desktop shell
and its IPC boundary, report generation. See `docs/THREAT_MODEL.md` for what
MediaFlow does and does not defend against.
