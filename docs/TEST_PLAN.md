# Test plan

All destructive scenarios run inside fresh temporary directories with
generated content (`tests/common/mod.rs`). No test touches a home folder or
real media.

Run everything:

```sh
cargo test --workspace --exclude mediaflow-app   # engine + CLI (Rust)
cd app && npm ci && npm test                     # UI
cd app && npx tsc --noEmit                       # types
cargo clippy --all-targets && cargo fmt --check
```

## Inventory (v0.1.0) — 102 automated tests

### Unit (crates/mediaflow-core/src, 49)
- `hash`: known vectors, streaming across chunk boundaries, cancel.
- `paths`: traversal/absolute/reserved names rejected, Unicode accepted,
  sanitizer always yields valid names, collision folding, symlink blocking.
- `classify`: signature confirms/contradicts extension, text formats, unknown.
- `template`: default layout, hostile values sanitized, invalid templates.
- `profile`: built-ins valid, exclusion globs, invalid profiles rejected.
- `matching`: name ≠ identity, size+name only probable, hash proves/disproves.
- `state`: happy path, terminal states, no completion without verification,
  interrupt/resume, op cannot be done without verification.
- `db`: migrations idempotent, newer schema refused, hash cache invalidation,
  grants, LIKE escaping.
- `fsops`: no-replace rename never replaces; placeholder fallback.
- `report`: CSV formula injection neutralized.
- `scan`: symlinks/temp files reported, skip prefixes. `util`: control.

### Integration (crates/mediaflow-core/tests, 44)
`import.rs` (28): verified copy with source byte+mtime unchanged · source
modified before run · source modified during copy · destination unplugged
then resumed · destination full · permission denied (injected; real chmod test
skipped as root) · identical file present · duplicate with different name ·
same size different content · quick-plan candidate re-verified at execution ·
same name different content renamed / flagged · source path collisions ·
cancel mid-transfer · pause/resume · simulated crash + restart · crash after
finalize before journal · abandoned temp before verify · foreign `.mfpart-`
file never deleted · corruption detected after copy · multiple destinations
with independent failure · Unicode and 240-byte names, deep paths · exclusions
reported · paranoid final pass detects tampering · finalized copy deleted
before resume is recopied · second process refused · progress monotonic.

`security.rs` (12): unauthorized paths · `..` containment · revoked grant ·
forbidden destinations · nesting · symlinks in source · symlink planted in
destination after planning · source swapped for symlink · target created
between plan and run / during copy · hostile card file names · report export
never overwrites.

`compare.rs` (4): verified completeness after reorganized import · quick
mode never claims complete · disconnected ≠ missing · corrupted copy.

### End-to-end process test (crates/mediaflow-cli/tests, 1)
`killed_process_resumes_cleanly`: spawns the real `mediaflow` binary, kills
it with SIGKILL mid-import, resumes from a new process, checks every byte, no
duplicates, no temp files, `interrupted` journaled.

### UI (app/src/test, 8, Vitest + Testing Library, mocked bridge)
Keyboard navigation and shortcuts · full import flow (no path ever sent to
the backend, conflict/exclusion tabs, real progress, no fake ETA,
`completed_with_errors` never shown as success) · start blocked on
insufficient space · engine errors shown in plain language · disconnected
folders not selectable · resume from overview · empty state · cancel needs
confirmation.

### Manual checks performed
- Linux (WebKitGTK, Xvfb): real app launched against a real library; an
  import killed with SIGKILL appeared as *Interrupted*, was resumed from the
  UI, completed; preview showed verified duplicates across two destinations
  with different layouts; history journal displayed the full sequence.
- Dark mode rendering (Chromium, mocked bridge).
- Offline: Rust suite and the release app run in `unshare -n` (no network interface).
- `npx tauri build --bundles deb` produced `MediaFlow_0.1.0_amd64.deb` (4.6 MiB).

## Not yet covered (gaps)
- Execution on macOS and Windows (APFS/NTFS/exFAT behaviours, `renamex_np`,
  `MoveFileExW`, reparse points, long `\\?\` paths).
- Real removable media unplug (simulated by renaming the root).
- Real ENOSPC on a small loopback file system (simulated by fault injection).
- Real permission errors (environment runs as root).
- UI tests against the real backend (tauri-driver / WebDriver).
- Fuzzing of path validation and template rendering.
- Load test beyond 20 000 files / 1 GB.
