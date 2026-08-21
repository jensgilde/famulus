#!/usr/bin/env bash
# Baut Famulus fuer macOS und installiert es sicher nach /Applications.
#
# Warum es dieses Skript braucht: ein blosses
#   cp -a target/release/bundle/macos/Famulus.app /Applications/Famulus.app
# nistet sich selbst, wenn /Applications/Famulus.app schon existiert - cp
# legt dann /Applications/Famulus.app/Famulus.app an, statt den Inhalt zu
# ersetzen. Das kaputte Bundle besteht die Signaturpruefung nicht mehr
# ("unsealed contents present in the bundle root") und macOS killt den
# Prozess beim naechsten Start oder waehrend er laeuft mit
# SIGKILL (Code Signature Invalid). Ausserdem: wer ueber eine laufende
# Instanz drueberkopiert, riskiert genau denselben Crash schon waehrend
# der laufende Prozess noch Code-Seiten von der jetzt veraenderten Datei
# nachlaedt.
#
# Dieses Skript beendet eine laufende Instanz zuerst, ersetzt das Bundle
# komplett (kein Reinkopieren in ein bestehendes Verzeichnis) und startet
# genau einmal neu - keine Wiederholungsversuche, die mehrere Dock-Icons
# hinterlassen.

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Baue famulus-gui (Release, signiert)..."
cargo tauri build --bundles app -c gui/tauri.conf.json

BUNDLE="target/release/bundle/macos/Famulus.app"
if [ ! -d "$BUNDLE" ]; then
    echo "Fehler: $BUNDLE wurde nicht gebaut." >&2
    exit 1
fi

echo "==> Pruefe Signatur des frischen Builds..."
codesign --verify --deep --strict --verbose=4 "$BUNDLE"

if pgrep -f "Famulus.app/Contents/MacOS/famulus-gui" > /dev/null 2>&1; then
    echo "==> Beende laufende Famulus-Instanz..."
    osascript -e 'quit app "Famulus"' 2>/dev/null || true
    for _ in $(seq 1 20); do
        pgrep -f "Famulus.app/Contents/MacOS/famulus-gui" > /dev/null 2>&1 || break
        sleep 0.5
    done
    pkill -f "Famulus.app/Contents/MacOS/famulus-gui" 2>/dev/null || true
    sleep 0.5
fi

echo "==> Entferne alte Installation..."
rm -rf /Applications/Famulus.app

echo "==> Installiere neues Bundle..."
cp -a "$BUNDLE" /Applications/

echo "==> Pruefe installiertes Bundle..."
codesign --verify --deep --strict --verbose=4 /Applications/Famulus.app

echo "==> Starte Famulus..."
open -a Famulus

echo "Fertig."
