# Changelog

All notable changes are documented here. Format: Keep a Changelog; versions
follow SemVer.

## [0.1.0] — unreleased

### Added
- Rust engine: scanning with signature sniffing, templates, six built-in
  profiles, hash-proven duplicate detection, import planning, verified copy
  to several destinations with a durable SQLite journal, no-replace
  finalization, pause/resume/cancel, crash and unplug recovery,
  completeness check, JSON/CSV reports, library index.
- `mediaflow` CLI: plan, import, resume, cancel, sessions, report, verify,
  library, profiles, volumes.
- Desktop app (Tauri 2 + React): overview, four-step import, projects,
  library, duplicates, verification, history with journal, settings;
  light/dark themes; keyboard navigation.
- 102 automated tests including a kill-and-resume end-to-end test.

### Known limitations
- Builds verified on Linux only. See docs/ROADMAP.md.
