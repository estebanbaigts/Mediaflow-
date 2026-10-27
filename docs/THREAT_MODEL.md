# Threat model

MediaFlow runs locally, offline, with the user's privileges. Running locally
does **not** make it secure by itself; this document lists what we defend
against, how, and what we do not.

## Assets

1. The user's media (sources and existing files at destinations) — integrity
   and availability matter far more than confidentiality.
2. The local library database (file names, sizes, dates, SHA-256 digests,
   folder paths). Mildly sensitive metadata.
3. Exported reports (same metadata).

## Trust boundaries

| Boundary | Trusted side | Untrusted / less trusted side |
|---|---|---|
| Web view ↔ Rust | Rust engine | Web view (treated as if it could be compromised by a rendering bug or a malicious file name rendered as text) |
| Engine ↔ file system | Engine logic | Contents and *names* of files on cards and drives; other processes changing files concurrently |
| User ↔ app | User intent expressed via native dialogs / CLI args | — |

## Threats and mitigations

| # | Threat | Mitigation | Test |
|---|---|---|---|
| T1 | Compromised UI asks the backend to read/write/delete arbitrary paths | Capability `main` grants only 18 app commands + event listen; no `fs`, `shell`, `http`, `dialog` plugin permissions for the web view. Commands accept grant/plan/session **ids**, never paths. No command deletes or overwrites anything. | `app.test.tsx` asserts no path is sent; capability file reviewed |
| T2 | Path traversal via file names on a card or template values (`..`, absolute, drive letters, NUL, reserved Windows names) | `paths::validate_relative`, `sanitize_segment`, template rendering re-validated; `resolve_within` confines to the root | `paths` unit tests, `hostile_file_names_from_card`, `hostile_values_are_sanitized` |
| T3 | Symlink / junction tricks: link in source pointing outside; destination folder replaced by a link after planning (TOCTOU) | Scanner never follows links and reports them; every existing component of a target path is checked with `symlink_metadata` at execution; directories are created one level at a time and re-checked | `symlinks_in_source_are_reported_not_followed`, `symlink_planted_in_destination_after_planning`, `source_file_swapped_for_symlink_after_planning` |
| T4 | File replaced/modified between scan and copy, or during copy | Source `FileStamp` (size, mtime, inode) compared at execution and after reading; digest computed during the single read is the one verified | `source_modified_between_plan_and_run`, `source_modified_during_copy` |
| T5 | Target created by another process between check and finalize | No-replace rename (kernel-level where available); fallback exclusive placeholder; on conflict a new name is chosen | `target_created_between_plan_and_execution…`, `target_created_during_copy…`, `fsops` tests |
| T6 | Writing outside authorized folders | Grants required for every root; writes require a *destination* grant; checked at plan, session creation and execution | `unauthorized_paths_refused`, `revoked_grant_blocks_execution` |
| T7 | Dangerous destinations (`/`, system folders, home root), or source/destination nesting (infinite recursion, self-overwrite) | Denylist + nesting check | `forbidden_destinations`, `source_and_destination_must_not_nest` |
| T8 | CSV formula injection through file names in exported reports | Cells starting with `= + - @ \t \r` are prefixed with `'` | `csv_injection_neutralized`, `hostile_file_names_from_card` |
| T9 | Script injection via file names displayed in the UI | React escapes text; no `dangerouslySetInnerHTML`; CSP `script-src 'self'` | code review |
| T10 | Two app instances driving the same journal | Exclusive lock on `library.lock` | `second_engine_on_same_library_is_refused` |
| T11 | Malicious/newer database file | Newer schema refused; SQL uses bound parameters only; LIKE wildcards escaped | `refuses_newer_schema`, `library_like_escapes` |
| T12 | Data exfiltration | No network code in the engine or app; no telemetry; CSP forbids remote origins | code review (no HTTP client dependency) |

## Out of scope / not mitigated

- **Malware running as the same user** can read the database, alter files,
  or modify MediaFlow itself. MediaFlow is not a sandbox.
- **Confidentiality at rest**: the database and reports are not encrypted.
  Use full-disk encryption.
- **SHA-256 limits**: equal digests are treated as proof of identical
  content. A deliberate collision attack is not considered practical today;
  accidental collisions are negligible. A hash proves the bytes read were the
  bytes expected — it does not prove a drive will keep them.
- **Hardware lying about flushes** (some USB bridges) can lose data that the
  OS reported as durable.
- **Code signing / supply chain**: release builds are not signed yet; see
  the release checklist.
- **Denial of service by huge directory trees** on a malicious card: the
  scan is bounded only by memory; cancellation is available.

## Reporting

See `SECURITY.md`.
