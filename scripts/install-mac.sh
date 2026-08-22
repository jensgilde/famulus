#!/usr/bin/env bash
# Baut Famulus fuer macOS und installiert es sicher nach /Applications.
#
# Geschichte: `cp -a Famulus.app /Applications/Famulus.app` nistet sich
# selbst, wenn das Ziel schon existiert (`Famulus.app/Famulus.app`), und
# `cp` in eine bestehende Datei hinein aendert deren Bytes UNTER einem
# laufenden Prozess - beides endet in SIGKILL (Code Signature Invalid),
# weil das Betriebssystem beim Nachladen einer Code-Seite merkt, dass die
# Datei nicht mehr zur Signatur passt, mit der sie gestartet wurde.
#
# Die erste Version dieses Skripts hat das umgangen, indem es die laufende
# Instanz zuerst beendet hat (osascript quit + warten). Das reicht nicht:
# es ist eine Race Condition, kein Ausschluss. Wenn zwischen "beendet" und
# "neues Bundle liegt vollstaendig an seinem Platz" irgendetwas die App neu
# startet - ein Doppelklick, ein `open -a Famulus` aus einem anderen
# Terminal, Famulus selbst in einem parallelen Shell-Aufruf - trifft dieser
# Start auf ein halb kopiertes oder frisch ueberschriebenes Bundle. Genau
# dieses Muster ist wiederholt aufgetreten.
#
# Deshalb jetzt: atomarer Austausch per rename(2) statt Kopieren.
# rename() auf demselben Volume ist eine einzelne, unteilbare
# Kernel-Operation - jeder Prozess, der gerade `open()`/`exec()` auf den
# Pfad macht, sieht entweder die komplette alte oder die komplette neue
# Version, nie etwas dazwischen. Und eine BEREITS laufende Instanz merkt
# vom Austausch ueberhaupt nichts: ihr offenes Binary-Handle haengt am
# Inode, nicht am Pfad - den kann man umbenennen oder loeschen, waehrend
# der Prozess laeuft, ohne dass er davon etwas mitbekommt (Standard-Unix-
# Verhalten). Das macht die Installation sicher *unabhaengig* davon, ob
# vorher sauber beendet wurde - nicht nur "sicher, wenn man sich an die
# Reihenfolge haelt".
#
# rename() kann ein bestehendes Verzeichnis nur ersetzen, wenn das Ziel
# leer ist - ein App-Bundle ist es nicht. Deshalb zwei Rename-Schritte
# statt einem: altes Bundle beiseite schieben, neues an seinen Platz,
# dann das alte aufraeumen. Jeder einzelne Schritt ist fuer sich atomar.

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

echo "==> Installiere atomar nach $DEST..."
rm -rf "$BACKUP"
if [ -d "$DEST" ]; then
    mv "$DEST" "$BACKUP"
fi
mv "$BUNDLE" "$DEST"
rm -rf "$BACKUP"

echo "==> Pruefe installiertes Bundle..."
codesign --verify --deep --strict --verbose=4 "$DEST"

# Ab hier reine Komfort-Sache, nicht mehr sicherheitsrelevant: eine bereits
# laufende (alte) Instanz laeuft dank des atomaren Austauschs oben
# unbeeindruckt weiter. Damit Jens aber nicht zwei Versionen gleichzeitig
# offen hat, ohne es zu merken, hier trotzdem sauber beenden und neu starten.
if pgrep -f "Famulus.app/Contents/MacOS/famulus-gui" > /dev/null 2>&1; then
    echo "==> Beende laufende (alte) Famulus-Instanz..."
    osascript -e 'quit app "Famulus"' 2>/dev/null || true
    for _ in $(seq 1 20); do
        pgrep -f "Famulus.app/Contents/MacOS/famulus-gui" > /dev/null 2>&1 || break
        sleep 0.5
    done
    pkill -f "Famulus.app/Contents/MacOS/famulus-gui" 2>/dev/null || true
fi

echo "==> Starte Famulus..."
open -a Famulus

echo "Fertig."
