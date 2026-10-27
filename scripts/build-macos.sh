#!/usr/bin/env bash
# Build MediaFlow.app with the WidgetKit extension embedded, sign it and
# (optionally) package a DMG. Run on macOS 14+ with Xcode, Rust and Node.
#
#   scripts/build-macos.sh                 # ad-hoc signed (local testing)
#   SIGN_IDENTITY="Apple Development: …" TEAM_ID=ABCDE12345 scripts/build-macos.sh
#   UNIVERSAL=1 scripts/build-macos.sh     # Apple Silicon + Intel in one app
#
# Ad-hoc signing is enough for the app, the menu-bar icon, Dock progress and
# notifications. macOS only shows widgets whose extension is signed with a
# real team identity (free Apple ID in Xcode works for your own Mac).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
IDENTITY="${SIGN_IDENTITY:--}"
DMG="${DMG:-1}"
UNIVERSAL="${UNIVERSAL:-0}"

command -v xcodegen >/dev/null || { echo "Install XcodeGen first: brew install xcodegen" >&2; exit 1; }

echo "==> Desktop app"
if [[ "$UNIVERSAL" == "1" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  (cd app && npm ci && npx tauri build --bundles app --target universal-apple-darwin)
  APP="$ROOT/target/universal-apple-darwin/release/bundle/macos/MediaFlow.app"
  WIDGET_ARCHS=(ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO)
else
  (cd app && npm ci && npx tauri build --bundles app)
  APP="$ROOT/target/release/bundle/macos/MediaFlow.app"
  WIDGET_ARCHS=()
fi
[[ -d "$APP" ]] || { echo "app bundle not found: $APP" >&2; exit 1; }

echo "==> Widget extension"
(cd app/macos-widget && xcodegen generate --quiet)
EXTRA=()
if [[ -n "${TEAM_ID:-}" ]]; then EXTRA+=(DEVELOPMENT_TEAM="$TEAM_ID"); fi
xcodebuild -quiet -project app/macos-widget/MediaFlowWidget.xcodeproj -scheme MediaFlowWidget \
  -configuration Release -derivedDataPath "$ROOT/target/widget" \
  CODE_SIGNING_ALLOWED=NO ${EXTRA[@]+"${EXTRA[@]}"} ${WIDGET_ARCHS[@]+"${WIDGET_ARCHS[@]}"} build
APPEX="$ROOT/target/widget/Build/Products/Release/MediaFlowWidget.appex"

echo "==> Embed and sign"
mkdir -p "$APP/Contents/PlugIns"
rm -rf "$APP/Contents/PlugIns/MediaFlowWidget.appex"
cp -R "$APPEX" "$APP/Contents/PlugIns/"
# Inner code first, then the app.
codesign --force --options runtime --timestamp=none -s "$IDENTITY" \
  --entitlements app/macos-widget/Widget/MediaFlowWidget.entitlements \
  "$APP/Contents/PlugIns/MediaFlowWidget.appex"
codesign --force --options runtime --timestamp=none -s "$IDENTITY" "$APP"
codesign --verify --deep --strict "$APP"
echo "Signed: $APP (identity: $IDENTITY)"

if [[ "$DMG" == "1" ]]; then
  OUT="${DMG_OUT:-$ROOT/target/release/MediaFlow-macos.dmg}"
  mkdir -p "$(dirname "$OUT")"
  rm -f "$OUT"
  STAGE=$(mktemp -d)
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -quiet -volname MediaFlow -srcfolder "$STAGE" -ov -format UDZO "$OUT"
  rm -rf "$STAGE"
  echo "DMG: $OUT"
fi
