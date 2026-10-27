# MediaFlow — Architecture

Status: describes the code in this repository as of v0.1.0. When code and this
document disagree, the code is right and this document has a bug.

## 1. Audit of the starting point (Phase 0)

The repository was **empty** when work started: no commits, no files, no
history to preserve. There was therefore no existing stack to respect, no
reusable component and no migration risk. Every decision below was made from
scratch.

Environment used for development and verification: Linux x86-64, Rust 1.97,
Node 22, WebKitGTK 4.1. CI runs the engine and CLI test suite on macOS and
Windows too, and compiles the desktop app there; the macOS and Windows apps
have not been used by hand yet (see §9 and `docs/RELEASE_CHECKLIST.md`).

## 2. Technology choices

| Need | Choice | Why | Alternatives considered |
|---|---|---|---|
| Desktop shell | **Tauri 2** | Native webview (small binaries), Rust backend in-process, capability-based IPC permissions that let us whitelist every command. Mature on macOS/Windows/Linux. | Electron (ships Chromium + Node: bigger attack surface and footprint, Node fs in the renderer is easy to misuse); native Swift/WinUI (two UIs to maintain). |
| UI | **React 19 + TypeScript (strict)**, hand-written CSS | Widely known, testable with Testing Library. No UI kit: the design is original and the bundle stays small and offline. | Svelte/Solid (fine, smaller ecosystem for contributors). |
| Engine | **Rust** (`crates/mediaflow-core`) | Memory safety, precise control of I/O, threads without data races, cross-platform syscalls (`renameat2`, `renamex_np`, `MoveFileExW`). | Go (fine, but Tauri integration and shared types favour Rust). |
| Storage | **SQLite** (rusqlite, bundled) with numbered forward-only migrations | Single local file, transactional, WAL journal, no server. Bundled build = same SQLite version on every OS. | sled/redb (less inspectable by users), JSON files (no transactions). |
| Hash | **SHA-256** (RustCrypto `sha2`) | Required by the brief; collision-resistant; widely recognised in reports. | xxHash/BLAKE3 (faster, but not cryptographic / less recognised by clients and archives). |

Dependencies are permissively licensed (MIT / Apache-2.0 / Unlicense /
BSD / Zlib for Rust crates; MIT for npm packages). `docs/RELEASE_CHECKLIST.md`
requires running `cargo deny` / `npm audit` before each release; those tools
were **not** run in this environment (no network access to advisory DBs was
attempted).

## 3. Components

```
┌──────────────────────────── Web view (React, TS) ─────────────────────────────┐
│ Overview · New import (4 steps) · Projects · Library · Duplicates ·           │
│ Verification · History · Settings                                             │
│   api.ts ── invoke(cmd, {grant ids, plan ids, session ids}) ── never paths    │
└───────────────┬──────────────────────────────────────▲────────────────────────┘
                │ Tauri IPC (capability "main": 18 app   │ events: import-progress,
                │ commands + event listen, nothing else)│ import-finished, plan-progress
┌───────────────▼──────────────────────────────────────┴────────────────────────┐
│ app/src-tauri  (commands.rs)                                                  │
│   • native folder picker → Engine::authorize → grant id                       │
│   • plans kept in memory by id; sessions run on worker threads                │
└───────────────┬───────────────────────────────────────────────────────────────┘
                │ Rust API                     ┌───────────────────────────────┐
┌───────────────▼──────────────────────────┐   │ crates/mediaflow-cli          │
│ crates/mediaflow-core  (UI-independent)  │◄──┤ `mediaflow` command line      │
│                                          │   └───────────────────────────────┘
│  engine.rs    facade, grants, sessions   │
│  executor.rs  copy → verify → finalize   │
│  plan.rs      import plan, duplicates    │
│  compare.rs   completeness check         │
│  scan.rs      inventory, FileStamp       │
│  classify.rs  extension + magic bytes    │
│  template.rs  folder/file templates      │
│  profile.rs   built-in profiles          │
│  matching.rs  identity levels            │
│  paths.rs     validation, confinement    │
│  fsops.rs     no-replace rename, fsync   │
│  state.rs     state machines             │
│  db.rs        SQLite + migrations        │
│  report.rs    JSON / CSV reports         │
│  hash.rs      streaming SHA-256          │
└──────────────────────────────────────────┘
```

The core has no dependency on Tauri. The CLI and the desktop app are thin
front ends; both go through `Engine`.

## 4. Data model (SQLite, migration v1)

| Table | Purpose | Key columns |
|---|---|---|
| `schema_migrations` | applied migrations | `version` |
| `grants` | folders the user authorized | `id`, `path` (canonical), `kind` (`source`/`destination`) |
| `sessions` | one import | `state`, `project`, `profile_json` (frozen copy of the profile), `source_root`, `source_scan_complete`, `scan_issues_json`, timestamps, `active_seconds`, `last_error` |
| `session_destinations` | destinations of a session | `(session_id, idx)`, `root` |
| `files` | source inventory at planning time | `(session_id, file_id)`, `rel_path`, `size`, `mtime_ns`, `inode`, `category`, `confidence`, exclusion, `source_sha256` |
| `ops` | **the operation journal**: one row per file × destination | `state`, `action` (`copy`/`skip_duplicate`/`conflict`), `target_rel`, `final_rel`, `temp_name`, `sha256`, `final_stamp_json`, error code/message, `notes_json` |
| `events` | human-readable journal | level, code, message, rel_path |
| `hash_cache` | digests keyed by path, invalidated by size/mtime/inode | never trusted when mtime is unknown |
| `library` | every verified copy | `(dest_root, rel_path)`, `sha256`, origin |
| `checks` | completeness check summaries | used for “last verification” |

A newer schema than the binary knows is refused (no silent downgrade).

## 5. State machines

Session (`state.rs`, every transition goes through `SessionState::apply`):

```
queued ─StartScan→ scanning ─Planned→ planned ─Start→ copying ─CopyFinished→ verifying ─VerifyOk→ completed
                                         │              │  ▲                    │ └─VerifyWithErrors→ completed_with_errors
                                         │        Pause │  │ Resume             │                         │
                                         │              ▼  │                    │                         │ Resume (retry failed ops)
                                         │            paused ◄──── Pause ───────┘                         ▼
                                         │   Interrupt (crash detected at startup, volume lost) → interrupted ─Resume→ copying
                                         └── Cancel (from planned/copying/verifying/paused/interrupted/failed) → cancelled
          any non-terminal ─Fail→ failed ─Resume→ copying
```

`completed` is reachable only through `verifying`; `completed_with_errors` is
never displayed as success; `cancelled` and `completed` are final.

Operation (one file × one destination):

```
pending ─BeginCopy→ copying ─CopyWritten→ verifying ─Verified→ done
   │                   │                     │
   ├─SkipVerifiedDuplicate→ skipped_duplicate   (only after both digests matched at execution time)
   ├─FlagConflict→ conflict
   └── Fail / Cancel from pending, copying, verifying → failed / cancelled
Reset (resume/retry): copying|verifying|failed|cancelled|done|skipped_duplicate → pending
```

`done` requires `verifying` first, which requires the temp file to have been
fully written and fsync'ed.

## 6. The copy pipeline (executor.rs)

For each file, for all its destinations at once:

1. **Re-check at execution time**: authorization of every root, path safety
   (`resolve_within`: no symlink/reparse point in any existing component, no
   escape), and that the source `FileStamp` (size, mtime, inode) still equals
   the one recorded during planning.
2. Duplicate candidates are **hashed now** (source and existing file). Skip
   only if equal; otherwise copy.
3. If something already sits at the target: identical content (hash) → accept
   without writing (this also covers “crash after rename, before journal”);
   otherwise pick `name (n).ext`. Nothing is ever overwritten.
4. Free-space check right before writing.
5. Journal `copying` with the deterministic temp name
   `.mfpart-<session8>-<file>-<dest>`, then create it with `O_EXCL`.
6. Read the source **once**, hash while reading, write to every temp file.
   Cooperative pause/cancel between 4 MiB chunks.
7. `fsync` each temp; re-stat the source (changed during copy → fail).
8. Journal `verifying`; **re-read each temp file** and compare SHA-256
   (one thread per destination).
9. Restore mtime, then finalize with a **no-replace rename**
   (`renameat2(RENAME_NOREPLACE)` on Linux, `renamex_np(RENAME_EXCL)` on
   macOS, `MoveFileExW` without `REPLACE_EXISTING` on Windows; fallback:
   exclusive placeholder + rename). `fsync` the directory (Unix).
10. Journal `done` with digest and the final stamp; add to the library.

After all files: session goes to `verifying`; every finalized copy must still
exist with its recorded stamp; profiles with `paranoid` re-read and re-hash
every copy.

Resume (`recover`): temp files of interrupted ops are removed (only regular
files with our prefix, at the journaled path) and the op restarts; `done`
copies whose file vanished are recopied, modified ones are flagged; failed
and cancelled ops are retried.

Volume loss: any I/O error is followed by a check of the root directory; if a
root is gone, the session stops as `interrupted` (temp files removed, ops back
to `pending`) instead of failing files one by one.

## 7. Security boundary

See `docs/THREAT_MODEL.md`. Summary: the web view holds no file system,
shell, HTTP or dialog permission; it can call 18 named commands which take
opaque ids. Folders enter the system only through the native picker (app) or
command-line arguments (CLI). The engine re-validates paths and grants when
executing. CSP: `default-src 'self'`, no remote origins.

## 8. Performance

Measured on the development VM (ext4 on virtio, release build,
`cargo run --release -p mediaflow-core --example bench`):

| Workload | Result |
|---|---|
| 4 × 256 MB clips | import 6.7 s ≈ **159 MB/s** of source data (read once, written once, re-read once; the re-read is likely served from the page cache) |
| 20 000 × 4 KB files | scan 0.09 s · plan 6.2 s (every file hashed because all sizes collide) · import 37.6 s (~530 files/s) |

Per-file overhead (~1.9 ms here) is dominated by `open`/`fsync` system calls
required for durability. The numbers are environment-specific; rerun the
benchmark on target hardware.

Design choices: bounded memory (4 MiB buffer per pass), cheap information
first (stat), hashes only for size-colliding candidates, content index built
once per size bucket (O(1) lookups), hash cache invalidated by
size/mtime/inode, progress events throttled to ~7/s.

## 9. Known limitations (honest list)

- Verification re-reads the copy through the OS; the read may come from the
  page cache rather than the medium. `paranoid` and later completeness checks
  reduce but do not eliminate this. A cache-bypassing read (O_DIRECT /
  F_NOCACHE / FILE_FLAG_NO_BUFFERING) is on the roadmap.
- Durability ultimately depends on the drive honouring flushes. USB
  enclosures and some exFAT drivers may lie.
- On file systems without `renameat2`/`renamex_np` support, finalization uses
  an exclusive placeholder; it never replaces a foreign file, but is not one
  atomic step.
- Byte-level resume of a partially copied file is not implemented: the file
  in progress is copied again from the start.
- Camera/EXIF/video metadata extraction is not implemented: `{date}` uses the
  file modification time; `{camera}` uses the source name.
- Case/Unicode-insensitive collision detection uses a conservative folding
  (lowercase + common Latin accents); exotic normalization differences may be
  treated as distinct names (they would then be caught by the no-replace
  rename, never overwritten).
- macOS and Windows code paths (`renamex_np`, `MoveFileExW`, reparse points)
  are exercised by the CI test suite on the runners' system drives (APFS,
  NTFS) only; external exFAT drives and real unplugs are untested there.
- Custom profiles cannot yet be created from the UI.
