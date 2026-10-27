# MediaFlow

**Import your media. Every file is verified. You stay in control.**

MediaFlow copies footage and photos from cards, cameras, phones, drives and
folders to one or more destinations, proves each copy with SHA-256, organizes
files with templates, recognizes what you already have, and keeps a journal
of everything. It works offline, needs no account, and never deletes or
overwrites your files.

> Status: **0.1.0, pre-release.** The engine, CLI and desktop app work and are
> tested on Linux. macOS and Windows builds have **not** been verified yet —
> see [docs/ROADMAP.md](docs/ROADMAP.md).

## What it does

- **Plan before copying** — see exactly what will be copied, what is already
  at the destination (content-verified), what conflicts, what is excluded
  and why, and how much space each destination needs.
- **Verified copy** — the source is hashed while it is read once; each copy
  is written to a temporary file, flushed, re-read and compared; only then
  does it receive its final name, with an operation that refuses to replace
  an existing file.
- **Several destinations at once** — one read of the card, one verified copy
  per destination; a failure on one never affects the others.
- **Duplicates without guesswork** — a same name or a same size is never
  treated as identity; only equal SHA-256 digests are.
- **Interruptions are normal** — pause, unplug a drive, kill the app: resume
  re-checks the disks and continues; nothing incomplete is ever reported
  as done.
- **Completeness check** — prove that one or more backups hold every file of
  a card, wherever the files were filed. A disconnected drive is reported as
  such, never as “missing files”.
- **Reports** — JSON and CSV per import, plus a searchable library of every
  verified copy.

## Install / build

Prerequisites: Rust ≥ 1.89, Node ≥ 20. Linux also needs WebKitGTK 4.1
(`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`).

```sh
# Command-line tool
cargo build --release -p mediaflow-cli        # → target/release/mediaflow

# Desktop app
cd app
npm ci
npx tauri build                               # bundles in target/release/bundle
npx tauri dev                                 # development
```

## Command line

```sh
mediaflow plan   /Volumes/A7S3 --dest /Volumes/RAID/Projects --project "Client X"
mediaflow import /Volumes/A7S3 --dest /Volumes/RAID/Projects --dest /Volumes/Shuttle --profile pro_video
mediaflow resume <session-id>
mediaflow verify /Volumes/A7S3 --dest /Volumes/RAID/Projects --profile youtube
mediaflow report <session-id> --format csv --out report.csv
mediaflow sessions | profiles | volumes | library <query>
```

Exit codes: `0` completed, `2` finished with problems / not proven complete,
`3` paused or interrupted, `4` cancelled, `1` error. Ctrl-C pauses (press
again to cancel).

## Documentation

- [User guide](docs/USER_GUIDE.md)
- [Architecture](docs/ARCHITECTURE.md) — components, data model, state machines, pipeline
- [Threat model](docs/THREAT_MODEL.md) and [data-loss risks](docs/DATA_SAFETY.md)
- [Test plan](docs/TEST_PLAN.md)
- [Roadmap & definition of done](docs/ROADMAP.md)
- [Release checklist](docs/RELEASE_CHECKLIST.md)
- [Contributing](CONTRIBUTING.md) · [Security policy](SECURITY.md) · [Changelog](CHANGELOG.md)

## Privacy

No network connections, no telemetry, no account. History and fingerprints
live in one SQLite file in your user data folder (file names, sizes, dates
and digests — never file contents). It is not encrypted.

## License

Apache-2.0. See [LICENSE](LICENSE).
