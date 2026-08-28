#!/usr/bin/env bash
# Famulus – native SwiftUI-App bauen + installieren (v0.10.0).
# Atomarer Swap nach /Applications wie bei der Tauri-Variante
# (scripts/install-mac.sh), aber für die native Swift-Hülle (swift-app/).
# Muster: identisch zu Famulus Games scripts/build-app.sh (dort bewährt).
#
# Ablauf:
#   1. Rust-Kern als staticlib bauen
#   2. Swift-Bindings erzeugen (UniFFI)
#   3. Xcode-Projekt generieren (xcodegen)
#   4. App bauen (xcodebuild Release)
#   5. signieren, atomar installieren, starten

set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH:/opt/homebrew/bin"
cd "$(dirname "$0")/.."

echo "==> 1/4 Kern + Bindings bauen..."
./scripts/build-ffi.sh

echo "==> 2/4 Xcode-Projekt generieren..."
(cd swift-app && xcodegen generate)

echo "==> 3/4 App bauen (Release)..."
(cd swift-app && xcodebuild \
    -project Famulus.xcodeproj \
    -scheme Famulus \
    -configuration Release \
    -derivedDataPath build >/dev/null)

BUNDLE="swift-app/build/Build/Products/Release/Famulus.app"
DEST="/Applications/Famulus.app"
BACKUP="/Applications/.Famulus.app.vorherige-version"

if [ ! -d "$BUNDLE" ]; then
    echo "Fehler: $BUNDLE wurde nicht gebaut." >&2
    exit 1
fi

echo "==> 4/4 Signiere und installiere atomar nach $DEST..."
codesign --force --deep --sign - "$BUNDLE" >/dev/null 2>&1 || true
rm -rf "$BACKUP"
if [ -d "$DEST" ]; then
    mv "$DEST" "$BACKUP"
fi
mv "$BUNDLE" "$DEST"
rm -rf "$BACKUP"

echo "Starte Famulus..."
open -a "Famulus"
echo "Fertig."
