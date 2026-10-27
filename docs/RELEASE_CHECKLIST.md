# Release checklist

Do not publish a release with any unchecked item. Write "not done" rather
than ticking something that was not actually run.

## Build & test, per OS (macOS arm64, macOS x86-64, Windows x86-64, Linux x86-64)
- [ ] `cargo test --workspace --exclude mediaflow-app` green
- [ ] `cd app && npm ci && npm test && npx tsc --noEmit` green
- [ ] `npx tauri build` produces a bundle; the app launches
- [ ] Manual import from a real card reader: plan, copy, verify, report
- [ ] Physical unplug of the destination mid-import → Interrupted → Resume → Completed
- [ ] Kill the app mid-import → relaunch shows Needs attention → Resume → Completed
- [ ] Destination on exFAT and on the native FS (APFS / NTFS / ext4)
- [ ] Destination nearly full → file fails with "not enough space", nothing else
- [ ] Run with networking disabled; the app works identically

## Security & supply chain
- [ ] `cargo deny check` (advisories, licences, bans) clean
- [ ] `npm audit --omit=dev` clean
- [ ] Capability file still lists only the intended commands; no plugin permission added for the web view
- [ ] CSP unchanged or reviewed
- [ ] macOS build signed and notarized; Windows installer signed
- [ ] SHA-256 checksums of release artifacts published

## Publishing
- [ ] Push a tag `vX.Y.Z` → the *Release* workflow builds every installer and opens a **draft** release with `SHA256SUMS`
- [ ] Download each installer from the draft and install it on a clean machine of that OS
- [ ] Publish the draft

## Docs
- [ ] CHANGELOG updated, version bumped in Cargo.toml / package.json / tauri.conf.json
- [ ] ROADMAP "definition of done" table updated with evidence
- [ ] Known limitations reviewed
