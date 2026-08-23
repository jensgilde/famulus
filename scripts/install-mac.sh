#!/usr/bin/env bash
# Baut Famulus fuer macOS und installiert es sicher nach /Applications.
#
# Der atomare Austausch per rename(2) verhindert, dass ein paralleler
# `open -a Famulus` auf ein halb kopiertes Bundle trifft. Aber macOS
# versucht nach einem osascript-Quit manchmal einen Auto-Relaunch der
# alten Instanz – und zwar aus dem BACKUP-Pfad, den wir gerade
# umbenannt haben. Wenn dieser Relaunch zeitlich mit dem rm -rf des
# Backups kollidiert, modifiziert macOS die Info.plist des Backups,
# und das ist dieselbe Inode wie die laufende alte Instanz – SIGKILL.
#
# Fix: Erst die alte Instanz sauber beenden und lange genug warten,
# dass kein Auto-Relaunch mehr kommt. Erst dann den atomaren Swap
# machen. Der atomare Swap selbst schützt dann vor parallelen Starts
# aus anderen Quellen (Terminal, Finder, andere Skripte).

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Baue famulus-gui (Release, signiert)..."
cargo tauri build --bundles app -c gui/tauri.conf.json

BUNDLE="target/release/bundle/macos/Famulus.app"
DEST="/Applications/Famulus.app"
BACKUP="/Applications/.Famulus.app.vorherige-version"

if [ ! -d "$BUNDLE" ]; then
    echo "Fehler: $BUNDLE wurde nicht gebaut." >&2
    exit 1
fi

echo "==> Pruefe Signatur des frischen Builds..."
codesign --verify --deep --strict --verbose=4 "$BUNDLE"

# ── Mikrofon-Berechtigung in Info.plist einbauen ─────────────────────
# Tauri 2 merge app.macOS.info_plist nicht immer zuverlaessig ins
# gebaute Bundle. Mit PlistBuddy direkt nachpatchen – das ist
# deterministisch und ueberlebt jeden Tauri-Update.
echo "==> Patche NSMicrophoneUsageDescription in Info.plist..."
/usr/libexec/PlistBuddy -c "Add :NSMicrophoneUsageDescription string 'Famulus benötigt das Mikrofon für die Spracheingabe.'" \
    "$BUNDLE/Contents/Info.plist" 2>/dev/null || \
/usr/libexec/PlistBuddy -c "Set :NSMicrophoneUsageDescription 'Famulus benötigt das Mikrofon für die Spracheingabe.'" \
    "$BUNDLE/Contents/Info.plist"

# Der PlistBuddy-Patch aendert Info.plist NACH dem Signieren durch
# `cargo tauri build` - genau das Muster, das den wiederkehrenden
# "Code Signature Invalid"-Crash verursacht hat (siehe Kommentar oben
# und vault/03-Wissen/Wiederkehrender-Absturz-Code-Signature.md). Ohne
# Neu-Signieren wuerde die naechste `codesign --verify` fehlschlagen
# und - mit set -e - das ganze Skript sofort abbrechen.
echo "==> Signiere nach dem Info.plist-Patch neu..."
SIGNING_IDENTITY=$(/usr/bin/python3 -c \
    "import json; print(json.load(open('gui/tauri.conf.json'))['bundle']['macOS']['signingIdentity'])")
codesign --force --deep --sign "$SIGNING_IDENTITY" "$BUNDLE"

# ── Alte Instanz ZUERST beenden, DANN swappen ────────────────────────
# Vorher: Swap zuerst, dann kill. macOS auto-relaunch aus dem Backup
# hat die Info.plist korrumpiert.
# Jetzt: Kill zuerst, warten bis kein Relaunch mehr kommt, dann Swap.
if pgrep -f "Famulus.app/Contents/MacOS/famulus-gui" > /dev/null 2>&1; then
    echo "==> Beende laufende Famulus-Instanz..."
    osascript -e 'quit app "Famulus"' 2>/dev/null || true
    # Warte, bis der Prozess wirklich weg ist – mit Sicherheitspuffer
    # gegen macOS Auto-Relaunch (LaunchServices / Resume).
    for _ in $(seq 1 20); do
        pgrep -f "Famulus.app/Contents/MacOS/famulus-gui" > /dev/null 2>&1 || break
        sleep 0.5
    done
    # Hartes Kill als Fallback
    pkill -f "Famulus.app/Contents/MacOS/famulus-gui" 2>/dev/null || true
    # Zusaetzliche Wartezeit: macOS Resume kann bis zu 2 Sekunden
    # nach dem Quit noch einen Relaunch triggern. Wir warten 3.
    sleep 3
fi

echo "==> Installiere atomar nach $DEST..."
rm -rf "$BACKUP"
if [ -d "$DEST" ]; then
    mv "$DEST" "$BACKUP"
fi
mv "$BUNDLE" "$DEST"
rm -rf "$BACKUP"

echo "==> Pruefe installiertes Bundle..."
codesign --verify --deep --strict --verbose=4 "$DEST"

echo "==> Starte Famulus..."
open -a Famulus

echo "Fertig."