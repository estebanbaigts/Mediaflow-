# Menu bar, notifications and the macOS widget

## What runs where

| Feature | macOS | Windows | Linux |
|---|---|---|---|
| Status icon with menu (status, pause/resume, open, new import, history, quit) | menu bar (template icon) | notification area | tray (AppIndicator; GNOME needs the AppIndicator extension) |
| Progress under the app icon | Dock | taskbar | Unity launcher only |
| Badge "imports needing attention" | Dock | — (taskbar turns red instead) | Unity only |
| Notifications (done / problems / interrupted / card connected) | ✓ | ✓ | ✓ (needs a notification daemon) |
| Card detection → "Import from this card" | ✓ | ✓ | ✓ |
| Desktop / Notification Center widget | ✓ (WidgetKit, macOS 14+) | — | — |

Nothing starts by itself: a detected card is only a suggestion. Closing the
window during an import hides it; the copy continues and can be followed from
the status icon. "Quit" is refused while a copy is running (pause first).

## Status snapshot (format v1)

The app writes `status.json` atomically (≈ every 3 s during an import, and
at every state change):

- macOS: `~/Library/Group Containers/group.org.mediaflow.app/status.json`
- Windows / Linux: in the app data folder (not used by anything yet)

Fields: `version`, `updated_at`, `current` (running import: project, state,
fraction, bytes, ETA, current file), `last` (last finished import and whether
it is fully verified), `attention`, `destinations` (label, connected, free
space). No file contents. The contract is pinned by
`app/macos-widget/Fixtures/status-v1.json`, decoded by both the Rust tests and
the Swift tests.

The widget never shows a stale snapshot as live progress: if an import's
snapshot is older than 2 minutes it says "Import paused?". A finished import
with any problem is never shown as "Verified".

## Installing on a Mac

### Ready-made build (no tools needed)
Each CI run on the pull request produces `MediaFlow-macos-unsigned` (a DMG)
under *Actions → the run → Artifacts*. It is ad-hoc signed, not notarized:

1. Open the DMG, drag MediaFlow to Applications.
2. First launch: right-click MediaFlow → Open → Open (Gatekeeper warning for
   unsigned apps). If macOS still refuses:
   `xattr -dr com.apple.quarantine /Applications/MediaFlow.app`
3. Allow notifications when asked.

With this build the app, menu-bar icon, Dock progress and notifications work.
**The widget may not appear**: macOS only lists widgets from extensions signed
with a developer identity.

### Build on your Mac (widget included)
Requirements: macOS 14+, Xcode, Rust, Node 22, `brew install xcodegen`.

```sh
scripts/build-macos.sh                                   # ad-hoc (no widget guarantee)
SIGN_IDENTITY="Apple Development: you@example.com (XXXXXXXXXX)" TEAM_ID=XXXXXXXXXX scripts/build-macos.sh
```

A free Apple ID added in Xcode → Settings → Accounts gives you an
"Apple Development" identity (`security find-identity -v -p codesigning`).
Then copy `target/release/bundle/macos/MediaFlow.app` to Applications, launch
it once, and add the widget: right-click the desktop → Edit Widgets → MediaFlow.

## Limits (honest)
- WidgetKit decides when widgets refresh; during an import the widget asks for
  a refresh every minute but macOS may space them out.
- The app is not sandboxed and writes the App Group folder directly. On
  recent macOS, an app signed by a different team than the widget may trigger
  a one-time "access data from other apps" prompt; signing both with the same
  identity avoids it.
- Not yet tested by hand on a real Mac: CI compiles the widget and runs its
  unit tests on macOS, and builds the DMG.
