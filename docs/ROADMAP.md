# Scope, roadmap and acceptance criteria

## MVP scope (v0.1)

In: choose source/destinations (native picker or CLI), inventory, plan
preview with copy/skip/conflict/exclusion/rename, verified copy to one or
more destinations, hash-proven duplicate detection, never-overwrite policy,
pause/resume/cancel, crash & unplug recovery, completeness check, history
and journal, JSON/CSV reports, six built-in profiles, folder templates,
media categories, light/dark UI, CLI.

Out (explicitly not in v0.1): custom profile editor, EXIF/video metadata,
previews/thumbnails, NLE integration, AI suggestions, any network feature,
any deletion/cleanup feature.

## Definition of done — status of v0.1.0

| # | Criterion | Status | Evidence |
|---|---|---|---|
| 1 | Select a source and a destination | ✅ | native picker (app), args (CLI) |
| 2 | Scan shows a usable inventory | ✅ | plan preview, `mediaflow scan` |
| 3 | Plan reviewable before execution | ✅ | Preview step, `mediaflow plan` |
| 4 | Copy really works | ✅ | integration tests, manual run |
| 5 | Copies verified by digest | ✅ | SHA-256 source-while-reading vs re-read copy |
| 6 | Duplicates without confusing name and content | ✅ | `matching` tests, duplicate tests |
| 7 | No silent overwrite | ✅ | no-replace rename, race tests |
| 8 | Error never shown as success | ✅ | `completed_with_errors`, UI test |
| 9 | Resume without false completion | ✅ | crash/kill tests |
| 10 | Permissions enforced in the engine | ✅ | grants re-checked at execution, security tests |
| 11 | Critical tests pass | ✅ | 102 tests |
| 12 | Available builds tested | ⚠️ Linux only | Linux: tests, `.deb` bundle built, release app launched and used; macOS/Windows not built or run |
| 13 | Errors understandable | ✅ | `user_message()`; UI shows them |
| 14 | Offline operation demonstrated | ✅ Linux | full Rust suite incl. kill/resume e2e, and the release desktop app, run inside an empty network namespace (`unshare -n`); no network code anywhere |
| 15 | Data stays local | ✅ | no network code, no telemetry |
| 16 | Docs match behaviour | ✅ | this repository's docs |

**v0.1.0 is therefore not a release-ready MVP until #12 is done (macOS and Windows builds verified).**

## Next phases

### Phase 5 — Stabilization (next)
- CI on macOS, Windows, Linux (workflow provided, never run yet). Acceptance:
  all tests green on the three OSes.
- Run the integration suite on exFAT, APFS and NTFS volumes, and on a real
  USB card reader with a physical unplug. Acceptance: same results as Linux.
- Offline proof on macOS/Windows (Linux done with `unshare -n`).
- Cache-bypassing verification reads. Acceptance: verify reads hit the
  device (measured with I/O counters).
- `cargo deny` + `npm audit` clean; dependency licence report.
- Accessibility audit (VoiceOver, Narrator) of the four import steps.

### Phase 4 completion — Organization
- Custom profile editor (templates, exclusions, overrides) with live preview.
- EXIF / QuickTime / MXF metadata for `{date}` and `{camera}`.
- Per-category rules UI.

### Phase 6 — Open-source release
- Signed and notarized macOS build, signed Windows installer.
- Reproducible build notes, release checklist executed.

### Later
- Thumbnails/previews, unreadable-video detection (ffprobe, optional).
- Multi-backup coverage dashboard.
- Rule-based automation (on card insert → plan, never auto-start).
- Optional local-only AI classification as suggestions.
