# Changelog

All notable changes are documented here. Format: Keep a Changelog; versions
follow SemVer.

## [0.1.0] — 2026-10-27 (pre-release)

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
- Menu-bar / tray icon, Dock and taskbar progress, native notifications,
  card detection (never auto-starts), macOS WidgetKit widget.
- Installers: macOS universal DMG (widget included), Windows setup + MSI,
  Linux AppImage / deb / rpm, CLI binaries.
- Automated tests on Linux, macOS and Windows, including a kill-and-resume
  end-to-end test.

### Known limitations
- The desktop app was used by hand on Linux only; macOS and Windows builds
  are tested in CI but not yet by hand. Builds are not signed by Apple or
  Microsoft. See docs/ROADMAP.md.
